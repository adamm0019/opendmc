//! Sampling a motion into a skeleton pose. Channel meanings are in
//! `docs/formats/README.md` §6–7; the conventions that are our inference
//! rather than documented fact are marked `speculative` there and named
//! below, so they can be flipped if captures disagree.
//!
//! Bind pose has no rotations: every bone sits at its parent plus its bind
//! offset, and the mesh is stored in that pose in model space. A skinning
//! matrix is therefore `global(bone) * translation(-bind_position(bone))`.

use crate::geometry::Skeleton;
use crate::motion::{IkRole, Motion, Target, ik_role};
use glam::{EulerRot, Mat4, Quat, Vec3};

/// One bone's transform relative to its parent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Local {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Local {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// A sampled pose: local and model-space transforms per bone.
#[derive(Debug, Clone)]
pub struct Pose {
    pub locals: Vec<Local>,
    pub globals: Vec<Mat4>,
}

impl Pose {
    /// The bind pose.
    pub fn bind(skeleton: &Skeleton) -> Self {
        let locals = skeleton
            .offsets
            .iter()
            .map(|o| Local {
                translation: Vec3::from_array(*o),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            })
            .collect::<Vec<_>>();
        let globals = globals(skeleton, &locals);
        Pose { locals, globals }
    }

    /// Skinning matrices (`global * inverse bind`).
    pub fn skinning(&self, skeleton: &Skeleton) -> Vec<Mat4> {
        skeleton
            .bind_positions()
            .iter()
            .zip(&self.globals)
            .map(|(b, g)| *g * Mat4::from_translation(-Vec3::from_array(*b)))
            .collect()
    }
}

/// Euler angles applied X, then Y, then Z (`speculative`: the notes give the
/// order but not whether the axes are intrinsic).
pub fn euler(x: f32, y: f32, z: f32) -> Quat {
    Quat::from_euler(EulerRot::ZYX, z, y, x)
}

fn parent(skeleton: &Skeleton, i: usize) -> Option<usize> {
    skeleton.parents[i]
        .map(|p| p as usize)
        .filter(|&p| p < skeleton.parents.len() && p != i)
}

fn globals(skeleton: &Skeleton, locals: &[Local]) -> Vec<Mat4> {
    let n = locals.len();
    let mut out = vec![Mat4::IDENTITY; n];
    let mut done = vec![false; n];
    // Parents normally precede children; loop until settled to be safe.
    for _ in 0..n {
        let mut progress = false;
        for i in 0..n {
            if done[i] {
                continue;
            }
            match parent(skeleton, i) {
                None => out[i] = locals[i].matrix(),
                Some(p) if done[p] => out[i] = out[p] * locals[i].matrix(),
                Some(_) => continue,
            }
            done[i] = true;
            progress = true;
        }
        if !progress {
            break;
        }
    }
    out
}

/// The shortest rotation taking direction `from` onto `to`.
fn arc(from: Vec3, to: Vec3) -> Quat {
    match (from.try_normalize(), to.try_normalize()) {
        (Some(a), Some(b)) => Quat::from_rotation_arc(a, b),
        _ => Quat::IDENTITY,
    }
}

/// Knee direction of a chain from its root flag (`speculative`: the notes say
/// odd and even flags bend opposite ways, not which is which).
fn bend_sign(root_flag: u8) -> f32 {
    if root_flag % 2 == 1 { 1.0 } else { -1.0 }
}

/// What to do with a motion's root motion (channel 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootMotion {
    /// Add it to bone 0, as the motion was authored (viewers, export).
    Apply,
    /// Drop it: the character is moved by the simulation instead.
    InPlace,
}

/// Sample `motion` at `frame` (60 fps frames, fractional allowed).
pub fn sample(skeleton: &Skeleton, motion: &Motion, frame: f32) -> Pose {
    sample_with(skeleton, motion, frame, RootMotion::Apply)
}

/// [`sample`], choosing what happens to root motion.
pub fn sample_with(skeleton: &Skeleton, motion: &Motion, frame: f32, root: RootMotion) -> Pose {
    let n = skeleton.bone_count();
    let mut locals = Pose::bind(skeleton).locals;
    let mut euler_xyz = vec![None::<[f32; 3]>; n];
    let mut ik_targets = vec![None::<Vec3>; n];
    let mut ik_hinges = vec![None::<Vec3>; n];

    for (c, channel) in motion.channels.iter().enumerate() {
        if channel.is_empty() {
            continue;
        }
        let v = channel.sample(frame);
        let or = |i: usize, d: f32| v[i].unwrap_or(d);
        match motion.target(c, skeleton) {
            Some(Target::RootMotion) if n > 0 && root == RootMotion::Apply => {
                locals[0].translation += Vec3::new(or(0, 0.0), or(1, 0.0), or(2, 0.0));
            }
            Some(Target::Rotation(b)) if (b as usize) < n => {
                euler_xyz[b as usize] = Some([or(0, 0.0), or(1, 0.0), or(2, 0.0)]);
            }
            Some(Target::Translation(b)) if (b as usize) < n => {
                let t = &mut locals[b as usize].translation;
                *t = Vec3::new(or(0, t.x), or(1, t.y), or(2, t.z));
            }
            Some(Target::Scale(b)) if (b as usize) < n => {
                locals[b as usize].scale = Vec3::new(or(0, 1.0), or(1, 1.0), or(2, 1.0));
            }
            Some(Target::IkHinge(b)) if (b as usize) < n => {
                ik_hinges[b as usize] = Some(Vec3::new(or(0, 0.0), or(1, 0.0), or(2, 0.0)));
            }
            Some(Target::IkTarget(b)) if (b as usize) < n => {
                ik_targets[b as usize] = Some(Vec3::new(or(0, 0.0), or(1, 0.0), or(2, 0.0)));
            }
            _ => {}
        }
    }
    for (b, e) in euler_xyz.iter().enumerate() {
        if let Some([x, y, z]) = *e
            && ik_role(skeleton, b) != IkRole::End
        {
            locals[b].rotation = euler(x, y, z);
        }
    }
    let mut globals = globals(skeleton, &locals);

    // Two-bone chains: root r, middle r+1, end r+2.
    for r in 0..n {
        if ik_role(skeleton, r) != IkRole::Root {
            continue;
        }
        let (m, e) = (r + 1, r + 2);
        let Some(target) = ik_targets[e] else {
            continue;
        };
        let parent_rot = parent(skeleton, r).map_or(Quat::IDENTITY, |p| {
            globals[p].to_scale_rotation_translation().1
        });
        let hinge = parent_rot * ik_hinges[r].unwrap_or(Vec3::X);
        let root_pos = globals[r].w_axis.truncate();
        let (upper, lower) = (
            locals[m].translation.length(),
            locals[e].translation.length(),
        );
        let solved = crate::ik::solve(
            root_pos.to_array(),
            target.to_array(),
            upper,
            lower,
            hinge.to_array(),
            bend_sign(skeleton.ik_flags[r]),
        );
        let (mid, end) = (
            Vec3::from_array(solved.middle),
            Vec3::from_array(solved.end),
        );

        // Aim the root at the middle joint and the middle at the end.
        let (_, root_rot, _) = globals[r].to_scale_rotation_translation();
        let root_rot = arc(root_rot * locals[m].translation, mid - root_pos) * root_rot;
        globals[r] = Mat4::from_rotation_translation(root_rot, root_pos);
        let mid_rot = arc(root_rot * locals[e].translation, end - mid) * root_rot;
        globals[m] = Mat4::from_rotation_translation(mid_rot, mid);
        // The end bone's rotation channel is already in model space.
        let end_rot = euler_xyz[e].map_or(mid_rot, |[x, y, z]| euler(x, y, z));
        globals[e] = Mat4::from_rotation_translation(end_rot, end);

        // Re-derive locals for the chain, then update everything below it.
        for b in [r, m, e] {
            let pg = parent(skeleton, b).map_or(Mat4::IDENTITY, |p| globals[p]);
            let (s, rot, t) = (pg.inverse() * globals[b]).to_scale_rotation_translation();
            locals[b] = Local {
                translation: t,
                rotation: rot,
                scale: s * locals[b].scale,
            };
        }
        globals = self::globals(skeleton, &locals);
    }
    Pose { locals, globals }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{Channel, Key, Track};

    fn skel() -> Skeleton {
        // 0 root, 1 hip (IK root), 2 knee, 3 foot, 4 toe.
        Skeleton {
            parents: vec![None, Some(0), Some(1), Some(2), Some(3)],
            ik_flags: vec![0, 3, 1, 1, 0],
            offsets: vec![
                [0.0, 1.0, 0.0],
                [0.1, 0.0, 0.0],
                [0.0, -0.5, 0.0],
                [0.0, -0.5, 0.0],
                [0.0, 0.0, 0.1],
            ],
            lengths: vec![0.0; 5],
        }
    }

    fn constant(v: [f32; 3]) -> Channel {
        let t = |x: f32| Track {
            keys: vec![Key {
                frame: 0,
                value: x,
                in_tangent: 0.0,
                out_tangent: 0.0,
            }],
        };
        Channel {
            tracks: [t(v[0]), t(v[1]), t(v[2])],
        }
    }

    fn motion(ids: Vec<u8>, channels: Vec<Channel>) -> Motion {
        Motion {
            frames: 1,
            channel_ids: ids,
            channels,
            frame_words: Vec::new(),
            event_offset: 0,
        }
    }

    #[test]
    fn empty_motion_is_bind_pose() {
        let s = skel();
        let p = sample(&s, &motion(vec![], vec![Channel::default(); 2]), 0.0);
        for (g, b) in p.globals.iter().zip(s.bind_positions()) {
            assert!(g.w_axis.truncate().abs_diff_eq(Vec3::from_array(b), 1e-6));
        }
        for m in p.skinning(&s) {
            assert!(m.abs_diff_eq(Mat4::IDENTITY, 1e-6));
        }
    }

    #[test]
    fn rotation_and_root_motion() {
        let s = skel();
        let quarter = std::f32::consts::FRAC_PI_2;
        let m = motion(
            vec![0],
            vec![
                Channel::default(),
                constant([0.0, 0.0, 2.0]),
                constant([0.0, quarter, 0.0]),
            ],
        );
        let p = sample(&s, &m, 0.0);
        assert!(
            p.globals[0]
                .w_axis
                .truncate()
                .abs_diff_eq(Vec3::new(0.0, 1.0, 2.0), 1e-5)
        );
        // Hip offset +X rotated a quarter turn about Y points along -Z.
        let hip = p.globals[1].w_axis.truncate() - p.globals[0].w_axis.truncate();
        assert!(hip.abs_diff_eq(Vec3::new(0.0, 0.0, -0.1), 1e-5), "{hip}");
    }

    #[test]
    fn in_place_drops_root_motion() {
        let s = skel();
        let m = motion(vec![], vec![Channel::default(), constant([0.0, 0.0, 2.0])]);
        let p = sample_with(&s, &m, 0.0, RootMotion::InPlace);
        assert!(
            p.globals[0]
                .w_axis
                .truncate()
                .abs_diff_eq(Vec3::new(0.0, 1.0, 0.0), 1e-6)
        );
    }

    #[test]
    fn ik_puts_the_foot_on_the_target_and_keeps_lengths() {
        let s = skel();
        let target = [0.1, 0.3, 0.2];
        let m = motion(
            vec![3 | 0x80, 1 | 0x80],
            vec![
                Channel::default(),
                Channel::default(),
                constant(target),
                constant([1.0, 0.0, 0.0]),
            ],
        );
        let p = sample(&s, &m, 0.0);
        let pos = |b: usize| p.globals[b].w_axis.truncate();
        assert!(
            pos(3).abs_diff_eq(Vec3::from_array(target), 1e-4),
            "{}",
            pos(3)
        );
        assert!(((pos(2) - pos(1)).length() - 0.5).abs() < 1e-4);
        assert!(((pos(3) - pos(2)).length() - 0.5).abs() < 1e-4);
        // The knee bends in the plane normal to the hinge (X).
        assert!((pos(2).x - pos(1).x).abs() < 1e-4);
        // The toe follows the foot.
        assert!(((pos(4) - pos(3)).length() - 0.1).abs() < 1e-4);
    }
}
