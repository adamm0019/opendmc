//! Room props (`.fsd` section 19, PC build): the room's separate models, such
//! as chandeliers, statues and doors, each a §4.2-style geometry
//! ([`Variant::Pc64Prop`]) with optional motion data and its own textures.
//! Where a prop stands in the room is not in this section.
//! Layout and evidence: `docs/formats/README.md` §4f. The table is fully
//! walked; several of its fields are kept raw.

use crate::bytes::{Endian, Reader, Writer};
use crate::error::{FormatError, Result};
use crate::geometry::{Geometry, Variant};
use serde::Serialize;

pub const SECTION: usize = 19;

const NONE: u32 = 0xFFFF_FFFF;
/// Texture word: an image index into the room's own textures (section 34).
const ROOM_TEXTURE: u32 = 0x8000_0000;
/// Texture word: an `ipum` frame sequence at the offset in the low bits.
const ANIMATED: u32 = 0x4000_0000;
const FIELDS: usize = 6;
/// Byte of the geometry header holding the texture count.
const TEX_COUNT: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TextureRef {
    None,
    /// An image of the room's texture container (section 34).
    Room(u32),
    /// A `T32` texture container at this offset in the section.
    Embedded(usize),
    /// An `ipum` frame sequence at this offset in the section (§10).
    Animated(usize),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Prop {
    /// Offset of the geometry ([`Variant::Pc64Prop`]) in the section.
    pub geometry: usize,
    /// Table fields 1–5 as stored. Field 1 is usually −1 or an offset to
    /// motion-like data; field 4 is a second geometry in 30 props; field 5
    /// holds references (top bit set) or offsets. Meaning not settled.
    pub fields: [u32; 5],
    /// One reference per texture slot for room textures, else a single
    /// container or frame sequence shared by all slots.
    pub textures: Vec<TextureRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Props {
    /// Table slots in order; `None` is an empty slot.
    pub slots: Vec<Option<Prop>>,
}

impl Props {
    pub fn parse(section: &[u8]) -> Result<Self> {
        let r = Reader::new(section, Endian::Little);
        let count = r.u32(0)? as usize;
        if count > section.len() / 4 {
            return Err(FormatError::invalid("props", format!("{count} slots")));
        }
        let mut at = 4;
        let mut first_data = section.len();
        let mut slots = Vec::with_capacity(count);
        for i in 0..count {
            let geometry = r.u32(at)?;
            if geometry == NONE {
                slots.push(None);
                at += 4;
                continue;
            }
            let geometry = geometry as usize;
            if !geometry.is_multiple_of(16) {
                return Err(FormatError::invalid(
                    "props",
                    format!("slot {i}: geometry at 0x{geometry:x}"),
                ));
            }
            let tex_count = r.u8(geometry + TEX_COUNT)? as usize;
            let fields = std::array::from_fn(|k| r.u32(at + 4 + 4 * k).unwrap_or(NONE));
            r.bytes(at, 4 * FIELDS)?;
            let first = r.u32(at + 4 * FIELDS)?;
            let words = if first != NONE && first & ROOM_TEXTURE != 0 && tex_count > 1 {
                tex_count
            } else {
                1
            };
            let textures = (0..words)
                .map(|k| texture_ref(&r, r.u32(at + 4 * (FIELDS + k))?))
                .collect::<Result<_>>()?;
            first_data = first_data.min(geometry);
            slots.push(Some(Prop {
                geometry,
                fields,
                textures,
            }));
            at += 4 * (FIELDS + words);
        }
        // The data follows the table, aligned to 16.
        if at > first_data {
            return Err(FormatError::invalid(
                "props",
                format!("table ends at 0x{at:x}, past data at 0x{first_data:x}"),
            ));
        }
        Ok(Props { slots })
    }

    /// The occupied slots with their indices.
    pub fn props(&self) -> impl Iterator<Item = (usize, &Prop)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.as_ref().map(|p| (i, p)))
    }
}

fn texture_ref(r: &Reader, word: u32) -> Result<TextureRef> {
    Ok(match word {
        NONE => TextureRef::None,
        w if w & ROOM_TEXTURE != 0 => TextureRef::Room(w & !ROOM_TEXTURE),
        w => {
            let offset = (w & !ANIMATED) as usize;
            r.bytes(offset, 4)?;
            if w & ANIMATED != 0 {
                TextureRef::Animated(offset)
            } else {
                TextureRef::Embedded(offset)
            }
        }
    })
}

impl Prop {
    /// Parse the prop's geometry; its offsets are relative to its header.
    pub fn geometry(&self, section: &[u8]) -> Result<Geometry> {
        let bytes = section
            .get(self.geometry..)
            .ok_or_else(|| FormatError::invalid("props", "geometry out of range"))?;
        Geometry::parse(bytes, Endian::Little, Variant::Pc64Prop)
    }
}

// ---------------------------------------------------------------- writer

/// A prop for [`build`]: a [`Variant::Pc64Prop`] geometry and the room
/// images it uses.
pub struct NewProp {
    pub geometry: Vec<u8>,
    /// Indices into the room's textures, one per texture slot (none: `[]`).
    pub room_textures: Vec<u32>,
}

/// Build a props section in the documented layout (fixtures). `None` is an
/// empty slot.
pub fn build(props: &[Option<NewProp>]) -> Vec<u8> {
    let table: usize = 4 + props
        .iter()
        .map(|p| match p {
            None => 4,
            Some(p) => 4 * (FIELDS + p.room_textures.len().max(1)),
        })
        .sum::<usize>();
    let mut offsets = Vec::new();
    let mut at = table.next_multiple_of(16);
    for p in props.iter().flatten() {
        offsets.push(at);
        at = (at + p.geometry.len()).next_multiple_of(16);
    }
    let mut w = Writer::new(Endian::Little);
    w.u32(props.len() as u32);
    let mut next = offsets.iter();
    for p in props {
        let Some(p) = p else {
            w.u32(NONE);
            continue;
        };
        w.u32(*next.next().unwrap() as u32);
        for _ in 1..FIELDS {
            w.u32(NONE);
        }
        if p.room_textures.is_empty() {
            w.u32(NONE);
        }
        for &t in &p.room_textures {
            w.u32(ROOM_TEXTURE | t);
        }
    }
    for p in props.iter().flatten() {
        w.pad_to(16, 0);
        w.bytes(&p.geometry);
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{self, tests::quad_object};

    fn prop(tex_count: u8, room_textures: Vec<u32>) -> NewProp {
        NewProp {
            geometry: geometry::build(
                Endian::Little,
                Variant::Pc64Prop,
                tex_count,
                &[quad_object()],
                None,
            ),
            room_textures,
        }
    }

    #[test]
    fn round_trip() {
        let bytes = build(&[Some(prop(2, vec![20, 27])), None, Some(prop(1, vec![]))]);
        let props = Props::parse(&bytes).unwrap();
        assert_eq!(props.slots.len(), 3);
        assert!(props.slots[1].is_none());
        let found: Vec<_> = props.props().map(|(i, _)| i).collect();
        assert_eq!(found, [0, 2]);
        let first = props.slots[0].as_ref().unwrap();
        assert_eq!(first.textures, [TextureRef::Room(20), TextureRef::Room(27)]);
        assert_eq!(first.fields, [NONE; 5]);
        let last = props.slots[2].as_ref().unwrap();
        assert_eq!(last.textures, [TextureRef::None]);
        let expected =
            Geometry::parse(&prop(1, vec![]).geometry, Endian::Little, Variant::Pc64Prop)
                .unwrap()
                .vertex_count();
        for (_, p) in props.props() {
            let g = p.geometry(&bytes).unwrap();
            assert_eq!(g.vertex_count(), expected);
            // Several meshes per object: the 80-byte descriptors walk right.
            let meshes = &g.objects[0].meshes;
            assert!(meshes.len() > 1);
            for m in meshes {
                assert_eq!(m.colours, vec![[0x80; 4]; m.vertex_count()]);
            }
        }
    }

    #[test]
    fn rejects_bad_input() {
        let bytes = build(&[Some(prop(1, vec![3])), None]);
        let parse_all = |b: &[u8]| -> Result<()> {
            for (_, p) in Props::parse(b)?.props() {
                p.geometry(b)?;
            }
            Ok(())
        };
        parse_all(&bytes).unwrap();
        for cut in (0..bytes.len()).step_by(13) {
            assert!(parse_all(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut huge = bytes.clone();
        huge[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Props::parse(&huge).is_err());
        let mut odd = bytes;
        odd[4] |= 4;
        assert!(Props::parse(&odd).is_err());
    }
}
