//! Global tuning and the fidelity profile (ADR-005). Gameplay code reads
//! behaviour switches from here and nowhere else.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    /// Aim for the original's measured behaviour. Parity tests run here.
    Original,
    /// Remaster/remake changes, each behind a visible setting.
    Enhanced,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    pub profile: Profile,
    /// Units/s². Placeholder until measured from jump arcs.
    pub gravity: f32,
    /// Gravity on airborne enemies in hit-stun (juggles float).
    pub juggle_gravity: f32,
    pub run_speed: f32,
    pub jump_speed: f32,
    /// Fraction of run speed available in the air.
    pub air_control: f32,
    /// How many ticks an early button press stays valid.
    pub input_buffer_ticks: usize,
    pub dt_damage_multiplier: f32,
    /// Devil Trigger gauge drained per tick while active.
    pub dt_drain_per_tick: f32,
    pub dt_heal_per_tick: f32,
    /// Style points lost per tick while not scoring.
    pub style_decay_per_tick: f32,
    /// Horizontal half-extent of the (graybox) arena until room collision exists.
    pub arena_half_extent: f32,
    pub source: String,
}

impl Rules {
    pub fn original() -> Self {
        Self {
            profile: Profile::Original,
            gravity: 30.0,
            juggle_gravity: 14.0,
            run_speed: 7.5,
            jump_speed: 10.0,
            air_control: 0.5,
            input_buffer_ticks: 6,
            dt_damage_multiplier: 1.5,
            dt_drain_per_tick: 2.0,
            dt_heal_per_tick: 0.05,
            style_decay_per_tick: 0.4,
            arena_half_extent: 15.0,
            source: "placeholder".into(),
        }
    }

    pub fn enhanced() -> Self {
        Self {
            profile: Profile::Enhanced,
            input_buffer_ticks: 10,
            ..Self::original()
        }
    }

    pub fn from_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }
}

impl Default for Rules {
    fn default() -> Self {
        Self::original()
    }
}
