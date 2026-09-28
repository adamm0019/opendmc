//! Behaviour tests for the combat core, written against the built-in
//! placeholder data. When real measurements replace the placeholders, these
//! tests keep checking the *mechanics* (chaining, buffering, hit-once,
//! hit-stop, juggling); frame-exact parity tests live alongside the data.

use dmc_sim::actor::State;
use dmc_sim::ai::{Brain, MeleeBrainParams};
use dmc_sim::input::{InputFrame, button};
use dmc_sim::sim::{Event, PLAYER};
use dmc_sim::tape::Tape;
use dmc_sim::{Rules, Sim, V3};

const NONE: InputFrame = InputFrame {
    buttons: 0,
    move_x: 0,
    move_z: 0,
};

fn press(b: u16) -> InputFrame {
    NONE.with(b)
}

fn room() -> Sim {
    Sim::training_room(Rules::original(), Brain::Passive)
}

/// Step `n` idle ticks, returning every event.
fn idle(sim: &mut Sim, n: usize) -> Vec<Event> {
    (0..n).flat_map(|_| sim.step(NONE).to_vec()).collect()
}

fn current(sim: &Sim) -> Option<(String, u16)> {
    sim.current_move(PLAYER).map(|(id, f)| (id.to_string(), f))
}

#[test]
fn ground_combo_chains_on_timely_presses() {
    let mut sim = Sim::new(Rules::original(), 1);
    sim.step(press(button::MELEE));
    assert_eq!(current(&sim), Some(("combo_1".into(), 1)));
    idle(&mut sim, 5); // frame 6: inside combo_1's input window (4..=24)
    sim.step(press(button::MELEE));
    idle(&mut sim, 7);
    // After a step, the reported frame is the next one to run.
    assert_eq!(
        current(&sim),
        Some(("combo_1".into(), 14)),
        "waits for the link frame"
    );
    idle(&mut sim, 1);
    assert_eq!(
        current(&sim),
        Some(("combo_2".into(), 1)),
        "combo_2 replaces frame 14"
    );
}

#[test]
fn combo_without_follow_up_returns_to_neutral() {
    let mut sim = Sim::new(Rules::original(), 1);
    sim.step(press(button::MELEE));
    idle(&mut sim, 29);
    assert!(matches!(sim.player().state, State::Locomotion));
}

#[test]
fn late_press_restarts_the_combo_via_the_buffer() {
    let mut sim = Sim::new(Rules::original(), 1);
    sim.step(press(button::MELEE));
    idle(&mut sim, 26); // frame 27: past the input window, move ends at 30
    sim.step(press(button::MELEE));
    idle(&mut sim, 3);
    assert_eq!(
        current(&sim).map(|c| c.0),
        Some("combo_1".into()),
        "buffered press starts a fresh combo"
    );
}

/// Start the launcher (no combo link, no move cancels) and press melee once
/// at `press_frame`; report what the player is doing after it ends.
fn after_launcher_with_press_at(press_frame: usize) -> Option<(String, u16)> {
    let mut sim = Sim::new(Rules::original(), 1);
    let back = NONE.with(button::LOCK_ON).toward(V3::new(0.0, 0.0, -1.0));
    sim.step(back.with(button::MELEE));
    assert_eq!(current(&sim).unwrap().0, "launcher");
    idle(&mut sim, press_frame - 1);
    sim.step(press(button::MELEE));
    idle(&mut sim, 38 - press_frame - 1 + 2);
    current(&sim)
}

#[test]
fn early_press_is_dropped_late_press_is_buffered() {
    assert_eq!(
        after_launcher_with_press_at(5),
        None,
        "33 ticks early: dropped"
    );
    assert_eq!(
        after_launcher_with_press_at(35).map(|c| c.0),
        Some("combo_1".into()),
        "3 ticks early: buffered"
    );
}

#[test]
fn jump_cancels_late_in_combo_1() {
    let mut sim = Sim::new(Rules::original(), 1);
    sim.step(press(button::MELEE));
    idle(&mut sim, 25);
    sim.step(press(button::JUMP)); // frame 26, inside the 12..=29 jump window
    assert!(matches!(sim.player().state, State::Air));
}

#[test]
fn a_swing_hits_each_target_once_and_hitstop_freezes_both() {
    let mut sim = room();
    let hits_before = sim.actors[1].health;
    sim.step(press(button::MELEE));
    let mut hit_tick = None;
    for t in 1..30 {
        let ev = sim.step(NONE).to_vec();
        if ev.iter().any(|e| matches!(e, Event::Hit { .. })) {
            assert!(hit_tick.is_none(), "second hit from one window");
            hit_tick = Some(t);
            let frame = current(&sim).unwrap().1;
            // Hit-stop 3: the next 3 ticks do not advance either side.
            for _ in 0..3 {
                sim.step(NONE);
                assert_eq!(current(&sim).unwrap().1, frame);
            }
            sim.step(NONE);
            assert_eq!(current(&sim).unwrap().1, frame + 1);
        }
    }
    assert!(hit_tick.is_some());
    assert_eq!(sim.actors[1].health, hits_before - 40.0);
}

#[test]
fn launcher_needs_lock_on_and_back_and_juggles() {
    let mut sim = room();
    let back = NONE.with(button::LOCK_ON).toward(V3::new(0.0, 0.0, -1.0));
    sim.step(back);
    sim.step(back.with(button::MELEE));
    assert_eq!(current(&sim).unwrap().0, "launcher");
    let mut launched = false;
    let mut airtime = 0;
    for _ in 0..120 {
        let ev = sim.step(back).to_vec();
        launched |= ev
            .iter()
            .any(|e| matches!(e, Event::Launched { target: 1 }));
        if !sim.actors[1].grounded {
            airtime += 1;
        }
    }
    assert!(launched);
    // Under normal gravity the lift would give 2*lift/g ≈ 32 ticks; juggle
    // gravity has to keep the target up much longer than that.
    assert!(airtime > 50, "airtime {airtime}");
}

/// The point of a launcher is the follow-up: jumping after the target and
/// attacking in the air must connect. Guards against move data (lift, jump
/// speed, gravities, hitbox reach) drifting out of step with each other.
#[test]
fn air_combo_can_follow_a_launch() {
    let mut sim = room();
    let lock = NONE.with(button::LOCK_ON);
    let back = lock.toward(V3::new(0.0, 0.0, -1.0));
    sim.step(back.with(button::MELEE));
    assert_eq!(current(&sim).unwrap().0, "launcher");
    let mut air_hits = 0;
    for t in 1..120 {
        let input = match t {
            20 => lock.with(button::JUMP),
            32 | 42 => lock.with(button::MELEE),
            _ => lock,
        };
        for e in sim.step(input) {
            if let Event::Hit {
                move_id, target: 1, ..
            } = e
                && move_id.starts_with("air_")
            {
                air_hits += 1;
            }
        }
    }
    assert!(air_hits >= 1, "no air hit connected after the launch");
}

#[test]
fn lunge_closes_distance() {
    let mut sim = Sim::training_room(Rules::original(), Brain::Passive);
    sim.actors[1].pos = V3::new(0.0, 0.0, 8.0);
    let fwd = NONE.with(button::LOCK_ON).toward(V3::FORWARD);
    sim.step(fwd);
    sim.step(fwd.with(button::MELEE));
    assert_eq!(current(&sim).unwrap().0, "lunge");
    let hit = (0..40).any(|_| sim.step(fwd).iter().any(|e| matches!(e, Event::Hit { .. })));
    assert!(hit, "lunge reaches a target 8 units away");
}

#[test]
fn devil_trigger_multiplies_damage() {
    let mut sim = room();
    sim.dt.gauge = 1500.0;
    sim.step(press(button::DEVIL_TRIGGER));
    assert!(sim.dt.active);
    let before = sim.actors[1].health;
    sim.step(press(button::MELEE));
    idle(&mut sim, 20);
    assert_eq!(
        before - sim.actors[1].health,
        40.0 * sim.rules.dt_damage_multiplier
    );
}

#[test]
fn enemy_attacks_and_player_loses_style() {
    let mut sim = Sim::training_room(
        Rules::original(),
        Brain::Melee {
            params: MeleeBrainParams::training_dummy(),
            cooldown: 0,
        },
    );
    sim.style.points = 300.0;
    let hp = sim.player().health;
    let mut got_hit = false;
    for _ in 0..200 {
        got_hit |= sim
            .step(NONE)
            .iter()
            .any(|e| matches!(e, Event::Hit { target: PLAYER, .. }));
    }
    assert!(got_hit);
    assert!(sim.player().health < hp);
    assert_eq!(sim.style.points, 0.0);
}

#[test]
fn enhanced_profile_buffers_longer() {
    // Press jump 7 ticks before landing: outside the Original buffer (6),
    // inside the Enhanced one (10).
    for (rules, expect) in [(Rules::original(), false), (Rules::enhanced(), true)] {
        let mut sim = Sim::new(rules, 1);
        sim.step(press(button::JUMP));
        let mut probe = sim.clone();
        let mut ticks_to_land = 0;
        while !probe.player().grounded {
            probe.step(NONE);
            ticks_to_land += 1;
        }
        idle(&mut sim, ticks_to_land - 7);
        sim.step(press(button::JUMP));
        idle(&mut sim, 7);
        let jumped_again = !sim.player().grounded;
        assert_eq!(jumped_again, expect, "profile {:?}", sim.rules.profile);
    }
}

fn pseudo_random_tape(n: usize, seed: u32) -> Tape {
    let mut x = seed;
    let frames = (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let buttons = if x.is_multiple_of(7) {
                (x >> 8) as u16 & 0x3F
            } else {
                0
            };
            InputFrame {
                buttons,
                move_x: (x >> 3) as i16,
                move_z: (x >> 11) as i16,
            }
        })
        .collect();
    Tape { frames }
}

fn run(tape: &Tape, rules: Rules) -> Vec<u64> {
    let mut sim = Sim::training_room(
        rules,
        Brain::Melee {
            params: MeleeBrainParams::training_dummy(),
            cooldown: 0,
        },
    );
    sim.spawn_enemy(
        V3::new(4.0, 0.0, 4.0),
        Brain::Melee {
            params: MeleeBrainParams::training_dummy(),
            cooldown: 20,
        },
    );
    tape.frames
        .iter()
        .map(|&f| {
            sim.step(f);
            sim.state_hash()
        })
        .collect()
}

#[test]
fn deterministic_replay() {
    let tape = pseudo_random_tape(3000, 0xC0FFEE);
    let a = run(&tape, Rules::original());
    let b = run(
        &Tape::from_bytes(&tape.to_bytes()).unwrap(),
        Rules::original(),
    );
    assert_eq!(a, b, "same tape, same run");
    let c = run(&tape, Rules::enhanced());
    assert_ne!(a.last(), c.last(), "profile changes behaviour");
}

#[test]
fn snapshot_and_resume_matches() {
    let tape = pseudo_random_tape(1200, 7);
    let mut sim = Sim::training_room(
        Rules::original(),
        Brain::Melee {
            params: MeleeBrainParams::training_dummy(),
            cooldown: 0,
        },
    );
    for f in &tape.frames[..600] {
        sim.step(*f);
    }
    let mut fork = sim.clone();
    for f in &tape.frames[600..] {
        sim.step(*f);
        fork.step(*f);
    }
    assert_eq!(sim.state_hash(), fork.state_hash());
}
