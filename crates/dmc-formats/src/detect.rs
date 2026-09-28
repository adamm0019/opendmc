//! Fingerprinting of unknown files for the inventory: known magics in both
//! byte orders, plus cheap statistics that help classify the rest.

use crate::bdp;
use crate::bytes::Endian;
use crate::model::ModelFile;
use crate::texture;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum Kind {
    PipeworksBundle {
        endian: Endian,
    },
    TextureSet {
        endian: Endian,
        images: usize,
    },
    Model {
        endian: Endian,
        layout: String,
        meshes: usize,
        vertices: usize,
        bones: u8,
        textures: usize,
    },
    Known {
        name: &'static str,
    },
    Unknown,
}

const SIMPLE_MAGICS: &[(&[u8], &str)] = &[
    (b"MZ", "PE executable"),
    (b"MOMO", "Pipeworks MOMO (DMC2-style)"),
    (b"AFS\0", "AFS archive"),
    (b"RIFF", "RIFF (WAV/AVI)"),
    (b"OggS", "Ogg"),
    (b"BIK", "Bink video"),
    (b"KB2", "Bink 2 video"),
    (b"CRID", "CRI USM video"),
    (b"AFS2", "CRI AWB audio"),
    (b"@UTF", "CRI table"),
    (b"\x89PNG", "PNG"),
    (b"DDS ", "DDS texture"),
    (b"\x1A\x45\xDF\xA3", "Matroska/WebM"),
    (b"\0\0\0\x18ftyp", "MP4"),
    (b"\0\0\0\x1Cftyp", "MP4"),
    (b"\0\0\0\x20ftyp", "MP4"),
    (b"PK\x03\x04", "ZIP"),
    (b"\x1F\x8B", "gzip"),
    (b"\x28\xB5\x2F\xFD", "zstd"),
    (b"MOD ", "MOD (DMC3-style model)"),
    (b"SCM ", "SCM (DMC3-style scene)"),
    (b"PAC\0", "PAC archive"),
    (b"TIM2", "PS2 TIM2 texture"),
    (b"XWMA", "xWMA audio"),
];

/// Classify a whole file. Cheap checks first; model detection last.
pub fn classify(data: &[u8]) -> Kind {
    if bdp::is_bundle(data) {
        let endian = bdp::Bundle::parse(data)
            .map(|b| b.endian)
            .unwrap_or(Endian::Big);
        return Kind::PipeworksBundle { endian };
    }
    if let Ok(set) = texture::TextureSet::parse(data) {
        return Kind::TextureSet {
            endian: set.endian,
            images: set.images.len(),
        };
    }
    for (magic, name) in SIMPLE_MAGICS {
        if data.starts_with(magic) {
            return Kind::Known { name };
        }
    }
    if let Ok((m, g)) = ModelFile::detect(data) {
        return Kind::Model {
            endian: m.endian,
            layout: format!("{:?}", m.layout),
            meshes: g.mesh_count(),
            vertices: g.vertex_count(),
            bones: g.bone_count,
            textures: texture::scan(data).len(),
        };
    }
    Kind::Unknown
}

/// Shannon entropy in bits per byte (0 = constant, 8 = random/compressed).
pub fn entropy(data: &[u8]) -> f32 {
    if data.is_empty() {
        return 0.0;
    }
    let mut counts = [0u64; 256];
    for &b in data {
        counts[b as usize] += 1;
    }
    let n = data.len() as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum::<f64>() as f32
}

/// Which byte order makes the first 16 words look more like small integers /
/// offsets. Returns `None` when the evidence is weak.
pub fn guess_endian(data: &[u8]) -> Option<Endian> {
    let (mut big, mut little) = (0, 0);
    for w in data.chunks_exact(4).take(16) {
        let b = u32::from_be_bytes(w.try_into().unwrap());
        let l = u32::from_le_bytes(w.try_into().unwrap());
        if b == l {
            continue;
        }
        let limit = data.len().max(1 << 16) as u32;
        big += (b < limit) as i32;
        little += (l < limit) as i32;
    }
    match big - little {
        d if d >= 3 => Some(Endian::Big),
        d if d <= -3 => Some(Endian::Little),
        _ => None,
    }
}

pub fn hex_prefix(data: &[u8], n: usize) -> String {
    data.iter()
        .take(n)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_things() {
        let b = bdp::BundleBuilder::new(Endian::Little, "x")
            .file("a/b.c", vec![1])
            .build();
        assert_eq!(
            classify(&b),
            Kind::PipeworksBundle {
                endian: Endian::Little
            }
        );
        assert_eq!(classify(b"OggS\0\0"), Kind::Known { name: "Ogg" });
        assert_eq!(classify(&[0x55; 64]), Kind::Unknown);
    }

    #[test]
    fn entropy_bounds() {
        assert_eq!(entropy(&[7; 100]), 0.0);
        let all: Vec<u8> = (0..=255).collect();
        assert!((entropy(&all) - 8.0).abs() < 1e-4);
    }

    #[test]
    fn endian_guess() {
        let words: Vec<u8> = (0u32..16)
            .flat_map(|i| (i * 64 + 16).to_be_bytes())
            .collect();
        assert_eq!(guess_endian(&words), Some(Endian::Big));
        let words: Vec<u8> = (0u32..16)
            .flat_map(|i| (i * 64 + 16).to_le_bytes())
            .collect();
        assert_eq!(guess_endian(&words), Some(Endian::Little));
    }
}
