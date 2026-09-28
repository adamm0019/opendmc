//! The character controller against room collision: walls stop the player,
//! low steps are climbed, ledges are fallen from, and replays stay exact.

use dmc_sim::input::InputFrame;
use dmc_sim::sim::{Event, PLAYER};
use dmc_sim::world::{Body, World};
use dmc_sim::{Rules, Sim, V3};

fn stick(x: i16, z: i16) -> InputFrame {
    InputFrame {
        buttons: 0,
        move_x: x,
        move_z: z,
    }
}

/// Two triangles of a quad `(a, b, c)`, `(c, b, d)`.
fn quad(a: V3, b: V3, c: V3, d: V3, flags: u32) -> [([V3; 3], u32); 2] {
    [([a, b, c], flags), ([c, b, d], flags)]
}

/// A 40-wide room: a raised platform (y = 1) for x < 0, the floor (y = 0)
/// for x > 0, a 0.25 step at x in [4, 6], and a wall at x = 15.
fn room() -> World {
    let v = V3::new;
    let mut t = Vec::new();
    t.extend(quad(
        v(-20., 1., -20.),
        v(-20., 1., 20.),
        v(0., 1., -20.),
        v(0., 1., 20.),
        1,
    ));
    t.extend(quad(
        v(0., 0., -20.),
        v(0., 0., 20.),
        v(20., 0., -20.),
        v(20., 0., 20.),
        1,
    ));
    t.extend(quad(
        v(4., 0.25, -20.),
        v(4., 0.25, 20.),
        v(6., 0.25, -20.),
        v(6., 0.25, 20.),
        1,
    ));
    t.extend(quad(
        v(15., 0., -20.),
        v(15., 0., 20.),
        v(15., 4., -20.),
        v(15., 4., 20.),
        0x10,
    ));
    World::new(t, 2.0)
}

fn sim_at(x: f32, y: f32) -> Sim {
    let mut sim = Sim::new(Rules::original(), 7).with_world(room());
    sim.actors[PLAYER].pos = V3::new(x, y, 0.0);
    sim.actors[PLAYER].grounded = true;
    sim
}

#[test]
fn walls_stop_the_player() {
    let mut sim = sim_at(10.0, 0.0);
    for _ in 0..120 {
        sim.step(stick(i16::MAX, 0));
    }
    let p = sim.actors[PLAYER].pos;
    assert!((p.x - (15.0 - Body::PLAYER.radius)).abs() < 1e-3, "{p:?}");
    assert_eq!(p.y, 0.0);
    assert!(sim.actors[PLAYER].grounded);
}

#[test]
fn low_steps_are_climbed_and_left() {
    let mut sim = sim_at(3.0, 0.0);
    let mut heights = Vec::new();
    for _ in 0..40 {
        sim.step(stick(i16::MAX, 0));
        heights.push(sim.actors[PLAYER].pos.y);
        assert!(sim.actors[PLAYER].grounded, "stays grounded over the step");
    }
    assert!(heights.contains(&0.25), "stood on the step");
    assert_eq!(*heights.last().unwrap(), 0.0, "stepped back down");
}

#[test]
fn ledges_are_fallen_from() {
    let mut sim = sim_at(-1.0, 1.0);
    let mut events = Vec::new();
    let mut airborne = false;
    for _ in 0..90 {
        events.extend(sim.step(stick(i16::MAX, 0)).to_vec());
        airborne |= !sim.actors[PLAYER].grounded;
    }
    assert!(airborne, "left the platform through the air");
    assert!(events.contains(&Event::Landed { actor: PLAYER }));
    assert_eq!(sim.actors[PLAYER].pos.y, 0.0);
}

#[test]
fn fast_falls_do_not_pass_through_floors() {
    let mut sim = sim_at(10.0, 40.0);
    sim.actors[PLAYER].grounded = false;
    sim.actors[PLAYER].vel = V3::new(0.0, -60.0, 0.0);
    for _ in 0..120 {
        sim.step(stick(0, 0));
    }
    assert_eq!(sim.actors[PLAYER].pos.y, 0.0);
    assert!(sim.actors[PLAYER].grounded);
}

#[test]
fn replays_with_a_world_are_identical() {
    let run = || {
        let mut sim = sim_at(-5.0, 1.0);
        for t in 0..300i32 {
            let a = (t % 97 - 48) * 600;
            let x = (i16::MAX as i32 / 2 + a).clamp(-32767, 32767) as i16;
            sim.step(stick(x, a as i16));
        }
        sim.state_hash()
    };
    assert_eq!(run(), run());
}
