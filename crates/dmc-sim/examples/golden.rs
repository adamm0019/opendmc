//! Golden traces for porting the sim: every tick of a set of scenarios, with
//! its input, events, actor state and [`Sim::state_hash`], as JSON. A port
//! (the Unreal C++ one, docs/UNREAL.md §5) replays the same inputs and must
//! match the hash on every tick.
//!
//! ```text
//! cargo run -p dmc-sim --example golden -- <out dir>
//! ```
//!
//! Also writes `data.json`: the move sets and rules the traces ran on.
//! Everything here is the training room and the built-in move sets: original
//! data only, so the traces may be committed as test fixtures.

use dmc_sim::ai::{Brain, MeleeBrainParams};
use dmc_sim::{InputFrame, Rules, Sim, V3, button};
use serde_json::{Value, json};
use std::path::PathBuf;

/// The shell's training room: a passive dummy ahead, a melee enemy to the side.
fn training_room(rules: Rules) -> Sim {
    let mut sim = Sim::training_room(rules, Brain::Passive);
    sim.spawn_enemy(
        V3::new(5.0, 0.0, 6.0),
        Brain::Melee {
            params: MeleeBrainParams::training_dummy(),
            cooldown: 90,
        },
    );
    sim
}

/// The shell's `--demo` script (opendmc `capture::demo_input`).
fn demo(tick: u64) -> InputFrame {
    let lock = InputFrame::default().with(button::LOCK_ON);
    let back = lock.toward(V3::new(0.0, 0.0, -1.0));
    match tick {
        0..=9 => lock,
        10 => back.with(button::MELEE),
        11..=29 => back,
        30 => lock.with(button::JUMP),
        31..=41 => lock,
        42 | 52 => lock.with(button::MELEE),
        _ => lock,
    }
}

/// Button mashing with held stretches, from a fixed seed.
struct Fuzz {
    state: u64,
    left: u32,
    held: InputFrame,
}

impl Fuzz {
    fn new(seed: u64) -> Self {
        Fuzz {
            state: seed,
            left: 0,
            held: InputFrame::default(),
        }
    }

    fn next_u32(&mut self) -> u32 {
        // PCG-style LCG step, high bits.
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.state >> 33) as u32
    }

    fn frame(&mut self) -> InputFrame {
        if self.left == 0 {
            self.left = 1 + self.next_u32() % 20;
            let mut buttons = 0;
            for b in 0..button::COUNT {
                // Lock-on is held often; the rest are taps.
                let odds = if 1 << b == button::LOCK_ON { 2 } else { 6 };
                if self.next_u32().is_multiple_of(odds) {
                    buttons |= 1 << b;
                }
            }
            let (move_x, move_z) = if self.next_u32().is_multiple_of(3) {
                (0, 0)
            } else {
                let x = (self.next_u32() % 65535) as i32 - 32767;
                let z = (self.next_u32() % 65535) as i32 - 32767;
                (x as i16, z as i16)
            };
            self.held = InputFrame {
                buttons,
                move_x,
                move_z,
            };
        }
        self.left -= 1;
        self.held
    }
}

fn run(sim: &mut Sim, ticks: u64, mut input: impl FnMut(u64) -> InputFrame) -> Vec<Value> {
    (0..ticks)
        .map(|t| {
            let frame = input(t);
            let events = serde_json::to_value(sim.step(frame)).expect("events");
            json!({
                "tick": sim.tick,
                "input": frame,
                "hash": format!("{:016x}", sim.state_hash()),
                "events": events,
                "actors": sim.actors,
                "style": sim.style,
                "devil_trigger": sim.dt,
                "lock_target": sim.lock_target,
            })
        })
        .collect()
}

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "golden".into())
        .into();
    std::fs::create_dir_all(&out).expect("output directory");
    let profiles = [
        ("original", Rules::original()),
        ("enhanced", Rules::enhanced()),
    ];
    // The data every trace ran on, as the sim parsed it.
    let data = json!({
        "schema": 1,
        "movesets": training_room(Rules::original()).movesets,
        "rules": { "original": Rules::original(), "enhanced": Rules::enhanced() },
    });
    let path = out.join("data.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&data).expect("json")).expect("write");
    println!("{}", path.display());
    for (profile, rules) in profiles {
        let mut scenarios: Vec<(String, Vec<Value>)> = Vec::new();
        scenarios.push((
            "demo".into(),
            run(&mut training_room(rules.clone()), 300, demo),
        ));
        scenarios.push((
            "idle".into(),
            run(&mut training_room(rules.clone()), 600, |_| {
                InputFrame::default()
            }),
        ));
        for seed in [1u64, 2, 3] {
            let mut fuzz = Fuzz::new(seed);
            scenarios.push((
                format!("fuzz{seed}"),
                run(&mut training_room(rules.clone()), 1800, |_| fuzz.frame()),
            ));
        }
        for (name, ticks) in scenarios {
            let path = out.join(format!("{name}.{profile}.json"));
            let doc = json!({
                "schema": 1,
                "scenario": name,
                "profile": profile,
                "setup": "training_room: passive dummy at (0,0,2), melee enemy at (5,0,6) cooldown 90",
                "ticks": ticks,
            });
            std::fs::write(&path, serde_json::to_vec(&doc).expect("json")).expect("write");
            println!("{}", path.display());
        }
    }
}
