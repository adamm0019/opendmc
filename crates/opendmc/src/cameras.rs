//! Fixed-camera director data: which authored camera is live for a given
//! player position. Pure logic, no Bevy types, so it can be unit-tested.

use dmc_sim::V3;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct CameraZone {
    pub name: String,
    /// Floor-plane bounds (x, z).
    pub min: (f32, f32),
    pub max: (f32, f32),
    pub eye: V3,
    pub look_at: V3,
    pub fov_degrees: f32,
    #[allow(dead_code)]
    pub source: String,
}

impl CameraZone {
    pub fn contains(&self, p: V3) -> bool {
        p.x >= self.min.0 && p.x <= self.max.0 && p.z >= self.min.1 && p.z <= self.max.1
    }

    pub fn forward(&self) -> V3 {
        (self.look_at - self.eye).normalize_or(V3::FORWARD)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CameraSet {
    #[allow(dead_code)]
    pub room: String,
    pub zones: Vec<CameraZone>,
}

pub const TRAINING_ROOM: &str = include_str!("../../../data/cameras/training_room.ron");

impl CameraSet {
    pub fn from_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }

    /// Keep the current zone while the player is still inside it (so
    /// overlapping zones don't flicker); otherwise take the first match.
    pub fn select(&self, current: Option<usize>, player: V3) -> Option<usize> {
        if let Some(c) = current
            && self.zones.get(c).is_some_and(|z| z.contains(player))
        {
            return Some(c);
        }
        self.zones
            .iter()
            .position(|z| z.contains(player))
            .or(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn training_room_cameras_cover_the_arena() {
        let set = CameraSet::from_ron(TRAINING_ROOM).unwrap();
        for x in [-14.0, 0.0, 14.0] {
            for z in [-14.0, -0.5, 0.5, 14.0] {
                assert!(
                    set.select(None, V3::new(x, 0.0, z)).is_some(),
                    "({x}, {z}) uncovered"
                );
            }
        }
    }

    #[test]
    fn hysteresis_on_shared_edge() {
        let set = CameraSet::from_ron(TRAINING_ROOM).unwrap();
        let south = set.select(None, V3::new(0.0, 0.0, -1.0)).unwrap();
        // z = 0 is inside both zones: stay on the current one.
        assert_eq!(set.select(Some(south), V3::new(0.0, 0.0, 0.0)), Some(south));
        let north = set.select(Some(south), V3::new(0.0, 0.0, 1.0)).unwrap();
        assert_ne!(north, south);
        assert_eq!(set.select(Some(north), V3::new(0.0, 0.0, 0.0)), Some(north));
    }
}
