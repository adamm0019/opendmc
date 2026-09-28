//! Milestone 0: map what is actually in the install.
//!
//! Writes `inventory.json` (every file, every bundle entry) and `inventory.md`
//! (a summary meant to be committed to `docs/inventory/`). Neither contains
//! game data: only names, sizes, byte prefixes and statistics.

use anyhow::Result;
use dmc_formats::bdp::Bundle;
use dmc_formats::detect::{self, Kind};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

const ENTROPY_SAMPLE: usize = 1 << 20;
/// Only the first bytes of files above this size are classified (videos etc.).
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
    bundle: Option<BundleRecord>,
}

#[derive(Serialize)]
struct BundleRecord {
    name: String,
    version: Option<String>,
    endian: dmc_formats::Endian,
    types: Vec<String>,
    entries: Vec<EntryRecord>,
}

#[derive(Serialize)]
struct EntryRecord {
    path: String,
    size: u32,
    raw_size: u32,
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
        let rel = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let data = if size <= FULL_READ_LIMIT {
            fs::read(entry.path())?
        } else {
            use std::io::Read;
            let mut buf = vec![0; ENTROPY_SAMPLE];
            let n = fs::File::open(entry.path())?.read(&mut buf)?;
            buf.truncate(n);
            buf
        };
        eprintln!("  {rel} ({size} bytes)");
        let kind = if size <= FULL_READ_LIMIT {
            detect::classify(&data)
        } else {
            Kind::Unknown
        };
        let bundle = matches!(kind, Kind::PipeworksBundle { .. })
            .then(|| bundle_record(&data))
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

fn bundle_record(data: &[u8]) -> Option<BundleRecord> {
    let b = Bundle::parse(data).ok()?;
    let entries = b
        .entries
        .iter()
        .map(|e| {
            let payload = b.entry_data(data, e).unwrap_or(&[]);
            EntryRecord {
                path: e.display_path(),
                size: e.stored_size,
                raw_size: e.raw_size,
                compressed: e.is_compressed(),
                head: detect::hex_prefix(payload, 16),
                entropy: detect::entropy(&payload[..payload.len().min(ENTROPY_SAMPLE)]),
                kind: if e.is_compressed() {
                    Kind::Unknown
                } else {
                    detect::classify(payload)
                },
            }
        })
        .collect();
    Some(BundleRecord {
        name: b.name,
        version: b.version,
        endian: b.endian,
        types: b.types.into_iter().map(|t| t.name).collect(),
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

    let bundles: Vec<_> = inv
        .files
        .iter()
        .filter_map(|f| f.bundle.as_ref().map(|b| (f, b)))
        .collect();
    let models: usize = inv
        .files
        .iter()
        .filter(|f| matches!(f.kind, Kind::Model { .. }))
        .count()
        + bundles
            .iter()
            .flat_map(|(_, b)| &b.entries)
            .filter(|e| matches!(e.kind, Kind::Model { .. }))
            .count();
    let _ = writeln!(md, "- Pipeworks bundles: **{}**", bundles.len());
    let _ = writeln!(md, "- Files parsed as DMC1 models: **{models}**\n");

    md.push_str("## Phase 1 gating questions\n\n");
    let endians: BTreeMap<String, usize> = bundles.iter().fold(BTreeMap::new(), |mut m, (_, b)| {
        *m.entry(format!("{:?}", b.endian)).or_default() += 1;
        m
    });
    let _ = writeln!(
        md,
        "1. **Container:** {}",
        if bundles.is_empty() {
            "no Pipeworks bundles found; data is loose files or another container (see tables below).".to_string()
        } else {
            format!(
                "Pipeworks bundles present: {}",
                bundles
                    .iter()
                    .map(|(f, _)| format!("`{}`", f.path))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );
    let _ = writeln!(
        md,
        "2. **Byte order:** bundles {endians:?}; model files {:?}",
        model_endians(inv)
    );
    let _ = writeln!(
        md,
        "3. **Layouts match the PS3 notes?** {models} model files validated structurally (vertex totals + array packing)."
    );
    md.push_str(
        "4. **Collision / cameras / scripts / audio:** see the unknown-file tables below.\n\n",
    );

    md.push_str("## Loose files by extension\n\n");
    tally_table(
        &mut md,
        &tally(inv.files.iter().map(|f| (f.path.as_str(), f.size, &f.kind))),
    );

    for (f, b) in &bundles {
        let _ = writeln!(
            md,
            "\n## Bundle `{}`\n\nName `{}`, version {}, {:?}, {} entries ({} compressed). Types: {}.\n",
            f.path,
            b.name,
            b.version.as_deref().unwrap_or("?"),
            b.endian,
            b.entries.len(),
            b.entries.iter().filter(|e| e.compressed).count(),
            b.types.join(", ")
        );
        tally_table(
            &mut md,
            &tally(
                b.entries
                    .iter()
                    .map(|e| (e.path.as_str(), e.size as u64, &e.kind)),
            ),
        );
    }

    md.push_str(
        "\n## Unknown files (first 40)\n\n| path | size | entropy | head |\n|---|---:|---:|---|\n",
    );
    let unknown_loose = inv
        .files
        .iter()
        .filter(|f| f.kind == Kind::Unknown)
        .map(|f| (f.path.clone(), f.size, f.entropy, f.head.clone()));
    let unknown_entries = bundles.iter().flat_map(|(f, b)| {
        b.entries
            .iter()
            .filter(|e| e.kind == Kind::Unknown)
            .map(move |e| {
                (
                    format!("{}::{}", f.path, e.path),
                    e.size as u64,
                    e.entropy,
                    e.head.clone(),
                )
            })
    });
    for (path, size, entropy, head) in unknown_loose.chain(unknown_entries).take(40) {
        let _ = writeln!(md, "| `{path}` | {size} | {entropy:.2} | `{head}` |");
    }
    md
}

fn model_endians(inv: &Inventory) -> BTreeMap<String, usize> {
    let loose = inv.files.iter().map(|f| &f.kind);
    let nested = inv
        .files
        .iter()
        .filter_map(|f| f.bundle.as_ref())
        .flat_map(|b| b.entries.iter().map(|e| &e.kind));
    loose.chain(nested).fold(BTreeMap::new(), |mut m, k| {
        if let Kind::Model { endian, .. } = k {
            *m.entry(format!("{endian:?}")).or_default() += 1;
        }
        m
    })
}
