//! DMC1 motion banks: Hermite-keyed channels at 60 fps.
//! Layout: `docs/formats/README.md` §6.

use crate::bytes::{Endian, Reader, Writer};
use crate::error::{FormatError, Result};
use crate::geometry::Skeleton;
use serde::Serialize;

pub const FPS: f32 = 60.0;
const MAX_MOTIONS: usize = 4096;
const SECOND_CHANNEL: u8 = 0x80;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Key {
    pub frame: u16,
    pub value: f32,
    pub in_tangent: f32,
    pub out_tangent: f32,
}

/// One scalar curve.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Track {
    pub keys: Vec<Key>,
}

impl Track {
    /// Cubic Hermite sample at `frame` (tangents are per frame). Before the
    /// first key and after the last, the end values hold.
    pub fn sample(&self, frame: f32) -> Option<f32> {
        let keys = &self.keys;
        let first = keys.first()?;
        if keys.len() == 1 || frame <= first.frame as f32 {
            return Some(first.value);
        }
        for pair in keys.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if frame <= b.frame as f32 {
                let span = b.frame as f32 - a.frame as f32;
                if span <= 0.0 {
                    return Some(b.value);
                }
                let t = (frame - a.frame as f32) / span;
                let (t2, t3) = (t * t, t * t * t);
                return Some(
                    (2.0 * t3 - 3.0 * t2 + 1.0) * a.value
                        + (t3 - 2.0 * t2 + t) * span * a.out_tangent
                        + (-2.0 * t3 + 3.0 * t2) * b.value
                        + (t3 - t2) * span * b.in_tangent,
                );
            }
        }
        keys.last().map(|k| k.value)
    }
}

/// x, y and z curves of one channel.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Channel {
    pub tracks: [Track; 3],
}

impl Channel {
    pub fn sample(&self, frame: f32) -> [Option<f32>; 3] {
        [0, 1, 2].map(|i| self.tracks[i].sample(frame))
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.iter().all(|t| t.keys.is_empty())
    }
}

/// What a channel drives, once the skeleton is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Target {
    /// Channel 0: a rotation not applied to the body.
    UnusedRotation,
    /// Channel 1: translation added to bone 0.
    RootMotion,
    /// Euler X→Y→Z, radians.
    Rotation(u8),
    Translation(u8),
    Scale(u8),
    /// Knee hinge axis of a two-bone chain, in the root's parent space.
    IkHinge(u8),
    /// Chain end target position, model space.
    IkTarget(u8),
}

#[derive(Debug, Clone, Serialize)]
pub struct Motion {
    pub frames: u16,
    /// Raw channel ids for channels 2.. (`bone | 0x80` for the second channel).
    pub channel_ids: Vec<u8>,
    /// All channels, starting with the two id-less ones.
    pub channels: Vec<Channel>,
    /// Channel 0 as stored on PC: blocks of words before the event table
    /// (`u16 count`, `u16` flags or filler, `count` × u32). The words step by
    /// 0x40 per frame with flag bits above; their meaning is not known yet
    /// (`docs/formats/README.md` §6). Empty when channel 0 is empty or keyed
    /// like the other channels.
    pub frame_words: Vec<u32>,
    /// Offset of this motion's event table from the bank start (format unknown).
    pub event_offset: u32,
}

impl Motion {
    pub fn duration_seconds(&self) -> f32 {
        self.frames as f32 / FPS
    }

    /// Target of channel `index`, resolved against the skeleton's IK flags.
    pub fn target(&self, index: usize, skeleton: &Skeleton) -> Option<Target> {
        match index {
            0 => return Some(Target::UnusedRotation),
            1 => return Some(Target::RootMotion),
            _ => {}
        }
        let id = *self.channel_ids.get(index - 2)?;
        let bone = id & !SECOND_CHANNEL;
        if id & SECOND_CHANNEL == 0 {
            return Some(Target::Rotation(bone));
        }
        let flag = *skeleton.ik_flags.get(bone as usize)?;
        Some(match ik_role(skeleton, bone as usize) {
            IkRole::Root => Target::IkHinge(bone),
            IkRole::End => Target::IkTarget(bone),
            _ if bone == 0 || flag == 8 => Target::Translation(bone),
            _ => Target::Scale(bone),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IkRole {
    None,
    Root,
    Middle,
    End,
}

/// Is `root_flag` a chain root marker? (`3..=6`, `0x11`, `0x12`, … in the
/// known files; `1` marks chain members and `7`, `8`, `0x0F` other things.)
fn is_chain_root_flag(f: u8) -> bool {
    !matches!(f, 0 | 1 | 7 | 8 | 0x0F)
}

/// A chain is a root flag followed, in bone order, by two bones flagged `1`.
pub fn ik_role(skeleton: &Skeleton, bone: usize) -> IkRole {
    let f = &skeleton.ik_flags;
    let chain_at =
        |r: usize| r + 2 < f.len() && is_chain_root_flag(f[r]) && f[r + 1] == 1 && f[r + 2] == 1;
    if chain_at(bone) {
        IkRole::Root
    } else if bone >= 1 && chain_at(bone - 1) {
        IkRole::Middle
    } else if bone >= 2 && chain_at(bone - 2) {
        IkRole::End
    } else {
        IkRole::None
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MotionBank {
    pub motions: Vec<Option<Motion>>,
}

impl MotionBank {
    pub fn parse(data: &[u8], endian: Endian) -> Result<Self> {
        let r = Reader::new(data, endian);
        let count = r.u32(0)? as usize;
        if count > MAX_MOTIONS || r.u32(4)? != 0 {
            return Err(FormatError::invalid(
                "motion bank",
                format!("header count {count}"),
            ));
        }
        let mut motions = Vec::with_capacity(count);
        for i in 0..count {
            let motion_off = r.u32(8 + 8 * i)? as usize;
            let event_off = r.u32(12 + 8 * i)?;
            motions.push(if motion_off == 0 {
                None
            } else {
                Some(parse_motion(&r, motion_off, event_off)?)
            });
        }
        Ok(MotionBank { motions })
    }
}

fn parse_motion(r: &Reader, base: usize, event_off: u32) -> Result<Motion> {
    let frames = r.u16(base)?;
    let channel_count = r.u8(base + 2)? as usize + 1;
    if channel_count < 2 {
        return Err(FormatError::invalid("motion", "fewer than two channels"));
    }
    let ids = r.bytes(base + 4, channel_count - 2)?.to_vec();
    let table = base + 4 + (channel_count - 2).next_multiple_of(4);
    let limit = (event_off as usize).saturating_sub(base);
    let mut channels = Vec::with_capacity(channel_count);
    let mut frame_words = Vec::new();
    for c in 0..channel_count {
        let rel = r.u32(table + 4 * c)? as usize;
        if rel == 0 || (limit > 0 && rel >= limit) {
            channels.push(Channel::default());
            continue;
        }
        if c == 0
            && let Some(words) = word_list(r, base + rel, base + limit)?
        {
            frame_words = words;
            channels.push(Channel::default());
            continue;
        }
        channels.push(parse_channel(r, base + rel)?);
    }
    Ok(Motion {
        frames,
        channel_ids: ids,
        channels,
        frame_words,
        event_offset: event_off,
    })
}

/// Channel 0 as blocks of words (`u16 count`, `u16` flags or filler, `count`
/// × u32), read while whole blocks fit before `end`. Motions that share a body
/// see different amounts of it, so this never fails; `None` when not even one
/// block fits.
fn word_list(r: &Reader, at: usize, end: usize) -> Result<Option<Vec<u32>>> {
    let mut words = Vec::new();
    let mut pos = at;
    let mut blocks = 0;
    while pos + 4 <= end {
        let n = r.u16(pos)? as usize;
        if n == 0 || pos + 4 + 4 * n > end {
            break;
        }
        for k in 0..n {
            words.push(r.u32(pos + 4 + 4 * k)?);
        }
        pos += 4 + 4 * n;
        blocks += 1;
    }
    Ok((blocks > 0).then_some(words))
}

fn parse_channel(r: &Reader, mut o: usize) -> Result<Channel> {
    let mut ch = Channel::default();
    for track in &mut ch.tracks {
        let n = r.u16(o)? as usize;
        let frames_at = o + 2;
        let values_at = (frames_at + 2 * n).next_multiple_of(4);
        r.bytes(values_at, 12 * n)?;
        track.keys = (0..n)
            .map(|k| {
                Ok(Key {
                    frame: r.u16(frames_at + 2 * k)?,
                    value: r.f32(values_at + 12 * k)?,
                    in_tangent: r.f32(values_at + 12 * k + 4)?,
                    out_tangent: r.f32(values_at + 12 * k + 8)?,
                })
            })
            .collect::<Result<_>>()?;
        o = values_at + 12 * n;
    }
    Ok(ch)
}

// ---------------------------------------------------------------- writer

pub struct NewMotion {
    pub frames: u16,
    pub channel_ids: Vec<u8>,
    pub channels: Vec<Channel>,
    /// Written as channel 0's word list (PC) when not empty.
    pub frame_words: Vec<u32>,
}

/// Build a bank in the documented layout. Each motion's (empty) event table is
/// placed right after it, so `event_offset` bounds its channels as in the game;
/// a channel-0 word list sits directly before it.
pub fn build(endian: Endian, motions: &[NewMotion]) -> Vec<u8> {
    let mut w = Writer::new(endian);
    w.u32(motions.len() as u32).u32(0);
    let table = w.pos();
    w.zeros(8 * motions.len());
    for (i, m) in motions.iter().enumerate() {
        w.pad_to(4, 0);
        let base = w.pos();
        w.u16(m.frames).u8((m.channels.len() - 1) as u8).u8(0);
        w.bytes(&m.channel_ids).pad_to(4, 0);
        let ch_table = w.pos();
        w.zeros(4 * m.channels.len());
        for (c, ch) in m.channels.iter().enumerate() {
            if ch.is_empty() {
                continue;
            }
            w.set_u32(ch_table + 4 * c, (w.pos() - base) as u32);
            for t in &ch.tracks {
                w.u16(t.keys.len() as u16);
                for k in &t.keys {
                    w.u16(k.frame);
                }
                w.pad_to(4, 0);
                for k in &t.keys {
                    w.f32(k.value).f32(k.in_tangent).f32(k.out_tangent);
                }
            }
        }
        if !m.frame_words.is_empty() {
            w.set_u32(ch_table, (w.pos() - base) as u32);
            w.u16(m.frame_words.len() as u16).bytes(&[0x44, 0x44]);
            for &v in &m.frame_words {
                w.u32(v);
            }
        }
        let events = w.pos();
        w.u32(0);
        w.set_u32(table + 8 * i, base as u32);
        w.set_u32(table + 8 * i + 4, events as u32);
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear(frames: &[(u16, f32)]) -> Track {
        // Tangents equal to the slope give a straight line through the keys.
        let keys = frames
            .windows(2)
            .map(|p| (p[1].1 - p[0].1) / (p[1].0 - p[0].0) as f32)
            .collect::<Vec<_>>();
        Track {
            keys: frames
                .iter()
                .enumerate()
                .map(|(i, &(frame, value))| {
                    let s = keys.get(i).or(keys.last()).copied().unwrap_or(0.0);
                    Key {
                        frame,
                        value,
                        in_tangent: s,
                        out_tangent: s,
                    }
                })
                .collect(),
        }
    }

    #[test]
    fn hermite_with_slope_tangents_is_linear() {
        let t = linear(&[(0, 0.0), (10, 10.0)]);
        for f in 0..=10 {
            assert!((t.sample(f as f32).unwrap() - f as f32).abs() < 1e-5);
        }
        assert_eq!(t.sample(-5.0), Some(0.0));
        assert_eq!(t.sample(99.0), Some(10.0));
        assert_eq!(Track::default().sample(0.0), None);
    }

    #[test]
    fn round_trip_both_orders() {
        for e in [Endian::Big, Endian::Little] {
            let root = Channel {
                tracks: [
                    linear(&[(0, 0.0), (30, 3.0)]),
                    Track::default(),
                    Track::default(),
                ],
            };
            let rot = Channel {
                tracks: [
                    linear(&[(0, 0.5)]),
                    linear(&[(0, 0.0), (15, 1.0), (30, 0.0)]),
                    Track::default(),
                ],
            };
            let bytes = build(
                e,
                &[NewMotion {
                    frames: 30,
                    channel_ids: vec![3, 3 | 0x80],
                    channels: vec![
                        Channel::default(),
                        root.clone(),
                        rot.clone(),
                        Channel::default(),
                    ],
                    frame_words: vec![0, 0x40, 0x0800_0080],
                }],
            );
            let bank = MotionBank::parse(&bytes, e).unwrap();
            let m = bank.motions[0].as_ref().unwrap();
            assert_eq!(m.frames, 30);
            assert_eq!(m.channels.len(), 4);
            assert!(m.channels[0].is_empty());
            assert_eq!(m.frame_words, vec![0, 0x40, 0x0800_0080]);
            assert_eq!(m.channels[1], root);
            assert_eq!(m.channels[2], rot);
            assert!((m.duration_seconds() - 0.5).abs() < 1e-6);
            assert!((m.channels[1].sample(15.0)[0].unwrap() - 1.5).abs() < 1e-5);
        }
    }

    #[test]
    fn classifies_ik_channels() {
        let skel = Skeleton {
            parents: vec![None, Some(0), Some(1), Some(2), Some(0)],
            ik_flags: vec![0, 3, 1, 1, 8],
            offsets: vec![[0.0; 3]; 5],
            lengths: vec![0.0; 5],
        };
        let m = Motion {
            frames: 1,
            channel_ids: vec![0x80, 1, 0x81, 0x83, 0x82, 0x84],
            channels: vec![Channel::default(); 8],
            frame_words: Vec::new(),
            event_offset: 0,
        };
        let targets: Vec<_> = (0..8).map(|i| m.target(i, &skel).unwrap()).collect();
        assert_eq!(
            targets,
            vec![
                Target::UnusedRotation,
                Target::RootMotion,
                Target::Translation(0),
                Target::Rotation(1),
                Target::IkHinge(1),
                Target::IkTarget(3),
                Target::Scale(2),
                Target::Translation(4),
            ]
        );
    }
}
