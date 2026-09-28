//! Two-bone IK as used by DMC1's leg motions: the motion stores where the
//! chain end should be and the knee hinge axis; the middle joint is recovered
//! with the law of cosines. See `docs/formats/README.md` §7.

use crate::geometry::{add, cross, dot, sub};

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn normalize(a: [f32; 3]) -> Option<[f32; 3]> {
    let len = dot(a, a).sqrt();
    (len > 1e-8).then(|| scale(a, 1.0 / len))
}

/// Result of a solve, in the same space as the inputs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwoBone {
    pub middle: [f32; 3],
    pub end: [f32; 3],
    /// `false` when the target was out of reach and the chain was straightened.
    pub reached: bool,
}

/// Place the middle joint of a `root → middle → end` chain.
///
/// * `upper`, `lower`: segment lengths.
/// * `hinge`: the knee's rotation axis; the joint bends in the plane normal to it.
/// * `bend_sign`: `+1.0` or `-1.0`; mirrored legs bend opposite ways (odd vs
///   even root flag in the data).
pub fn solve(
    root: [f32; 3],
    target: [f32; 3],
    upper: f32,
    lower: f32,
    hinge: [f32; 3],
    bend_sign: f32,
) -> TwoBone {
    let to_target = sub(target, root);
    let dist = dot(to_target, to_target).sqrt();
    let Some(dir) = normalize(to_target) else {
        // Target on the root: fold the chain along any direction.
        let middle = add(root, [0.0, upper, 0.0]);
        return TwoBone {
            middle,
            end: root,
            reached: (upper - lower).abs() < 1e-5,
        };
    };
    let reach = upper + lower;
    if dist >= reach {
        let middle = add(root, scale(dir, upper));
        return TwoBone {
            middle,
            end: add(root, scale(dir, reach)),
            reached: dist - reach < 1e-5,
        };
    }
    let min_reach = (upper - lower).abs();
    let d = dist.max(min_reach + 1e-6);
    // Angle at the root between the root→target line and the upper segment.
    let cos_a = ((upper * upper + d * d - lower * lower) / (2.0 * upper * d)).clamp(-1.0, 1.0);
    let sin_a = (1.0 - cos_a * cos_a).sqrt();
    let bend = normalize(cross(hinge, dir))
        .or_else(|| normalize(cross([0.0, 0.0, 1.0], dir)))
        .or_else(|| normalize(cross([1.0, 0.0, 0.0], dir)))
        .unwrap_or([0.0, 1.0, 0.0]);
    let middle = add(
        root,
        add(
            scale(dir, upper * cos_a),
            scale(bend, upper * sin_a * bend_sign),
        ),
    );
    TwoBone {
        middle,
        end: target,
        reached: dist >= min_reach,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
        let d = sub(a, b);
        dot(d, d).sqrt()
    }

    #[test]
    fn keeps_segment_lengths_and_reaches() {
        let root = [0.0, 1.0, 0.0];
        let target = [0.0, 0.2, 0.3];
        let s = solve(root, target, 0.5, 0.5, [1.0, 0.0, 0.0], 1.0);
        assert!(s.reached);
        assert!((dist(root, s.middle) - 0.5).abs() < 1e-4);
        assert!((dist(s.middle, target) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn middle_joint_lies_in_the_hinge_plane() {
        let hinge = [1.0, 0.0, 0.0];
        let s = solve([0.0; 3], [0.0, -0.8, 0.1], 0.5, 0.5, hinge, 1.0);
        assert!(dot(s.middle, hinge).abs() < 1e-5);
    }

    #[test]
    fn bend_sign_mirrors_the_knee() {
        let a = solve([0.0; 3], [0.0, -0.8, 0.0], 0.5, 0.5, [1.0, 0.0, 0.0], 1.0);
        let b = solve([0.0; 3], [0.0, -0.8, 0.0], 0.5, 0.5, [1.0, 0.0, 0.0], -1.0);
        assert!((a.middle[2] + b.middle[2]).abs() < 1e-5);
        assert!(a.middle[2].abs() > 0.1);
    }

    #[test]
    fn unreachable_straightens() {
        let s = solve([0.0; 3], [0.0, -5.0, 0.0], 0.5, 0.5, [1.0, 0.0, 0.0], 1.0);
        assert!(!s.reached);
        assert!((s.middle[1] + 0.5).abs() < 1e-5);
        assert!((s.end[1] + 1.0).abs() < 1e-5);
    }
}
