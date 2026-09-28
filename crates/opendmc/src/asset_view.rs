//! `--model <file>`: load a DMC1 model from the user's own install and show it
//! beside the arena, skinned. With `--motion <section>:<index>` it plays that
//! motion from the model's own motion banks, clocked by the sim tick (60 fps),
//! so `--screenshot --at-tick N` captures frame N exactly. `--focus` adds a
//! close-up view of the model on the right half of the window. Nothing is
//! cached or written.

use crate::Options;
use bevy::asset::RenderAssetUsages;
use bevy::camera::Viewport;
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
        app.add_systems(Startup, load_model)
            .add_systems(Update, animate);
    }
}

/// Where the preview stands, and how tall it is scaled to be (game units are
/// not yet calibrated against the sim's).
const ANCHOR: Vec3 = Vec3::new(-6.0, 0.0, 4.0);
const PREVIEW_HEIGHT: f32 = 2.0;

/// The loaded model's skeleton, joints and chosen motion.
#[derive(Component)]
struct Animated {
    skeleton: Skeleton,
    joints: Vec<Entity>,
    motion: Option<Motion>,
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

#[allow(clippy::too_many_arguments)]
fn load_model(
    options: Res<Options>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    windows: Query<&Window>,
) {
    let Some(path) = &options.model else { return };
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => return error!("{}: {e}", path.display()),
    };
    let (model, geo) = match ModelFile::detect(&data) {
        Ok(found) => found,
        Err(e) => return error!("{}: not a recognised model ({e})", path.display()),
    };
    let slot_materials = texture_materials(&data, &geo, &mut materials, &mut images);
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
    let scale = PREVIEW_HEIGHT / (hi.y - lo.y).max(1e-3);
    let root = commands
        .spawn((
            Transform::from_translation(
                ANCHOR - Vec3::new((lo.x + hi.x) / 2.0, lo.y, (lo.z + hi.z) / 2.0) * scale,
            )
            .with_scale(Vec3::splat(scale)),
            Visibility::default(),
        ))
        .id();

    // Joints: one entity per bone, parented like the skeleton.
    let skeleton = geo.skeleton.clone();
    let mut joints = Vec::new();
    if let Some(s) = &skeleton {
        for (i, o) in s.offsets.iter().enumerate() {
            let j = commands
                .spawn((
                    Transform::from_translation(Vec3::from_array(*o)),
                    Visibility::default(),
                    Name::new(format!("bone{i:02}")),
                ))
                .id();
            joints.push(j);
        }
        for (i, p) in s.parents.iter().enumerate() {
            let parent = match p {
                Some(p) if (*p as usize) < joints.len() && *p as usize != i => joints[*p as usize],
                _ => root,
            };
            commands.entity(joints[i]).insert(ChildOf(parent));
        }
    }
    let skin = skeleton.as_ref().map(|s| {
        let inverse: Vec<Mat4> = s
            .bind_positions()
            .iter()
            .map(|b| Mat4::from_translation(-Vec3::from_array(*b)))
            .collect();
        SkinnedMesh {
            inverse_bindposes: bindposes.add(SkinnedMeshInverseBindposes::from(inverse)),
            joints: joints.clone(),
        }
    });

    let max_joint = joints.len().saturating_sub(1) as u16;
    let mut count = 0;
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
        if skin.is_some() {
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
        let mut child = commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material),
            ChildOf(root),
        ));
        if let Some(skin) = &skin {
            child.insert(skin.clone());
        }
        count += 1;
    }

    let motion = options
        .motion
        .as_deref()
        .and_then(|spec| pick_motion(&data, &model, spec));
    info!(
        "{}: {count} meshes, {} vertices, {} texture slots, motion {}",
        path.display(),
        geo.vertex_count(),
        slot_materials.len(),
        motion
            .as_ref()
            .map_or("none".into(), |m| format!("{} frames", m.frames))
    );
    if let Some(skeleton) = skeleton {
        commands.entity(root).insert(Animated {
            skeleton,
            joints,
            motion,
        });
    }

    if options.focus {
        let size = windows
            .iter()
            .next()
            .map_or(UVec2::new(1280, 720), |w| w.physical_size());
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

fn animate(
    state: Res<crate::play::SimState>,
    models: Query<&Animated>,
    mut transforms: Query<&mut Transform>,
) {
    for a in &models {
        let Some(motion) = &a.motion else { continue };
        let frame = (state.sim.tick % motion.frames.max(1) as u64) as f32;
        let p = pose::sample(&a.skeleton, motion, frame);
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
