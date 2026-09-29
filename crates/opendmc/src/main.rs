//! OpenDMC game shell.
//!
//! ```text
//! opendmc [--profile original|enhanced] [--look modern|reference] [--content <dir>]
//!         [--model <your model file>
//!         [--motion <section>:<index>] [--focus]] [--room <your .fsd>]
//!         [--avatar <your pl00.pld>] [--walk [--start-camera N] [--through-door N]]
//!         [--record <tape>]
//!         [--demo] [--screenshot <png> [--at-tick N]] [--no-vsync] [--size WxH]
//! ```
//!
//! Without game data it runs the graybox training room: an original capsule
//! figure against a training dummy, driven by `dmc-sim` at a fixed 60 Hz, with
//! fixed cameras. `--model` loads a model file from *your own* install next to
//! the arena, skinned; `--motion` plays one of its motions and `--focus` adds
//! a close-up view of it. `--room` shows one of your rooms; add `--walk` to
//! play in it, on its collision and under its own cameras (`--start-camera`
//! starts in that camera's zone). Its doors lead to the rooms beside it, read
//! from the same folder; `--through-door` takes one straight away.

mod asset_view;
mod avatar;
mod cameras;
mod capture;
mod content;
mod dust;
mod play;
mod render;
mod room_cameras;
mod room_doors;
mod room_view;
#[cfg(feature = "rt")]
mod rt;

use bevy::prelude::*;
use dmc_sim::Rules;
use render::Look;
use std::path::PathBuf;

#[derive(Resource, Clone, Default)]
pub struct Options {
    pub enhanced: bool,
    /// `--look modern|reference` (docs/REMAKE.md §3).
    pub look: Look,
    /// The private content store (ADR-009): rebuilt visuals and room looks.
    pub content: Option<PathBuf>,
    pub model: Option<PathBuf>,
    /// `section:index` of a motion in the model's own banks.
    pub motion: Option<String>,
    /// Show a close-up of the model on the right half of the window.
    pub focus: bool,
    /// `section[:first]`: show 24 motions of a bank side by side.
    pub grid: Option<String>,
    /// A room file (`.fsd`) from your own install.
    pub room: Option<PathBuf>,
    /// A player model (`pl00.pld`) to draw the player with.
    pub avatar: Option<PathBuf>,
    /// With `room`: play in the room (its collision and cameras) instead of
    /// flying through it.
    pub walk: bool,
    /// With `walk`: start in this room camera's zone.
    pub start_camera: Option<usize>,
    /// With `walk`: go straight through this door of the room.
    pub through_door: Option<usize>,
    pub record: Option<PathBuf>,
    /// Drive the player from the built-in script (`capture::demo_input`).
    pub demo: bool,
    pub screenshot: Option<PathBuf>,
    pub at_tick: u64,
    /// Present without waiting for vsync, so the logged frame times show the
    /// real cost.
    pub no_vsync: bool,
    /// Window size in physical pixels.
    pub size: Option<(u32, u32)>,
}

fn parse_args() -> Result<Options, String> {
    let mut o = Options {
        at_tick: 60,
        content: std::env::var_os("OPENDMC_CONTENT").map(PathBuf::from),
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
            "--look" => {
                o.look = args
                    .next()
                    .as_deref()
                    .and_then(Look::parse)
                    .ok_or("--look needs modern, rt or reference")?;
                if o.look == Look::Rt && !cfg!(feature = "rt") {
                    return Err("--look rt needs a build with `--features rt`".into());
                }
            }
            "--content" => o.content = Some(args.next().ok_or("--content needs a path")?.into()),
            "--model" => o.model = Some(args.next().ok_or("--model needs a path")?.into()),
            "--motion" => o.motion = Some(args.next().ok_or("--motion needs section:index")?),
            "--focus" => o.focus = true,
            "--avatar" => o.avatar = Some(args.next().ok_or("--avatar needs a path")?.into()),
            "--room" => o.room = Some(args.next().ok_or("--room needs a path")?.into()),
            "--walk" => o.walk = true,
            "--start-camera" => {
                o.start_camera = Some(
                    args.next()
                        .and_then(|n| n.parse().ok())
                        .ok_or("--start-camera needs a camera index")?,
                )
            }
            "--through-door" => {
                o.through_door = Some(
                    args.next()
                        .and_then(|n| n.parse().ok())
                        .ok_or("--through-door needs a door index")?,
                )
            }
            "--grid" => o.grid = Some(args.next().ok_or("--grid needs section[:first]")?),
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
            "--no-vsync" => o.no_vsync = true,
            "--size" => {
                o.size = args
                    .next()
                    .and_then(|s| {
                        let (w, h) = s.split_once('x')?;
                        Some((w.parse().ok()?, h.parse().ok()?))
                    })
                    .map(Some)
                    .ok_or("--size needs WxH, e.g. 2560x1440")?
            }
            "-h" | "--help" => {
                println!(
                    "opendmc [--profile original|enhanced] [--look modern|reference] [--content <dir>] [--model <file> [--motion <s>:<i>] [--focus]] [--room <file.fsd> [--walk [--start-camera N] [--through-door N]]] [--avatar <pl00.pld>] [--record <tape.odt>] \\
                     [--demo] [--screenshot <png> [--at-tick N]] [--no-vsync] [--size WxH]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if o.walk && o.room.is_none() {
        return Err("--walk needs --room".into());
    }
    if o.start_camera.is_some() && !o.walk {
        return Err("--start-camera needs --walk".into());
    }
    if o.through_door.is_some() && !o.walk {
        return Err("--through-door needs --walk".into());
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
    let mut app = App::new();
    // Asset sources must exist before the asset plugin starts.
    if let Some(store) = &options.content {
        content::register_source(&mut app, store);
    }
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "OpenDMC".into(),
            present_mode: if options.no_vsync {
                bevy::window::PresentMode::AutoNoVsync
            } else {
                bevy::window::PresentMode::AutoVsync
            },
            resolution: match options.size {
                Some((w, h)) => {
                    bevy::window::WindowResolution::new(w, h).with_scale_factor_override(1.0)
                }
                None => default(),
            },
            ..default()
        }),
        ..default()
    }))
    .add_plugins(render::RenderPlugin { look: options.look })
    .insert_resource(options)
    .add_plugins(play::PlayPlugin { rules })
    .add_plugins(asset_view::AssetViewPlugin)
    .add_plugins(room_view::RoomViewPlugin)
    .add_plugins(avatar::AvatarPlugin)
    .add_plugins(capture::CapturePlugin)
    .run();
}
