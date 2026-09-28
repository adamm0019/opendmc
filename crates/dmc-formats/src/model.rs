//! DMC1 model files (`.pld`, `.pws`, `.pwd`, `.emd`, `.fsd`): a flat list of
//! section offsets, optionally behind a 0x800-byte texture directory.
//! Layout: `docs/formats/README.md` §3.

use crate::bytes::{Endian, Reader};
use crate::error::{FormatError, Result};
use crate::geometry::Geometry;
use crate::texture::{self, TextureSet};
use serde::Serialize;

pub const TEXTURE_DIRECTORY: usize = 0x800;
const MAX_SECTIONS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Layout {
    /// Section list at offset 0 (player bodies `pl00`, `pl05`).
    Bare,
    /// 0x800-byte texture directory, then the section list (DT bodies, enemies).
    TextureDirectory,
}

impl Layout {
    pub fn base(self) -> usize {
        match self {
            Layout::Bare => 0,
            Layout::TextureDirectory => TEXTURE_DIRECTORY,
        }
    }
}

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

    /// Try both byte orders and both layouts; keep the combination whose
    /// section 0 parses as geometry.
    pub fn detect(data: &[u8]) -> Result<(Self, Geometry)> {
        let mut last = FormatError::invalid("model", "no candidate layout");
        for endian in [Endian::Big, Endian::Little] {
            for layout in [Layout::Bare, Layout::TextureDirectory] {
                let parsed = Self::parse_with(data, endian, layout)
                    .and_then(|m| m.geometry(data).map(|g| (m, g)));
                match parsed {
                    Ok(found) => return Ok(found),
                    Err(e) => last = e,
                }
            }
        }
        Err(last)
    }

    pub fn section<'a>(&self, data: &'a [u8], index: usize) -> Option<&'a [u8]> {
        let s = self.sections.get(index)?;
        data.get(s.offset?..s.offset? + s.len)
    }

    pub fn geometry(&self, data: &[u8]) -> Result<Geometry> {
        let s = self
            .section(data, 0)
            .ok_or_else(|| FormatError::invalid("model", "section 0 is empty"))?;
        Geometry::parse(s, self.endian)
    }

    /// Texture containers anywhere in the file, in file order.
    pub fn textures(data: &[u8]) -> Vec<(usize, TextureSet)> {
        texture::scan(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::Writer;
    use crate::geometry::{build, tests::quad_object};

    fn model(endian: Endian, layout: Layout) -> Vec<u8> {
        let geo = build(endian, 1, &[quad_object()], None);
        let mut w = Writer::new(endian);
        w.zeros(layout.base());
        // 4 sections: geometry, empty, 16 bytes, 8 bytes.
        let list = 4 * 4;
        let s0 = list;
        let s2 = s0 + geo.len().next_multiple_of(16);
        let s3 = s2 + 16;
        w.u32(s0 as u32).u32(0).u32(s2 as u32).u32(s3 as u32);
        w.bytes(&geo).pad_to(16, 0);
        w.zeros(16).zeros(8);
        w.finish()
    }

    #[test]
    fn detects_every_combination() {
        for endian in [Endian::Big, Endian::Little] {
            for layout in [Layout::Bare, Layout::TextureDirectory] {
                let data = model(endian, layout);
                let (m, g) = ModelFile::detect(&data).unwrap();
                assert_eq!((m.endian, m.layout), (endian, layout));
                assert_eq!(m.sections.len(), 4);
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
