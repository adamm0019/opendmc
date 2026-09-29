//! `--demo` drives the player from a built-in script instead of the keyboard;
//! `--screenshot <png>` saves one frame at `--at-tick` and exits. Together they
//! give reproducible images for visual checks (docs/PLAN.md §4).

use crate::Options;
use crate::play::SimState;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use dmc_sim::V3;
use dmc_sim::input::{InputFrame, button};

pub struct CapturePlugin;

/// Rendered frames to wait before a capture is allowed.
const WARMUP_FRAMES: u32 = 60;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CaptureState::default())
            .add_systems(Update, capture);
    }
}

#[derive(Resource, Default)]
struct CaptureState {
    requested_at_frame: Option<u32>,
    frames: u32,
    /// Frame times after the first half of the warm-up, in milliseconds.
    frame_ms: Vec<f32>,
}

/// Mean and 95th percentile of the frame times, in milliseconds.
fn frame_stats(ms: &[f32]) -> Option<(f32, f32)> {
    if ms.is_empty() {
        return None;
    }
    let mut sorted = ms.to_vec();
    sorted.sort_by(f32::total_cmp);
    let p95 = sorted[((sorted.len() - 1) as f32 * 0.95) as usize];
    Some((ms.iter().sum::<f32>() / ms.len() as f32, p95))
}

/// The scripted demo: lock on, launch the dummy, jump after it, air combo.
/// Ticks are sim ticks.
pub fn demo_input(tick: u64) -> InputFrame {
    let lock = InputFrame::default().with(button::LOCK_ON);
    let back = lock.toward(V3::new(0.0, 0.0, -1.0));
    match tick {
        0..=9 => lock,
        10 => back.with(button::MELEE),
        11..=29 => back,
        30 => lock.with(button::JUMP),
        31..=41 => lock,
        42 | 52 => lock.with(button::MELEE),
        _ => lock,
    }
}

fn capture(
    mut commands: Commands,
    time: Res<Time>,
    options: Res<Options>,
    state: Res<SimState>,
    mut cap: ResMut<CaptureState>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &options.screenshot else {
        return;
    };
    cap.frames += 1;
    if cap.frames > WARMUP_FRAMES / 2 && cap.requested_at_frame.is_none() {
        cap.frame_ms.push(time.delta_secs() * 1000.0);
    }
    match cap.requested_at_frame {
        // The sim holds at `at_tick`, so waiting for the renderer to warm up
        // (pipelines compile asynchronously) cannot change what is captured.
        None if state.sim.tick >= options.at_tick && cap.frames >= WARMUP_FRAMES => {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path.clone()));
            cap.requested_at_frame = Some(cap.frames);
            if let Some((mean, p95)) = frame_stats(&cap.frame_ms) {
                info!(
                    "frame time over {} frames: mean {mean:.2} ms ({:.0} fps), p95 {p95:.2} ms",
                    cap.frame_ms.len(),
                    1000.0 / mean
                );
            }
        }
        // Give the render world a few frames to deliver the capture.
        Some(f) if cap.frames > f + 10 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
