//! Room cameras (`.fsd` section 2, PC build, version 2 records): an
//! activation zone per camera, the eye, a look-at offset, a field of view,
//! and optional rails: one the eye moves along and one that follows the
//! player's path and sets how far along its rail the eye is.
//! Layout and evidence: `docs/formats/README.md` §4e. What each field means
//! is partly inferred; the uncertain ones are kept raw.

use crate::bytes::{Endian, Reader, Writer};
use crate::error::{FormatError, Result};
use serde::Serialize;

pub const SECTION: usize = 2;
pub const VERSION_TAG: &[u8; 4] = b"ver\x02";

const HEADER: usize = 16;
const RECORD: usize = 0x220;
const SLOT_PARAMS: usize = 128;
/// `flags` bit: the record ends with the two rails.
pub const HAS_RAILS: u32 = 0x10;
const MAX_RAIL_POINTS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Camera {
    /// Two opposite corners of the activation zone (`A`, then `B`).
    pub zone_corners: [[f32; 3]; 2],
    /// Outward unit normals of the zone's six faces: the first three faces
    /// pass through `B`, the last three through `A`.
    pub zone_normals: [[f32; 3]; 6],
    /// Added to the player's position to get the point looked at; usually
    /// `(0, 900, 0)`, the player's head height.
    pub look_offset: [f32; 3],
    /// The eye (+0x90): where a camera without rails views from, and the
    /// last point of the eye rail of one with rails.
    pub eye: [f32; 3],
    /// The last point of the track rail (+0xA0).
    pub track_end: [f32; 3],
    /// An unexplained point (+0xC0).
    pub extra_point: [f32; 3],
    /// 128 bytes at +0xD0: all `0x01` in half the cameras, else `0x00`,
    /// `0xFF` and percent-like values (10–200). Not per-object occluder
    /// flags (§4e); meaning unknown.
    pub slot_params: Vec<u8>,
    /// Eight type bytes at +0x150. Bytes 4 and 5 are the point counts of
    /// the eye and track rails (also set on some records without rails);
    /// the rest are unknown.
    pub kind: [u8; 8],
    /// Flags at +0x158; see [`HAS_RAILS`].
    pub flags: u32,
    /// Field of view in degrees, probably (55 in most records).
    pub fov: f32,
    /// The stored arc lengths of the eye and track rails.
    pub rail_lengths: [f32; 2],
    /// Four i32 at +0x190, usually −1 (links to other cameras?).
    pub links: [i32; 4],
    /// Eye rail: the points the eye moves along, each with a fourth
    /// component (−0.65 to 1.12, median ≈ 0; meaning unknown).
    pub eye_rail: Vec<[f32; 4]>,
    /// Track rail, one point per eye-rail point: runs near the player's path
    /// at about chest height. The player's nearest point on it, as a fraction
    /// of its length, is (probably) how far along the eye rail the eye is.
    pub track_rail: Vec<[f32; 3]>,
}

impl Camera {
    /// Is `p` inside the activation zone (with `tolerance` room units of
    /// slack on every face)?
    pub fn zone_contains(&self, p: [f32; 3], tolerance: f32) -> bool {
        let [a, b] = self.zone_corners;
        self.zone_normals.iter().enumerate().all(|(i, n)| {
            let origin = if i < 3 { b } else { a };
            let d: f32 = (0..3).map(|k| n[k] * (p[k] - origin[k])).sum();
            d <= tolerance
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Cameras {
    pub cameras: Vec<Camera>,
}

impl Cameras {
    pub fn parse(section: &[u8]) -> Result<Self> {
        let r = Reader::new(section, Endian::Little);
        if r.bytes(8, 4)? != VERSION_TAG {
            return Err(FormatError::invalid(
                "cameras",
                "no `ver\\x02` tag (older record layout, not supported yet)",
            ));
        }
        let count = r.u32(0)? as usize;
        if count > section.len() / RECORD {
            return Err(FormatError::invalid("cameras", format!("{count} records")));
        }
        let mut at = HEADER;
        let mut cameras = Vec::with_capacity(count);
        for i in 0..count {
            let camera = parse_record(&r, at)
                .map_err(|e| FormatError::invalid("cameras", format!("record {i}: {e}")))?;
            at += RECORD + 16 * (camera.eye_rail.len() + camera.track_rail.len());
            cameras.push(camera);
        }
        if at != section.len() {
            return Err(FormatError::invalid(
                "cameras",
                format!("records end at 0x{at:x}, section is 0x{:x}", section.len()),
            ));
        }
        Ok(Cameras { cameras })
    }
}

fn parse_record(r: &Reader, o: usize) -> Result<Camera> {
    r.bytes(o, RECORD)?;
    let vec3 = |at: usize| r.vec3(o + at);
    let flags = r.u32(o + 0x158)?;
    let points = r.u32(o + 0x15C)? as usize;
    let (eye_rail, track_rail) = if flags & HAS_RAILS != 0 {
        if !points.is_multiple_of(2) || points > MAX_RAIL_POINTS {
            return Err(FormatError::invalid(
                "camera",
                format!("{points} rail points"),
            ));
        }
        let base = o + RECORD;
        r.bytes(base, 16 * points)?;
        let half = points / 2;
        let eye = (0..half)
            .map(|k| {
                let p = r.vec3(base + 16 * k)?;
                Ok([p[0], p[1], p[2], r.f32(base + 16 * k + 12)?])
            })
            .collect::<Result<_>>()?;
        let track = (half..points)
            .map(|k| r.vec3(base + 16 * k))
            .collect::<Result<_>>()?;
        (eye, track)
    } else {
        (Vec::new(), Vec::new())
    };
    Ok(Camera {
        zone_corners: [vec3(0x00)?, vec3(0x10)?],
        zone_normals: [
            vec3(0x20)?,
            vec3(0x30)?,
            vec3(0x40)?,
            vec3(0x50)?,
            vec3(0x60)?,
            vec3(0x70)?,
        ],
        look_offset: vec3(0x80)?,
        eye: vec3(0x90)?,
        track_end: vec3(0xA0)?,
        extra_point: vec3(0xC0)?,
        slot_params: r.bytes(o + 0xD0, SLOT_PARAMS)?.to_vec(),
        kind: r.bytes(o + 0x150, 8)?.try_into().unwrap(),
        flags,
        fov: r.f32(o + 0x180)?,
        rail_lengths: [r.f32(o + 0x168)?, r.f32(o + 0x16C)?],
        links: std::array::from_fn(|k| r.u32(o + 0x190 + 4 * k).unwrap_or(0) as i32),
        eye_rail,
        track_rail,
    })
}

// ---------------------------------------------------------------- writer

/// A camera for [`build`]: an axis-aligned zone between `min` and `max`,
/// an eye (the last rail point if there are rails), and optional rails.
pub struct NewCamera {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub fov: f32,
    pub eye: [f32; 3],
    /// `(eye, track)` point pairs.
    pub rails: Vec<([f32; 3], [f32; 3])>,
}

/// Build a camera section in the documented layout (fixtures).
pub fn build(cameras: &[NewCamera]) -> Vec<u8> {
    let mut w = Writer::new(Endian::Little);
    w.u32(cameras.len() as u32).u32(0).bytes(VERSION_TAG).u32(0);
    let point = |w: &mut Writer, p: [f32; 3], last: f32| {
        w.vec3(p).f32(last);
    };
    for c in cameras {
        let start = w.pos();
        point(&mut w, c.min, 1.0);
        point(&mut w, c.max, 1.0);
        // Faces through B (= max): +X, +Z, +Y; through A (= min): -Y, -Z, -X.
        for n in [
            [1., 0., 0.],
            [0., 0., 1.],
            [0., 1., 0.],
            [0., -1., 0.],
            [0., 0., -1.],
            [-1., 0., 0.],
        ] {
            point(&mut w, n, 0.0);
        }
        point(&mut w, [0., 900., 0.], 1.0);
        let (eye, track_end) = c.rails.last().copied().unwrap_or((c.eye, [0.0; 3]));
        point(&mut w, eye, 1.0);
        point(&mut w, track_end, 1.0);
        w.zeros(16);
        point(&mut w, [0.0; 3], 1.0);
        w.bytes(&[1; SLOT_PARAMS]);
        w.zeros(8);
        let flags = if c.rails.is_empty() { 0 } else { HAS_RAILS };
        w.u32(flags).u32(2 * c.rails.len() as u32);
        let length = |pts: Vec<[f32; 3]>| -> f32 {
            pts.windows(2)
                .map(|s| {
                    (0..3)
                        .map(|k| (s[1][k] - s[0][k]).powi(2))
                        .sum::<f32>()
                        .sqrt()
                })
                .sum()
        };
        w.zeros(8)
            .f32(length(c.rails.iter().map(|r| r.0).collect()))
            .f32(length(c.rails.iter().map(|r| r.1).collect()));
        w.zeros(16);
        w.f32(c.fov).zeros(12);
        for _ in 0..4 {
            w.u32(u32::MAX);
        }
        w.zeros(start + RECORD - w.pos());
        for (eye, _) in &c.rails {
            point(&mut w, *eye, 0.0);
        }
        for (_, track) in &c.rails {
            point(&mut w, *track, 1.0);
        }
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two() -> Vec<NewCamera> {
        vec![
            NewCamera {
                min: [0., 0., 0.],
                max: [1000., 1500., 2000.],
                fov: 55.0,
                eye: [500., 2500., -800.],
                rails: vec![],
            },
            NewCamera {
                min: [-500., 0., 0.],
                max: [0., 1500., 1000.],
                fov: 42.5,
                eye: [0.0; 3],
                rails: vec![
                    ([0., 2000., -1000.], [0., 700., 0.]),
                    ([0., 2000., -600.], [0., 700., 300.]),
                    ([300., 2000., -600.], [400., 700., 300.]),
                ],
            },
        ]
    }

    #[test]
    fn round_trip() {
        let bytes = build(&two());
        let c = Cameras::parse(&bytes).unwrap();
        assert_eq!(c.cameras.len(), 2);
        let fixed = &c.cameras[0];
        assert_eq!((fixed.fov, fixed.flags & HAS_RAILS), (55.0, 0));
        assert!(fixed.eye_rail.is_empty());
        assert_eq!(fixed.eye, [500., 2500., -800.]);
        assert_eq!(fixed.look_offset, [0., 900., 0.]);
        assert_eq!(fixed.links, [-1; 4]);
        assert!(fixed.zone_contains([500., 100., 1000.], 0.0));
        assert!(!fixed.zone_contains([500., 100., 2100.], 0.0));
        assert!(!fixed.zone_contains([-1., 100., 1000.], 0.0));
        let rail = &c.cameras[1];
        assert_eq!((rail.eye_rail.len(), rail.track_rail.len()), (3, 3));
        assert_eq!(rail.track_rail[2], [400., 700., 300.]);
        assert_eq!(rail.eye, [300., 2000., -600.]);
        assert_eq!(rail.track_end, [400., 700., 300.]);
        assert_eq!(rail.rail_lengths, [700.0, 700.0]);
        assert_eq!(rail.slot_params.len(), 128);
    }

    #[test]
    fn rejects_bad_input() {
        let bytes = build(&two());
        for cut in (0..bytes.len()).step_by(37) {
            assert!(Cameras::parse(&bytes[..cut]).is_err());
        }
        let mut old = bytes.clone();
        old[8..12].copy_from_slice(&[0; 4]);
        assert!(Cameras::parse(&old).is_err(), "version 1 is not supported");
        let mut odd = bytes;
        let second = HEADER + RECORD;
        odd[second + 0x15C] = 5;
        assert!(Cameras::parse(&odd).is_err());
    }
}
