//! DMC1 model files (`.pld`, `.pws`, `.pwd`, `.emd`, `.fsd`): a table of
//! section offsets. PS3 notes: a flat list, optionally behind a 0x800-byte
//! texture directory. PC: a count followed by the offsets.
//! Layout: `docs/formats/README.md` §3.

use crate::bytes::{Endian, Reader};
use crate::error::{FormatError, Result};
use crate::geometry::{Geometry, PC_PAD, Variant};
use crate::texture::{self, TextureSet};
use serde::Serialize;

pub const TEXTURE_DIRECTORY: usize = 0x800;
const MAX_SECTIONS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Layout {
    /// PS3: section list at offset 0 (player bodies `pl00`, `pl05`).
    Bare,
    /// PS3: 0x800-byte texture directory, then the section list (DT bodies, enemies).
    TextureDirectory,
    /// PC `.pld/.pws/.pwd/.emd`: `u32 count`, then `count` u32 offsets.
    Counted,
    /// PC `.fsd`: `u32 count`, 4 bytes of `0xCC`, then `count` u64 offsets.
    Counted64,
}

impl Layout {
    pub const ALL: [Layout; 4] = [
        Layout::Counted,
        Layout::Counted64,
        Layout::Bare,
        Layout::TextureDirectory,
    ];

    pub fn base(self) -> usize {
        match self {
            Layout::TextureDirectory => TEXTURE_DIRECTORY,
            _ => 0,
        }
    }

    /// The geometry record layout that goes with this section table.
    pub fn variant(self) -> Variant {
        match self {
            Layout::Bare | Layout::TextureDirectory => Variant::Ps3,
            Layout::Counted | Layout::Counted64 => Variant::Pc64,
        }
    }

    /// Which section holds the geometry: 0, except in PC room files (`.fsd`,
    /// 35 sections), where it is section 14.
    pub fn geometry_section(self) -> usize {
        match self {
            Layout::Counted64 => ROOM_GEOMETRY_SECTION,
            _ => 0,
        }
    }
}

pub const ROOM_GEOMETRY_SECTION: usize = 14;

#[derive(Debug, Clone, Serialize)]
pub struct Section {
    pub index: usize,
    /// Absolute offset within the file; `None` for an empty section.
    pub offset: Option<usize>,
    pub len: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelFile {
    pub endian: Endian,
    pub layout: Layout,
    pub sections: Vec<Section>,
}

impl ModelFile {
    /// Read the section list with a known byte order and layout.
    pub fn parse_with(data: &[u8], endian: Endian, layout: Layout) -> Result<Self> {
        let base = layout.base();
        let r = Reader::new(data, endian).tail(base)?;
        let raw = match layout {
            Layout::Bare | Layout::TextureDirectory => flat_list(&r)?,
            Layout::Counted | Layout::Counted64 => counted_list(&r, layout)?,
        };

        let mut ends: Vec<usize> = raw.iter().copied().filter(|&v| v != 0).collect();
        ends.push(r.len());
        ends.sort_unstable();
        ends.dedup();
        let sections = raw
            .iter()
            .enumerate()
            .map(|(index, &v)| {
                if v == 0 {
                    return Section {
                        index,
                        offset: None,
                        len: 0,
                    };
                }
                let end = ends.iter().copied().find(|&e| e > v).unwrap_or(r.len());
                Section {
                    index,
                    offset: Some(base + v),
                    len: end - v,
                }
            })
            .collect();
        Ok(ModelFile {
            endian,
            layout,
            sections,
        })
    }

    /// Try both byte orders and every layout; keep the combination whose
    /// geometry section parses.
    pub fn detect(data: &[u8]) -> Result<(Self, Geometry)> {
        // Report the geometry error of a layout whose table did parse, if any:
        // it says far more than "no consistent section list".
        let mut table_err = None;
        let mut geometry_err = None;
        for endian in [Endian::Little, Endian::Big] {
            for layout in Layout::ALL {
                match Self::parse_with(data, endian, layout) {
                    Ok(m) => match m.geometry(data) {
                        Ok(g) => return Ok((m, g)),
                        Err(e) => {
                            geometry_err.get_or_insert(FormatError::invalid(
                                "model",
                                format!("{endian:?} {layout:?}: {e}"),
                            ));
                        }
                    },
                    Err(e) => table_err = Some(e),
                }
            }
        }
        Err(geometry_err
            .or(table_err)
            .unwrap_or_else(|| FormatError::invalid("model", "no candidate layout")))
    }

    pub fn section<'a>(&self, data: &'a [u8], index: usize) -> Option<&'a [u8]> {
        let s = self.sections.get(index)?;
        data.get(s.offset?..s.offset? + s.len)
    }

    pub fn geometry(&self, data: &[u8]) -> Result<Geometry> {
        let index = self.layout.geometry_section();
        let s = self
            .section(data, index)
            .ok_or_else(|| FormatError::invalid("model", format!("section {index} is empty")))?;
        Geometry::parse(s, self.endian, self.layout.variant())
    }

    /// Texture containers anywhere in the file, in file order.
    pub fn textures(data: &[u8]) -> Vec<(usize, TextureSet)> {
        texture::scan(data)
    }
}

/// PS3: offsets until the first section starts.
fn flat_list(r: &Reader) -> Result<Vec<usize>> {
    let mut raw = Vec::new();
    let mut first_section = usize::MAX;
    while raw.len() < MAX_SECTIONS && raw.len() * 4 < first_section {
        let Ok(v) = r.u32(raw.len() * 4) else { break };
        let v = v as usize;
        if v != 0 {
            if v >= r.len() || !v.is_multiple_of(4) {
                break;
            }
            first_section = first_section.min(v);
        }
        raw.push(v);
    }
    if first_section == usize::MAX || raw.len() * 4 != first_section {
        return Err(FormatError::invalid("model", "no consistent section list"));
    }
    Ok(raw)
}

/// PC: a count, then offsets that all land after the table.
fn counted_list(r: &Reader, layout: Layout) -> Result<Vec<usize>> {
    let count = r.u32(0)? as usize;
    if count == 0 || count > MAX_SECTIONS {
        return Err(FormatError::invalid("model", format!("{count} sections")));
    }
    let (start, width) = match layout {
        Layout::Counted64 => {
            if r.bytes(4, 4)? != [PC_PAD; 4] {
                return Err(FormatError::invalid("model", "no padding after count"));
            }
            (8, 8)
        }
        _ => (4, 4),
    };
    let table_end = start + width * count;
    let mut raw = Vec::with_capacity(count);
    for i in 0..count {
        let at = start + width * i;
        let v = if width == 8 {
            r.offset64(at)?
        } else {
            r.u32(at)? as usize
        };
        if v != 0 && (v < table_end || v >= r.len() || !v.is_multiple_of(4)) {
            return Err(FormatError::invalid(
                "model",
                format!("section {i} offset 0x{v:x}"),
            ));
        }
        raw.push(v);
    }
    if raw.iter().all(|&v| v == 0) {
        return Err(FormatError::invalid("model", "every section is empty"));
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::Writer;
    use crate::geometry::{build, tests::quad_object};

    fn model(endian: Endian, layout: Layout) -> Vec<u8> {
        let geo = build(endian, layout.variant(), 1, &[quad_object()], None);
        let mut w = Writer::new(endian);
        w.zeros(layout.base());
        // Sections: geometry (at the layout's geometry index), empty,
        // 16 bytes, 8 bytes; rooms have 15 so the geometry lands at 14.
        let (head, width): (usize, usize) = match layout {
            Layout::Bare | Layout::TextureDirectory => (0, 4),
            Layout::Counted => (4, 4),
            Layout::Counted64 => (8, 8),
        };
        let count = layout.geometry_section().max(3) + 1;
        let s0 = (head + count * width).next_multiple_of(16);
        let s2 = s0 + geo.len().next_multiple_of(16);
        let s3 = s2 + 16;
        let mut table = vec![0; count];
        table[layout.geometry_section()] = s0;
        table[2] = s2;
        table[3] = s3;
        if head > 0 {
            w.u32(count as u32);
        }
        if head == 8 {
            w.bytes(&[PC_PAD; 4]);
        }
        for v in table {
            if width == 8 {
                w.u64(v as u64);
            } else {
                w.u32(v as u32);
            }
        }
        w.pad_to(16, 0);
        w.bytes(&geo).pad_to(16, 0);
        w.zeros(16).zeros(8);
        w.finish()
    }

    #[test]
    fn detects_every_combination() {
        for (endian, layout) in [
            (Endian::Big, Layout::Bare),
            (Endian::Little, Layout::Bare),
            (Endian::Big, Layout::TextureDirectory),
            (Endian::Little, Layout::TextureDirectory),
            (Endian::Little, Layout::Counted),
            (Endian::Little, Layout::Counted64),
        ] {
            {
                let data = model(endian, layout);
                let (m, g) = ModelFile::detect(&data).unwrap();
                assert_eq!((m.endian, m.layout), (endian, layout));
                assert_eq!(m.sections.len(), layout.geometry_section().max(3) + 1);
                assert!(m.sections[1].offset.is_none());
                assert_eq!(m.sections[2].len, 16);
                assert_eq!(m.sections[3].len, 8);
                assert_eq!(g.mesh_count(), 2);
            }
        }
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(ModelFile::detect(&[0xAB; 4096]).is_err());
        assert!(ModelFile::detect(&[]).is_err());
    }
}
