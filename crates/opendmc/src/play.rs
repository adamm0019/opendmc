//! The playable graybox: input → `dmc-sim` at a fixed 60 Hz → interpolated
//! visuals, fixed-camera director, HUD and hitbox debug view.
//!
//! All art here is our own primitives: a capsule "test figure" with a
//! direction marker and capsule training dummies. No character likeness is
//! recreated (ADR-007).

use crate::Options;
use crate::cameras::{CameraSet, TRAINING_ROOM};
use crate::room_cameras::RoomDirector;
use bevy::prelude::*;
use dmc_sim::actor::{State, Team};
use dmc_sim::ai::{Brain, MeleeBrainParams};
use dmc_sim::controls::CameraRelative;
use dmc_sim::input::{InputFrame, button};
use dmc_sim::sim::{Event, PLAYER};
use dmc_sim::tape::Tape;
use dmc_sim::{Rules, Sim, TICK_HZ, V3};

pub struct PlayPlugin {
    pub rules: Rules,
}

impl Plugin for PlayPlugin {
    fn build(&self, app: &mut App) {
        let mut sim = Sim::training_room(self.rules.clone(), Brain::Passive);
        sim.spawn_enemy(
            V3::new(5.0, 0.0, 6.0),
            Brain::Melee {
                params: MeleeBrainParams::training_dummy(),
                cooldown: 90,
            },
        );
        let prev = positions(&sim);
        app.insert_resource(Time::<Fixed>::from_hz(TICK_HZ as f64))
            .insert_resource(SimState {
                sim,
                prev,
                tape: Tape::default(),
                last_events: Vec::new(),
            })
            .insert_resource(Controls::default())
            .insert_resource(Director {
                cameras: CameraSet::from_ron(TRAINING_ROOM).expect("training room cameras"),
                active: None,
                relative: CameraRelative::default(),
            })
            .insert_resource(DebugView(false))
            .add_systems(Startup, setup_scene)
            .add_systems(
                Update,
                (
                    read_controls,
                    sync_visuals,
                    direct_camera,
                    update_hud,
                    draw_debug,
                    save_tape_on_exit,
                ),
            )
            .add_systems(FixedUpdate, step_sim);
    }
}

#[derive(Resource)]
pub struct SimState {
    pub sim: Sim,
    /// Positions before the latest tick, for render interpolation.
    prev: Vec<V3>,
    tape: Tape,
    last_events: Vec<Event>,
}

/// Input gathered per rendered frame, consumed by the next fixed tick.
/// Presses are latched so a tap shorter than a tick is never lost.
#[derive(Resource, Default)]
struct Controls {
    held: u16,
    latched: u16,
    stick: (f32, f32),
}

#[derive(Resource)]
struct Director {
    cameras: CameraSet,
    active: Option<usize>,
    relative: CameraRelative,
}

#[derive(Resource)]
struct DebugView(bool);

/// `--walk`: the room's own cameras, which replace the training cameras.
#[derive(Resource)]
pub struct RoomCams {
    pub director: RoomDirector,
    pub active: Option<usize>,
}

#[derive(Component)]
pub struct ActorVisual(pub usize);

#[derive(Component)]
struct Hud;

#[derive(Component)]
struct MainCamera;

fn positions(sim: &Sim) -> Vec<V3> {
    sim.actors.iter().map(|a| a.pos).collect()
}

fn v(p: V3) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}

fn setup_scene(
    options: Res<Options>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    state: Res<SimState>,
) {
    if !options.walk {
        spawn_arena(&mut commands, &mut meshes, &mut materials, &state);
    }
    spawn_actors(&mut commands, &mut meshes, &mut materials, &state);
}

/// The graybox floor, pillars and tiles (not used when walking a room).
fn spawn_arena(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    state: &SimState,
) {
    let extent = state.sim.rules.arena_half_extent;
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(extent * 2.0, extent * 2.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.32, 0.30, 0.34))),
    ));
    // Decorative pillars and a floor grid of tiles, so motion reads on screen.
    let pillar = meshes.add(Cuboid::new(1.2, 6.0, 1.2));
    let stone = materials.add(Color::srgb(0.45, 0.42, 0.40));
    for (x, z) in [(-10.0, -10.0), (10.0, -10.0), (-10.0, 10.0), (10.0, 10.0)] {
        commands.spawn((
            Mesh3d(pillar.clone()),
            MeshMaterial3d(stone.clone()),
            Transform::from_xyz(x, 3.0, z),
        ));
    }
    let tile = meshes.add(Cuboid::new(1.9, 0.02, 1.9));
    let tile_mat = materials.add(Color::srgb(0.36, 0.34, 0.38));
    for i in -7..=7 {
        for j in -7..=7 {
            if (i + j) % 2 == 0 {
                commands.spawn((
                    Mesh3d(tile.clone()),
                    MeshMaterial3d(tile_mat.clone()),
                    Transform::from_xyz(i as f32 * 2.0, 0.01, j as f32 * 2.0),
                ));
            }
        }
    }
}

fn spawn_actors(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    state: &SimState,
) {
    // Actors: capsule body plus a small block showing which way it faces.
    let marker = meshes.add(Cuboid::new(0.25, 0.25, 0.5));
    for (i, a) in state.sim.actors.iter().enumerate() {
        let (body_color, radius) = match a.team {
            Team::Player => (Color::srgb(0.20, 0.65, 0.75), a.radius),
            Team::Enemy => (Color::srgb(0.70, 0.22, 0.20), a.radius),
        };
        let body = meshes.add(Capsule3d::new(radius, a.height - 2.0 * radius));
        commands
            .spawn((
                ActorVisual(i),
                Transform::from_translation(v(a.pos)),
                Visibility::default(),
            ))
            .with_children(|c| {
                c.spawn((
                    Mesh3d(body),
                    MeshMaterial3d(materials.add(body_color)),
                    Transform::from_xyz(0.0, a.height / 2.0, 0.0),
                ));
                c.spawn((
                    Mesh3d(marker.clone()),
                    MeshMaterial3d(materials.add(Color::srgb(0.95, 0.85, 0.3))),
                    Transform::from_xyz(0.0, a.height * 0.75, radius + 0.1),
                ));
            });
    }

    commands.spawn((
        DirectionalLight {
            shadows_enabled: true,
            illuminance: 9000.0,
            ..default()
        },
        Transform::from_xyz(6.0, 12.0, -4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        MainCamera,
        Camera3d::default(),
        Transform::from_xyz(0.0, 6.0, -14.0).looking_at(Vec3::Y, Vec3::Y),
    ));
    commands.spawn((
        Hud,
        Text::new(""),
        TextFont {
            font_size: 16.0,
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            left: Val::Px(12.0),
            ..default()
        },
    ));
}

fn read_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut controls: ResMut<Controls>,
    mut debug: ResMut<DebugView>,
) {
    let map = [
        (KeyCode::KeyJ, button::MELEE),
        (KeyCode::KeyK, button::GUN),
        (KeyCode::Space, button::JUMP),
        (KeyCode::ShiftLeft, button::LOCK_ON),
        (KeyCode::KeyL, button::LOCK_ON),
        (KeyCode::KeyU, button::DEVIL_TRIGGER),
        (KeyCode::KeyT, button::TAUNT),
    ];
    controls.held = 0;
    for (key, b) in map {
        if keys.pressed(key) {
            controls.held |= b;
        }
        if keys.just_pressed(key) {
            controls.latched |= b;
        }
    }
    let axis = |neg: KeyCode, pos: KeyCode| {
        keys.pressed(pos) as i32 as f32 - keys.pressed(neg) as i32 as f32
    };
    controls.stick = (
        axis(KeyCode::KeyA, KeyCode::KeyD),
        axis(KeyCode::KeyS, KeyCode::KeyW),
    );
    if keys.just_pressed(KeyCode::F1) {
        debug.0 = !debug.0;
    }
}

fn step_sim(
    mut state: ResMut<SimState>,
    mut controls: ResMut<Controls>,
    mut director: ResMut<Director>,
    room_cams: Option<Res<RoomCams>>,
    options: Res<Options>,
    mut debug: ResMut<DebugView>,
) {
    // Hold the sim on the capture tick so screenshots are reproducible no
    // matter how many fixed steps each rendered frame runs.
    if options.screenshot.is_some() && state.sim.tick >= options.at_tick {
        return;
    }
    let frame = if options.demo {
        // The script is already in world space; show hitboxes while it runs.
        debug.0 = true;
        crate::capture::demo_input(state.sim.tick)
    } else {
        let forward = match &room_cams {
            Some(rc) => rc
                .active
                .map(|i| {
                    let view = rc.director.cameras[i].view(state.sim.player().pos);
                    (view.target - view.eye).flat().normalize_or(V3::FORWARD)
                })
                .unwrap_or(V3::FORWARD),
            None => director
                .active
                .map(|i| director.cameras.zones[i].forward())
                .unwrap_or(V3::FORWARD),
        };
        let world = director.relative.world_direction(controls.stick, forward);
        InputFrame {
            buttons: controls.held | controls.latched,
            ..default()
        }
        .toward(world)
    };
    controls.latched = 0;

    let state = &mut *state;
    state.prev = positions(&state.sim);
    state.tape.frames.push(frame);
    let events = state.sim.step(frame).to_vec();
    if !events.is_empty() {
        state.last_events = events;
    }
}

fn sync_visuals(
    time: Res<Time<Fixed>>,
    state: Res<SimState>,
    mut q: Query<(&ActorVisual, &mut Transform, &mut Visibility)>,
) {
    let alpha = time.overstep_fraction();
    for (ActorVisual(i), mut t, mut vis) in &mut q {
        let a = &state.sim.actors[*i];
        let prev = state.prev.get(*i).copied().unwrap_or(a.pos);
        t.translation = v(prev).lerp(v(a.pos), alpha);
        t.rotation = Quat::from_rotation_arc(Vec3::Z, v(a.facing).normalize_or(Vec3::Z));
        *vis = if a.alive() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

fn direct_camera(
    state: Res<SimState>,
    mut director: ResMut<Director>,
    room_cams: Option<ResMut<RoomCams>>,
    mut cam: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
) {
    let player = state.sim.player().pos;
    if let Some(mut rc) = room_cams {
        // Room cameras can move with the player (rails), so update every frame.
        rc.active = rc.director.select(rc.active, player);
        let Some(i) = rc.active else { return };
        let view = rc.director.cameras[i].view(player);
        for (mut t, mut proj) in &mut cam {
            *t = Transform::from_translation(v(view.eye)).looking_at(v(view.target), Vec3::Y);
            if let Projection::Perspective(p) = &mut *proj {
                p.fov = view.fov_degrees.to_radians();
            }
        }
        return;
    }
    let selected = director.cameras.select(director.active, player);
    if selected == director.active && director.active.is_some() {
        return;
    }
    director.active = selected;
    let Some(zone) = selected.map(|i| director.cameras.zones[i].clone()) else {
        return;
    };
    for (mut t, mut proj) in &mut cam {
        *t = Transform::from_translation(v(zone.eye)).looking_at(v(zone.look_at), Vec3::Y);
        if let Projection::Perspective(p) = &mut *proj {
            p.fov = zone.fov_degrees.to_radians();
        }
    }
}

fn update_hud(
    state: Res<SimState>,
    director: Res<Director>,
    room_cams: Option<Res<RoomCams>>,
    mut hud: Query<&mut Text, With<Hud>>,
) {
    let sim = &state.sim;
    let p = sim.player();
    let doing = match &p.state {
        State::Attack(_) => sim
            .current_move(PLAYER)
            .map(|(id, f)| format!("{id} f{f}"))
            .unwrap_or_default(),
        other => format!("{other:?}"),
    };
    let target = sim
        .lock_target
        .map(|t| format!("  lock-on: #{t} ({:.0} hp)", sim.actors[t].health))
        .unwrap_or_default();
    let camera = match &room_cams {
        Some(rc) => rc
            .active
            .map(|i| format!("room camera {i}"))
            .unwrap_or_else(|| "-".into()),
        None => director
            .active
            .map(|i| director.cameras.zones[i].name.clone())
            .unwrap_or_else(|| "-".into()),
    };
    let text = format!(
        "OpenDMC graybox | {:?} profile | tick {}\n\
         HP {:.0}/{:.0}   Style {} ({:.0})   DT {:.0}{}\n\
         {doing}{target}\ncamera: {camera}   last events: {:?}\n\n\
         WASD move | J melee | Space jump | Shift/L lock-on | U devil trigger | F1 hitboxes\n\
         lock-on + forward + J = lunge | lock-on + back + J = launcher | J in air = air combo",
        sim.rules.profile,
        sim.tick,
        p.health,
        p.max_health,
        sim.style.rank().unwrap_or("-"),
        sim.style.points,
        sim.dt.gauge,
        if sim.dt.active { " ACTIVE" } else { "" },
        state.last_events,
    );
    for mut t in &mut hud {
        *t = Text::new(text.clone());
    }
}

fn draw_debug(debug: Res<DebugView>, state: Res<SimState>, mut gizmos: Gizmos) {
    if !debug.0 {
        return;
    }
    let sim = &state.sim;
    for (i, a) in sim.actors.iter().enumerate() {
        if !a.alive() {
            continue;
        }
        let hurt = Color::srgba(0.2, 0.9, 0.3, 0.8);
        gizmos.sphere(
            Isometry3d::from_translation(v(a.pos) + Vec3::Y * a.radius),
            a.radius,
            hurt,
        );
        gizmos.sphere(
            Isometry3d::from_translation(v(a.pos) + Vec3::Y * (a.height - a.radius)),
            a.radius,
            hurt,
        );
        if let Some(st) = a.attack() {
            let m = sim.move_def(i, st.move_index);
            for h in &m.hits {
                let live = (h.frames.0..=h.frames.1).contains(&st.frame);
                let color = if live {
                    Color::srgb(1.0, 0.2, 0.2)
                } else {
                    Color::srgba(1.0, 0.8, 0.2, 0.3)
                };
                let centre = a.pos + V3::local_to_world(a.facing, h.offset);
                gizmos.sphere(Isometry3d::from_translation(v(centre)), h.radius, color);
            }
        }
    }
}

fn save_tape_on_exit(
    mut exit: MessageReader<AppExit>,
    options: Res<Options>,
    state: Res<SimState>,
) {
    if exit.read().next().is_none() {
        return;
    }
    if let Some(path) = &options.record {
        match std::fs::write(path, state.tape.to_bytes()) {
            Ok(()) => info!(
                "wrote {} ticks to {}",
                state.tape.frames.len(),
                path.display()
            ),
            Err(e) => error!("could not write tape: {e}"),
        }
    }
}
