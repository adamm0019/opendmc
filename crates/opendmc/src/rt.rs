//! `--look rt` (trial, feature `rt`): real-time ray-traced direct and indirect
//! lighting with Bevy's experimental Solari, built straight from the room
//! file with no baking:
//! - the room's meshes join the ray-tracing scene as they are drawn;
//! - Solari only takes directional lights and emissive meshes as light
//!   sources, so each light of the room's first light set (`.fsd` section 4,
//!   kinds 3 and 4) becomes a small glowing sphere of its colour, and the
//!   look adds its own emitters (moonlight panels).
//!
//! Needs hardware ray tracing. Nothing here is required by the other looks
//! (docs/REMAKE.md §3: ray tracing stays optional).

use crate::render::{RtLook, srgb_to_linear};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::solari::prelude::{RaytracingMesh3d, SolariPlugins};
use dmc_formats::lights::LightSet;
use dmc_formats::room::Room;
use dmc_sim::world::ROOM_UNITS_PER_METRE;

/// Solari, and the cut-out workaround below.
pub struct RtPlugin;

impl Plugin for RtPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "cutout.wgsl");
        app.add_plugins((
            SolariPlugins,
            MaterialPlugin::<ExtendedMaterial<StandardMaterial, Cutout>>::default(),
        ))
        .add_systems(Update, split_cutouts);
    }
}

/// An alpha-tested material that renders as opaque. Bevy 0.18.1 queues
/// alpha-masked deferred meshes into the forward pass as well (the mask
/// branch lacks the opaque branch's deferred check), and there they draw
/// black over Solari's lighting. This one tests alpha in its own G-buffer
/// shader instead.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct Cutout {}

impl MaterialExtension for Cutout {
    fn deferred_fragment_shader() -> ShaderRef {
        "embedded://opendmc/cutout.wgsl".into()
    }
}

/// Moves each alpha-masked ray-traced mesh onto [`Cutout`] for drawing, and
/// leaves an undrawn proxy with its [`StandardMaterial`] in the ray-tracing
/// scene (Solari reads only that material, and treats every triangle there
/// as opaque).
#[allow(clippy::type_complexity)]
fn split_cutouts(
    mut commands: Commands,
    added: Query<
        (
            Entity,
            &RaytracingMesh3d,
            &MeshMaterial3d<StandardMaterial>,
            &Transform,
            &ChildOf,
        ),
        (Added<RaytracingMesh3d>, With<Mesh3d>),
    >,
    standard: Res<Assets<StandardMaterial>>,
    mut cutouts: ResMut<Assets<ExtendedMaterial<StandardMaterial, Cutout>>>,
) {
    for (entity, traced, material, transform, parent) in &added {
        let Some(base) = standard.get(&material.0) else {
            continue;
        };
        if !matches!(base.alpha_mode, AlphaMode::Mask(_)) {
            continue;
        }
        let drawn = cutouts.add(ExtendedMaterial {
            base: StandardMaterial {
                alpha_mode: AlphaMode::Opaque,
                ..base.clone()
            },
            extension: Cutout {},
        });
        commands
            .entity(entity)
            .remove::<(RaytracingMesh3d, MeshMaterial3d<StandardMaterial>)>()
            .insert(MeshMaterial3d(drawn));
        commands.spawn((
            traced.clone(),
            material.clone(),
            *transform,
            ChildOf(parent.parent()),
        ));
    }
}

/// Blender watts per square metre of a light's near falloff distance, the
/// same starting brightness the Blender kit gives the original rig.
const WATTS_PER_NEAR_M2: f32 = 40.0;
/// Lumens per watt, as Blender's glTF exporter converts (authoring parity).
const LUMENS_PER_WATT: f32 = 683.0;

/// Solari needs tangents on every ray-traced mesh.
pub fn prepare_mesh(mesh: &mut Mesh) {
    if !mesh.contains_attribute(Mesh::ATTRIBUTE_TANGENT)
        && let Err(e) = mesh.generate_tangents()
    {
        warn!("rt: no tangents ({e}); zero tangents instead");
        let n = mesh.count_vertices();
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, vec![[1.0, 0.0, 0.0, 1.0]; n]);
    }
}

pub fn raytraced(handle: &Handle<Mesh>) -> RaytracingMesh3d {
    RaytracingMesh3d(handle.clone())
}

/// Luminance (cd/m²) of a sphere of `radius` metres that emits `lumens`.
fn sphere_nits(lumens: f32, radius: f32) -> f32 {
    lumens / (4.0 * std::f32::consts::PI * std::f32::consts::PI * radius * radius)
}

/// The room's own lights, plus the look's emitters, as glowing meshes under
/// `root` (a metre-scaled room root at the origin).
pub fn spawn_emitters(
    room: &Room,
    data: &[u8],
    look: &RtLook,
    root: Entity,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let mut sphere = Sphere::new(look.light_radius).mesh().build();
    prepare_mesh(&mut sphere);
    let sphere = meshes.add(sphere);
    let s = 1.0 / ROOM_UNITS_PER_METRE;
    let mut count = 0;
    if let Some(set) = room
        .light_sets(data, 4)
        .ok()
        .and_then(|sets| sets.into_iter().next())
    {
        for l in set.lights.iter().filter(|l| matches!(l.kind, 3 | 4)) {
            count += spawn_light(&set, l, look, s, &sphere, root, commands, materials) as usize;
        }
    }
    let mut quad = Rectangle::new(1.0, 1.0).mesh().build();
    prepare_mesh(&mut quad);
    let quad = meshes.add(quad);
    for e in &look.emitters {
        let p = Vec3::from_array(e.position) * s;
        let facing = Vec3::from_array(e.facing).normalize_or(Vec3::NEG_Y);
        let colour = LinearRgba::rgb(e.colour[0], e.colour[1], e.colour[2]) * e.nits;
        commands.spawn((
            Mesh3d(quad.clone()),
            raytraced(&quad),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::BLACK,
                emissive: colour,
                unlit: false,
                double_sided: true,
                cull_mode: None,
                ..default()
            })),
            Transform::from_translation(p)
                .looking_to(
                    facing,
                    if facing.y.abs() > 0.9 {
                        Vec3::Z
                    } else {
                        Vec3::Y
                    },
                )
                .with_scale(Vec3::new(e.size[0], e.size[1], 1.0)),
            ChildOf(root),
        ));
        count += 1;
    }
    info!("rt: {count} emitters");
}

#[allow(clippy::too_many_arguments)]
fn spawn_light(
    _set: &LightSet,
    l: &dmc_formats::lights::Light,
    look: &RtLook,
    s: f32,
    sphere: &Handle<Mesh>,
    root: Entity,
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
) -> bool {
    let near_m = (l.near * s).max(0.5);
    let watts = WATTS_PER_NEAR_M2 * near_m * near_m * look.light_scale;
    if watts <= 0.0 {
        return false;
    }
    let [r, g, b] = l.colour.map(|c| srgb_to_linear(c / 255.0));
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let d = look.desaturate;
    let (r, g, b) = (
        r + (y - r) * d,
        (g + (y - g) * d) * look.green,
        b + (y - b) * d,
    );
    let nits = sphere_nits(watts * LUMENS_PER_WATT, look.light_radius);
    // Ray traced only, not drawn: the original rig's lights are invisible
    // too, and a glowing ball reads as an object.
    commands.spawn((
        raytraced(sphere),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::BLACK,
            emissive: LinearRgba::rgb(r, g, b) * nits,
            ..default()
        })),
        Transform::from_translation(Vec3::from_array(l.position) * s),
        ChildOf(root),
    ));
    true
}
