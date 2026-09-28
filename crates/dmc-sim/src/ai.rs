//! Enemy behaviour. Each enemy type is a small state machine with authored
//! parameters; this is the shared shape and a generic melee brain used by the
//! training dummy. Real enemy brains arrive in Phase 7 (`data/enemies/`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeleeBrainParams {
    pub walk_speed: f32,
    /// Distance at which the enemy commits to its attack.
    pub attack_range: f32,
    /// Attack move id in the enemy's move set.
    pub attack: String,
    /// Idle ticks between attacks: `min + rng % spread`.
    pub cooldown_min: u16,
    pub cooldown_spread: u16,
    /// Distance beyond which the enemy stops noticing the player.
    pub leash: f32,
    pub source: String,
}

impl MeleeBrainParams {
    pub fn training_dummy() -> Self {
        Self {
            walk_speed: 2.2,
            attack_range: 1.8,
            attack: "swipe".into(),
            cooldown_min: 50,
            cooldown_spread: 60,
            leash: 30.0,
            source: "placeholder".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Brain {
    Melee {
        params: MeleeBrainParams,
        cooldown: u16,
    },
    /// Stands still and takes hits (training mode).
    Passive,
}

/// What a brain wants to do this tick; the sim applies it.
#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    Idle,
    Walk { toward_player: bool },
    Attack(String),
}

/// Decide for one tick. `distance` is to the player; `rng` is a value the sim
/// drew from its deterministic generator.
pub fn think(brain: &mut Brain, distance: f32, can_act: bool, rng: u32) -> Intent {
    match brain {
        Brain::Passive => Intent::Idle,
        Brain::Melee { params, cooldown } => {
            if !can_act {
                return Intent::Idle;
            }
            if *cooldown > 0 {
                *cooldown -= 1;
            }
            if distance > params.leash {
                Intent::Idle
            } else if distance > params.attack_range {
                Intent::Walk {
                    toward_player: true,
                }
            } else if *cooldown == 0 {
                *cooldown =
                    params.cooldown_min + (rng % params.cooldown_spread.max(1) as u32) as u16;
                Intent::Attack(params.attack.clone())
            } else {
                Intent::Idle
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approaches_then_attacks_then_waits() {
        let mut b = Brain::Melee {
            params: MeleeBrainParams::training_dummy(),
            cooldown: 0,
        };
        assert_eq!(
            think(&mut b, 10.0, true, 0),
            Intent::Walk {
                toward_player: true
            }
        );
        assert_eq!(think(&mut b, 1.0, true, 7), Intent::Attack("swipe".into()));
        assert_eq!(think(&mut b, 1.0, true, 0), Intent::Idle, "cooling down");
        assert_eq!(think(&mut b, 100.0, true, 0), Intent::Idle, "out of leash");
    }
}
