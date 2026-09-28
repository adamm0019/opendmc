//! Where game files come from: a directory of loose files, or one of the HD
//! Collection's `.nbz` archives (plain ZIP files; `docs/formats/README.md`
//! §0). Entries are read into memory; nothing is written to disk here.

use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::ZipArchive;

pub const ZIP_MAGIC: &[u8] = b"PK\x03\x04";
/// Separates an archive path from an entry name: `dmc1-0.nbz::Pld/pl00.pld`.
pub const ENTRY_SEPARATOR: &str = "::";

/// One entry of an archive listing.
#[derive(Debug, Clone)]
pub struct EntryInfo {
    pub name: String,
    pub size: u64,
    pub compressed_size: u64,
    pub compressed: bool,
    /// CRC-32 of the contents as recorded in the archive (0 for loose files).
    pub crc32: u32,
    /// Modification date recorded in the archive, `YYYY-MM-DD`.
    pub modified: Option<String>,
}

/// Identifies one data build: a hash over every entry's name, size and
/// recorded CRC-32, plus the newest entry date. Two installs with the same
/// fingerprint hold byte-identical archives. Computed from the archive's
/// directory alone, so it costs nothing to decompress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub hash: String,
    pub newest: Option<String>,
}

impl Fingerprint {
    pub fn of(entries: &[EntryInfo]) -> Self {
        // FNV-1a, 64-bit: stable and dependency-free; this is an identity
        // check, not a security boundary.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |bytes: &[u8]| {
            for &b in bytes {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        let mut sorted: Vec<_> = entries.iter().collect();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        for e in sorted {
            eat(e.name.as_bytes());
            eat(&[0]);
            eat(&e.size.to_le_bytes());
            eat(&e.crc32.to_le_bytes());
        }
        Fingerprint {
            hash: format!("{:08x}", (h >> 32) as u32 ^ h as u32),
            newest: entries.iter().filter_map(|e| e.modified.clone()).max(),
        }
    }
}

pub enum Source {
    Dir(PathBuf),
    Zip(ZipArchive<BufReader<File>>),
}

impl Source {
    pub fn open(path: &Path) -> Result<Self> {
        if path.is_dir() {
            return Ok(Source::Dir(path.to_path_buf()));
        }
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let zip = ZipArchive::new(BufReader::new(file))
            .with_context(|| format!("{} is not a ZIP/.nbz archive", path.display()))?;
        Ok(Source::Zip(zip))
    }

    /// Every file (not directory), `/`-separated, in name order.
    pub fn entries(&mut self) -> Result<Vec<EntryInfo>> {
        let mut out = Vec::new();
        match self {
            Source::Dir(root) => {
                for e in WalkDir::new(&*root)
                    .sort_by_file_name()
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_file())
                {
                    let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                    out.push(EntryInfo {
                        name: rel_name(root, e.path()),
                        size,
                        compressed_size: size,
                        compressed: false,
                        crc32: 0,
                        modified: None,
                    });
                }
            }
            Source::Zip(zip) => {
                for i in 0..zip.len() {
                    let f = zip.by_index_raw(i)?;
                    if f.is_dir() {
                        continue;
                    }
                    out.push(EntryInfo {
                        name: f.name().replace('\\', "/"),
                        size: f.size(),
                        compressed_size: f.compressed_size(),
                        compressed: f.compression() != zip::CompressionMethod::Stored,
                        crc32: f.crc32(),
                        modified: f
                            .last_modified()
                            .map(|d| format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())),
                    });
                }
                out.sort_by(|a, b| a.name.cmp(&b.name));
            }
        }
        Ok(out)
    }

    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        match self {
            Source::Dir(root) => crate::read(&root.join(name)),
            Source::Zip(zip) => {
                let mut f = zip
                    .by_name(name)
                    .with_context(|| format!("no entry {name}"))?;
                let mut buf = Vec::with_capacity(f.size() as usize);
                f.read_to_end(&mut buf)?;
                Ok(buf)
            }
        }
    }
}

pub fn rel_name(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Read a file named either by path or as `archive.nbz::entry/name`.
pub fn read_spec(spec: &Path) -> Result<Vec<u8>> {
    let text = spec.to_string_lossy();
    match text.split_once(ENTRY_SEPARATOR) {
        Some((archive, entry)) => Source::open(Path::new(archive))?.read(entry),
        None => fs::read(spec).with_context(|| format!("reading {}", spec.display())),
    }
}

/// The file stem of a path or of the entry part of an `archive::entry` spec.
pub fn spec_stem(spec: &Path) -> String {
    let text = spec.to_string_lossy();
    let name = text
        .rsplit_once(ENTRY_SEPARATOR)
        .map_or(&*text, |(_, e)| e)
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("file");
    name.rsplit_once('.')
        .map_or(name, |(stem, _)| stem)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    pub fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(File::create(path).unwrap());
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        z.add_directory("Pld/", opts).unwrap();
        for (name, data) in files {
            z.start_file(*name, opts).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn reads_entries_from_an_archive_and_by_spec() {
        let dir = std::env::temp_dir().join(format!("dmc-archive-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let nbz = dir.join("x-0.nbz");
        write_zip(&nbz, &[("Pld/b.pld", &[2; 100]), ("Emd/a.emd", b"hello")]);

        let mut src = Source::open(&nbz).unwrap();
        let names: Vec<_> = src.entries().unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["Emd/a.emd", "Pld/b.pld"]);
        assert_eq!(src.read("Pld/b.pld").unwrap(), vec![2; 100]);

        let entries = src.entries().unwrap();
        let fp = Fingerprint::of(&entries);
        assert_eq!(fp.hash.len(), 8);
        assert!(fp.newest.is_some());
        let mut reordered = entries.clone();
        reordered.reverse();
        assert_eq!(Fingerprint::of(&reordered), fp, "order-independent");
        reordered[0].crc32 ^= 1;
        assert_ne!(Fingerprint::of(&reordered).hash, fp.hash);

        let spec = PathBuf::from(format!("{}::Emd/a.emd", nbz.display()));
        assert_eq!(read_spec(&spec).unwrap(), b"hello");
        assert_eq!(spec_stem(&spec), "a");
        let _ = fs::remove_dir_all(dir);
    }
}
