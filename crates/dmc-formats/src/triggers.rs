//! Room triggers (`.fsd` section 3, PC build): 64 fixed records, each a
//! volume and a kind. Doors among them name the room they lead to and where
//! the player arrives there.
//! Layout and evidence: `docs/formats/README.md` §4g. Kinds other than
//! doors are kept raw.

use crate::bytes::{Endian, Reader, Writer};
use crate::error::{FormatError, Result};
use serde::Serialize;

pub const SECTION: usize = 3;

const RECORD: usize = 0xB0;
const RECORDS: usize = 64;
/// The eight vectors come first; the head follows.
const HEAD: usize = 0x80;
const HEAD_LEN: usize = 0x30;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Volume {
    /// A convex box as in the camera zones (§4e): corners `A`, `B` and the
    /// outward normals of its faces, the first three through `B`, the last
    /// three through `A`.
    Box {
        corners: [[f32; 3]; 2],
        normals: [[f32; 3]; 6],
    },
    /// A centre and a size, e.g. `(500, 500, 0)` (the normal slots hold
    /// `(0, 0, 0, 1)`). Probably a cylinder of that radius and height.
    Round { centre: [f32; 3], size: [f32; 3] },
}

impl Volume {
    /// Is `p` inside, with `tolerance` units of slack? `Round` is taken as a
    /// vertical cylinder standing on its centre (unconfirmed).
    pub fn contains(&self, p: [f32; 3], tolerance: f32) -> bool {
        match self {
            Volume::Box { corners, normals } => normals.iter().enumerate().all(|(i, n)| {
                let o = corners[if i < 3 { 1 } else { 0 }];
                (0..3).map(|k| n[k] * (p[k] - o[k])).sum::<f32>() <= tolerance
            }),
            Volume::Round { centre, size } => {
                let (dx, dz) = (p[0] - centre[0], p[2] - centre[2]);
                (dx * dx + dz * dz).sqrt() <= size[0] + tolerance
                    && p[1] >= centre[1] - tolerance
                    && p[1] <= centre[1] + size[1] + tolerance
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Trigger {
    /// Record index (0–63).
    pub index: usize,
    /// Head byte 0: 1, 2, 3 or 7 (meaning unknown; doors occur in all four).
    pub kind: u8,
    /// Head byte 1: 0 for doors, 1-14 for other triggers.
    pub sub: u8,
    /// Head bytes 2 and 3 (flag-like: `0x11`, `0x21`, `0x91`, `0xA1`, `0xC1`).
    pub flags: [u8; 2],
    pub volume: Volume,
    /// Head +8: a point (for doors, where the player arrives in `room`).
    pub point: [f32; 3],
    /// Head +24: a room id (`r%03x`), 0 when none.
    pub room: u16,
    /// Head +26 (0, 2 or 3 on doors; meaning unknown).
    pub extra: u8,
    /// The whole 48-byte head as stored.
    pub head: Vec<u8>,
}

/// A trigger that sends the player to another room.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Door {
    pub room: u16,
    /// Where the player arrives, in the target room's coordinates.
    pub arrival: [f32; 3],
}

impl Trigger {
    /// A door is a record with sub-kind 0 and a room id, whatever its kind.
    pub fn door(&self) -> Option<Door> {
        (self.sub == 0 && self.room != 0).then_some(Door {
            room: self.room,
            arrival: self.point,
        })
    }
}

/// The file stem of room `id`, e.g. `r101` (`r00d` for 0x0D).
pub fn room_name(id: u16) -> String {
    format!("r{id:03x}")
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Triggers {
    /// The non-empty records (kind ≠ 0) in record order.
    pub triggers: Vec<Trigger>,
}

impl Triggers {
    pub fn parse(section: &[u8]) -> Result<Self> {
        if section.len() != RECORDS * RECORD {
            return Err(FormatError::invalid(
                "triggers",
                format!(
                    "section is 0x{:x} bytes, not 0x{:x}",
                    section.len(),
                    RECORDS * RECORD
                ),
            ));
        }
        let r = Reader::new(section, Endian::Little);
        let mut triggers = Vec::new();
        for index in 0..RECORDS {
            let o = index * RECORD;
            let head = r.bytes(o + HEAD, HEAD_LEN)?;
            if head[0] == 0 {
                continue;
            }
            let v = |k: usize| r.vec3(o + 16 * k);
            let w = |k: usize| r.f32(o + 16 * k + 12);
            let volume = if w(2)? == 1.0 {
                Volume::Round {
                    centre: v(0)?,
                    size: v(1)?,
                }
            } else {
                Volume::Box {
                    corners: [v(0)?, v(1)?],
                    normals: [v(2)?, v(3)?, v(4)?, v(5)?, v(6)?, v(7)?],
                }
            };
            triggers.push(Trigger {
                index,
                kind: head[0],
                sub: head[1],
                flags: [head[2], head[3]],
                volume,
                point: r.vec3(o + HEAD + 8)?,
                room: r.u16(o + HEAD + 24)?,
                extra: head[26],
                head: head.to_vec(),
            });
        }
        Ok(Triggers { triggers })
    }

    pub fn doors(&self) -> impl Iterator<Item = (&Trigger, Door)> {
        self.triggers
            .iter()
            .filter_map(|t| t.door().map(|d| (t, d)))
    }
}

// ---------------------------------------------------------------- writer

/// A door for [`build`]: an axis-aligned box between `min` and `max`.
pub struct NewDoor {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub room: u16,
    pub arrival: [f32; 3],
}

/// Build a triggers section in the documented layout (fixtures): the doors
/// in the first records, the rest empty.
pub fn build(doors: &[NewDoor]) -> Vec<u8> {
    let mut w = Writer::new(Endian::Little);
    for k in 0..RECORDS {
        let Some(d) = doors.get(k) else {
            w.zeros(HEAD).bytes(&[0, 0, 0, 0x11]).zeros(HEAD_LEN - 4);
            continue;
        };
        let a = [d.max[0], d.min[1], d.max[2]];
        let b = [d.min[0], d.max[1], d.min[2]];
        w.vec3(a).f32(1.0).vec3(b).f32(1.0);
        // Through B: -Z, -X, +Y; through A: -Y, +X, +Z.
        for n in [
            [0., 0., -1.],
            [-1., 0., 0.],
            [0., 1., 0.],
            [0., -1., 0.],
            [1., 0., 0.],
            [0., 0., 1.],
        ] {
            w.vec3(n).f32(0.0);
        }
        w.bytes(&[3, 0, 0, 0xC1]).u32(0).vec3(d.arrival).u32(0);
        w.u16(d.room).zeros(HEAD_LEN - 26);
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        build(&[
            NewDoor {
                min: [0., 0., 0.],
                max: [100., 500., 50.],
                room: 0x101,
                arrival: [2140., 0., -6325.],
            },
            NewDoor {
                min: [-100., 0., 0.],
                max: [0., 500., 50.],
                room: 0x11b,
                arrival: [1., 2., 3.],
            },
        ])
    }

    #[test]
    fn round_trip() {
        let t = Triggers::parse(&fixture()).unwrap();
        assert_eq!(t.triggers.len(), 2);
        let doors: Vec<_> = t.doors().map(|(_, d)| d).collect();
        assert_eq!(
            doors[0],
            Door {
                room: 0x101,
                arrival: [2140., 0., -6325.]
            }
        );
        assert_eq!(room_name(doors[1].room), "r11b");
        assert_eq!(room_name(0xd), "r00d");
        let v = &t.triggers[0].volume;
        assert!(v.contains([50., 10., 25.], 0.0));
        assert!(!v.contains([150., 10., 25.], 0.0));
        assert!(!v.contains([50., 600., 25.], 0.0));
        assert!(!t.triggers[1].volume.contains([50., 10., 25.], 0.0));
    }

    #[test]
    fn round_volumes_and_bad_sizes() {
        let mut bytes = fixture();
        // Turn record 0 into a round volume: centre, size, (0, 0, 0, 1)s.
        let mut w = Writer::new(Endian::Little);
        w.vec3([0., 0., 0.])
            .f32(1.0)
            .vec3([500., 500., 0.])
            .f32(1.0);
        for _ in 0..6 {
            w.vec3([0.; 3]).f32(1.0);
        }
        bytes[..HEAD].copy_from_slice(&w.finish());
        let t = Triggers::parse(&bytes).unwrap();
        assert!(matches!(t.triggers[0].volume, Volume::Round { .. }));
        assert!(t.triggers[0].volume.contains([300., 100., 300.], 0.0));
        assert!(!t.triggers[0].volume.contains([400., 100., 400.], 0.0));
        assert!(Triggers::parse(&bytes[..bytes.len() - 1]).is_err());
        assert!(Triggers::parse(&[]).is_err());
    }
}
