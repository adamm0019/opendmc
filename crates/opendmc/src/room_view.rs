//! `--room <file.fsd>`: show a room from the user's own install, every object
//! placed by its own matrix, textured and lit by its stored vertex colours
//! (the original's baked lighting, so materials are unlit). A fly camera
//! starts inside at eye height: arrow keys move, PageUp/PageDown rise and
//! sink, `,`/`.` turn. With `--walk` the room is the play space instead: the
//! sim runs on its collision, the room's own cameras follow the player, and
//! its doors lead on to the rooms beside it. Nothing is cached or written.
//!
//! A room is loaded in two halves (ADR-011): its visuals, which can be
//! replaced, and its gameplay (collision, cameras, doors), which only ever
//! comes from the room's own data.

use crate::Options;
use crate::play::{RoomCams, SimState};
use crate::render::{Look, set_room_look};
use crate::room_cameras::{RoomCamera, RoomDirector};
use crate::room_doors::{RoomDoors, room_path};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use dmc_formats::room::Room;
use dmc_formats::triggers::room_name;
use dmc_sim::V3;
use dmc_sim::world::{ROOM_UNITS_PER_METRE, World};
use std::path::{Path, PathBuf};

pub struct RoomViewPlugin;

impl Plugin for RoomViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<LoadRoom>()
            .add_systems(Startup, load_room)
            .add_systems(Update, (fly, use_doors, change_room).chain());
    }
}

/// Replace the walked room with another, the player arriving at `arrival`
/// (in the new room's units).
#[derive(Message, Clone, Debug)]
pub struct LoadRoom {
    pub path: PathBuf,
    pub arrival: [f32; 3],
}

/// The room being walked: where it came from and its doors.
#[derive(Resource)]
pub struct ActiveRoom {
    pub path: PathBuf,
    pub doors: RoomDoors,
}

/// The root of the loaded room's visuals, despawned when a door replaces it.
#[derive(Component)]
struct RoomRoot;

/// Where the player starts in a room.
#[derive(Clone, Copy, Debug)]
enum Start {
    /// In this camera's zone, or else in the first zone with a floor.
    Camera(Option<usize>),
    /// At a door's arrival point, in room units.
    Arrival([f32; 3]),
}

/// Metres per room unit (ADR-010), the same scale as `--model`.
pub const ROOM_SCALE: f32 = 1.0 / ROOM_UNITS_PER_METRE;
/// Far from the arena so the two never overlap.
const ROOM_ORIGIN: Vec3 = Vec3::new(400.0, 0.0, 0.0);
/// Vertex colours are stored with 0x80 as full brightness (provisional,
/// `docs/formats/README.md` §4b).
const COLOUR_ONE: f32 = 128.0;
/// Rooms and their camera live on their own layer, apart from the arena.
const ROOM_LAYER: usize = 1;
/// Eye height above the floor, in metres (the figure is 2 tall).
const EYE_HEIGHT: f32 = 1.7;

/// Where a model loaded alongside the room should stand: on the floor in
/// front of the start camera, at room scale, on the room's render layer.
#[derive(Resource, Clone, Copy)]
pub struct RoomSpot {
    pub position: Vec3,
    pub facing: Quat,
}

pub fn room_layer() -> RenderLayers {
    RenderLayers::layer(ROOM_LAYER)
}

/// Where the room sits: apart from the arena when viewing, at the sim's
/// origin when walking (room space is then sim space).
fn origin(options: &Options) -> Vec3 {
    if options.walk {
        Vec3::ZERO
    } else {
        ROOM_ORIGIN
    }
}

#[derive(Component)]
struct FlyCamera {
    yaw: f32,
}

fn read_room(path: &Path) -> Option<(Room, Vec<u8>)> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            error!("{}: {e}", path.display());
            return None;
        }
    };
    match Room::parse(&data) {
        Ok(room) => Some((room, data)),
        Err(e) => {
            error!("{}: not a room ({e})", path.display());
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn load_room(
    options: Res<Options>,
    mut state: ResMut<SimState>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut loads: MessageWriter<LoadRoom>,
) {
    let Some(path) = &options.room else { return };
    let Some((room, data)) = read_room(path) else {
        return;
    };
    let points = spawn_visual(
        path,
        &room,
        &data,
        origin(&options),
        options.walk,
        options.look == Look::Modern,
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
    );
    if points.is_empty() {
        return;
    }
    set_room_look(
        &mut commands,
        &options,
        path,
        bounds(&points, origin(&options)),
    );

    // Start inside, at eye height near one end, looking along the room.
    // Percentiles keep outliers such as sky domes from skewing it.
    let pct = |axis: usize, q: f32| {
        let mut v: Vec<f32> = points.iter().map(|p| p[axis]).collect();
        v.sort_by(f32::total_cmp);
        v[((v.len() - 1) as f32 * q) as usize]
    };
    let at = |x: f32, y: f32, z: f32| origin(&options) + Vec3::new(x, y, z) * ROOM_SCALE;
    let floor = pct(1, 0.05);
    let (x, z0, z1) = (pct(0, 0.5), pct(2, 0.1), pct(2, 0.9));
    let eye = at(x, floor, z1) + Vec3::Y * EYE_HEIGHT;
    let target = at(x, floor, z0) + Vec3::Y * EYE_HEIGHT * 0.6;
    let spot = at(x, floor, z1 + (z0 - z1) * 0.25);
    commands.insert_resource(RoomSpot {
        position: spot,
        // Models face +Z natively, which is towards the camera here.
        facing: Quat::IDENTITY,
    });
    if options.walk {
        let doors = start_walking(
            path,
            &room,
            &data,
            Start::Camera(options.start_camera),
            &mut state,
            &mut commands,
        );
        if let Some(i) = options.through_door {
            match doors.door(i) {
                Some(d) => {
                    loads.write(LoadRoom {
                        path: room_path(path, d.room),
                        arrival: d.arrival,
                    });
                }
                None => error!("--through-door {i}: the room has {} doors", doors.len()),
            }
        }
        return;
    }
    // Lights respect render layers too. The room itself is unlit (its
    // lighting is baked into vertex colours); this lights models in it.
    commands.spawn((
        DirectionalLight {
            illuminance: 6000.0,
            ..default()
        },
        Transform::from_translation(eye + Vec3::new(2.0, 4.0, 0.0)).looking_at(spot, Vec3::Y),
        RenderLayers::layer(ROOM_LAYER),
    ));
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

/// Most of a room, in metres at `origin`: the 1st to 99th percentile of its
/// vertices on each axis, so sky domes and stray far geometry don't count.
fn bounds(points: &[Vec3], origin: Vec3) -> (Vec3, Vec3) {
    let pct = |axis: usize, q: f32| {
        let mut v: Vec<f32> = points.iter().map(|p| p[axis]).collect();
        v.sort_by(f32::total_cmp);
        v[((v.len() - 1) as f32 * q) as usize]
    };
    let at = |q: f32| origin + Vec3::new(pct(0, q), pct(1, q), pct(2, q)) * ROOM_SCALE;
    (at(0.01), at(0.99))
}

/// The room's reference visuals (its own meshes and textures) under one
/// [`RoomRoot`] at `origin`. Unlit with the original's baked vertex lighting,
/// or `lit` by the modern stack without it. Returns every vertex in room
/// space, empty when the room has no geometry.
#[allow(clippy::too_many_arguments)]
fn spawn_visual(
    path: &Path,
    room: &Room,
    data: &[u8],
    origin: Vec3,
    walk: bool,
    lit: bool,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Vec<Vec3> {
    let mut slots = Vec::new();
    if let Some((bytes, set)) = room.textures(data) {
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
                        unlit: !lit,
                        perceptual_roughness: 0.8,
                        reflectance: 0.3,
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    }
                }
                _ => StandardMaterial {
                    base_color: Color::srgb(0.8, 0.0, 0.8),
                    unlit: !lit,
                    ..default()
                },
            };
            slots.push(materials.add(material));
        }
    }
    let fallback = materials.add(StandardMaterial {
        base_color: Color::srgb(0.6, 0.6, 0.6),
        unlit: !lit,
        double_sided: true,
        cull_mode: None,
        ..default()
    });

    let root = commands
        .spawn((
            RoomRoot,
            Transform::from_translation(origin).with_scale(Vec3::splat(ROOM_SCALE)),
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
            // Lit, the modern stack does the lighting the colours baked in.
            if !lit && colours.len() == m.positions.len() {
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
            }
            triangles += tris.len();
            mesh.insert_indices(Indices::U32(tris.into_iter().flatten().collect()));
            let material = slots
                .get(m.tex_index as usize)
                .cloned()
                .unwrap_or_else(|| fallback.clone());
            // Walking, the room shares the main camera's layer with the actors.
            let layer = if walk {
                RenderLayers::default()
            } else {
                room_layer()
            };
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(material),
                ChildOf(node),
                layer,
            ));
        }
        objects += 1;
    }
    info!(
        "{}: {objects} objects, {triangles} triangles, {} texture slots, bounds {lo} .. {hi}",
        path.display(),
        slots.len()
    );
    points
}

/// Put the sim on the room's collision, the actors on its floor, and its
/// cameras and doors in charge. Returns the doors.
fn start_walking(
    path: &Path,
    room: &Room,
    data: &[u8],
    start: Start,
    state: &mut SimState,
    commands: &mut Commands,
) -> RoomDoors {
    let s = ROOM_SCALE;
    let doors = match room.triggers(data) {
        Ok(t) => RoomDoors::new(&t),
        Err(e) => {
            info!("--walk: no triggers ({e}), so no doors");
            RoomDoors::default()
        }
    };
    commands.insert_resource(ActiveRoom {
        path: path.to_path_buf(),
        doors: doors.clone(),
    });
    let collision = match room.collision(data) {
        Ok(c) => c,
        Err(e) => {
            error!("--walk: no collision ({e})");
            return doors;
        }
    };
    let world = World::new(
        collision
            .triangles()
            .map(|(t, flags)| (t.map(|p| V3::new(p[0] * s, p[1] * s, p[2] * s)), flags)),
        2.0,
    );
    let director = match room.cameras(data) {
        Ok(c) => RoomDirector::new(&c.cameras, s),
        Err(e) => {
            warn!("--walk: no room cameras ({e}); the view stays on the start camera");
            RoomDirector::default()
        }
    };
    let floor_in = |c: &RoomCamera| {
        let m = c.zone_centre();
        world
            .ground(m, 0.0, 1e4)
            .map(|g| V3::new(m.x, g.height, m.z))
    };
    let start = match start {
        Start::Camera(Some(i)) => match director.cameras.get(i) {
            Some(c) => floor_in(c),
            None => {
                error!(
                    "--start-camera {i}: the room has {} cameras",
                    director.cameras.len()
                );
                return doors;
            }
        },
        Start::Camera(None) => director.cameras.iter().find_map(floor_in),
        Start::Arrival(a) => {
            let p = V3::new(a[0] * s, a[1] * s, a[2] * s);
            // Onto the floor under the point; most arrivals lie on one.
            let floor = world
                .ground(p, 1.0, 4.0)
                .map(|g| V3::new(p.x, g.height, p.z));
            if floor.is_none() {
                warn!("arrival {a:?} has no floor under it; starting in a camera zone");
            }
            floor.or_else(|| director.cameras.iter().find_map(floor_in))
        }
    };
    let start = start.or_else(|| {
        world
            .triangles()
            .iter()
            .find(|t| t.normal.y > 0.7)
            .map(|t| (t.a + t.b + t.c) * (1.0 / 3.0))
    });
    let Some(start) = start else {
        error!("--walk: the room has no floor");
        return doors;
    };
    let sim = &mut state.sim;
    sim.world = Some(world);
    for (i, a) in sim.actors.iter_mut().enumerate() {
        // The player at the start, the others a few steps away; everyone
        // settles onto the floor on the first ticks.
        let offset = if i == 0 {
            V3::ZERO
        } else {
            V3::new(1.5 * i as f32, 0.5, 2.5)
        };
        a.pos = start + offset;
        a.vel = V3::ZERO;
        a.grounded = false;
    }
    info!(
        "--walk: {} collision triangles, {} cameras, {} doors, start {start:?}",
        sim.world.as_ref().map_or(0, |w| w.triangles().len()),
        director.cameras.len(),
        doors.len()
    );
    state.reset_interpolation();
    commands.insert_resource(RoomCams {
        director,
        active: None,
    });
    doors
}

/// Send the player through a door they walk into.
fn use_doors(
    state: Res<SimState>,
    room: Option<ResMut<ActiveRoom>>,
    mut loads: MessageWriter<LoadRoom>,
) {
    let Some(mut room) = room else { return };
    let p = state.sim.player().pos * (1.0 / ROOM_SCALE);
    let Some(door) = room.doors.update([p.x, p.y, p.z]) else {
        return;
    };
    let path = room_path(&room.path, door.room);
    if path.is_file() {
        info!("door to {}", room_name(door.room));
        loads.write(LoadRoom {
            path,
            arrival: door.arrival,
        });
    } else {
        warn!(
            "the door to {} leads nowhere: {} is missing",
            room_name(door.room),
            path.display()
        );
    }
}

/// Replace the walked room: new visuals, collision, cameras and doors, and
/// the actors at the arrival point.
#[allow(clippy::too_many_arguments)]
fn change_room(
    options: Res<Options>,
    mut loads: MessageReader<LoadRoom>,
    mut state: ResMut<SimState>,
    roots: Query<Entity, With<RoomRoot>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(load) = loads.read().last().cloned() else {
        return;
    };
    let Some((room, data)) = read_room(&load.path) else {
        return;
    };
    for root in &roots {
        commands.entity(root).despawn();
    }
    let points = spawn_visual(
        &load.path,
        &room,
        &data,
        Vec3::ZERO,
        true,
        options.look == Look::Modern,
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
    );
    if !points.is_empty() {
        set_room_look(
            &mut commands,
            &options,
            &load.path,
            bounds(&points, Vec3::ZERO),
        );
    }
    start_walking(
        &load.path,
        &room,
        &data,
        Start::Arrival(load.arrival),
        &mut state,
        &mut commands,
    );
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
