//! Pipeworks bundle (`.BDP`), the HD Collection's archive.
//! Layout: `docs/formats/README.md` §1.

use crate::bytes::{Endian, Reader, Writer};
use crate::error::{FormatError, Result};
use serde::Serialize;
use std::collections::HashMap;

pub const BANNER_PREFIX: &[u8] = b"Pipeworks bundle";
const BANNER_LEN: usize = 0x24;
const NAME_OFF: usize = 0x26;
const ENTRY_SIZE: usize = 16;
const NAME_RECORD_SIZE: usize = 12;

#[derive(Debug, Clone, Serialize)]
pub struct BundleType {
    pub flags: u8,
    pub unknown: u8,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub index: usize,
    /// Absolute offset of the payload within the bundle file.
    pub offset: u32,
    pub stored_size: u32,
    pub raw_size: u32,
    pub flags1: u8,
    pub flags2: u8,
    pub hash: u32,
    /// `dir/name.ext` from the name table, with `/` separators.
    pub path: Option<String>,
}

impl Entry {
    pub fn is_compressed(&self) -> bool {
        self.stored_size != self.raw_size
    }

    pub fn display_path(&self) -> String {
        self.path
            .clone()
            .unwrap_or_else(|| format!("_unnamed/{:08x}", self.hash))
    }

    pub fn extension(&self) -> Option<String> {
        let p = self.path.as_deref()?;
        let file = p.rsplit('/').next()?;
        file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Bundle {
    pub endian: Endian,
    pub banner: String,
    pub version: Option<String>,
    pub name: String,
    pub declared_size: u32,
    pub types: Vec<BundleType>,
    pub entries: Vec<Entry>,
}

pub fn is_bundle(data: &[u8]) -> bool {
    data.starts_with(BANNER_PREFIX)
}

/// Byte order from the banner text, falling back to the declared file size.
fn detect_endian(data: &[u8]) -> Result<Endian> {
    let banner = String::from_utf8_lossy(&data[..BANNER_LEN.min(data.len())]).to_ascii_lowercase();
    if banner.contains("little endian") {
        return Ok(Endian::Little);
    }
    if banner.contains("big endian") {
        return Ok(Endian::Big);
    }
    for e in [Endian::Big, Endian::Little] {
        if Reader::new(data, e).u32(0x4C)? as usize == data.len() {
            return Ok(e);
        }
    }
    Err(FormatError::invalid(
        "bundle",
        "cannot determine byte order",
    ))
}

impl Bundle {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if !is_bundle(data) {
            return Err(FormatError::BadMagic("expected 'Pipeworks bundle'".into()));
        }
        let endian = detect_endian(data)?;
        let r = Reader::new(data, endian);

        let banner_bytes = r.bytes(0, BANNER_LEN)?;
        let banner = String::from_utf8_lossy(banner_bytes).trim_end().to_string();
        let version = banner
            .split_whitespace()
            .find(|w| w.starts_with('v') && w[1..].contains('.'))
            .map(|w| w[1..].to_string());
        let name_raw = r.bytes(NAME_OFF, 0x4C - NAME_OFF)?;
        let name_end = name_raw
            .iter()
            .position(|&b| b == 0 || b == 0xEB)
            .unwrap_or(name_raw.len());
        let name = String::from_utf8_lossy(&name_raw[..name_end]).into_owned();

        let declared_size = r.u32(0x4C)?;
        let type_count = r.u32(0x50)? as usize;
        let type_off = r.u32(0x54)? as usize;
        let type_size = r.u32(0x58)? as usize;
        let entry_count = r.u32(0x5C)? as usize;
        let entry_off = r.u32(0x60)? as usize;

        if type_off < 0x7C {
            return Err(FormatError::invalid(
                "bundle",
                format!("type table at 0x{type_off:x} overlaps header"),
            ));
        }
        let name_tbl_off = r.u32(type_off - 8)? as usize;
        let name_tbl_size = r.u32(type_off - 4)? as usize;

        let types = parse_types(&r.sub(type_off, type_size)?, type_count);

        if entry_count.saturating_mul(ENTRY_SIZE) > data.len() {
            return Err(FormatError::invalid(
                "bundle",
                format!("{entry_count} entries cannot fit"),
            ));
        }
        let names = if name_tbl_size > 0 {
            parse_names(&r.sub(name_tbl_off, name_tbl_size)?)?
        } else {
            HashMap::new()
        };

        let mut entries = Vec::with_capacity(entry_count);
        for index in 0..entry_count {
            let o = entry_off + index * ENTRY_SIZE;
            let a = r.u32(o + 4)?;
            let b = r.u32(o + 8)?;
            let hash = r.u32(o + 12)?;
            entries.push(Entry {
                index,
                offset: r.u32(o)?,
                stored_size: a & 0x00FF_FFFF,
                flags1: (a >> 24) as u8,
                raw_size: b & 0x00FF_FFFF,
                flags2: (b >> 24) as u8,
                hash,
                path: names.get(&hash).cloned(),
            });
        }

        Ok(Bundle {
            endian,
            banner,
            version,
            name,
            declared_size,
            types,
            entries,
        })
    }

    /// The stored bytes of an entry (compressed entries are returned as stored).
    pub fn entry_data<'a>(&self, data: &'a [u8], entry: &Entry) -> Result<&'a [u8]> {
        Reader::new(data, self.endian).bytes(entry.offset as usize, entry.stored_size as usize)
    }
}

fn parse_types(r: &Reader, count: usize) -> Vec<BundleType> {
    let d = r.data();
    let mut out = Vec::new();
    let mut o = 0;
    while out.len() < count && o + 2 < d.len() {
        let (flags, unknown) = (d[o], d[o + 1]);
        let Some(len) = d[o + 2..].iter().position(|&b| b == 0) else {
            break;
        };
        let name = String::from_utf8_lossy(&d[o + 2..o + 2 + len]).into_owned();
        out.push(BundleType {
            flags,
            unknown,
            name,
        });
        o += 2 + len + 1;
        while o < d.len() && d[o] == 0xFF {
            o += 1;
        }
    }
    out
}

fn parse_names(r: &Reader) -> Result<HashMap<u32, String>> {
    let rec_count = r.u32(0x20)? as usize;
    let str_count = r.u32(0x24)? as usize;
    let rec_off = 0x28usize;
    let str_tbl = rec_off
        .checked_add(rec_count.saturating_mul(NAME_RECORD_SIZE + 4))
        .ok_or_else(|| FormatError::invalid("name table", "record count overflow"))?;
    let blob = str_tbl + str_count.saturating_mul(4);
    if blob > r.len() {
        return Err(FormatError::invalid(
            "name table",
            "tables exceed table size",
        ));
    }
    let strings = (0..str_count)
        .map(|i| r.cstr(blob + r.u32(str_tbl + i * 4)? as usize))
        .collect::<Result<Vec<_>>>()?;
    let get = |i: u16| {
        strings
            .get(i as usize)
            .cloned()
            .unwrap_or_else(|| format!("?{i}"))
    };

    let mut out = HashMap::with_capacity(rec_count);
    for i in 0..rec_count {
        let o = rec_off + i * NAME_RECORD_SIZE;
        let hash = r.u32(o)?;
        let (dir, name, ext) = (r.u16(o + 4)?, r.u16(o + 6)?, r.u16(o + 8)?);
        let dir = get(dir).replace('\\', "/");
        let mut path = if dir.is_empty() {
            get(name)
        } else {
            format!("{dir}/{}", get(name))
        };
        if ext != 0 {
            path.push('.');
            path.push_str(&get(ext));
        }
        out.insert(hash, path);
    }
    Ok(out)
}

/// FNV-1a, used by [`BundleBuilder`] to key entries. The real bundles' hash
/// function is not known; the reader never recomputes it.
pub fn fnv1a(s: &str) -> u32 {
    s.bytes()
        .fold(0x811C_9DC5, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

/// Builds a bundle in the documented layout. Used for fixtures and mod tooling.
pub struct BundleBuilder {
    pub endian: Endian,
    pub name: String,
    pub files: Vec<(String, Vec<u8>)>,
}

impl BundleBuilder {
    pub fn new(endian: Endian, name: &str) -> Self {
        Self {
            endian,
            name: name.into(),
            files: Vec::new(),
        }
    }

    pub fn file(mut self, path: &str, data: impl Into<Vec<u8>>) -> Self {
        self.files.push((path.into(), data.into()));
        self
    }

    pub fn build(&self) -> Vec<u8> {
        let e = self.endian;
        let mut w = Writer::new(e);
        let order = match e {
            Endian::Big => "big",
            Endian::Little => "little",
        };
        let mut banner = format!("Pipeworks bundle v1.30 ({order} endian)").into_bytes();
        banner.resize(BANNER_LEN, b' ');
        w.bytes(&banner)
            .bytes(&[0x1A, 0x00])
            .bytes(self.name.as_bytes());
        while w.pos() < 0x4C {
            w.u8(0xEB);
        }

        // Header words 0x4C..0x80, patched once the layout is known.
        let header_words = w.pos();
        w.zeros(0x80 - 0x4C);

        let type_off = w.pos();
        w.u8(0).u8(0).bytes(b"dat\0");
        let type_size = w.pos() - type_off;
        w.pad_to(16, 0xFF);

        let entry_off = w.pos();
        w.zeros(self.files.len() * ENTRY_SIZE);

        // Strings: index 0 is "", so ext 0 / dir "" mean "none".
        let mut strings: Vec<String> = vec![String::new()];
        let mut intern = |s: &str| -> u16 {
            if let Some(i) = strings.iter().position(|x| x == s) {
                return i as u16;
            }
            strings.push(s.to_string());
            (strings.len() - 1) as u16
        };
        let records: Vec<(u32, u16, u16, u16)> = self
            .files
            .iter()
            .map(|(p, _)| {
                let (dir, file) = p.rsplit_once('/').unwrap_or(("", p));
                let (stem, ext) = file.rsplit_once('.').unwrap_or((file, ""));
                (
                    fnv1a(p),
                    intern(&dir.replace('/', "\\")),
                    intern(stem),
                    intern(ext),
                )
            })
            .collect();

        let name_off = w.pos();
        let mut tag = b"names".to_vec();
        tag.resize(32, 0);
        w.bytes(&tag)
            .u32(records.len() as u32)
            .u32(strings.len() as u32);
        for &(h, d, n, x) in &records {
            w.u32(h).u16(d).u16(n).u16(x).u16(0);
        }
        for &(h, ..) in &records {
            w.u32(h);
        }
        let mut blob = Vec::new();
        for s in &strings {
            w.u32(blob.len() as u32);
            blob.extend_from_slice(s.as_bytes());
            blob.push(0);
        }
        w.bytes(&blob);
        let name_size = w.pos() - name_off;
        w.pad_to(16, 0);

        let data_off = w.pos();
        for (i, (p, data)) in self.files.iter().enumerate() {
            let off = w.pos();
            w.bytes(data).pad_to(16, 0);
            let o = entry_off + i * ENTRY_SIZE;
            w.set_u32(o, off as u32);
            w.set_u32(o + 4, data.len() as u32);
            w.set_u32(o + 8, data.len() as u32);
            w.set_u32(o + 12, fnv1a(p));
        }

        // 0x4C..0x80; the last two words (0x78, 0x7C) sit right before the
        // type table, which is where readers look for the name table.
        let total = w.pos();
        let words = [
            total,
            1,
            type_off,
            type_size,
            self.files.len(),
            entry_off,
            0,
            0,
            0,
            data_off,
            0,
            name_off,
            name_size,
        ];
        debug_assert_eq!(header_words + words.len() * 4, type_off);
        for (i, v) in words.iter().enumerate() {
            w.set_u32(header_words + i * 4, *v as u32);
        }
        w.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_both_orders() {
        for e in [Endian::Big, Endian::Little] {
            let bytes = BundleBuilder::new(e, "DMC1")
                .file("data/pld/pl00.pld", vec![1, 2, 3, 4, 5])
                .file("data/emd/em00.emd", vec![9; 40])
                .file("readme", b"hi".to_vec())
                .build();
            let b = Bundle::parse(&bytes).unwrap();
            assert_eq!(b.endian, e);
            assert_eq!(b.name, "DMC1");
            assert_eq!(b.version.as_deref(), Some("1.30"));
            assert_eq!(b.declared_size as usize, bytes.len());
            assert_eq!(b.types.len(), 1);
            assert_eq!(b.entries.len(), 3);
            let e0 = &b.entries[0];
            assert_eq!(e0.path.as_deref(), Some("data/pld/pl00.pld"));
            assert_eq!(e0.extension().as_deref(), Some("pld"));
            assert_eq!(b.entry_data(&bytes, e0).unwrap(), &[1, 2, 3, 4, 5]);
            assert_eq!(b.entries[2].path.as_deref(), Some("readme"));
            assert_eq!(b.entry_data(&bytes, &b.entries[1]).unwrap().len(), 40);
        }
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        assert!(Bundle::parse(b"nope").is_err());
        let mut bytes = BundleBuilder::new(Endian::Big, "x")
            .file("a", vec![0])
            .build();
        bytes.truncate(0x90);
        assert!(Bundle::parse(&bytes).is_err());
    }
}
