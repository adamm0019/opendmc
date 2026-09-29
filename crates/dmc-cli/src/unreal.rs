//! `dmc unreal`: everything the Unreal project imports, from *your own*
//! install, in one bundle (docs/UNREAL.md §3):
//!
//! ```text
//! <out>/manifest.json            units, axes, every room and model
//! <out>/rooms/r100/r100.glb      the room's meshes (room units, Y up)
//!                  r100.collision.glb, r100.props.glb
//!                  r100.cameras.json, r100.triggers.json, r100.lights.json
//! <out>/models/...               every model, skinned, with its motions
//! ```
//!
//! The bundle is game data: it stays on the user's machine and is never
//! committed (DECISIONS.md, rule 6).

use crate::archive::Source;
use crate::room::{RoomStats, room_to_glb};
use anyhow::{Context, Result};
use dmc_formats::room::Room;
use dmc_formats::triggers::room_name;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

/// Bumped when the bundle's layout or conventions change.
const BUNDLE_SCHEMA: u32 = 1;
/// ADR-010: one metre is 450 room units.
const ROOM_UNITS_PER_METRE: f32 = 450.0;

pub fn run(archive: &Path, out: &Path, models: bool) -> Result<()> {
    let mut src = Source::open(archive)?;
    let mut rooms = Vec::new();
    let mut failed = Vec::new();
    for entry in src.entries()? {
        let file = entry.name.rsplit('/').next().unwrap_or(&entry.name);
        let Some(stem) = file
            .to_ascii_lowercase()
            .strip_suffix(".fsd")
            .map(str::to_owned)
        else {
            continue;
        };
        let dir = out.join("rooms").join(&stem);
        let data = src.read(&entry.name)?;
        match room_to_glb(&data, &dir.join(format!("{stem}.glb"))) {
            Ok(stats) => {
                println!("OK   {stem:<8} {stats}");
                rooms.push(room_entry(&stem, &entry.name, &data, &stats, &dir)?);
            }
            Err(e) => {
                println!("FAIL {stem:<8} {e:#}");
                failed.push(json!({ "room": stem, "error": format!("{e:#}") }));
            }
        }
    }

    let mut model_files = Vec::new();
    if models {
        let dir = out.join("models");
        crate::export::batch(archive, &dir, None, true)?;
        collect_glbs(&dir, &dir, &mut model_files)?;
        model_files.sort();
    }

    let manifest = json!({
        "schema": BUNDLE_SCHEMA,
        "source": archive.file_name().map(|n| n.to_string_lossy().into_owned()),
        "tool": concat!("dmc ", env!("CARGO_PKG_VERSION")),
        "units": {
            "length": "room units",
            "room_units_per_metre": ROOM_UNITS_PER_METRE,
            "unreal_cm_per_room_unit": 100.0 / ROOM_UNITS_PER_METRE,
        },
        "axes": "glTF convention: right-handed, +Y up; JSON points use the same axes and units as the meshes",
        "rooms": rooms,
        "failed_rooms": failed,
        "models": model_files,
    });
    fs::create_dir_all(out)?;
    fs::write(
        out.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    println!(
        "\n{} rooms ({} failed), {} models -> {}",
        rooms.len(),
        failed.len(),
        model_files.len(),
        out.display()
    );
    Ok(())
}

/// One room's manifest entry: its files, bounds and doors.
fn room_entry(
    stem: &str,
    entry: &str,
    data: &[u8],
    stats: &RoomStats,
    dir: &Path,
) -> Result<Value> {
    let room = Room::parse(data).with_context(|| format!("{stem}: parsing"))?;
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for obj in &room.geometry.objects {
        for m in &obj.meshes {
            for p in &m.positions {
                let w = obj.to_room(*p);
                for i in 0..3 {
                    lo[i] = lo[i].min(w[i]);
                    hi[i] = hi[i].max(w[i]);
                }
            }
        }
    }
    let doors: Vec<Value> = room
        .triggers(data)
        .map(|t| {
            t.doors()
                .map(|(trigger, door)| {
                    json!({
                        "trigger": trigger.index,
                        "to": room_name(door.room),
                        "arrival": door.arrival,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let files: Vec<String> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    Ok(json!({
        "name": stem,
        "entry": entry,
        "dir": format!("rooms/{stem}"),
        "files": files,
        "bounds": { "min": lo, "max": hi },
        "objects": stats.objects,
        "triangles": stats.triangles,
        "collision_polygons": stats.collision,
        "cameras": stats.cameras,
        "props": stats.props,
        "lights": stats.lights,
        "doors": doors,
        "looks_valid": stats.looks_valid(),
    }))
}

fn collect_glbs(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    for e in fs::read_dir(dir)? {
        let path = e?.path();
        if path.is_dir() {
            collect_glbs(root, &path, out)?;
        } else if path.extension().is_some_and(|x| x == "glb") {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            out.push(format!(
                "models/{}",
                rel.to_string_lossy().replace('\\', "/")
            ));
        }
    }
    Ok(())
}
