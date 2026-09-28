//! `--room <file.fsd>`: show a room from the user's own install, every object
//! placed by its own matrix, textured and lit by its stored vertex colours
//! (the original's baked lighting, so materials are unlit). A fly camera
//! starts inside at eye height: arrow keys move, PageUp/PageDown rise and
//! sink, `,`/`.` turn. Nothing is cached or written.

use crate::Options;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use dmc_formats::room::Room;

pub struct RoomViewPlugin;

impl Plugin for RoomViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_room).add_systems(Update, fly);
    }
}

/// Room units per metre-ish sim unit: Dante is ~900 units tall and the sim's
/// figure 2 (the same scale as `--model`).
pub const ROOM_SCALE: f32 = 2.0 / 900.0;
/// Far from the arena so the two never overlap.
const ROOM_ORIGIN: Vec3 = Vec3::new(400.0, 0.0, 0.0);
/// Vertex colours are stored with 0x80 as full brightness (provisional,
/// `docs/formats/README.md` §4b).
const COLOUR_ONE: f32 = 128.0;
/// Rooms and their camera live on their own layer, apart from the arena.
const ROOM_LAYER: usize = 1;
/// Eye height above the floor, in sim units (the figure is 2 tall).
const EYE_HEIGHT: f32 = 1.7;

#[derive(Component)]
struct FlyCamera {
    yaw: f32,
}

fn load_room(
    options: Res<Options>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(path) = &options.room else { return };
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => return error!("{}: {e}", path.display()),
    };
    let room = match Room::parse(&data) {
        Ok(r) => r,
        Err(e) => return error!("{}: not a room ({e})", path.display()),
    };

    let mut slots = Vec::new();
    if let Some((bytes, set)) = room.textures(&data) {
        for img in &set.images {
            let material = match set.decode_rgba(bytes, img) {
                Ok(Some(rgba)) => {
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
                    StandardMaterial {
                        base_color_texture: Some(images.add(image)),
                        alpha_mode: AlphaMode::Mask(0.5),
                        unlit: true,
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    }
                }
                _ => StandardMaterial {
                    base_color: Color::srgb(0.8, 0.0, 0.8),
                    unlit: true,
                    ..default()
                },
            };
            slots.push(materials.add(material));
        }
    }
    let fallback = materials.add(StandardMaterial {
        base_color: Color::srgb(0.6, 0.6, 0.6),
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    });

    let root = commands
        .spawn((
            Transform::from_translation(ROOM_ORIGIN).with_scale(Vec3::splat(ROOM_SCALE)),
            Visibility::default(),
        ))
        .id();
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let (mut objects, mut triangles) = (0, 0);
    let mut points = Vec::new();
    for obj in &room.geometry.objects {
        let t = obj.transform;
        // Row-major with translation in the last column.
        let matrix = Mat4::from_cols_array_2d(&[
            [t[0][0], t[1][0], t[2][0], t[3][0]],
            [t[0][1], t[1][1], t[2][1], t[3][1]],
            [t[0][2], t[1][2], t[2][2], t[3][2]],
            [t[0][3], t[1][3], t[2][3], t[3][3]],
        ]);
        let node = commands
            .spawn((
                Transform::from_matrix(matrix),
                Visibility::default(),
                ChildOf(root),
            ))
            .id();
        for m in &obj.meshes {
            let tris = m.triangles();
            if tris.is_empty() {
                continue;
            }
            for p in &m.positions {
                let w = Vec3::from_array(obj.to_room(*p));
                lo = lo.min(w);
                hi = hi.max(w);
                points.push(w);
            }
            let colours: Vec<[f32; 4]> = m
                .colours
                .iter()
                .map(|c| {
                    let [r, g, b] = c.map(|v| (v as f32 / COLOUR_ONE).min(2.0));
                    [r, g, b, 1.0]
                })
                .collect();
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.positions.clone());
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, m.normals.clone());
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, m.uvs.clone());
            if colours.len() == m.positions.len() {
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
            }
            triangles += tris.len();
            mesh.insert_indices(Indices::U32(tris.into_iter().flatten().collect()));
            let material = slots
                .get(m.tex_index as usize)
                .cloned()
                .unwrap_or_else(|| fallback.clone());
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(material),
                ChildOf(node),
                RenderLayers::layer(ROOM_LAYER),
            ));
        }
        objects += 1;
    }
    info!(
        "{}: {objects} objects, {triangles} triangles, {} texture slots, bounds {lo} .. {hi}",
        path.display(),
        slots.len()
    );
    if lo.x > hi.x {
        return;
    }

    // Start inside, at eye height near one end, looking along the room.
    // Percentiles keep outliers such as sky domes from skewing it.
    let pct = |axis: usize, q: f32| {
        let mut v: Vec<f32> = points.iter().map(|p| p[axis]).collect();
        v.sort_by(f32::total_cmp);
        v[((v.len() - 1) as f32 * q) as usize]
    };
    let at = |x: f32, y: f32, z: f32| ROOM_ORIGIN + Vec3::new(x, y, z) * ROOM_SCALE;
    let floor = pct(1, 0.05);
    let (x, z0, z1) = (pct(0, 0.5), pct(2, 0.1), pct(2, 0.9));
    let eye = at(x, floor, z1) + Vec3::Y * EYE_HEIGHT;
    let target = at(x, floor, z0) + Vec3::Y * EYE_HEIGHT * 0.6;
    let look = Transform::from_translation(eye).looking_at(target, Vec3::Y);
    let (yaw, _, _) = look.rotation.to_euler(EulerRot::YXZ);
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 3,
            ..default()
        },
        look,
        FlyCamera { yaw },
        RenderLayers::layer(ROOM_LAYER),
    ));
}

fn fly(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut cams: Query<(&mut Transform, &mut FlyCamera)>,
) {
    for (mut t, mut cam) in &mut cams {
        let dt = time.delta_secs();
        let turn =
            (keys.pressed(KeyCode::Period) as i32 - keys.pressed(KeyCode::Comma) as i32) as f32;
        if turn != 0.0 {
            cam.yaw -= turn * 1.5 * dt;
            let (_, pitch, _) = t.rotation.to_euler(EulerRot::YXZ);
            t.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, pitch, 0.0);
        }
        let forward = (t.forward().as_vec3() * Vec3::new(1.0, 0.0, 1.0)).normalize_or_zero();
        let right = t.right().as_vec3();
        let mut v = Vec3::ZERO;
        v += forward
            * (keys.pressed(KeyCode::ArrowUp) as i32 - keys.pressed(KeyCode::ArrowDown) as i32)
                as f32;
        v += right
            * (keys.pressed(KeyCode::ArrowRight) as i32 - keys.pressed(KeyCode::ArrowLeft) as i32)
                as f32;
        v.y +=
            (keys.pressed(KeyCode::PageUp) as i32 - keys.pressed(KeyCode::PageDown) as i32) as f32;
        t.translation += v * 8.0 * dt;
    }
}
