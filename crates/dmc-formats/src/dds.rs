//! DirectDraw Surface files, per Microsoft's public DDS documentation. The PC
//! build stores every texture image as a complete DDS file inside its T32/TM2
//! containers and `ipum` packs (`docs/formats/README.md` §2).

use crate::dxt::{self, Block};
use crate::error::{FormatError, Result};
use serde::Serialize;

pub const MAGIC: &[u8; 4] = b"DDS ";
pub const HEADER_SIZE: usize = 128;
const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const MAX_DIM: u32 = 16384;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Format {
    Dxt1,
    Dxt3,
    Dxt5,
    /// Uncompressed with bit masks for R, G, B, A (little-endian words).
    Rgba {
        bits: u32,
        masks: [u32; 4],
    },
}

impl Format {
    /// Byte size of one mip level.
    pub fn level_size(self, w: u32, h: u32) -> usize {
        match self {
            Format::Dxt1 => Block::Bc1.size(w, h),
            Format::Dxt3 => Block::Bc2.size(w, h),
            Format::Dxt5 => Block::Bc3.size(w, h),
            Format::Rgba { bits, .. } => (w * h * bits / 8) as usize,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Dds {
    pub width: u32,
    pub height: u32,
    /// Mip levels stored, at least 1.
    pub mips: u32,
    pub format: Format,
    /// Total size of the file: header plus every level.
    pub size: usize,
}

fn u32_at(b: &[u8], o: usize) -> Result<u32> {
    let s = b.get(o..o + 4).ok_or(FormatError::OutOfBounds {
        offset: o,
        len: 4,
        size: b.len(),
    })?;
    Ok(u32::from_le_bytes(s.try_into().unwrap()))
}

impl Dds {
    /// Parse the header of a DDS file starting at `data[0]`. The pixel data
    /// (`HEADER_SIZE..size`) must be present too.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.get(..4) != Some(MAGIC) {
            return Err(FormatError::BadMagic("expected DDS".into()));
        }
        if u32_at(data, 4)? != 124 {
            return Err(FormatError::invalid("dds", "header size is not 124"));
        }
        let height = u32_at(data, 12)?;
        let width = u32_at(data, 16)?;
        let mips = u32_at(data, 28)?.max(1);
        if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM || mips > 16 {
            return Err(FormatError::invalid(
                "dds",
                format!("{width}x{height}, {mips} mips"),
            ));
        }
        let pf_flags = u32_at(data, 80)?;
        let format = if pf_flags & DDPF_FOURCC != 0 {
            match &data[84..88] {
                b"DXT1" => Format::Dxt1,
                b"DXT3" => Format::Dxt3,
                b"DXT5" => Format::Dxt5,
                cc => {
                    return Err(FormatError::invalid(
                        "dds",
                        format!("unsupported FourCC {:?}", String::from_utf8_lossy(cc)),
                    ));
                }
            }
        } else if pf_flags & DDPF_RGB != 0 {
            let bits = u32_at(data, 88)?;
            if !matches!(bits, 16 | 24 | 32) {
                return Err(FormatError::invalid("dds", format!("{bits}-bit RGB")));
            }
            let alpha = if pf_flags & DDPF_ALPHAPIXELS != 0 {
                u32_at(data, 104)?
            } else {
                0
            };
            Format::Rgba {
                bits,
                masks: [
                    u32_at(data, 92)?,
                    u32_at(data, 96)?,
                    u32_at(data, 100)?,
                    alpha,
                ],
            }
        } else {
            return Err(FormatError::invalid(
                "dds",
                format!("pixel format flags 0x{pf_flags:x}"),
            ));
        };
        let size = HEADER_SIZE
            + (0..mips)
                .map(|m| format.level_size((width >> m).max(1), (height >> m).max(1)))
                .sum::<usize>();
        if data.len() < size {
            return Err(FormatError::OutOfBounds {
                offset: 0,
                len: size,
                size: data.len(),
            });
        }
        Ok(Dds {
            width,
            height,
            mips,
            format,
            size,
        })
    }

    /// The top mip level as tightly packed RGBA8. `data` starts at the magic.
    pub fn decode_rgba(&self, data: &[u8]) -> Vec<u8> {
        let (w, h) = (self.width, self.height);
        let px = &data[HEADER_SIZE..HEADER_SIZE + self.format.level_size(w, h)];
        match self.format {
            Format::Dxt1 => dxt::decode_block(px, w, h, Block::Bc1),
            Format::Dxt3 => dxt::decode_block(px, w, h, Block::Bc2),
            Format::Dxt5 => dxt::decode_block(px, w, h, Block::Bc3),
            Format::Rgba { bits, masks } => {
                let step = (bits / 8) as usize;
                px.chunks_exact(step)
                    .flat_map(|c| {
                        let mut word = [0u8; 4];
                        word[..step].copy_from_slice(c);
                        let v = u32::from_le_bytes(word);
                        masks.map(|m| channel(v, m))
                    })
                    .collect()
            }
        }
    }
}

/// Extract the channel selected by `mask`, scaled to 8 bits (opaque when the
/// mask is empty, which is only meaningful for alpha).
fn channel(v: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 255;
    }
    let shift = mask.trailing_zeros();
    let max = mask >> shift;
    (((v & mask) >> shift) * 255 / max) as u8
}

/// Build a DDS file (fixtures and mod tooling). `levels` holds every mip level.
pub fn build(width: u32, height: u32, format: Format, levels: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_SIZE);
    let mut put = |v: u32| out.extend_from_slice(&v.to_le_bytes());
    put(u32::from_le_bytes(*MAGIC));
    put(124);
    put(0x1007 | if levels.len() > 1 { 0x20000 } else { 0 });
    put(height);
    put(width);
    put(0);
    put(0);
    put(levels.len() as u32);
    for _ in 0..11 {
        put(0);
    }
    put(32);
    match format {
        Format::Dxt1 | Format::Dxt3 | Format::Dxt5 => {
            put(DDPF_FOURCC);
            let cc = match format {
                Format::Dxt1 => b"DXT1",
                Format::Dxt3 => b"DXT3",
                _ => b"DXT5",
            };
            put(u32::from_le_bytes(*cc));
            for _ in 0..5 {
                put(0);
            }
        }
        Format::Rgba { bits, masks } => {
            put(DDPF_RGB | if masks[3] != 0 { DDPF_ALPHAPIXELS } else { 0 });
            put(0);
            put(bits);
            for m in masks {
                put(m);
            }
        }
    }
    put(0x1000);
    for _ in 0..4 {
        put(0);
    }
    for l in levels {
        out.extend_from_slice(l);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const BGRA8: Format = Format::Rgba {
        bits: 32,
        masks: [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000],
    };

    #[test]
    fn round_trips_dxt_with_mips() {
        let l0 = [0x00, 0xF8, 0, 0, 0, 0, 0, 0].repeat(4); // 8x8 red
        let l1 = [0x00, 0xF8, 0, 0, 0, 0, 0, 0];
        let l2 = l1;
        let f = build(8, 8, Format::Dxt1, &[&l0, &l1, &l2]);
        assert_eq!(f.len(), HEADER_SIZE + 32 + 8 + 8);
        let d = Dds::parse(&f).unwrap();
        assert_eq!((d.width, d.height, d.mips, d.size), (8, 8, 3, f.len()));
        assert!(d.decode_rgba(&f).chunks(4).all(|p| p == [255, 0, 0, 255]));
    }

    #[test]
    fn decodes_bgra8() {
        let f = build(1, 1, BGRA8, &[&[0x10, 0x20, 0x30, 0x80]]);
        let d = Dds::parse(&f).unwrap();
        assert_eq!(d.decode_rgba(&f), vec![0x30, 0x20, 0x10, 0x80]);
    }

    #[test]
    fn rejects_truncation_and_garbage() {
        let f = build(8, 8, Format::Dxt5, &[&[0; 64]]);
        assert!(Dds::parse(&f[..f.len() - 1]).is_err());
        assert!(Dds::parse(b"DDS nonsense").is_err());
        assert!(Dds::parse(&[]).is_err());
    }
}
