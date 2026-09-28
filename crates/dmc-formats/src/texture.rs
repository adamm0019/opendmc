//! Pipeworks texture containers (`T32` / `TM2`) embedded in DMC1 model files.
//! Layout: `docs/formats/README.md` §2.

use crate::bytes::{Endian, Reader, Writer};
use crate::dxt;
use crate::error::{FormatError, Result};
use serde::Serialize;

const FILE_HEADER: usize = 0x10;
const IMAGE_HEADER: usize = 0xA0;
const MAX_IMAGES: u32 = 256;
const MAX_DIM: u16 = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Kind {
    T32,
    Tm2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PixelFormat {
    Argb8888,
    Dxt1,
    Dxt5,
    Unknown(u8),
}

impl PixelFormat {
    pub fn from_code(code: u8) -> Self {
        match code & 0x0F {
            5 => Self::Argb8888,
            6 => Self::Dxt1,
            8 => Self::Dxt5,
            n => Self::Unknown(n),
        }
    }

    /// Byte size of the top mip level, if the format is known.
    pub fn level0_size(self, w: u32, h: u32) -> Option<usize> {
        match self {
            Self::Argb8888 => Some((w * h * 4) as usize),
            Self::Dxt1 => Some(dxt::bc1_size(w, h)),
            Self::Dxt5 => Some(dxt::bc3_size(w, h)),
            Self::Unknown(_) => None,
        }
    }
}

/// The on-disc magic is the ASCII tag stored as a word in the file's byte
/// order: `00 32 33 54` in a big-endian file, `54 33 32 00` ("T32\0") in a
/// little-endian one.
pub fn detect_magic(b: &[u8]) -> Option<(Kind, Endian)> {
    let m: [u8; 4] = b.get(..4)?.try_into().ok()?;
    match &m {
        b"T32\0" => Some((Kind::T32, Endian::Little)),
        b"TM2\0" => Some((Kind::Tm2, Endian::Little)),
        b"\x0023T" => Some((Kind::T32, Endian::Big)),
        b"\x002MT" => Some((Kind::Tm2, Endian::Big)),
        _ => None,
    }
}

fn magic_bytes(kind: Kind, e: Endian) -> [u8; 4] {
    let tag = match kind {
        Kind::T32 => *b"T32\0",
        Kind::Tm2 => *b"TM2\0",
    };
    match e {
        Endian::Little => tag,
        Endian::Big => [tag[3], tag[2], tag[1], tag[0]],
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Image {
    pub index: u32,
    pub format_code: u8,
    pub format: PixelFormat,
    pub has_mips: bool,
    pub width: u16,
    pub height: u16,
    /// Offset of the pixel data relative to the container start.
    pub data_offset: usize,
    pub data_size: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextureSet {
    pub kind: Kind,
    pub endian: Endian,
    /// Total container size in bytes (headers + data).
    pub size: usize,
    pub images: Vec<Image>,
}

impl TextureSet {
    /// Parse a container that starts at `data[0]`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let (kind, endian) = detect_magic(data)
            .ok_or_else(|| FormatError::BadMagic("expected T32/TM2 container".into()))?;
        let r = Reader::new(data, endian);
        let count = r.u32(4)?;
        let header_size = r.u32(8)? as usize;
        let data_size = r.u32(12)? as usize;
        if count == 0 || count > MAX_IMAGES {
            return Err(FormatError::invalid(
                "texture",
                format!("image count {count}"),
            ));
        }
        if header_size != FILE_HEADER + IMAGE_HEADER * count as usize {
            return Err(FormatError::invalid(
                "texture",
                format!("header size 0x{header_size:x} for {count} images"),
            ));
        }
        let size = header_size + data_size;
        r.bytes(0, size)?;

        let mut images = Vec::with_capacity(count as usize);
        let mut cursor = header_size;
        for i in 0..count as usize {
            let h = FILE_HEADER + IMAGE_HEADER * i;
            let format_code = r.u8(h + 0x38)?;
            let format = PixelFormat::from_code(format_code);
            let (width, height) = (r.u16(h + 0x40)?, r.u16(h + 0x42)?);
            let image_size = r.u32(h + 0x24)? as usize;
            if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
                return Err(FormatError::invalid(
                    "texture",
                    format!("image {i} is {width}x{height}"),
                ));
            }
            if cursor + image_size > size {
                return Err(FormatError::invalid(
                    "texture",
                    format!("image {i} data exceeds container"),
                ));
            }
            if let Some(need) = format.level0_size(width as u32, height as u32)
                && need > image_size
            {
                return Err(FormatError::invalid(
                    "texture",
                    format!("image {i}: {image_size} bytes < level 0 ({need})"),
                ));
            }
            images.push(Image {
                index: r.u32(h)?,
                format_code,
                format,
                has_mips: format_code & 0x20 == 0,
                width,
                height,
                data_offset: cursor,
                data_size: image_size,
            });
            cursor += image_size;
        }
        Ok(TextureSet {
            kind,
            endian,
            size,
            images,
        })
    }

    /// Top mip level of `image` as tightly packed RGBA8, or `None` for an
    /// unknown pixel format. `data` is the same slice given to [`parse`].
    pub fn decode_rgba(&self, data: &[u8], image: &Image) -> Result<Option<Vec<u8>>> {
        let px = Reader::new(data, self.endian).bytes(image.data_offset, image.data_size)?;
        let (w, h) = (image.width as u32, image.height as u32);
        Ok(match image.format {
            PixelFormat::Dxt1 => Some(dxt::decode(px, w, h, false)),
            PixelFormat::Dxt5 => Some(dxt::decode(px, w, h, true)),
            PixelFormat::Argb8888 => {
                // One ARGB word per texel, in the file's byte order.
                let out = px[..(w * h * 4) as usize]
                    .chunks_exact(4)
                    .flat_map(|c| match self.endian {
                        Endian::Big => [c[1], c[2], c[3], c[0]],
                        Endian::Little => [c[2], c[1], c[0], c[3]],
                    })
                    .collect();
                Some(out)
            }
            PixelFormat::Unknown(_) => None,
        })
    }
}

/// Every valid container inside `data`, as `(offset, set)`. Candidates are
/// found by magic and kept only if the header validates, which rejects stray
/// magic bytes inside vertex data.
pub fn scan(data: &[u8]) -> Vec<(usize, TextureSet)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + FILE_HEADER <= data.len() {
        if detect_magic(&data[i..]).is_some()
            && let Ok(set) = TextureSet::parse(&data[i..])
        {
            let size = set.size;
            out.push((i, set));
            i += size.max(4);
            continue;
        }
        i += 4;
    }
    out
}

/// Input image for [`build`].
pub struct NewImage<'a> {
    pub format_code: u8,
    pub width: u16,
    pub height: u16,
    pub pixels: &'a [u8],
}

/// Build a container in the documented layout (fixtures and mod tooling).
pub fn build(kind: Kind, endian: Endian, images: &[NewImage]) -> Vec<u8> {
    let mut w = Writer::new(endian);
    let header_size = FILE_HEADER + IMAGE_HEADER * images.len();
    let data_size: usize = images.iter().map(|i| i.pixels.len()).sum();
    w.bytes(&magic_bytes(kind, endian))
        .u32(images.len() as u32)
        .u32(header_size as u32)
        .u32(data_size as u32);
    for (i, img) in images.iter().enumerate() {
        let start = w.pos();
        w.u32(i as u32).zeros(0x20).u32(img.pixels.len() as u32);
        w.zeros(0x38 - (w.pos() - start));
        w.u8(img.format_code);
        w.zeros(0x40 - (w.pos() - start));
        w.u16(img.width).u16(img.height);
        w.zeros(IMAGE_HEADER - (w.pos() - start));
    }
    for img in images {
        w.bytes(img.pixels);
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red_bc1_4x4() -> Vec<u8> {
        vec![0x00, 0xF8, 0x00, 0x00, 0, 0, 0, 0]
    }

    #[test]
    fn magic_detection() {
        assert_eq!(
            detect_magic(&[0x00, 0x32, 0x33, 0x54]),
            Some((Kind::T32, Endian::Big))
        );
        assert_eq!(detect_magic(b"TM2\0"), Some((Kind::Tm2, Endian::Little)));
        assert_eq!(detect_magic(b"MOMO"), None);
    }

    #[test]
    fn round_trip_and_decode() {
        for e in [Endian::Big, Endian::Little] {
            let dxt = red_bc1_4x4();
            let argb: Vec<u8> = match e {
                Endian::Big => vec![0x80, 0x10, 0x20, 0x30],
                Endian::Little => vec![0x30, 0x20, 0x10, 0x80],
            };
            let bytes = build(
                Kind::T32,
                e,
                &[
                    NewImage {
                        format_code: 0x26,
                        width: 4,
                        height: 4,
                        pixels: &dxt,
                    },
                    NewImage {
                        format_code: 0x05,
                        width: 1,
                        height: 1,
                        pixels: &argb,
                    },
                ],
            );
            let set = TextureSet::parse(&bytes).unwrap();
            assert_eq!(set.endian, e);
            assert_eq!(set.images.len(), 2);
            let a = &set.images[0];
            assert_eq!(
                (a.format, a.has_mips, a.width),
                (PixelFormat::Dxt1, false, 4)
            );
            let rgba = set.decode_rgba(&bytes, a).unwrap().unwrap();
            assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
            let b = &set.images[1];
            assert!(b.has_mips);
            assert_eq!(
                set.decode_rgba(&bytes, b).unwrap().unwrap(),
                vec![0x10, 0x20, 0x30, 0x80]
            );
        }
    }

    #[test]
    fn scan_finds_embedded_containers_and_skips_stray_magic() {
        let dxt = red_bc1_4x4();
        let tex = build(
            Kind::Tm2,
            Endian::Big,
            &[NewImage {
                format_code: 6,
                width: 4,
                height: 4,
                pixels: &dxt,
            }],
        );
        let mut file = vec![0xCD; 64];
        file.extend_from_slice(&[0x00, 0x32, 0x33, 0x54, 0xFF, 0xFF, 0xFF, 0xFF]); // stray magic
        file.extend_from_slice(&[0; 8]);
        let at = file.len();
        file.extend_from_slice(&tex);
        file.extend_from_slice(&[0xCD; 16]);
        let found = scan(&file);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, at);
    }

    #[test]
    fn rejects_short_image_data() {
        let bytes = build(
            Kind::T32,
            Endian::Big,
            &[NewImage {
                format_code: 8,
                width: 8,
                height: 8,
                pixels: &[0; 16],
            }],
        );
        assert!(TextureSet::parse(&bytes).is_err());
    }
}
