//! Dust motes for the lit looks: specks drifting in a box that travels with
//! the camera (so the density around the view never changes), faint in the
//! dark and brighter inside the look's moonlight shafts. Each speck is a
//! tiny camera-facing disc that adds its glow; brightness is set through its
//! size, so they all share one mesh and one material and draw in a batch.
//!
//! Configured by the room look's `dust` block (none, no dust). Visual only:
//! nothing in the sim reads it (ADR-011).

use crate::play::MainCamera;
use crate::render::CurrentLook;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use dmc_sim::world::ROOM_UNITS_PER_METRE;
use serde::Deserialize;

/// A look's dust.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Dust {
    pub count: u32,
    /// Edge of the box around the camera, metres.
    pub extent: f32,
    /// Speck diameter, metres.
    pub size: f32,
    /// Linear RGB.
    pub colour: [f32; 3],
    /// Luminance of a speck in the dark, cd/m².
    pub nits: f32,
    /// How much brighter a speck gets at the centre of a moonlight shaft.
    pub shaft_gain: f32,
    /// How far a speck sways, metres.
    pub sway: f32,
    /// How fast specks settle, metres per second.
    pub fall: f32,
}

impl Default for Dust {
    fn default() -> Self {
        Dust {
            count: 1500,
            extent: 14.0,
            size: 0.015,
            colour: [1.0, 0.93, 0.82],
            nits: 40.0,
            shaft_gain: 12.0,
            sway: 0.3,
            fall: 0.01,
        }
    }
}

pub struct DustPlugin;

impl Plugin for DustPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                respawn_dust.run_if(resource_changed::<CurrentLook>),
                drift_dust,
            )
                .chain(),
        );
    }
}

#[derive(Component)]
struct Mote {
    /// Resting place in the unit box.
    home: Vec3,
    phase: f32,
}

/// A moonlight shaft: a beam from `origin` along `dir`, `radius` wide.
struct Shaft {
    origin: Vec3,
    dir: Vec3,
    radius: f32,
}

#[derive(Resource)]
struct DustField {
    dust: Dust,
    shafts: Vec<Shaft>,
}

fn respawn_dust(
    mut commands: Commands,
    current: Res<CurrentLook>,
    motes: Query<Entity, With<Mote>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for e in &motes {
        commands.entity(e).despawn();
    }
    commands.remove_resource::<DustField>();
    let Some(dust) = current.look.dust.clone() else {
        return;
    };
    let shafts = current
        .look
        .rt
        .emitters
        .iter()
        .map(|e| Shaft {
            origin: current.origin + Vec3::from_array(e.position) / ROOM_UNITS_PER_METRE,
            dir: Vec3::from_array(e.facing).normalize_or(Vec3::NEG_Y),
            radius: 0.5 * e.size[0].max(e.size[1]),
        })
        .collect();
    let disc = meshes.add(Circle::new(0.5).mesh().resolution(6));
    let [r, g, b] = dust.colour;
    let glow = materials.add(StandardMaterial {
        base_color: Color::BLACK,
        emissive: LinearRgba::rgb(r, g, b) * dust.nits,
        alpha_mode: AlphaMode::Add,
        reflectance: 0.0,
        perceptual_roughness: 1.0,
        fog_enabled: false,
        ..default()
    });
    // A fixed low-discrepancy scatter, so every run places them alike.
    let mut s = 0.5_f32;
    let mut next = move || {
        s = (s + 0.618_034).fract();
        s
    };
    for i in 0..dust.count {
        let home = Vec3::new(
            (i as f32 * 0.754_877_7).fract(),
            (i as f32 * 0.569_840_3).fract(),
            next(),
        );
        commands.spawn((
            Mote {
                home,
                phase: next() * std::f32::consts::TAU,
            },
            Mesh3d(disc.clone()),
            MeshMaterial3d(glow.clone()),
            Transform::from_scale(Vec3::ZERO),
            NotShadowCaster,
        ));
    }
    commands.insert_resource(DustField { dust, shafts });
}

fn drift_dust(
    time: Res<Time>,
    field: Option<Res<DustField>>,
    cams: Query<&GlobalTransform, With<MainCamera>>,
    mut motes: Query<(&Mote, &mut Transform)>,
) {
    let (Some(field), Ok(cam)) = (field, cams.single()) else {
        return;
    };
    let d = &field.dust;
    let t = time.elapsed_secs();
    let (_, facing, eye) = cam.to_scale_rotation_translation();
    let e = d.extent;
    let corner = eye - Vec3::splat(e / 2.0);
    for (m, mut tf) in &mut motes {
        let p = m.phase;
        let sway = Vec3::new(
            (t * 0.13 + p).sin(),
            (t * 0.09 + p * 1.7).sin() * 0.5,
            (t * 0.11 + p * 2.3).cos(),
        ) * d.sway
            - Vec3::Y * d.fall * t;
        // Fixed in the world apart from the sway, wrapped into the box.
        let world = m.home * e + sway;
        let pos = corner + (world - corner).rem_euclid(Vec3::splat(e));
        let lit = field
            .shafts
            .iter()
            .map(|s| {
                let along = (pos - s.origin).dot(s.dir);
                if along <= 0.0 {
                    return 0.0;
                }
                let off = (pos - s.origin - s.dir * along).length();
                1.0 - (off / s.radius).clamp(0.0, 1.0)
            })
            .fold(0.0, f32::max);
        let twinkle = 0.55 + 0.45 * (t * 1.3 + p * 7.0).sin();
        // Fade out at the box's edge and right in front of the lens.
        let dist = pos.distance(eye);
        let fade = (1.0 - dist / (e / 2.0))
            .clamp(0.0, 1.0)
            .min((dist - 0.3).clamp(0.0, 1.0));
        let brightness = (1.0 + d.shaft_gain * lit * lit) * twinkle * fade;
        *tf = Transform {
            translation: pos,
            rotation: facing,
            scale: Vec3::splat(d.size * brightness.sqrt()),
        };
    }
}
