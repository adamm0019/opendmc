//! `dmc`: tooling that runs against *your own* install. Nothing it produces
//! may be committed except `inventory.md` (see DECISIONS.md).

mod archive;
mod export;
mod gltf;
mod inventory;
mod wildcard;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dmc_formats::bdp::Bundle;
use dmc_formats::model::ModelFile;
use dmc_formats::motion::MotionBank;
use dmc_formats::texture;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "dmc",
    version,
    about = "OpenDMC asset tooling (bring your own game files)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Fingerprint every file under an install directory and write
    /// `inventory.json` + `inventory.md`.
    Inventory {
        install_dir: PathBuf,
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
    /// List the entries of a Pipeworks bundle.
    BundleList {
        bundle: PathBuf,
        /// Only entries whose file name matches (`*` and `?` wildcards).
        #[arg(long)]
        pattern: Option<String>,
    },
    /// Extract entries of a Pipeworks bundle.
    BundleExtract {
        bundle: PathBuf,
        out: PathBuf,
        #[arg(long)]
        pattern: Option<String>,
    },
    /// Describe one file: sections, geometry, textures, motion banks.
    /// Files inside an archive are named `archive.nbz::Dir/name.ext`.
    Info { file: PathBuf },
    /// Decode every embedded texture container to PNG.
    Tex { file: PathBuf, out: PathBuf },
    /// Convert a model file to glTF binary (.glb), textured and skinned.
    Model { file: PathBuf, out: PathBuf },
    /// Batch-convert every model under a directory or inside an `.nbz`
    /// archive, with validation stats.
    Export {
        /// A directory or an `.nbz` archive.
        dir: PathBuf,
        out: PathBuf,
        #[arg(long)]
        pattern: Option<String>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Inventory { install_dir, out } => inventory::run(&install_dir, &out),
        Cmd::BundleList { bundle, pattern } => bundle_list(&bundle, pattern.as_deref()),
        Cmd::BundleExtract {
            bundle,
            out,
            pattern,
        } => bundle_extract(&bundle, &out, pattern.as_deref()),
        Cmd::Info { file } => info(&file),
        Cmd::Tex { file, out } => {
            let n = export::textures_to_png(
                &archive::read_spec(&file)?,
                &out,
                &archive::spec_stem(&file),
            )?;
            println!("{n} images -> {}", out.display());
            Ok(())
        }
        Cmd::Model { file, out } => {
            let stats = export::model_to_glb(&archive::read_spec(&file)?, &out)?;
            println!("{}  {stats}", out.display());
            Ok(())
        }
        Cmd::Export { dir, out, pattern } => export::batch(&dir, &out, pattern.as_deref()),
    }
}

pub fn read(p: &Path) -> Result<Vec<u8>> {
    fs::read(p).with_context(|| format!("reading {}", p.display()))
}

fn matches(pattern: Option<&str>, path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    pattern.is_none_or(|p| wildcard::matches(&p.to_ascii_lowercase(), &name.to_ascii_lowercase()))
}

fn bundle_list(path: &Path, pattern: Option<&str>) -> Result<()> {
    let data = read(path)?;
    let b = Bundle::parse(&data)?;
    println!(
        "{}  '{}'  v{}  {:?}  {} entries",
        path.display(),
        b.name,
        b.version.as_deref().unwrap_or("?"),
        b.endian,
        b.entries.len()
    );
    for e in b
        .entries
        .iter()
        .filter(|e| matches(pattern, &e.display_path()))
    {
        let head = b
            .entry_data(&data, e)
            .map(|d| dmc_formats::detect::hex_prefix(d, 4))
            .unwrap_or_default();
        println!(
            "  [{:5}] {:<48} off={:>10} size={:>9} raw={:>9} f={:02x}/{:02x} {}",
            e.index,
            e.display_path(),
            e.offset,
            e.stored_size,
            e.raw_size,
            e.flags1,
            e.flags2,
            head
        );
    }
    Ok(())
}

fn bundle_extract(path: &Path, out: &Path, pattern: Option<&str>) -> Result<()> {
    let data = read(path)?;
    let b = Bundle::parse(&data)?;
    let (mut n, mut compressed) = (0, 0);
    for e in b
        .entries
        .iter()
        .filter(|e| matches(pattern, &e.display_path()))
    {
        let mut dest = out.join(e.display_path());
        // Several entries can share a logical name (chunked payloads).
        if dest.exists() {
            let ext = dest
                .extension()
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or_default();
            dest.set_extension(format!("{}.{}", e.index, ext));
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&dest, b.entry_data(&data, e)?)?;
        n += 1;
        compressed += e.is_compressed() as usize;
    }
    println!(
        "{n} entries -> {} ({compressed} stored compressed, written as-is)",
        out.display()
    );
    Ok(())
}

fn info(path: &Path) -> Result<()> {
    let data = archive::read_spec(path)?;
    println!(
        "{}  {} bytes  entropy {:.2}",
        path.display(),
        data.len(),
        dmc_formats::detect::entropy(&data)
    );
    println!("kind: {:?}", dmc_formats::detect::classify(&data));
    if let Ok((m, g)) = ModelFile::detect(&data) {
        println!("model: {:?} {:?}", m.endian, m.layout);
        for s in &m.sections {
            match s.offset {
                Some(o) => println!("  section {:2}: 0x{o:08x} len {}", s.index, s.len),
                None => println!("  section {:2}: empty", s.index),
            }
        }
        println!(
            "geometry: {} objects, {} meshes, {} vertices, {} bones, {} tex slots",
            g.objects.len(),
            g.mesh_count(),
            g.vertex_count(),
            g.bone_count,
            g.tex_count
        );
        for (i, sec) in m.sections.iter().enumerate().skip(1) {
            let Some(bytes) = m.section(&data, i) else {
                continue;
            };
            if let Ok(bank) = MotionBank::parse(bytes, m.endian) {
                let frames: Vec<u16> = bank.motions.iter().flatten().map(|mo| mo.frames).collect();
                if !frames.is_empty() {
                    println!(
                        "  section {:2}: motion bank, {} motions, frames {:?}{}",
                        sec.index,
                        frames.len(),
                        &frames[..frames.len().min(12)],
                        if frames.len() > 12 { " …" } else { "" }
                    );
                }
            }
        }
    }
    for (off, set) in texture::scan(&data) {
        println!(
            "textures @0x{off:x}: {:?} {:?}, {} images",
            set.kind,
            set.endian,
            set.images.len()
        );
        for img in &set.images {
            println!(
                "    #{:<3} {:>4}x{:<4} {:?}{}",
                img.index,
                img.width,
                img.height,
                img.format,
                if img.has_mips { " +mips" } else { "" }
            );
        }
    }
    Ok(())
}
