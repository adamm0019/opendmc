//! Per-tick input and the buffer that makes early presses count.

use crate::math::V3;
use serde::{Deserialize, Serialize};

pub mod button {
    pub const MELEE: u16 = 1 << 0;
    pub const GUN: u16 = 1 << 1;
    pub const JUMP: u16 = 1 << 2;
    pub const LOCK_ON: u16 = 1 << 3;
    pub const DEVIL_TRIGGER: u16 = 1 << 4;
    pub const TAUNT: u16 = 1 << 5;
    pub const COUNT: usize = 6;
}

/// One tick of input. `move_x`/`move_z` are the *world-space* movement
/// direction (already camera-relative, see [`crate::controls`]), scaled so
/// ±32767 is full deflection. Storing world space keeps tapes deterministic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputFrame {
    pub buttons: u16,
    pub move_x: i16,
    pub move_z: i16,
}

impl InputFrame {
    pub fn held(&self, b: u16) -> bool {
        self.buttons & b != 0
    }

    pub fn direction(&self) -> V3 {
        V3::new(
            self.move_x as f32 / 32767.0,
            0.0,
            self.move_z as f32 / 32767.0,
        )
    }

    pub fn with(mut self, b: u16) -> Self {
        self.buttons |= b;
        self
    }

    pub fn toward(mut self, dir: V3) -> Self {
        self.move_x = (dir.x.clamp(-1.0, 1.0) * 32767.0) as i16;
        self.move_z = (dir.z.clamp(-1.0, 1.0) * 32767.0) as i16;
        self
    }
}

/// Stick direction relative to the character, for command inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dir {
    Any,
    Neutral,
    Forward,
    Back,
    Side,
}

impl Dir {
    pub fn of(input: &InputFrame, facing: V3) -> Dir {
        let d = input.direction();
        if d.length() < 0.3 {
            return Dir::Neutral;
        }
        let c = d.normalize_or(facing).dot(facing);
        if c > 0.7 {
            Dir::Forward
        } else if c < -0.7 {
            Dir::Back
        } else {
            Dir::Side
        }
    }

    pub fn accepts(self, actual: Dir) -> bool {
        self == Dir::Any || self == actual
    }
}

const HISTORY: usize = 32;

/// The last [`HISTORY`] ticks of input, newest first, with per-button
/// consumption so one press starts at most one action.
#[derive(Debug, Clone)]
pub struct InputBuffer {
    frames: [(u64, InputFrame); HISTORY],
    len: usize,
    consumed_up_to: [u64; button::COUNT],
}

impl Default for InputBuffer {
    fn default() -> Self {
        Self {
            frames: [(0, InputFrame::default()); HISTORY],
            len: 0,
            consumed_up_to: [0; button::COUNT],
        }
    }
}

impl InputBuffer {
    pub fn push(&mut self, tick: u64, f: InputFrame) {
        self.frames.copy_within(0..HISTORY - 1, 1);
        self.frames[0] = (tick, f);
        self.len = (self.len + 1).min(HISTORY);
    }

    pub fn current(&self) -> InputFrame {
        self.frames[0].1
    }

    /// Tick of the most recent unconsumed press of `b` within the last `window` ticks.
    pub fn pending_press(&self, b: u16, window: usize) -> Option<u64> {
        let slot = b.trailing_zeros() as usize;
        let window = window.min(self.len);
        (0..window).find_map(|age| {
            let (tick, f) = self.frames[age];
            let before = if age + 1 < self.len {
                self.frames[age + 1].1.held(b)
            } else {
                false
            };
            (f.held(b) && !before && tick + 1 > self.consumed_up_to[slot]).then_some(tick)
        })
    }

    /// Mark every press of `b` up to `tick` as used.
    pub fn consume(&mut self, b: u16, tick: u64) {
        let slot = b.trailing_zeros() as usize;
        self.consumed_up_to[slot] = self.consumed_up_to[slot].max(tick + 1);
    }

    pub fn take_press(&mut self, b: u16, window: usize) -> bool {
        match self.pending_press(b, window) {
            Some(t) => {
                self.consume(b, t);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(buf: &mut InputBuffer, frames: &[u16]) {
        for (t, &b) in frames.iter().enumerate() {
            buf.push(
                t as u64,
                InputFrame {
                    buttons: b,
                    ..Default::default()
                },
            );
        }
    }

    #[test]
    fn press_is_an_edge_and_is_consumed_once() {
        let mut b = InputBuffer::default();
        feed(&mut b, &[0, button::MELEE, button::MELEE, button::MELEE]);
        assert!(b.take_press(button::MELEE, 8));
        assert!(
            !b.take_press(button::MELEE, 8),
            "holding is not a new press"
        );
    }

    #[test]
    fn early_press_expires_after_window() {
        let mut b = InputBuffer::default();
        feed(&mut b, &[button::MELEE, 0, 0, 0, 0, 0]);
        assert!(b.pending_press(button::MELEE, 4).is_none());
        assert!(b.pending_press(button::MELEE, 6).is_some());
    }

    #[test]
    fn direction_relative_to_facing() {
        let fwd = V3::FORWARD;
        let f = InputFrame::default();
        assert_eq!(Dir::of(&f, fwd), Dir::Neutral);
        assert_eq!(
            Dir::of(&f.toward(V3::new(0.0, 0.0, 1.0)), fwd),
            Dir::Forward
        );
        assert_eq!(Dir::of(&f.toward(V3::new(0.0, 0.0, -1.0)), fwd), Dir::Back);
        assert_eq!(Dir::of(&f.toward(V3::new(1.0, 0.0, 0.0)), fwd), Dir::Side);
    }
}
