//! `--model <file>`: load a DMC1 model from the user's own install and show it
//! in bind pose beside the arena. First step of Phase 2/3 (asset pipeline →
//! room viewer). Nothing is cached or written.

use crate::Options;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use dmc_formats::model::ModelFile;
use dmc_formats::texture;

pub struct AssetViewPlugin;

impl Plugin for AssetViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_model);
    }
}

/// Where the preview stands, and how tall it is scaled to be (game units are
/// not yet calibrated against the sim's).
const ANCHOR: Vec3 = Vec3::new(-6.0, 0.0, 4.0);
const PREVIEW_HEIGHT: f32 = 2.0;

fn load_model(
    options: Res<Options>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(path) = &options.model else { return };
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => return error!("{}: {e}", path.display()),
    };
    let (_, geo) = match ModelFile::detect(&data) {
        Ok(found) => found,
        Err(e) => return error!("{}: not a recognised model ({e})", path.display()),
    };

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
    let mut slot_materials = Vec::new();
    if let Some((off, set)) = texture::scan(&data)
        .into_iter()
        .find(|(_, s)| s.images.len() >= need)
    {
        for img in &set.images {
            let Ok(Some(rgba)) = set.decode_rgba(&data[off..], img) else {
                slot_materials.push(materials.add(Color::srgb(0.8, 0.0, 0.8)));
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
            slot_materials.push(materials.add(StandardMaterial {
                base_color_texture: Some(images.add(image)),
                alpha_mode: AlphaMode::Mask(0.5),
                double_sided: true,
                cull_mode: None,
                perceptual_roughness: 1.0,
                ..default()
            }));
        }
    }
    let fallback = materials.add(StandardMaterial {
        base_color: Color::srgb(0.7, 0.7, 0.7),
        double_sided: true,
        cull_mode: None,
        ..default()
    });

    // Normalise size and put the feet on the floor.
    let all = geo
        .objects
        .iter()
        .flat_map(|o| &o.meshes)
        .flat_map(|m| &m.positions);
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in all {
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

    let mut count = 0;
    for obj in &geo.objects {
        for m in &obj.meshes {
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
            mesh.insert_indices(Indices::U32(tris.into_iter().flatten().collect()));
            let material = slot_materials
                .get(m.tex_index as usize)
                .cloned()
                .unwrap_or_else(|| fallback.clone());
            let child = commands
                .spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material)))
                .id();
            commands.entity(root).add_child(child);
            count += 1;
        }
    }
    info!(
        "{}: {count} meshes, {} vertices, {} texture slots",
        path.display(),
        geo.vertex_count(),
        slot_materials.len()
    );
}
