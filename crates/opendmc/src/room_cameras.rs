//! A camera director for a room's own cameras (`.fsd` section 2): which
//! camera is live for the player's position, and where it views from.
//! Pure logic, no Bevy types, so it can be unit-tested.
//!
//! The reading is the one that keeps the player in view across the data
//! (`docs/formats/README.md` §4e), not yet confirmed in the running game, so
//! it is kept in one place:
//! - the live camera is the one whose zone holds the player (the current one
//!   is kept while the player is still inside it, so overlaps don't flicker);
//! - a rail camera puts its eye on the eye rail at the same fraction of its
//!   length as the player's nearest point on the track rail;
//! - a camera without rails views from its stored eye;
//! - every camera aims at the player's head (`player + look_offset`).

use dmc_formats::camera::{Camera, HAS_RAILS};
use dmc_sim::V3;

/// Where a camera without a stored eye views from, relative to the player,
/// in metres.
const FALLBACK_OFFSET: V3 = V3::new(0.0, 4.0, -7.0);

#[derive(Debug, Clone)]
pub struct RoomCamera {
    corners: [V3; 2],
    normals: [V3; 6],
    look_offset: V3,
    fov_degrees: f32,
    fixed_eye: Option<V3>,
    /// `(eye, track)` rails.
    rails: Option<(Vec<V3>, Vec<V3>)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub eye: V3,
    pub target: V3,
    pub fov_degrees: f32,
}

impl RoomCamera {
    /// Convert a parsed camera, scaling room units by `scale`.
    pub fn new(c: &Camera, scale: f32) -> Self {
        let p = |v: [f32; 3]| V3::new(v[0] * scale, v[1] * scale, v[2] * scale);
        let n = |v: [f32; 3]| V3::new(v[0], v[1], v[2]);
        let rails = (c.flags & HAS_RAILS != 0 && c.eye_rail.len() >= 2).then(|| {
            (
                c.eye_rail.iter().map(|q| p([q[0], q[1], q[2]])).collect(),
                c.track_rail.iter().map(|&q| p(q)).collect(),
            )
        });
        RoomCamera {
            corners: [p(c.zone_corners[0]), p(c.zone_corners[1])],
            normals: c.zone_normals.map(n),
            look_offset: p(c.look_offset),
            fov_degrees: if c.fov > 1.0 && c.fov < 170.0 {
                c.fov
            } else {
                55.0
            },
            fixed_eye: (c.eye != [0.0; 3]).then(|| p(c.eye)),
            rails,
        }
    }

    /// The middle of the zone.
    pub fn zone_centre(&self) -> V3 {
        (self.corners[0] + self.corners[1]) * 0.5
    }

    pub fn contains(&self, q: V3) -> bool {
        self.normals.iter().enumerate().all(|(i, n)| {
            let origin = self.corners[if i < 3 { 1 } else { 0 }];
            n.dot(q - origin) <= 1e-3
        })
    }

    pub fn view(&self, player: V3) -> View {
        let target = player + self.look_offset;
        let eye = match &self.rails {
            Some((eye, track)) => at_fraction(eye, nearest_fraction(track, target)),
            None => self.fixed_eye.unwrap_or(player + FALLBACK_OFFSET),
        };
        View {
            eye,
            target,
            fov_degrees: self.fov_degrees,
        }
    }
}

fn length(path: &[V3]) -> f32 {
    path.windows(2).map(|s| (s[1] - s[0]).length()).sum()
}

/// The fraction of the way along `path` (by length) of its point nearest `q`.
fn nearest_fraction(path: &[V3], q: V3) -> f32 {
    let total = length(path);
    if total <= 0.0 {
        return 0.0;
    }
    let (mut best, mut best_d, mut walked) = (0.0, f32::INFINITY, 0.0);
    for s in path.windows(2) {
        let ab = s[1] - s[0];
        let len2 = ab.dot(ab);
        let t = if len2 > 0.0 {
            ((q - s[0]).dot(ab) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = (s[0] + ab * t - q).length();
        let seg = len2.sqrt();
        if d < best_d {
            best_d = d;
            best = (walked + seg * t) / total;
        }
        walked += seg;
    }
    best
}

/// The point `f` of the way along `path` by length.
fn at_fraction(path: &[V3], f: f32) -> V3 {
    let total = length(path);
    if path.len() < 2 || total <= 0.0 {
        return path.first().copied().unwrap_or(V3::ZERO);
    }
    let mut left = f.clamp(0.0, 1.0) * total;
    for s in path.windows(2) {
        let seg = (s[1] - s[0]).length();
        if left <= seg {
            return s[0] + (s[1] - s[0]) * if seg > 0.0 { left / seg } else { 0.0 };
        }
        left -= seg;
    }
    *path.last().unwrap()
}

#[derive(Debug, Clone, Default)]
pub struct RoomDirector {
    pub cameras: Vec<RoomCamera>,
}

impl RoomDirector {
    pub fn new(cameras: &[Camera], scale: f32) -> Self {
        RoomDirector {
            cameras: cameras.iter().map(|c| RoomCamera::new(c, scale)).collect(),
        }
    }

    /// Keep `current` while the player is inside it; otherwise the first
    /// camera whose zone holds the player; otherwise stay on `current`.
    pub fn select(&self, current: Option<usize>, player: V3) -> Option<usize> {
        let probe = player + V3::new(0.0, 0.1, 0.0);
        if let Some(c) = current
            && self.cameras.get(c).is_some_and(|z| z.contains(probe))
        {
            return Some(c);
        }
        self.cameras
            .iter()
            .position(|z| z.contains(probe))
            .or(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmc_formats::camera::{Cameras, NewCamera, build};

    fn cameras() -> Vec<Camera> {
        Cameras::parse(&build(&[
            NewCamera {
                min: [0., 0., 0.],
                max: [1000., 1500., 1000.],
                fov: 55.0,
                eye: [500., 2500., -800.],
                rails: vec![],
            },
            NewCamera {
                min: [1000., 0., 0.],
                max: [2000., 1500., 1000.],
                fov: 40.0,
                eye: [0.0; 3],
                rails: vec![
                    ([1000., 2000., -500.], [1000., 900., 500.]),
                    ([2000., 2000., -500.], [2000., 900., 500.]),
                ],
            },
            NewCamera {
                min: [0., 0., 1000.],
                max: [1000., 1500., 2000.],
                fov: 55.0,
                eye: [0.0; 3],
                rails: vec![],
            },
        ]))
        .unwrap()
        .cameras
    }

    #[test]
    fn zones_pick_the_camera_with_hysteresis() {
        let d = RoomDirector::new(&cameras(), 0.01);
        assert_eq!(d.select(None, V3::new(5.0, 0.0, 5.0)), Some(0));
        assert_eq!(d.select(None, V3::new(15.0, 0.0, 5.0)), Some(1));
        // x = 10 is on both zones' shared face: stay where we are.
        assert_eq!(d.select(Some(1), V3::new(10.0, 0.0, 5.0)), Some(1));
        assert_eq!(d.select(Some(0), V3::new(10.0, 0.0, 5.0)), Some(0));
        // Outside every zone: keep the last camera.
        assert_eq!(d.select(Some(1), V3::new(50.0, 0.0, 5.0)), Some(1));
        assert_eq!(d.select(None, V3::new(50.0, 0.0, 5.0)), None);
    }

    #[test]
    fn rail_cameras_track_along_their_rail() {
        let d = RoomDirector::new(&cameras(), 0.01);
        let cam = &d.cameras[1];
        let v = cam.view(V3::new(15.0, 0.0, 5.0));
        assert_eq!(v.fov_degrees, 40.0);
        assert!((v.eye.x - 15.0).abs() < 1e-4, "{v:?}");
        assert!((v.eye.y - 20.0).abs() < 1e-4 && (v.eye.z + 5.0).abs() < 1e-4);
        assert_eq!(v.target, V3::new(15.0, 9.0, 5.0));
        // Past the rail's end the eye stops at the end.
        assert!((cam.view(V3::new(30.0, 0.0, 5.0)).eye.x - 20.0).abs() < 1e-4);
    }

    #[test]
    fn fixed_cameras_turn_to_follow() {
        let d = RoomDirector::new(&cameras(), 0.01);
        let v = d.cameras[0].view(V3::new(5.0, 0.0, 5.0));
        assert_eq!(v.eye, V3::new(5.0, 25.0, -8.0));
        assert_eq!(v.target, V3::new(5.0, 9.0, 5.0));
        let w = d.cameras[0].view(V3::new(2.0, 0.0, 8.0));
        assert_eq!((w.eye, w.target), (v.eye, V3::new(2.0, 9.0, 8.0)));
    }

    #[test]
    fn cameras_without_an_eye_follow_from_behind() {
        let d = RoomDirector::new(&cameras(), 0.01);
        let v = d.cameras[2].view(V3::new(5.0, 0.0, 15.0));
        assert_eq!(v.eye, V3::new(5.0, 0.0, 15.0) + FALLBACK_OFFSET);
    }
}
