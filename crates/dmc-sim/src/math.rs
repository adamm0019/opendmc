//! A deliberately small vector type. The sim avoids SIMD-backed math crates
//! and transcendental functions so results are bit-identical across builds.

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl V3 {
    pub const ZERO: V3 = V3::new(0.0, 0.0, 0.0);
    pub const UP: V3 = V3::new(0.0, 1.0, 0.0);
    pub const FORWARD: V3 = V3::new(0.0, 0.0, 1.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    pub fn flat(self) -> V3 {
        V3::new(self.x, 0.0, self.z)
    }

    pub fn normalize_or(self, fallback: V3) -> V3 {
        let len = self.length();
        if len > 1e-6 {
            self * (1.0 / len)
        } else {
            fallback
        }
    }

    /// Right-hand side vector for a Y-up, horizontal `forward`.
    pub fn right_of(forward: V3) -> V3 {
        V3::new(forward.z, 0.0, -forward.x)
    }

    /// Transform a local offset (x right, y up, z forward) into world space.
    pub fn local_to_world(forward: V3, local: V3) -> V3 {
        V3::right_of(forward) * local.x + V3::UP * local.y + forward * local.z
    }

    pub fn bits(self) -> [u32; 3] {
        [self.x.to_bits(), self.y.to_bits(), self.z.to_bits()]
    }
}

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        V3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl AddAssign for V3 {
    fn add_assign(&mut self, o: V3) {
        *self = *self + o;
    }
}

impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        V3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f32> for V3 {
    type Output = V3;
    fn mul(self, s: f32) -> V3 {
        V3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        V3::new(-self.x, -self.y, -self.z)
    }
}

/// Distance from `p` to the segment `a..b`.
pub fn point_segment_distance(p: V3, a: V3, b: V3) -> f32 {
    let ab = b - a;
    let len2 = ab.dot(ab);
    let t = if len2 > 0.0 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p - (a + ab * t)).length()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_to_world_respects_facing() {
        let east = V3::new(1.0, 0.0, 0.0);
        let p = V3::local_to_world(east, V3::new(0.0, 1.0, 2.0));
        assert_eq!(p, V3::new(2.0, 1.0, 0.0));
        let r = V3::local_to_world(V3::FORWARD, V3::new(1.0, 0.0, 0.0));
        assert_eq!(r, V3::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn segment_distance() {
        let d = point_segment_distance(V3::new(1.0, 5.0, 0.0), V3::ZERO, V3::new(0.0, 2.0, 0.0));
        assert!((d - (1.0f32 + 9.0).sqrt()).abs() < 1e-6);
    }
}
