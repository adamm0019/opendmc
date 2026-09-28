//! Camera-relative movement under fixed cameras.
//!
//! With fixed cameras, "up on the stick" changes meaning at every cut. The
//! convention (DMC1 and most fixed-camera games) is to keep the *old*
//! camera's basis while the stick stays held in roughly the same direction,
//! and to adopt the new camera once the stick is released or turned. This runs
//! before the sim, so tapes record the resulting world-space direction.

use crate::math::V3;

/// How far (cosine) the stick may turn before the held basis is dropped.
const HOLD_COS: f32 = 0.707;
const DEADZONE: f32 = 0.2;

#[derive(Debug, Clone, Default)]
pub struct CameraRelative {
    /// Camera forward (flat, normalised) currently used for conversion.
    basis: Option<V3>,
    /// Stick direction at the moment the held basis was captured.
    held_stick: Option<(f32, f32)>,
}

impl CameraRelative {
    /// `stick` is (x right, y up) in −1..1; `camera_forward` is the active
    /// camera's view direction. Returns a world-space XZ direction.
    pub fn world_direction(&mut self, stick: (f32, f32), camera_forward: V3) -> V3 {
        let cam = camera_forward.flat().normalize_or(V3::FORWARD);
        let mag = (stick.0 * stick.0 + stick.1 * stick.1).sqrt();
        if mag < DEADZONE {
            self.basis = Some(cam);
            self.held_stick = None;
            return V3::ZERO;
        }
        let dir = (stick.0 / mag, stick.1 / mag);
        let basis = match (self.basis, self.held_stick) {
            // Camera changed while the stick is held: keep the old basis as
            // long as the stick keeps pointing the same way.
            (Some(old), Some(h)) if old != cam && h.0 * dir.0 + h.1 * dir.1 >= HOLD_COS => old,
            _ => {
                self.basis = Some(cam);
                self.held_stick = Some(dir);
                cam
            }
        };
        let out = V3::right_of(basis) * stick.0 + basis * stick.1;
        out * (1.0 / mag.max(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V3, b: V3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn up_means_camera_forward() {
        let mut c = CameraRelative::default();
        let d = c.world_direction((0.0, 1.0), V3::new(1.0, -0.5, 0.0));
        assert!(close(d, V3::new(1.0, 0.0, 0.0)));
    }

    #[test]
    fn held_direction_survives_a_camera_cut() {
        let mut c = CameraRelative::default();
        let a = V3::FORWARD;
        let b = V3::new(-1.0, 0.0, 0.0); // cut to a camera facing the other way
        let before = c.world_direction((0.0, 1.0), a);
        let after = c.world_direction((0.0, 1.0), b);
        assert!(
            close(before, after),
            "keeps running the same way through the cut"
        );
        // Turning the stick well away drops the old basis.
        let turned = c.world_direction((1.0, 0.0), b);
        assert!(close(turned, V3::right_of(b)));
    }

    #[test]
    fn release_adopts_the_new_camera() {
        let mut c = CameraRelative::default();
        c.world_direction((0.0, 1.0), V3::FORWARD);
        let b = V3::new(1.0, 0.0, 0.0);
        c.world_direction((0.0, 1.0), b);
        c.world_direction((0.0, 0.0), b);
        assert!(close(c.world_direction((0.0, 1.0), b), b));
    }
}
