//! Authored move data (ADR-004). A move is a timeline of frames with hit
//! windows, cancel windows, a combo link and root motion.

use crate::input::{Dir, button};
use crate::math::V3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Button {
    Melee,
    Gun,
    Jump,
    Taunt,
}

impl Button {
    pub fn mask(self) -> u16 {
        match self {
            Button::Melee => button::MELEE,
            Button::Gun => button::GUN,
            Button::Jump => button::JUMP,
            Button::Taunt => button::TAUNT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stance {
    Ground,
    Air,
    Either,
}

/// How a move starts from neutral (locomotion or airborne).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trigger {
    pub button: Button,
    #[serde(default = "any_stance")]
    pub stance: Stance,
    #[serde(default = "any_dir")]
    pub direction: Dir,
    /// Needs lock-on held (command moves).
    #[serde(default)]
    pub lock_on: bool,
}

fn any_stance() -> Stance {
    Stance::Either
}
fn any_dir() -> Dir {
    Dir::Any
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HitWindow {
    /// First and last active frame, inclusive.
    pub frames: (u16, u16),
    /// Sphere centre relative to the attacker (x right, y up, z forward).
    pub offset: V3,
    pub radius: f32,
    pub damage: f32,
    /// Ticks the target can't act.
    pub stun: u16,
    /// Ticks both sides freeze on contact.
    #[serde(default)]
    pub hitstop: u8,
    /// Horizontal push along the attacker's facing (units/s).
    #[serde(default)]
    pub knockback: f32,
    /// Vertical velocity given to the target (launchers, juggles).
    #[serde(default)]
    pub lift: f32,
    #[serde(default)]
    pub style: f32,
    #[serde(default)]
    pub dt_gain: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CancelInto {
    /// Any move whose trigger matches.
    AnyMove,
    /// A specific move by id.
    Move(String),
    Jump,
    /// Back to free movement when the stick is pushed.
    Movement,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelWindow {
    pub frames: (u16, u16),
    pub into: CancelInto,
}

/// Next hit of a combo: pressing `button` during `input` queues `next`,
/// which starts at `link_frame` (or immediately if pressed later).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComboLink {
    pub button: Button,
    pub input: (u16, u16),
    pub link_frame: u16,
    pub next: String,
}

/// Root motion for a frame range. `forward` is units/s along facing;
/// `vertical`, when set, replaces vertical velocity on the first frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotionSegment {
    pub frames: (u16, u16),
    #[serde(default)]
    pub forward: f32,
    #[serde(default)]
    pub vertical: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveDef {
    pub id: String,
    /// `None` for moves reachable only through combos or AI.
    #[serde(default)]
    pub trigger: Option<Trigger>,
    pub total_frames: u16,
    #[serde(default)]
    pub hits: Vec<HitWindow>,
    #[serde(default)]
    pub cancels: Vec<CancelWindow>,
    #[serde(default)]
    pub combo: Option<ComboLink>,
    #[serde(default)]
    pub motion: Vec<MotionSegment>,
    /// Multiplier on gravity while the move runs (air moves hang).
    #[serde(default = "one")]
    pub gravity_scale: f32,
    /// Keep tracking the lock-on target during the first N frames.
    #[serde(default)]
    pub track_frames: u16,
    /// Name of the source motion, for the renderer (e.g. `"pl00/bank6_012"`).
    #[serde(default)]
    pub animation: Option<String>,
    /// How these numbers were obtained (ADR-004).
    pub source: String,
}

fn one() -> f32 {
    1.0
}

impl MoveDef {
    pub fn is_placeholder(&self) -> bool {
        self.source.trim().eq_ignore_ascii_case("placeholder")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveSet {
    pub name: String,
    pub moves: Vec<MoveDef>,
}

#[derive(Debug, thiserror::Error)]
pub enum MoveSetError {
    #[error("parse error: {0}")]
    Parse(#[from] ron::error::SpannedError),
    #[error("move '{0}' defined twice")]
    Duplicate(String),
    #[error("move '{from}' references unknown move '{to}'")]
    UnknownMove { from: String, to: String },
    #[error("move '{id}': {detail}")]
    Invalid { id: String, detail: String },
}

impl MoveSet {
    pub fn from_ron(text: &str) -> Result<Self, MoveSetError> {
        let set: MoveSet = ron::from_str(text)?;
        set.validate()?;
        Ok(set)
    }

    pub fn index(&self, id: &str) -> Option<usize> {
        self.moves.iter().position(|m| m.id == id)
    }

    pub fn placeholders(&self) -> impl Iterator<Item = &MoveDef> {
        self.moves.iter().filter(|m| m.is_placeholder())
    }

    pub fn validate(&self) -> Result<(), MoveSetError> {
        for (i, m) in self.moves.iter().enumerate() {
            if self.moves[..i].iter().any(|o| o.id == m.id) {
                return Err(MoveSetError::Duplicate(m.id.clone()));
            }
            let bad = |detail: String| MoveSetError::Invalid {
                id: m.id.clone(),
                detail,
            };
            let in_range = |(a, b): (u16, u16)| a <= b && b < m.total_frames;
            if m.total_frames == 0 {
                return Err(bad("total_frames is 0".into()));
            }
            for h in &m.hits {
                if !in_range(h.frames) {
                    return Err(bad(format!(
                        "hit window {:?} outside 0..{}",
                        h.frames, m.total_frames
                    )));
                }
            }
            for c in &m.cancels {
                if !in_range(c.frames) {
                    return Err(bad(format!(
                        "cancel window {:?} outside 0..{}",
                        c.frames, m.total_frames
                    )));
                }
                if let CancelInto::Move(to) = &c.into
                    && self.index(to).is_none()
                {
                    return Err(MoveSetError::UnknownMove {
                        from: m.id.clone(),
                        to: to.clone(),
                    });
                }
            }
            if let Some(c) = &m.combo {
                if !in_range(c.input) || c.link_frame >= m.total_frames {
                    return Err(bad("combo windows outside the move".into()));
                }
                if self.index(&c.next).is_none() {
                    return Err(MoveSetError::UnknownMove {
                        from: m.id.clone(),
                        to: c.next.clone(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// Built-in move sets, loaded from the repository's `data/` directory.
pub mod builtin {
    pub const PLAYER_SWORD: &str = include_str!("../../../data/moves/player_sword.ron");
    pub const TRAINING_DUMMY: &str = include_str!("../../../data/moves/training_dummy.ron");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_sets_load_and_validate() {
        let sword = MoveSet::from_ron(builtin::PLAYER_SWORD).unwrap();
        assert!(sword.index("combo_1").is_some());
        assert!(sword.index("launcher").is_some());
        let dummy = MoveSet::from_ron(builtin::TRAINING_DUMMY).unwrap();
        assert!(dummy.index("swipe").is_some());
    }

    #[test]
    fn validation_catches_mistakes() {
        let bad_ref = r#"(name: "x", moves: [(id: "a", total_frames: 10, source: "t",
            combo: Some((button: Melee, input: (0, 5), link_frame: 6, next: "missing")))])"#;
        assert!(matches!(
            MoveSet::from_ron(bad_ref),
            Err(MoveSetError::UnknownMove { .. })
        ));
        let bad_window = r#"(name: "x", moves: [(id: "a", total_frames: 10, source: "t",
            hits: [(frames: (5, 12), offset: (x: 0, y: 0, z: 1), radius: 1, damage: 1, stun: 1)])])"#;
        assert!(matches!(
            MoveSet::from_ron(bad_window),
            Err(MoveSetError::Invalid { .. })
        ));
        let dup = r#"(name: "x", moves: [(id: "a", total_frames: 1, source: "t"), (id: "a", total_frames: 1, source: "t")])"#;
        assert!(matches!(
            MoveSet::from_ron(dup),
            Err(MoveSetError::Duplicate(_))
        ));
    }
}
