//! Checks against a real install. They run only when `OPENDMC_GAME_DIR`
//! names the HD Collection folder (the one holding `data/dmc1`), so CI and
//! machines without the game skip them. Nothing from the game is written.

use dmc_formats::room::Room;
use dmc_formats::triggers::room_name;
use dmc_sim::input::InputFrame;
use dmc_sim::sim::PLAYER;
use dmc_sim::world::{ROOM_UNITS_PER_SIM_UNIT, World};
use dmc_sim::{Rules, Sim, V3};
use std::io::Read;
use std::path::PathBuf;

fn archive() -> Option<zip::ZipArchive<std::fs::File>> {
    let dir = PathBuf::from(std::env::var_os("OPENDMC_GAME_DIR")?);
    let nbz = dir.join("data/dmc1/dmc1-0.nbz");
    let file = std::fs::File::open(&nbz)
        .unwrap_or_else(|e| panic!("OPENDMC_GAME_DIR is set but {}: {e}", nbz.display()));
    Some(zip::ZipArchive::new(file).expect("dmc1-0.nbz is a ZIP"))
}

/// Every room file, as (name, bytes).
fn rooms() -> Option<Vec<(String, Vec<u8>)>> {
    let mut z = archive()?;
    let mut out = Vec::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        if !f.name().to_ascii_lowercase().ends_with(".fsd") {
            continue;
        }
        let mut data = Vec::new();
        f.read_to_end(&mut data).unwrap();
        out.push((f.name().to_string(), data));
    }
    Some(out)
}

#[test]
fn every_room_parses() {
    let Some(rooms) = rooms() else {
        eprintln!("OPENDMC_GAME_DIR not set; skipped");
        return;
    };
    assert_eq!(rooms.len(), 106);
    let (mut cameras, mut polys, mut props, mut unit_normals) = (0, 0, 0, 0);
    let names: std::collections::HashSet<String> = rooms
        .iter()
        .map(|(n, _)| {
            n.rsplit('/')
                .next()
                .unwrap()
                .trim_end_matches(".fsd")
                .to_lowercase()
        })
        .collect();
    let (mut doors, mut door_targets) = (0, 0);
    for (name, data) in &rooms {
        let room = Room::parse(data).unwrap_or_else(|e| panic!("{name}: {e}"));
        polys += room
            .collision(data)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .polys
            .len();
        if let Ok(c) = room.cameras(data) {
            cameras += c.cameras.len();
        }
        if let Ok(t) = room.triggers(data) {
            for (_, d) in t.doors() {
                doors += 1;
                door_targets += names.contains(&room_name(d.room)) as usize;
            }
        }
        if let Some(section) = room.section(data, dmc_formats::props::SECTION) {
            let table = room.props(data).unwrap_or_else(|e| panic!("{name}: {e}"));
            for (i, prop) in table.props() {
                let g = prop
                    .geometry(section)
                    .unwrap_or_else(|e| panic!("{name} prop {i}: {e}"));
                let meshes = g.objects.iter().flat_map(|o| &o.meshes);
                let (sum, n) = meshes.fold((0.0, 0), |(s, n), m| {
                    let v = m.vertex_count();
                    (s + m.mean_normal_length() * v as f32, n + v)
                });
                unit_normals += (0.9..1.1).contains(&(sum / n as f32)) as usize;
                props += 1;
            }
        }
    }
    assert_eq!(polys, 62_614);
    assert_eq!(cameras, 1_575);
    // Four doors name rooms that are not in the archive (r00f, r105, r20a).
    assert_eq!((doors, door_targets), (DOORS, DOORS - 4));
    assert_eq!(props, 1_190);
    // Read with the right layout, normals are unit length. The exceptions
    // are one 112-vertex model (three copies each in r408 and r40b).
    assert_eq!(unit_normals, 1_184);
}

/// Sections 4 and 5 are whole light sets, and every light slot stores its
/// colour twice (0–255, then divided by 255).
#[test]
fn light_sets_parse_in_every_room() {
    let Some(rooms) = rooms() else {
        eprintln!("OPENDMC_GAME_DIR not set; skipped");
        return;
    };
    let (mut sets, mut missing, mut lights, mut consistent) = ([0; 2], [0; 2], 0, 0);
    let mut kinds = std::collections::BTreeMap::new();
    for (name, data) in &rooms {
        let room = Room::parse(data).unwrap();
        for (i, &section) in dmc_formats::lights::SECTIONS.iter().enumerate() {
            let Some(raw) = room.section(data, section) else {
                missing[i] += 1;
                continue;
            };
            let all = room
                .light_sets(data, section)
                .unwrap_or_else(|e| panic!("{name} section {section}: {e}"));
            sets[i] += all.len();
            // Each slot also stores its colour divided by 255.
            for (s, set) in all.iter().enumerate() {
                for l in &set.lights {
                    lights += 1;
                    *kinds.entry(l.kind).or_insert(0) += 1;
                    let o = s * 0xC60 + 0x60 + l.index * 0x30 + 0x20;
                    let f = |k: usize| {
                        f32::from_le_bytes(raw[o + 4 * k..o + 4 * k + 4].try_into().unwrap())
                    };
                    consistent +=
                        (0..3).all(|k| (f(k) - l.colour[k] / 255.0).abs() < 0.01) as usize;
                }
            }
        }
    }
    // Six rooms have no section 5.
    assert_eq!((sets, missing), ([216, 241], [0, 6]));
    assert_eq!((lights, consistent), (20_800, 20_800));
    // Kinds 3 and 4 are most placed lights; kind 0 slots sit on the Y axis.
    assert_eq!((kinds[&0], kinds[&3], kinds[&4]), (10_196, 5_131, 3_388));
}

/// Records with sub-kind 0 and a room id, over kinds 1, 2, 3 and 7.
const DOORS: usize = 249;

fn stem(name: &str) -> String {
    name.rsplit('/')
        .next()
        .unwrap()
        .trim_end_matches(".fsd")
        .to_lowercase()
}

/// Doors lead somewhere sensible: the player arrives on a floor in the room
/// named, and usually beside a door back.
#[test]
fn doors_arrive_on_floors_beside_a_door_back() {
    let Some(rooms) = rooms() else {
        eprintln!("OPENDMC_GAME_DIR not set; skipped");
        return;
    };
    let s = 1.0 / ROOM_UNITS_PER_SIM_UNIT;
    let by_name: std::collections::HashMap<String, &Vec<u8>> =
        rooms.iter().map(|(n, d)| (stem(n), d)).collect();
    let (mut doors, mut on_floor, mut with_back, mut beside_back) = (0, 0, 0, 0);
    for (name, data) in &rooms {
        let from = stem(name);
        let Ok(triggers) = Room::parse(data).unwrap().triggers(data) else {
            continue;
        };
        for (_, door) in triggers.doors() {
            let Some(target) = by_name.get(&room_name(door.room)) else {
                continue;
            };
            doors += 1;
            let room = Room::parse(target).unwrap();
            let col = room.collision(target).unwrap();
            let world = World::new(
                col.triangles()
                    .map(|(t, flags)| (t.map(|p| V3::new(p[0] * s, p[1] * s, p[2] * s)), flags)),
                2.0,
            );
            let [x, y, z] = door.arrival;
            let reach = 150.0 * s;
            on_floor += world
                .ground(V3::new(x * s, y * s, z * s), reach, reach)
                .is_some() as usize;
            let Ok(back) = room.triggers(target) else {
                continue;
            };
            let backs: Vec<_> = back
                .doors()
                .filter(|(_, d)| room_name(d.room) == from)
                .collect();
            with_back += !backs.is_empty() as usize;
            beside_back += backs
                .iter()
                .any(|(t, _)| t.volume.contains(door.arrival, 800.0))
                as usize;
        }
    }
    assert_eq!(doors, DOORS - 4);
    // Within 150 units of a floor; a door back in the target room; the
    // arrival within 800 units of that door's volume.
    assert_eq!((on_floor, with_back, beside_back), (203, 205, 134));
}

#[test]
fn the_player_stays_on_real_floors() {
    let Some(rooms) = rooms() else {
        eprintln!("OPENDMC_GAME_DIR not set; skipped");
        return;
    };
    let s = 1.0 / ROOM_UNITS_PER_SIM_UNIT;
    let (mut runs, mut left) = (0, Vec::new());
    for (name, data) in &rooms {
        let room = Room::parse(data).unwrap();
        let col = room.collision(data).unwrap();
        let world = World::new(
            col.triangles()
                .map(|(t, flags)| (t.map(|p| V3::new(p[0] * s, p[1] * s, p[2] * s)), flags)),
            2.0,
        );
        let lowest = world
            .triangles()
            .iter()
            .flat_map(|t| [t.a.y, t.b.y, t.c.y])
            .fold(f32::INFINITY, f32::min);
        let Ok(cams) = room.cameras(data) else {
            continue;
        };
        // Start in the middle of up to four camera zones, on whatever ground
        // is below, and run in a circle for five seconds.
        for cam in cams.cameras.iter().take(4) {
            let [a, b] = cam.zone_corners;
            let mid = V3::new(
                (a[0] + b[0]) * 0.5 * s,
                (a[1] + b[1]) * 0.5 * s,
                (a[2] + b[2]) * 0.5 * s,
            );
            let Some(g) = world.ground(mid, 1e4, 1e4) else {
                continue;
            };
            let mut sim = Sim::new(Rules::original(), 3).with_world(world.clone());
            sim.actors[PLAYER].pos = V3::new(mid.x, g.height, mid.z);
            sim.actors[PLAYER].grounded = true;
            let mut last_ground = sim.actors[PLAYER].pos;
            for t in 0..300 {
                let (x, z) = [(1, 0), (0, 1), (-1, 0), (0, -1)][(t / 40) % 4];
                sim.step(InputFrame {
                    buttons: 0,
                    move_x: x * i16::MAX,
                    move_z: z * i16::MAX,
                });
                let p = sim.actors[PLAYER].pos;
                if sim.actors[PLAYER].grounded {
                    last_ground = p;
                }
                assert!(
                    p.x.is_finite() && p.y.is_finite() && p.z.is_finite(),
                    "{name}: {p:?}"
                );
                if p.y < lowest - 0.5 {
                    left.push(format!("{name}: left the mesh at {last_ground:?}"));
                    break;
                }
            }
            runs += 1;
        }
    }
    // The static collision has openings the game closes by other means
    // (doorways are props; the r200/r211 stairwell has a 1.2-unit gap
    // between the floor and the first step, which starts 0.9 units up), and
    // some zone centres sit on ledges the player can't normally reach. So
    // this guards against regressions rather than demanding zero exits.
    eprintln!("{runs} runs, {} left the mesh: {left:#?}", left.len());
    assert!(runs > 200, "only {runs} runs");
    assert!(
        left.len() * 20 <= runs,
        "more than 5% of runs left the mesh: {left:#?}"
    );
}
