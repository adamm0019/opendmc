//! Input tapes: a recorded run is the list of per-tick inputs. Because the sim
//! is deterministic, a tape plus its starting setup reproduces the run
//! exactly, which is what parity tests and bug reports are built on.

use crate::input::InputFrame;

const MAGIC: &[u8; 4] = b"ODT1";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tape {
    pub frames: Vec<InputFrame>,
}

impl Tape {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + self.frames.len() * 6);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(self.frames.len() as u32).to_le_bytes());
        for f in &self.frames {
            out.extend_from_slice(&f.buttons.to_le_bytes());
            out.extend_from_slice(&f.move_x.to_le_bytes());
            out.extend_from_slice(&f.move_z.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.get(..4)? != MAGIC {
            return None;
        }
        let n = u32::from_le_bytes(b.get(4..8)?.try_into().ok()?) as usize;
        let body = b.get(8..8 + n.checked_mul(6)?)?;
        let frames = body
            .chunks_exact(6)
            .map(|c| InputFrame {
                buttons: u16::from_le_bytes([c[0], c[1]]),
                move_x: i16::from_le_bytes([c[2], c[3]]),
                move_z: i16::from_le_bytes([c[4], c[5]]),
            })
            .collect();
        Some(Tape { frames })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let t = Tape {
            frames: vec![
                InputFrame {
                    buttons: 5,
                    move_x: -3,
                    move_z: 32767
                };
                3
            ],
        };
        assert_eq!(Tape::from_bytes(&t.to_bytes()), Some(t));
        assert_eq!(Tape::from_bytes(b"ODT1\xff\xff\xff\xff"), None);
    }
}
