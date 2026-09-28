//! `--model <file>`: load a DMC1 model from the user's own install and show it
//! beside the arena, skinned. With `--motion <section>:<index>` it plays that
//! motion from the model's own motion banks, clocked by the sim tick (60 fps),
//! so `--screenshot --at-tick N` captures frame N exactly. `--focus` adds a
//! close-up view of the model on the right half of the window, and
//! `--grid <section>[:<first>]` shows 24 motions of a bank side by side (to
//! identify them). Nothing is cached or written.

use crate::Options;
use bevy::asset::RenderAssetUsages;
use bevy::camera::Viewport;
use bevy::camera::visibility::RenderLayers;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use dmc_formats::geometry::{Geometry, Skeleton};
use dmc_formats::model::ModelFile;
use dmc_formats::motion::{Motion, MotionBank};
use dmc_formats::{pose, texture};

pub struct AssetViewPlugin;

impl Plugin for AssetViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_model.after(crate::room_view::load_room))
            .add_systems(Update, animate);
    }
}

/// Where the preview stands, and how tall it is scaled to be (game units are
/// not yet calibrated against the sim's).
const ANCHOR: Vec3 = Vec3::new(-6.0, 0.0, 4.0);
const PREVIEW_HEIGHT: f32 = 2.0;

/// The loaded model's skeleton, joints and chosen motion.
#[derive(Component)]
pub(crate) struct Animated {
    pub skeleton: Skeleton,
    pub joints: Vec<Entity>,
    pub motion: Option<Motion>,
    /// Frame to show; `None` loops the motion on the sim tick.
    pub frame: Option<f32>,
    pub root: pose::RootMotion,
}

fn texture_materials(
    data: &[u8],
    geo: &Geometry,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Vec<Handle<StandardMaterial>> {
    // Texture slots: the first container large enough for every slot used
    // (same heuristic as `dmc model`, see COVERAGE.md).
    let need = geo
        .objects
        .iter()
        .flat_map(|o| &o.meshes)
        .map(|m| m.tex_index as usize)
        .max()
        .unwrap_or(0)
        + 1;
    let mut out = Vec::new();
    let Some((off, set)) = texture::scan(data)
        .into_iter()
        .find(|(_, s)| s.images.len() >= need)
    else {
        return out;
    };
    for img in &set.images {
        let Ok(Some(rgba)) = set.decode_rgba(&data[off..], img) else {
            out.push(materials.add(Color::srgb(0.8, 0.0, 0.8)));
            continue;
        };
        let image = Image::new(
            Extent3d {
                width: img.width as u32,
                height: img.height as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        out.push(materials.add(StandardMaterial {
            base_color_texture: Some(images.add(image)),
            alpha_mode: AlphaMode::Mask(0.5),
            double_sided: true,
            cull_mode: None,
            perceptual_roughness: 1.0,
            ..default()
        }));
    }
    out
}

/// `section:index` into the model's motion banks.
fn pick_motion(data: &[u8], model: &ModelFile, spec: &str) -> Option<Motion> {
    let (s, i) = spec.split_once(':')?;
    let (s, i): (usize, usize) = (s.parse().ok()?, i.parse().ok()?);
    let bank = MotionBank::parse(model.section(data, s)?, model.endian)
        .map_err(|e| error!("section {s} is not a motion bank: {e}"))
        .ok()?;
    let found = bank.motions.into_iter().nth(i).flatten();
    if found.is_none() {
        error!("section {s} has no motion {i}");
    }
    found
}

/// Everything shared by the instances of one model.
pub(crate) struct Parts {
    meshes: Vec<(Handle<Mesh>, Handle<StandardMaterial>)>,
    skeleton: Option<Skeleton>,
    inverse_bindposes: Option<Handle<SkinnedMeshInverseBindposes>>,
    /// Model-space offset that centres the model and puts its feet at 0.
    pub origin: Vec3,
    pub scale: f32,
    /// Render layers for the meshes (`None`: the default layer).
    layers: Option<RenderLayers>,
}

/// Motions shown by `--grid`, per screenshot page.
const GRID_COUNT: usize = 24;
const GRID_COLUMNS: usize = 6;
const GRID_SPACING: f32 = 2.6;
/// Far enough from the arena that nothing overlaps.
const GRID_ORIGIN: Vec3 = Vec3::new(0.0, 0.0, -80.0);

/// A model file read into GPU-ready parts, plus its parsed file.
pub(crate) struct LoadedModel {
    pub data: Vec<u8>,
    pub model: ModelFile,
    pub parts: Parts,
    /// Height in model units (feet to top of head in bind pose).
    pub height: f32,
    pub vertices: usize,
    pub texture_slots: usize,
}

pub(crate) fn load_parts(
    path: &std::path::Path,
    mesh_assets: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    bindposes: &mut Assets<SkinnedMeshInverseBindposes>,
) -> Option<LoadedModel> {
    let data = std::fs::read(path)
        .map_err(|e| error!("{}: {e}", path.display()))
        .ok()?;
    let (model, geo) = ModelFile::detect(&data)
        .map_err(|e| error!("{}: not a recognised model ({e})", path.display()))
        .ok()?;
    let slot_materials = texture_materials(&data, &geo, materials, images);
    let fallback = materials.add(StandardMaterial {
        base_color: Color::srgb(0.7, 0.7, 0.7),
        double_sided: true,
        cull_mode: None,
        ..default()
    });

    // Normalise size and put the feet on the floor.
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in geo
        .objects
        .iter()
        .flat_map(|o| &o.meshes)
        .flat_map(|m| &m.positions)
    {
        lo = lo.min(Vec3::from_array(*p));
        hi = hi.max(Vec3::from_array(*p));
    }
    let skeleton = geo.skeleton.clone();
    let max_joint = skeleton
        .as_ref()
        .map_or(0, |s| s.bone_count().saturating_sub(1)) as u16;
    let mut parts = Parts {
        meshes: Vec::new(),
        inverse_bindposes: skeleton.as_ref().map(|s| {
            let inverse: Vec<Mat4> = s
                .bind_positions()
                .iter()
                .map(|b| Mat4::from_translation(-Vec3::from_array(*b)))
                .collect();
            bindposes.add(SkinnedMeshInverseBindposes::from(inverse))
        }),
        skeleton,
        origin: Vec3::new((lo.x + hi.x) / 2.0, lo.y, (lo.z + hi.z) / 2.0),
        layers: None,
        scale: PREVIEW_HEIGHT / (hi.y - lo.y).max(1e-3),
    };
    for m in geo.objects.iter().flat_map(|o| &o.meshes) {
        let tris = m.triangles();
        if tris.is_empty() {
            continue;
        }
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.positions.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, m.normals.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, m.uvs.clone());
        if parts.skeleton.is_some() {
            let idx: Vec<[u16; 4]> = m
                .joints
                .iter()
                .map(|j| [0, 1, 2].map(|k| (j[k] as u16).min(max_joint)))
                .map(|[a, b, c]| [a, b, c, 0])
                .collect();
            let w: Vec<[f32; 4]> = m.weights.iter().map(|w| [w[0], w[1], w[2], 0.0]).collect();
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_JOINT_INDEX,
                VertexAttributeValues::Uint16x4(idx),
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, w);
        }
        mesh.insert_indices(Indices::U32(tris.into_iter().flatten().collect()));
        let material = slot_materials
            .get(m.tex_index as usize)
            .cloned()
            .unwrap_or_else(|| fallback.clone());
        parts.meshes.push((mesh_assets.add(mesh), material));
    }
    Some(LoadedModel {
        vertices: geo.vertex_count(),
        texture_slots: slot_materials.len(),
        height: (hi.y - lo.y).max(1e-3),
        data,
        model,
        parts,
    })
}

#[allow(clippy::too_many_arguments)]
fn load_model(
    options: Res<Options>,
    mut commands: Commands,
    mut mesh_assets: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    windows: Query<&Window>,
    room: Option<Res<crate::room_view::RoomSpot>>,
) {
    let Some(path) = &options.model else { return };
    let Some(LoadedModel {
        data,
        model,
        parts,
        vertices,
        texture_slots,
        ..
    }) = load_parts(
        path,
        &mut mesh_assets,
        &mut materials,
        &mut images,
        &mut bindposes,
    )
    else {
        return;
    };

    let size = windows
        .iter()
        .next()
        .map_or(UVec2::new(1280, 720), |w| w.physical_size());
    if let Some(spec) = options.grid.as_deref() {
        let (section, start) = spec.split_once(':').unwrap_or((spec, "0"));
        let (Ok(section), Ok(start)) = (section.parse::<usize>(), start.parse::<usize>()) else {
            return error!("--grid needs <section>[:<first motion>]");
        };
        let Some(bank) = model
            .section(&data, section)
            .and_then(|b| MotionBank::parse(b, model.endian).ok())
        else {
            return error!("section {section} is not a motion bank");
        };
        let shown: Vec<_> = bank
            .motions
            .into_iter()
            .enumerate()
            .skip(start)
            .take(GRID_COUNT)
            .collect();
        for (k, (index, motion)) in shown.iter().enumerate() {
            let (col, row) = ((k % GRID_COLUMNS) as f32, (k / GRID_COLUMNS) as f32);
            let at = GRID_ORIGIN
                + Vec3::new(
                    (col - (GRID_COLUMNS as f32 - 1.0) / 2.0) * GRID_SPACING,
                    0.0,
                    row * GRID_SPACING * 1.4,
                );
            spawn_instance(&mut commands, &parts, at, motion.clone());
            info!("grid cell {k}: motion {section}:{index}");
        }
        let rows = shown.len().div_ceil(GRID_COLUMNS) as f32;
        let centre = GRID_ORIGIN + Vec3::new(0.0, 1.0, (rows - 1.0) * GRID_SPACING * 0.7);
        commands.spawn((
            Camera3d::default(),
            Camera {
                order: 2,
                ..default()
            },
            Transform::from_translation(centre + Vec3::new(0.0, 7.0, 13.0))
                .looking_at(centre, Vec3::Y),
        ));
        commands.spawn((
            DirectionalLight {
                illuminance: 8000.0,
                ..default()
            },
            Transform::from_translation(GRID_ORIGIN + Vec3::new(3.0, 10.0, 12.0))
                .looking_at(GRID_ORIGIN, Vec3::Y),
        ));
        return;
    }

    let motion = options
        .motion
        .as_deref()
        .and_then(|spec| pick_motion(&data, &model, spec));
    info!(
        "{}: {} meshes, {} vertices, {} texture slots, motion {}",
        path.display(),
        parts.meshes.len(),
        vertices,
        texture_slots,
        motion
            .as_ref()
            .map_or("none".into(), |m| format!("{} frames", m.frames))
    );
    match room {
        // In a room: true scale, standing on its floor, on its render layer.
        Some(spot) => {
            let in_room = Parts {
                origin: Vec3::ZERO,
                scale: crate::room_view::ROOM_SCALE,
                layers: Some(crate::room_view::room_layer()),
                ..parts
            };
            let root = spawn_instance(&mut commands, &in_room, spot.position, motion);
            commands.entity(root).insert(
                Transform::from_translation(spot.position)
                    .with_rotation(spot.facing)
                    .with_scale(Vec3::splat(in_room.scale)),
            );
            return;
        }
        None => {
            spawn_instance(&mut commands, &parts, ANCHOR, motion);
        }
    }

    if options.focus {
        let centre = ANCHOR + Vec3::Y * PREVIEW_HEIGHT * 0.5;
        commands.spawn((
            Camera3d::default(),
            Camera {
                order: 1,
                viewport: Some(Viewport {
                    physical_position: UVec2::new(size.x / 2, 0),
                    physical_size: UVec2::new(size.x / 2, size.y),
                    ..default()
                }),
                ..default()
            },
            Transform::from_translation(centre + Vec3::new(0.0, 0.4, 4.6))
                .looking_at(centre, Vec3::Y),
        ));
    }
}

/// One copy of the model standing at `at`, with its own joints.
/// Returns the instance's root entity.
pub(crate) fn spawn_instance(
    commands: &mut Commands,
    parts: &Parts,
    at: Vec3,
    motion: Option<Motion>,
) -> Entity {
    let root = commands
        .spawn((
            Transform::from_translation(at - parts.origin * parts.scale)
                .with_scale(Vec3::splat(parts.scale)),
            Visibility::default(),
        ))
        .id();
    let mut joints = Vec::new();
    if let Some(s) = &parts.skeleton {
        for (i, o) in s.offsets.iter().enumerate() {
            joints.push(
                commands
                    .spawn((
                        Transform::from_translation(Vec3::from_array(*o)),
                        Visibility::default(),
                        Name::new(format!("bone{i:02}")),
                    ))
                    .id(),
            );
        }
        for (i, p) in s.parents.iter().enumerate() {
            let parent = match p {
                Some(p) if (*p as usize) < joints.len() && *p as usize != i => joints[*p as usize],
                _ => root,
            };
            commands.entity(joints[i]).insert(ChildOf(parent));
        }
    }
    for (mesh, material) in &parts.meshes {
        let mut child = commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            ChildOf(root),
        ));
        if let Some(layers) = &parts.layers {
            child.insert(layers.clone());
        }
        if let Some(inverse_bindposes) = &parts.inverse_bindposes {
            child.insert(SkinnedMesh {
                inverse_bindposes: inverse_bindposes.clone(),
                joints: joints.clone(),
            });
        }
    }
    if let Some(skeleton) = parts.skeleton.clone() {
        commands.entity(root).insert(Animated {
            skeleton,
            joints,
            motion,
            frame: None,
            root: pose::RootMotion::Apply,
        });
    }
    root
}

fn animate(
    state: Res<crate::play::SimState>,
    models: Query<&Animated>,
    mut transforms: Query<&mut Transform>,
) {
    for a in &models {
        let Some(motion) = &a.motion else { continue };
        let frame = a
            .frame
            .unwrap_or((state.sim.tick % motion.frames.max(1) as u64) as f32);
        let p = pose::sample_with(&a.skeleton, motion, frame, a.root);
        for (j, l) in a.joints.iter().zip(&p.locals) {
            if let Ok(mut t) = transforms.get_mut(*j) {
                *t = Transform {
                    translation: l.translation,
                    rotation: l.rotation,
                    scale: l.scale,
                };
            }
        }
    }
}
