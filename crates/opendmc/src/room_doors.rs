//! The doors of the room being walked (`.fsd` section 3,
//! `docs/formats/README.md` §4g): stepping into a door's volume sends the
//! player to the room it names, arriving at its stored point. Pure logic, no
//! Bevy types, so it can be unit-tested.
//!
//! Whether the original also wants a button press at a door, and which way
//! the player faces on arrival, is still to be checked against captures.

use dmc_formats::triggers::{Door, Triggers, Volume, room_name};
use std::path::{Path, PathBuf};

/// How far above the feet the door volumes are tested, in room units. The
/// volumes stand on the floor (500 units tall in `r100`).
pub const PROBE_HEIGHT: f32 = 100.0;

#[derive(Debug, Clone, Default)]
pub struct RoomDoors {
    doors: Vec<(Volume, Door)>,
    /// Doors only fire once the player has stood outside all of them, so an
    /// arrival on or beside the door back doesn't bounce straight back.
    armed: bool,
}

impl RoomDoors {
    pub fn new(triggers: &Triggers) -> Self {
        RoomDoors {
            doors: triggers
                .doors()
                .map(|(t, d)| (t.volume.clone(), d))
                .collect(),
            armed: false,
        }
    }

    pub fn len(&self) -> usize {
        self.doors.len()
    }

    pub fn door(&self, i: usize) -> Option<Door> {
        self.doors.get(i).map(|(_, d)| *d)
    }

    /// The door the player at `feet` (room units) is taking, if any.
    pub fn update(&mut self, feet: [f32; 3]) -> Option<Door> {
        let probe = [feet[0], feet[1] + PROBE_HEIGHT, feet[2]];
        let inside = self
            .doors
            .iter()
            .find(|(v, _)| v.contains(probe, 0.0))
            .map(|(_, d)| *d);
        if !self.armed {
            self.armed = inside.is_none();
            return None;
        }
        if inside.is_some() {
            self.armed = false;
        }
        inside
    }
}

/// Where the room a door leads to is: beside the current room's file
/// (`r101.fsd` next to `r100.fsd`).
pub fn room_path(current: &Path, room: u16) -> PathBuf {
    current.with_file_name(format!("{}.fsd", room_name(room)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmc_formats::triggers::{NewDoor, build};

    fn doors() -> RoomDoors {
        let section = build(&[
            NewDoor {
                min: [0., 0., 0.],
                max: [100., 500., 50.],
                room: 0x101,
                arrival: [2140., 0., -6325.],
            },
            NewDoor {
                min: [-1000., 0., 0.],
                max: [-900., 500., 50.],
                room: 0x11b,
                arrival: [1., 2., 3.],
            },
        ]);
        RoomDoors::new(&Triggers::parse(&section).unwrap())
    }

    #[test]
    fn walking_in_takes_the_door() {
        let mut d = doors();
        assert_eq!(d.len(), 2);
        assert_eq!(d.update([50., 0., -100.]), None);
        let door = d.update([50., 0., 25.]).expect("inside the first door");
        assert_eq!(door.room, 0x101);
        assert_eq!(door.arrival, [2140., 0., -6325.]);
        // Taken once, not again on the next frames.
        assert_eq!(d.update([50., 0., 25.]), None);
    }

    #[test]
    fn arriving_inside_a_door_waits_until_the_player_leaves() {
        let mut d = doors();
        assert_eq!(d.update([-950., 0., 25.]), None);
        assert_eq!(d.update([-950., 0., 30.]), None);
        assert_eq!(d.update([-950., 0., 100.]), None);
        assert_eq!(d.update([-950., 0., 25.]).map(|d| d.room), Some(0x11b));
    }

    #[test]
    fn feet_below_or_above_a_volume_miss_it() {
        let mut d = doors();
        d.update([500., 0., 500.]);
        assert_eq!(d.update([50., -200., 25.]), None);
        assert_eq!(d.update([50., 600., 25.]), None);
    }

    #[test]
    fn target_rooms_sit_beside_the_current_one() {
        let p = room_path(Path::new("rooms/r100.fsd"), 0x11b);
        assert_eq!(p, Path::new("rooms/r11b.fsd"));
    }
}
