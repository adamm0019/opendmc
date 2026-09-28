//! Milestone 0: map what is actually in the install.
//!
//! Writes `inventory.json` (every file, every container entry) and
//! `inventory.md` (a summary meant to be committed to `docs/inventory/`).
//! Neither contains game data: only names, sizes, byte prefixes and statistics.

use crate::archive::{self, Source};
use anyhow::Result;
use dmc_formats::bdp::Bundle;
use dmc_formats::detect::{self, Kind};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::Path;
use walkdir::WalkDir;

const ENTROPY_SAMPLE: usize = 1 << 20;
/// Files above this size are fingerprinted from their first bytes only
/// (videos, the big `.nbz` archives). Archives are still listed entry by entry.
const FULL_READ_LIMIT: u64 = 1 << 30;

#[derive(Serialize)]
struct FileRecord {
    path: String,
    size: u64,
    ext: String,
    head: String,
    entropy: f32,
    endian_guess: Option<dmc_formats::Endian>,
    #[serde(flatten)]
    kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    bundle: Option<ContainerRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    archive: Option<ContainerRecord>,
}

#[derive(Serialize)]
struct ContainerRecord {
    format: &'static str,
    name: String,
    version: Option<String>,
    endian: Option<dmc_formats::Endian>,
    types: Vec<String>,
    /// ZIP only: see [`archive::Fingerprint`].
    #[serde(skip_serializing_if = "Option::is_none")]
    fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    newest_entry: Option<String>,
    entries: Vec<EntryRecord>,
}

#[derive(Serialize)]
struct EntryRecord {
    path: String,
    size: u64,
    stored_size: u64,
    compressed: bool,
    head: String,
    entropy: f32,
    #[serde(flatten)]
    kind: Kind,
}

#[derive(Serialize)]
struct Inventory {
    tool: &'static str,
    root: String,
    files: Vec<FileRecord>,
}

impl FileRecord {
    fn container(&self) -> Option<&ContainerRecord> {
        self.bundle.as_ref().or(self.archive.as_ref())
    }
}

impl Inventory {
    fn containers(&self) -> impl Iterator<Item = (&FileRecord, &ContainerRecord)> {
        self.files
            .iter()
            .filter_map(|f| f.container().map(|c| (f, c)))
    }

    /// Every classified thing: loose files and container entries.
    fn kinds(&self) -> impl Iterator<Item = &Kind> {
        self.files.iter().map(|f| &f.kind).chain(
            self.containers()
                .flat_map(|(_, c)| c.entries.iter().map(|e| &e.kind)),
        )
    }
}

fn ext_of(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

fn kind_label(k: &Kind) -> String {
    match k {
        Kind::PipeworksBundle { endian } => format!("bundle ({endian:?})"),
        Kind::TextureSet { endian, .. } => format!("texture set ({endian:?})"),
        Kind::Model { endian, layout, .. } => format!("model ({endian:?}, {layout})"),
        Kind::Known { name } => name.to_string(),
        Kind::Unknown => "unknown".into(),
    }
}

fn read_head(path: &Path, n: usize) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(n);
    fs::File::open(path)?.take(n as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

pub fn run(root: &Path, out: &Path) -> Result<()> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = archive::rel_name(root, entry.path());
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let data = if size <= FULL_READ_LIMIT {
            fs::read(entry.path())?
        } else {
            read_head(entry.path(), ENTROPY_SAMPLE)?
        };
        eprintln!("  {rel} ({size} bytes)");
        // For a huge file this classifies its first MiB, which is enough for
        // every magic-based format.
        let kind = detect::classify(&data);
        let bundle = matches!(kind, Kind::PipeworksBundle { .. })
            .then(|| bundle_record(&data))
            .flatten();
        let archive = data
            .starts_with(archive::ZIP_MAGIC)
            .then(|| zip_record(entry.path()))
            .transpose()?
            .flatten();
        files.push(FileRecord {
            ext: ext_of(&rel),
            path: rel,
            size,
            head: detect::hex_prefix(&data, 16),
            entropy: detect::entropy(&data[..data.len().min(ENTROPY_SAMPLE)]),
            endian_guess: detect::guess_endian(&data),
            kind,
            bundle,
            archive,
        });
    }

    let inv = Inventory {
        tool: concat!("dmc ", env!("CARGO_PKG_VERSION")),
        root: root.display().to_string(),
        files,
    };
    fs::create_dir_all(out)?;
    fs::write(out.join("inventory.json"), serde_json::to_vec_pretty(&inv)?)?;
    fs::write(out.join("inventory.md"), markdown(&inv))?;
    println!(
        "wrote {} and {}",
        out.join("inventory.json").display(),
        out.join("inventory.md").display()
    );
    Ok(())
}

fn entry_record(
    path: String,
    size: u64,
    stored_size: u64,
    compressed: bool,
    data: &[u8],
) -> EntryRecord {
    EntryRecord {
        path,
        size,
        stored_size,
        compressed,
        head: detect::hex_prefix(data, 16),
        entropy: detect::entropy(&data[..data.len().min(ENTROPY_SAMPLE)]),
        kind: detect::classify(data),
    }
}

fn zip_record(path: &Path) -> Result<Option<ContainerRecord>> {
    let Ok(mut src) = Source::open(path) else {
        return Ok(None);
    };
    let listing = src.entries()?;
    let fp = archive::Fingerprint::of(&listing);
    let mut entries = Vec::new();
    for e in listing {
        let data = src.read(&e.name)?;
        entries.push(entry_record(
            e.name,
            e.size,
            e.compressed_size,
            e.compressed,
            &data,
        ));
    }
    Ok(Some(ContainerRecord {
        format: "ZIP (.nbz)",
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        version: None,
        endian: None,
        types: Vec::new(),
        fingerprint: Some(fp.hash),
        newest_entry: fp.newest,
        entries,
    }))
}

fn bundle_record(data: &[u8]) -> Option<ContainerRecord> {
    let b = Bundle::parse(data).ok()?;
    let entries = b
        .entries
        .iter()
        .map(|e| {
            let payload = b.entry_data(data, e).unwrap_or(&[]);
            let mut r = entry_record(
                e.display_path(),
                e.raw_size as u64,
                e.stored_size as u64,
                e.is_compressed(),
                payload,
            );
            if e.is_compressed() {
                r.kind = Kind::Unknown;
            }
            r
        })
        .collect();
    Some(ContainerRecord {
        format: "Pipeworks bundle",
        name: b.name,
        version: b.version,
        endian: Some(b.endian),
        types: b.types.into_iter().map(|t| t.name).collect(),
        fingerprint: None,
        newest_entry: None,
        entries,
    })
}

#[derive(Default)]
struct Tally {
    count: usize,
    bytes: u64,
    kinds: BTreeMap<String, usize>,
    example: String,
}

fn tally<'a>(items: impl Iterator<Item = (&'a str, u64, &'a Kind)>) -> BTreeMap<String, Tally> {
    let mut by_ext: BTreeMap<String, Tally> = BTreeMap::new();
    for (path, size, kind) in items {
        let t = by_ext.entry(ext_of(path)).or_default();
        t.count += 1;
        t.bytes += size;
        *t.kinds.entry(kind_label(kind)).or_default() += 1;
        if t.example.is_empty() {
            t.example = path.to_string();
        }
    }
    by_ext
}

fn tally_table(md: &mut String, by_ext: &BTreeMap<String, Tally>) {
    md.push_str("| ext | files | MiB | classified as | example |\n|---|---:|---:|---|---|\n");
    for (ext, t) in by_ext {
        let kinds = t
            .kinds
            .iter()
            .map(|(k, n)| format!("{k} ×{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            md,
            "| `{}` | {} | {:.1} | {} | `{}` |",
            if ext.is_empty() { "—" } else { ext },
            t.count,
            t.bytes as f64 / 1048576.0,
            kinds,
            t.example
        );
    }
}

fn count_by(items: impl Iterator<Item = String>) -> BTreeMap<String, usize> {
    items.fold(BTreeMap::new(), |mut m, k| {
        *m.entry(k).or_default() += 1;
        m
    })
}

fn list(m: &BTreeMap<String, usize>) -> String {
    if m.is_empty() {
        return "none".into();
    }
    m.iter()
        .map(|(k, n)| format!("{k} ×{n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn markdown(inv: &Inventory) -> String {
    let mut md = String::new();
    let total: u64 = inv.files.iter().map(|f| f.size).sum();
    let _ = writeln!(
        md,
        "# Install inventory\n\nGenerated by `{}`. File names, sizes and statistics only; no game data.\n",
        inv.tool
    );
    let _ = writeln!(
        md,
        "- Files: **{}**, total **{:.1} MiB**",
        inv.files.len(),
        total as f64 / 1048576.0
    );
    let bundles = inv.files.iter().filter(|f| f.bundle.is_some()).count();
    let archives: Vec<_> = inv
        .files
        .iter()
        .filter_map(|f| f.archive.as_ref())
        .collect();
    let models = inv
        .kinds()
        .filter(|k| matches!(k, Kind::Model { .. }))
        .count();
    let textures = inv
        .kinds()
        .filter(|k| matches!(k, Kind::TextureSet { .. }))
        .count();
    let _ = writeln!(md, "- Pipeworks bundles: **{bundles}**");
    let _ = writeln!(
        md,
        "- ZIP archives (`.nbz`): **{}** holding **{}** entries",
        archives.len(),
        archives.iter().map(|a| a.entries.len()).sum::<usize>()
    );
    let _ = writeln!(md, "- Files parsed as DMC1 models: **{models}**");
    let _ = writeln!(md, "- Files parsed as texture containers: **{textures}**\n");

    md.push_str("## Phase 1 gating questions\n\n");
    let containers = inv
        .containers()
        .map(|(f, c)| format!("`{}` ({}, {} entries)", f.path, c.format, c.entries.len()))
        .collect::<Vec<_>>();
    let _ = writeln!(
        md,
        "1. **Container:** {}",
        if containers.is_empty() {
            "no containers found; data is loose files (see tables below).".to_string()
        } else {
            containers.join(", ")
        }
    );
    let model_endians = count_by(inv.kinds().filter_map(|k| match k {
        Kind::Model { endian, .. } => Some(format!("{endian:?}")),
        _ => None,
    }));
    let texture_endians = count_by(inv.kinds().filter_map(|k| match k {
        Kind::TextureSet { endian, .. } => Some(format!("{endian:?}")),
        _ => None,
    }));
    let _ = writeln!(
        md,
        "2. **Byte order:** models: {}; texture containers: {}",
        list(&model_endians),
        list(&texture_endians)
    );
    let layouts = count_by(inv.kinds().filter_map(|k| match k {
        Kind::Model { layout, .. } => Some(layout.clone()),
        _ => None,
    }));
    let _ = writeln!(
        md,
        "3. **Layouts match the PS3 notes?** {models} model files validated structurally \
         (vertex totals, bounds, skeleton bone counts). Section-table layouts: {}.",
        list(&layouts)
    );
    md.push_str(
        "4. **Collision / cameras / scripts / audio:** see the per-container tables and unknown files below.\n\n",
    );

    md.push_str("## Loose files by extension\n\n");
    tally_table(
        &mut md,
        &tally(inv.files.iter().map(|f| (f.path.as_str(), f.size, &f.kind))),
    );

    for (f, c) in inv.containers() {
        let _ = writeln!(
            md,
            "\n## {} `{}`\n\n{} entries ({} compressed){}{}.\n",
            c.format,
            f.path,
            c.entries.len(),
            c.entries.iter().filter(|e| e.compressed).count(),
            c.version
                .as_deref()
                .map(|v| format!(", version {v}"))
                .unwrap_or_default(),
            c.endian.map(|e| format!(", {e:?}")).unwrap_or_default(),
        );
        if let Some(fp) = &c.fingerprint {
            let _ = writeln!(
                md,
                "Data build: fingerprint `{fp}`, newest entry {}.\n",
                c.newest_entry.as_deref().unwrap_or("unknown")
            );
        }
        tally_table(
            &mut md,
            &tally(c.entries.iter().map(|e| (e.path.as_str(), e.size, &e.kind))),
        );
    }

    md.push_str(
        "\n## Unknown files\n\nUp to three examples per container and extension.\n\n\
         | path | size | entropy | head |\n|---|---:|---:|---|\n",
    );
    let unknown_loose = inv
        .files
        .iter()
        .filter(|f| f.kind == Kind::Unknown)
        .map(|f| ("", f.path.clone(), f.size, f.entropy, f.head.clone()));
    let unknown_entries = inv.containers().flat_map(|(f, c)| {
        c.entries
            .iter()
            .filter(|e| e.kind == Kind::Unknown)
            .map(move |e| {
                (
                    f.path.as_str(),
                    format!("{}::{}", f.path, e.path),
                    e.size,
                    e.entropy,
                    e.head.clone(),
                )
            })
    });
    let mut shown: BTreeMap<(&str, String), usize> = BTreeMap::new();
    for (container, path, size, entropy, head) in unknown_loose.chain(unknown_entries) {
        let n = shown.entry((container, ext_of(&path))).or_default();
        *n += 1;
        if *n <= 3 {
            let _ = writeln!(md, "| `{path}` | {size} | {entropy:.2} | `{head}` |");
        }
    }
    md
}
