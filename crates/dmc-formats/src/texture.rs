//! Pipeworks texture containers (`T32` / `TM2`), loose or embedded in DMC1
//! model files. Layout: `docs/formats/README.md` §2.

use crate::bytes::{Endian, Reader, Writer};
use crate::dds::{self, Dds};
use crate::dxt;
use crate::error::{FormatError, Result};
use serde::Serialize;

const FILE_HEADER: usize = 0x10;
const IMAGE_HEADER: usize = 0xA0;
const PC_IMAGE_HEADER: usize = 0xA8;
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
    Dxt3,
    Dxt5,
    /// Uncompressed with other bit masks (PC DDS only).
    Rgba,
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

    fn from_dds(f: dds::Format) -> Self {
        match f {
            dds::Format::Dxt1 => Self::Dxt1,
            dds::Format::Dxt3 => Self::Dxt3,
            dds::Format::Dxt5 => Self::Dxt5,
            dds::Format::Rgba {
                bits: 32,
                masks: [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000],
            } => Self::Argb8888,
            dds::Format::Rgba { .. } => Self::Rgba,
        }
    }

    /// Byte size of the top mip level, if the format is known.
    pub fn level0_size(self, w: u32, h: u32) -> Option<usize> {
        match self {
            Self::Argb8888 => Some((w * h * 4) as usize),
            Self::Dxt1 => Some(dxt::bc1_size(w, h)),
            Self::Dxt3 | Self::Dxt5 => Some(dxt::bc3_size(w, h)),
            Self::Rgba | Self::Unknown(_) => None,
        }
    }
}

/// Container layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Layout {
    /// PS3 notes: 0xA0-byte image headers carrying format and size, raw pixels.
    Ps3,
    /// PC build: 0xA8-byte image header slots (the block aligned to 16), then
    /// one complete DDS file per image, back to back.
    Pc,
}

/// The tag is stored as the fixed bytes `00 32 33 54` (`T32`) or `00 32 4D 54`
/// (`TM2`) on both PS3 and PC, so it says nothing about byte order: the PC
/// files put these bytes in front of little-endian fields. The ASCII form
/// (`"T32\0"`) is accepted too, in case another build uses it.
pub fn detect_magic(b: &[u8]) -> Option<Kind> {
    match b.get(..4)? {
        b"\x0023T" | b"T32\0" => Some(Kind::T32),
        b"\x002MT" | b"TM2\0" => Some(Kind::Tm2),
        _ => None,
    }
}

fn magic_bytes(kind: Kind) -> [u8; 4] {
    match kind {
        Kind::T32 => *b"\x0023T",
        Kind::Tm2 => *b"\x002MT",
    }
}

/// Layout and field byte order. PS3: the header size is exactly
/// `0x10 + 0xA0 * count`. PC: little-endian, the header block is 16-aligned
/// with room for at least one 0xA8 slot, and a DDS file starts right after it.
/// (Some PC containers have fewer slots than images; the DDS files are
/// self-describing, so the count is what matters.)
fn detect_layout(data: &[u8]) -> Option<(Layout, Endian)> {
    let fields = |e| {
        let r = Reader::new(data, e);
        match (r.u32(4), r.u32(8)) {
            (Ok(n), Ok(h)) if (1..=MAX_IMAGES).contains(&n) => Some((n as usize, h as usize)),
            _ => None,
        }
    };
    if let Some((n, h)) = fields(Endian::Little)
        && h.is_multiple_of(16)
        && h >= FILE_HEADER + PC_IMAGE_HEADER
        && h <= (FILE_HEADER + PC_IMAGE_HEADER * n).next_multiple_of(16)
        && data.get(h..h + 4) == Some(dds::MAGIC)
    {
        return Some((Layout::Pc, Endian::Little));
    }
    [Endian::Big, Endian::Little].into_iter().find_map(|e| {
        fields(e)
            .filter(|&(n, h)| h == FILE_HEADER + IMAGE_HEADER * n)
            .map(|_| (Layout::Ps3, e))
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct Image {
    pub index: u32,
    pub format_code: u8,
    pub format: PixelFormat,
    pub has_mips: bool,
    pub width: u16,
    pub height: u16,
    /// Offset of the image data relative to the container start: raw pixels
    /// on PS3, the embedded DDS file on PC.
    pub data_offset: usize,
    pub data_size: usize,
    /// The embedded DDS header (PC only).
    pub dds: Option<Dds>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextureSet {
    pub kind: Kind,
    pub layout: Layout,
    pub endian: Endian,
    /// Total container size in bytes (headers + data).
    pub size: usize,
    pub images: Vec<Image>,
}

impl TextureSet {
    /// Parse a container that starts at `data[0]`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let kind = detect_magic(data)
            .ok_or_else(|| FormatError::BadMagic("expected T32/TM2 container".into()))?;
        let (layout, endian) = detect_layout(data)
            .ok_or_else(|| FormatError::invalid("texture", "header size fits no layout"))?;
        match layout {
            Layout::Ps3 => Self::parse_ps3(data, kind, endian),
            Layout::Pc => Self::parse_pc(data, kind),
        }
    }

    fn parse_pc(data: &[u8], kind: Kind) -> Result<Self> {
        let r = Reader::new(data, Endian::Little);
        let count = r.u32(4)? as usize;
        let header_size = r.u32(8)? as usize;
        let slots = (header_size - FILE_HEADER) / PC_IMAGE_HEADER;
        let mut images = Vec::with_capacity(count);
        let mut cursor = header_size;
        for i in 0..count {
            // Images whose size leaves the next one unaligned are padded to 16.
            if data.get(cursor..cursor + 4) != Some(dds::MAGIC) {
                cursor = cursor.next_multiple_of(16);
            }
            let d = Dds::parse(r.tail(cursor)?.data()).map_err(|e| {
                FormatError::invalid("texture", format!("image {i} at 0x{cursor:x}: {e}"))
            })?;
            let (width, height) = (d.width as u16, d.height as u16);
            let h = FILE_HEADER + PC_IMAGE_HEADER * i.min(slots.saturating_sub(1));
            let format_code = r.u8(h + 0x1A)?;
            images.push(Image {
                index: i as u32,
                format_code,
                format: PixelFormat::from_dds(d.format),
                has_mips: d.mips > 1,
                width,
                height,
                data_offset: cursor,
                data_size: d.size,
                dds: Some(d.clone()),
            });
            cursor += d.size;
        }
        Ok(TextureSet {
            kind,
            layout: Layout::Pc,
            endian: Endian::Little,
            size: cursor,
            images,
        })
    }

    fn parse_ps3(data: &[u8], kind: Kind, endian: Endian) -> Result<Self> {
        let r = Reader::new(data, endian);
        let count = r.u32(4)?;
        let header_size = r.u32(8)? as usize;
        let data_size = r.u32(12)? as usize;
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
                dds: None,
            });
            cursor += image_size;
        }
        Ok(TextureSet {
            kind,
            layout: Layout::Ps3,
            endian,
            size,
            images,
        })
    }

    /// Top mip level of `image` as tightly packed RGBA8, or `None` for an
    /// unknown pixel format. `data` is the same slice given to [`parse`].
    pub fn decode_rgba(&self, data: &[u8], image: &Image) -> Result<Option<Vec<u8>>> {
        let px = Reader::new(data, self.endian).bytes(image.data_offset, image.data_size)?;
        if let Some(d) = &image.dds {
            return Ok(Some(d.decode_rgba(px)));
        }
        let (w, h) = (image.width as u32, image.height as u32);
        Ok(match image.format {
            PixelFormat::Dxt1 => Some(dxt::decode(px, w, h, false)),
            PixelFormat::Dxt5 => Some(dxt::decode(px, w, h, true)),
            PixelFormat::Dxt3 => Some(dxt::decode_block(px, w, h, dxt::Block::Bc2)),
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
            PixelFormat::Rgba | PixelFormat::Unknown(_) => None,
        })
    }
}

/// Build a PC-layout container from complete DDS files (fixtures).
pub fn build_pc(kind: Kind, dds_files: &[Vec<u8>]) -> Vec<u8> {
    let mut w = Writer::new(Endian::Little);
    let header_size = (FILE_HEADER + PC_IMAGE_HEADER * dds_files.len()).next_multiple_of(16);
    let logical: usize = dds_files.iter().map(|d| d.len() - dds::HEADER_SIZE).sum();
    w.bytes(&magic_bytes(kind))
        .u32(dds_files.len() as u32)
        .u32(header_size as u32)
        .u32(logical as u32);
    for (i, d) in dds_files.iter().enumerate() {
        let start = w.pos();
        w.u32(i as u32).zeros(0x28).u32(d.len() as u32);
        w.zeros(PC_IMAGE_HEADER - (w.pos() - start));
    }
    w.zeros(header_size - w.pos());
    for d in dds_files {
        w.bytes(d).pad_to(16, 0);
    }
    w.finish()
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
    w.bytes(&magic_bytes(kind))
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
        assert_eq!(detect_magic(&[0x00, 0x32, 0x33, 0x54]), Some(Kind::T32));
        assert_eq!(detect_magic(b"TM2\0"), Some(Kind::Tm2));
        assert_eq!(detect_magic(b"MOMO"), None);
    }

    #[test]
    fn pc_container_of_dds_files() {
        let red = dds::build(4, 4, dds::Format::Dxt1, &[&red_bc1_4x4()]);
        let bgra = dds::Format::Rgba {
            bits: 32,
            masks: [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000],
        };
        // 1x1 BGRA: 132 bytes, so the next image needs 16-byte alignment.
        let px = dds::build(1, 1, bgra, &[&[0x10, 0x20, 0x30, 0x80]]);
        let bytes = build_pc(Kind::Tm2, &[px, red]);
        assert_eq!(&bytes[..4], &[0x00, 0x32, 0x4D, 0x54]);
        let set = TextureSet::parse(&bytes).unwrap();
        assert_eq!((set.layout, set.endian), (Layout::Pc, Endian::Little));
        let [a, b] = &set.images[..] else {
            panic!("two images")
        };
        assert_eq!((a.format, a.width), (PixelFormat::Argb8888, 1));
        assert_eq!(
            set.decode_rgba(&bytes, a).unwrap().unwrap(),
            vec![0x30, 0x20, 0x10, 0x80]
        );
        assert_eq!((b.format, b.width, b.height), (PixelFormat::Dxt1, 4, 4));
        let rgba = set.decode_rgba(&bytes, b).unwrap().unwrap();
        assert_eq!(&rgba[..4], &[255, 0, 0, 255]);

        let mut file = vec![0xCD; 32];
        file.extend_from_slice(&bytes);
        assert_eq!(scan(&file)[0].0, 32);
    }

    #[test]
    fn byte_order_comes_from_the_header_not_the_magic() {
        for e in [Endian::Big, Endian::Little] {
            let px = red_bc1_4x4();
            let img = NewImage {
                format_code: 6,
                width: 4,
                height: 4,
                pixels: &px,
            };
            let bytes = build(Kind::T32, e, &[img]);
            assert_eq!(&bytes[..4], &[0x00, 0x32, 0x33, 0x54]);
            assert_eq!(TextureSet::parse(&bytes).unwrap().endian, e);
        }
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
