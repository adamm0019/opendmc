use crate::ai::Brain;
use crate::math::V3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Team {
    Player,
    Enemy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttackState {
    pub move_index: usize,
    pub frame: u16,
    /// Unique per started move, so each swing hits a target at most once per window.
    pub instance: u32,
    /// Combo follow-up queued by an early press.
    pub queued: Option<usize>,
    /// `(target actor index, hit window index)` pairs already connected.
    pub connected: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum State {
    /// Grounded free movement (idle/run).
    Locomotion,
    /// Airborne free movement.
    Air,
    Attack(AttackState),
    HitStun {
        ticks: u16,
    },
    Dead,
}

impl State {
    pub fn tag(&self) -> u8 {
        match self {
            State::Locomotion => 0,
            State::Air => 1,
            State::Attack(_) => 2,
            State::HitStun { .. } => 3,
            State::Dead => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Actor {
    pub team: Team,
    /// Index into `Sim::movesets`.
    pub moveset: usize,
    pub pos: V3,
    pub vel: V3,
    /// Horizontal unit vector.
    pub facing: V3,
    pub grounded: bool,
    pub state: State,
    pub health: f32,
    pub max_health: f32,
    /// Ticks left frozen by hit-stop.
    pub hitstop: u8,
    pub radius: f32,
    pub height: f32,
    pub brain: Option<Brain>,
}

impl Actor {
    pub fn new(team: Team, moveset: usize, pos: V3, max_health: f32) -> Self {
        Self {
            team,
            moveset,
            pos,
            vel: V3::ZERO,
            facing: V3::FORWARD,
            grounded: true,
            state: State::Locomotion,
            health: max_health,
            max_health,
            hitstop: 0,
            radius: 0.5,
            height: 1.8,
            brain: None,
        }
    }

    pub fn alive(&self) -> bool {
        !matches!(self.state, State::Dead)
    }

    pub fn attack(&self) -> Option<&AttackState> {
        match &self.state {
            State::Attack(a) => Some(a),
            _ => None,
        }
    }

    /// Free movement state appropriate for the current ground contact.
    pub fn neutral_state(&self) -> State {
        if self.grounded {
            State::Locomotion
        } else {
            State::Air
        }
    }
}
