//! Deterministic gameplay core for OpenDMC.
//!
//! * No engine, file or clock access: [`Sim::step`] takes one [`InputFrame`]
//!   and advances exactly one 1/60 s tick.
//! * Everything that decides how the game feels (move frame data, cancel
//!   windows, enemy timings, meter values) is authored data loaded from RON.
//!   Each entry carries a `source` saying how it was measured. The shipped
//!   values are placeholders until they have been measured against the
//!   original (see `docs/PLAN.md` §4 and ADR-004).
//! * The same inputs always give the same state; [`Sim::state_hash`] makes
//!   that testable, and input tapes make it replayable.

pub mod actor;
pub mod ai;
pub mod controls;
pub mod input;
pub mod math;
pub mod meters;
pub mod moves;
pub mod rules;
pub mod sim;
pub mod tape;
pub mod world;

pub use input::{InputFrame, button};
pub use math::V3;
pub use rules::{Profile, Rules};
pub use sim::{Event, Sim};
pub use world::{Body, World};

/// Simulation rate. Motions in the game data are keyed at this rate too.
pub const TICK_HZ: u32 = 60;
pub const DT: f32 = 1.0 / TICK_HZ as f32;
