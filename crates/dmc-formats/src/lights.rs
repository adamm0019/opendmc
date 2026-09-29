//! Room lighting (`.fsd` sections 4 and 5, PC build): whole light sets of
//! 0xC60 bytes, each a header (ambient, a key colour, fog) and 64 light
//! slots. Rooms carry one to eight sets per section; which set is used when
//! (mission state, geometry or characters) is not known yet.
//! Layout and evidence: `docs/formats/README.md` §4h.

use crate::bytes::{Endian, Reader, Writer};
use crate::error::{FormatError, Result};
use serde::Serialize;

pub const SECTIONS: [usize; 2] = [4, 5];

const SET: usize = 0xC60;
const HEADER: usize = 0x60;
const SLOT: usize = 0x30;
const SLOTS: usize = 64;

/// Distance fog, as stored.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fog {
    /// Fog amount at `near` and at `far`, 0–255 (provisional reading).
    pub amount: [f32; 2],
    /// Distances in room units.
    pub near: f32,
    pub far: f32,
    /// 0–255.
    pub colour: [u32; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Light {
    /// Slot index (0–63).
    pub index: usize,
    /// Low byte of the type word: 4 and 3 are most lights; 0 marks slots
    /// whose position is on the Y axis (meaning unknown).
    pub kind: u8,
    /// The type word's top byte (0x80 on some lights; meaning unknown).
    pub flags: u8,
    /// Room units.
    pub position: [f32; 3],
    /// RGB, 0–255 (the record also stores it divided by 255).
    pub colour: [f32; 3],
    /// Two falloff terms stored as 1/distance²; these are the distances in
    /// room units (0 when the term is 0). `near` < `far` for point lights.
    pub near: f32,
    pub far: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LightSet {
    /// The first 32 header bytes, as stored (not understood).
    pub head: [u32; 8],
    /// RGB, 0–255.
    pub ambient: [f32; 3],
    /// RGB, 0–255: a key or character light colour (its role is unconfirmed).
    pub key: [f32; 3],
    pub fog: Fog,
    /// The non-empty slots, in slot order.
    pub lights: Vec<Light>,
}

fn distance(k: f32) -> f32 {
    if k > 0.0 { 1.0 / k.sqrt() } else { 0.0 }
}

impl LightSet {
    /// Every set in a lighting section.
    pub fn parse_all(section: &[u8]) -> Result<Vec<LightSet>> {
        if section.is_empty() || !section.len().is_multiple_of(SET) {
            return Err(FormatError::invalid(
                "lights",
                format!(
                    "section is 0x{:x} bytes, not whole 0x{SET:x}-byte sets",
                    section.len()
                ),
            ));
        }
        section.chunks(SET).map(LightSet::parse).collect()
    }

    pub fn parse(set: &[u8]) -> Result<LightSet> {
        let r = Reader::new(set, Endian::Little);
        let mut head = [0u32; 8];
        for (i, h) in head.iter_mut().enumerate() {
            *h = r.u32(4 * i)?;
        }
        let mut lights = Vec::new();
        for index in 0..SLOTS {
            let o = HEADER + index * SLOT;
            if r.bytes(o, SLOT)?.iter().all(|&b| b == 0) {
                continue;
            }
            let ty = r.u32(o + 0x1C)?;
            lights.push(Light {
                index,
                kind: ty as u8,
                flags: (ty >> 24) as u8,
                position: r.vec3(o)?,
                colour: r.vec3(o + 0x10)?,
                near: distance(r.f32(o + 0x0C)?),
                far: distance(r.f32(o + 0x2C)?),
            });
        }
        Ok(LightSet {
            head,
            ambient: r.vec3(0x20)?,
            key: r.vec3(0x30)?,
            fog: Fog {
                amount: [r.f32(0x40)?, r.f32(0x44)?],
                near: r.f32(0x48)?,
                far: r.f32(0x4C)?,
                colour: [r.u32(0x50)?, r.u32(0x54)?, r.u32(0x58)?],
            },
            lights,
        })
    }
}

// ---------------------------------------------------------------- writer

/// A light for [`build`]: a point light of kind 4.
pub struct NewLight {
    pub position: [f32; 3],
    pub colour: [f32; 3],
    pub near: f32,
    pub far: f32,
}

/// Build one light set in the documented layout (fixtures).
pub fn build(
    ambient: [f32; 3],
    fog: ([f32; 2], f32, f32, [u32; 3]),
    lights: &[NewLight],
) -> Vec<u8> {
    let mut w = Writer::new(Endian::Little);
    w.zeros(0x20);
    w.vec3(ambient).u32(0).vec3([128.0; 3]).u32(0);
    let (amount, near, far, colour) = fog;
    w.f32(amount[0]).f32(amount[1]).f32(near).f32(far);
    w.u32(colour[0]).u32(colour[1]).u32(colour[2]).u32(0);
    for k in 0..SLOTS {
        let Some(l) = lights.get(k) else {
            w.zeros(SLOT);
            continue;
        };
        let inv = |d: f32| if d > 0.0 { 1.0 / (d * d) } else { 0.0 };
        w.vec3(l.position).f32(inv(l.near));
        w.vec3(l.colour).u32(4);
        w.vec3(l.colour.map(|c| c / 255.0)).f32(inv(l.far));
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        build(
            [25.5; 3],
            ([0.0, 220.0], 7700.0, 114_900.0, [40, 38, 33]),
            &[
                NewLight {
                    position: [-10.0, 9500.0, -4240.0],
                    colour: [255.0, 255.0, 130.0],
                    near: 2500.0,
                    far: 3260.0,
                },
                NewLight {
                    position: [-6240.0, 1580.0, -2410.0],
                    colour: [255.0, 122.0, 57.0],
                    near: 1900.0,
                    far: 2800.0,
                },
            ],
        )
    }

    #[test]
    fn round_trip() {
        let bytes = fixture();
        assert_eq!(bytes.len(), SET);
        let set = LightSet::parse(&bytes).unwrap();
        assert_eq!(set.ambient, [25.5; 3]);
        assert_eq!(set.fog.colour, [40, 38, 33]);
        assert_eq!(set.fog.far, 114_900.0);
        assert_eq!(set.lights.len(), 2);
        let l = &set.lights[1];
        assert_eq!((l.index, l.kind, l.flags), (1, 4, 0));
        assert_eq!(l.colour, [255.0, 122.0, 57.0]);
        assert!((l.near - 1900.0).abs() < 0.5 && (l.far - 2800.0).abs() < 0.5);
    }

    #[test]
    fn sections_hold_whole_sets() {
        let mut two = fixture();
        two.extend(fixture());
        assert_eq!(LightSet::parse_all(&two).unwrap().len(), 2);
        assert!(LightSet::parse_all(&two[..SET + 1]).is_err());
        assert!(LightSet::parse_all(&[]).is_err());
    }
}
