//! OpenDMC game shell.
//!
//! ```text
//! opendmc [--profile original|enhanced] [--model <your model file>
//!         [--motion <section>:<index>] [--focus]] [--record <tape>]
//!         [--demo] [--screenshot <png> [--at-tick N]]
//! ```
//!
//! Without game data it runs the graybox training room: an original capsule
//! figure against a training dummy, driven by `dmc-sim` at a fixed 60 Hz, with
//! fixed cameras. `--model` loads a model file from *your own* install next to
//! the arena, skinned; `--motion` plays one of its motions and `--focus` adds
//! a close-up view of it.

mod asset_view;
mod cameras;
mod capture;
mod play;

use bevy::prelude::*;
use dmc_sim::Rules;
use std::path::PathBuf;

#[derive(Resource, Clone, Default)]
pub struct Options {
    pub enhanced: bool,
    pub model: Option<PathBuf>,
    /// `section:index` of a motion in the model's own banks.
    pub motion: Option<String>,
    /// Show a close-up of the model on the right half of the window.
    pub focus: bool,
    pub record: Option<PathBuf>,
    /// Drive the player from the built-in script (`capture::demo_input`).
    pub demo: bool,
    pub screenshot: Option<PathBuf>,
    pub at_tick: u64,
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options {
        at_tick: 60,
        ..Options::default()
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--profile" => match args.next().as_deref() {
                Some("original") => o.enhanced = false,
                Some("enhanced") => o.enhanced = true,
                other => return Err(format!("unknown profile {other:?}")),
            },
            "--model" => o.model = Some(args.next().ok_or("--model needs a path")?.into()),
            "--motion" => o.motion = Some(args.next().ok_or("--motion needs section:index")?),
            "--focus" => o.focus = true,
            "--record" => o.record = Some(args.next().ok_or("--record needs a path")?.into()),
            "--demo" => o.demo = true,
            "--screenshot" => {
                o.screenshot = Some(args.next().ok_or("--screenshot needs a path")?.into())
            }
            "--at-tick" => {
                o.at_tick = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .ok_or("--at-tick needs a number")?
            }
            "-h" | "--help" => {
                println!(
                    "opendmc [--profile original|enhanced] [--model <file> [--motion <s>:<i>] [--focus]] [--record <tape.odt>] \\
                     [--demo] [--screenshot <png> [--at-tick N]]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(o)
}

fn main() {
    let options = parse_args().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let rules = if options.enhanced {
        Rules::enhanced()
    } else {
        Rules::original()
    };
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "OpenDMC".into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(options)
        .add_plugins(play::PlayPlugin { rules })
        .add_plugins(asset_view::AssetViewPlugin)
        .add_plugins(capture::CapturePlugin)
        .run();
}
