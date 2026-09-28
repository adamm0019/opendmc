//! `--avatar <pl00.pld>`: the player is drawn as the model from the user's own
//! install instead of the graybox capsule, animated from the sim state:
//! locomotion picks idle/walk/run by speed, air plays the jump motion, and an
//! attack plays its move's `animation` (a motion reference in the move data)
//! at the attack's frame. Motions play in place: the sim moves the actor.
//! The state-to-motion map is `data/animations/pl00.ron`.

use crate::Options;
use crate::asset_view::{Animated, load_parts, spawn_instance};
use crate::play::{ActorVisual, SimState};
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use dmc_formats::motion::{Motion, MotionBank};
use dmc_formats::pose::RootMotion;
use dmc_sim::actor::State;
use serde::Deserialize;
use std::collections::HashMap;

pub const PL00_ANIMATIONS: &str = include_str!("../../../data/animations/pl00.ron");

pub struct AvatarPlugin;

impl Plugin for AvatarPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, attach_avatar)
            .add_systems(Update, (drive_avatar, follow_player));
    }
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // `source` documents provenance (ADR-004); tests read it.
pub struct Clip {
    pub motion: String,
    pub source: String,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // `model` names the file the map is for; tests read it.
pub struct AnimationMap {
    pub model: String,
    pub idle: Clip,
    pub walk: Clip,
    pub run: Clip,
    pub air: Clip,
    pub walk_speed: f32,
    pub run_speed: f32,
}

impl AnimationMap {
    pub fn from_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }
}

/// Parse `"section:index"` or a move's `"pl00/bank6_012"`.
pub fn motion_ref(s: &str) -> Option<(usize, usize)> {
    if let Some((sec, idx)) = s.split_once(':') {
        return Some((sec.parse().ok()?, idx.parse().ok()?));
    }
    let bank = s.rsplit('/').next()?.strip_prefix("bank")?;
    let (sec, idx) = bank.split_once('_')?;
    Some((sec.parse().ok()?, idx.parse().ok()?))
}

/// `--focus` with an avatar: a close-up on the right half that follows it.
#[derive(Component)]
struct FollowCamera;

#[derive(Component)]
struct Avatar {
    actor: usize,
    banks: HashMap<usize, Vec<Option<Motion>>>,
    map: AnimationMap,
    /// The clip playing and the tick it started, to loop it from frame 0.
    playing: Option<((usize, usize), u64)>,
}

impl Avatar {
    fn motion(&self, r: (usize, usize)) -> Option<&Motion> {
        self.banks.get(&r.0)?.get(r.1)?.as_ref()
    }
}

#[allow(clippy::too_many_arguments)]
fn attach_avatar(
    options: Res<Options>,
    state: Res<SimState>,
    mut commands: Commands,
    mut mesh_assets: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    visuals: Query<(Entity, &ActorVisual, &Children)>,
) {
    let Some(path) = &options.avatar else { return };
    let map = match AnimationMap::from_ron(PL00_ANIMATIONS) {
        Ok(m) => m,
        Err(e) => return error!("data/animations/pl00.ron: {e}"),
    };
    let Some(loaded) = load_parts(
        path,
        &mut mesh_assets,
        &mut materials,
        &mut images,
        &mut bindposes,
    ) else {
        return;
    };
    let mut banks = HashMap::new();
    for sec in &loaded.model.sections {
        if let Some(bytes) = loaded.model.section(&loaded.data, sec.index)
            && let Ok(bank) = MotionBank::parse(bytes, loaded.model.endian)
        {
            banks.insert(sec.index, bank.motions);
        }
    }
    let player = dmc_sim::sim::PLAYER;
    let Some((entity, _, children)) = visuals.iter().find(|(_, v, _)| v.0 == player) else {
        return;
    };
    // The capsule and facing marker give way to the model.
    for c in children.iter() {
        commands.entity(c).despawn();
    }
    let height = state.sim.actors[player].height;
    let mut parts = loaded.parts;
    parts.origin = Vec3::ZERO;
    parts.scale = height / loaded.height;
    let root = spawn_instance(&mut commands, &parts, Vec3::ZERO, None);
    commands.entity(root).insert(ChildOf(entity));
    commands.entity(root).insert(Avatar {
        actor: player,
        banks,
        map,
        playing: None,
    });
    info!("avatar: {} as the player", path.display());
    if options.focus {
        commands.spawn((
            Camera3d::default(),
            Camera {
                order: 1,
                viewport: Some(bevy::camera::Viewport {
                    physical_position: UVec2::new(640, 0),
                    physical_size: UVec2::new(640, 720),
                    ..default()
                }),
                ..default()
            },
            Transform::default(),
            FollowCamera,
        ));
    }
}

fn follow_player(
    avatars: Query<&GlobalTransform, With<Avatar>>,
    mut cams: Query<&mut Transform, With<FollowCamera>>,
) {
    let Some(at) = avatars.iter().next().map(|g| g.translation()) else {
        return;
    };
    for mut t in &mut cams {
        let centre = at + Vec3::Y * 1.0;
        *t = Transform::from_translation(centre + Vec3::new(1.5, 1.0, -4.0))
            .looking_at(centre, Vec3::Y);
    }
}

fn drive_avatar(state: Res<SimState>, mut q: Query<(&mut Avatar, &mut Animated)>) {
    for (mut avatar, mut anim) in &mut q {
        let actor = &state.sim.actors[avatar.actor];
        let speed = (actor.vel.x * actor.vel.x + actor.vel.z * actor.vel.z).sqrt();
        let tick = state.sim.tick;
        // Attacks play their move's motion at the attack's own frame.
        let attack = match &actor.state {
            State::Attack(a) => state
                .sim
                .move_def(avatar.actor, a.move_index)
                .animation
                .as_deref()
                .and_then(motion_ref)
                .map(|r| (r, a.frame as f32)),
            _ => None,
        };
        let (clip, frame) = match attack {
            Some((r, f)) => (r, Some(f)),
            None => {
                let m = &avatar.map;
                let clip = match actor.state {
                    State::Air => &m.air,
                    _ if speed >= m.run_speed => &m.run,
                    _ if speed >= m.walk_speed => &m.walk,
                    _ => &m.idle,
                };
                let Some(r) = motion_ref(&clip.motion) else {
                    continue;
                };
                (r, None)
            }
        };
        let started = match avatar.playing {
            Some((c, t)) if c == clip => t,
            _ => {
                avatar.playing = Some((clip, tick));
                tick
            }
        };
        let Some(motion) = avatar.motion(clip).cloned() else {
            continue;
        };
        let frames = motion.frames.max(1) as u64;
        anim.frame = Some(frame.unwrap_or(((tick - started) % frames) as f32));
        anim.root = RootMotion::InPlace;
        anim.motion = Some(motion);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_map_parses_and_refs_resolve() {
        let m = AnimationMap::from_ron(PL00_ANIMATIONS).unwrap();
        assert_eq!(m.model, "pl00");
        for c in [&m.idle, &m.walk, &m.run, &m.air] {
            assert!(motion_ref(&c.motion).is_some(), "{}", c.motion);
            assert!(!c.source.is_empty());
        }
        assert!(m.walk_speed < m.run_speed);
        assert_eq!(motion_ref("pl00/bank6_012"), Some((6, 12)));
        assert_eq!(motion_ref("6:74"), Some((6, 74)));
        assert_eq!(motion_ref("nonsense"), None);
    }
}
