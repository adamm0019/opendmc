//! The private content store (ADR-009): rooms rebuilt in Blender and exported
//! by `tools/blender/export_room.py`. When a walked room has a manifest
//! (`<content>/rooms/<room>/room.ron`) and the look is modern, its exported
//! scene replaces the reference visuals. Gameplay never reads anything from
//! here (ADR-011): collision, cameras and doors still come from the `.fsd`.

use bevy::asset::io::AssetSourceBuilder;
use bevy::gltf::{GltfAssetLabel, GltfExtras};
use bevy::image::{ImageLoaderSettings, ImageSampler};
use bevy::pbr::Lightmap;
use bevy::prelude::*;
use bevy::scene::SceneInstanceReady;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// The newest manifest schema this build reads.
pub const MANIFEST_SCHEMA: u32 = 1;
/// The asset source the store is mounted as (`content://rooms/r100/...`).
pub const SOURCE: &str = "content";

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct RoomManifest {
    pub schema: u32,
    pub room: String,
    /// The exported scene, relative to the room's folder.
    pub visual: String,
    /// Scales baked lightmaps into the engine's light units. Blender's
    /// exporter turns a watt into 683 lumens, and a Cycles diffuse bake
    /// stores E/π in watts, so a bake matches the exported lights at 683.
    #[serde(default = "default_lightmap_exposure")]
    pub lightmap_exposure: f32,
}

fn default_lightmap_exposure() -> f32 {
    683.0
}

impl RoomManifest {
    pub fn parse(text: &str) -> Result<RoomManifest, String> {
        let m: RoomManifest = ron::from_str(text).map_err(|e| e.to_string())?;
        if m.schema > MANIFEST_SCHEMA {
            return Err(format!(
                "manifest schema {} is newer than this build's {MANIFEST_SCHEMA}",
                m.schema
            ));
        }
        Ok(m)
    }
}

/// Mount the store as the `content` asset source. Must run before the asset
/// plugin is added.
pub fn register_source(app: &mut App, content: &Path) {
    app.register_asset_source(
        SOURCE,
        AssetSourceBuilder::platform_default(&content.to_string_lossy(), None),
    );
}

/// A room's folder name in the store: its file stem, lower case.
pub fn room_stem(room: &Path) -> String {
    room.file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// The room's manifest, if the store has one this build can read.
pub fn read_manifest(content: &Path, room: &Path) -> Option<RoomManifest> {
    let path = content.join("rooms").join(room_stem(room)).join("room.ron");
    let text = std::fs::read_to_string(&path).ok()?;
    RoomManifest::parse(&text)
        .map_err(|e| warn!("{}: {e}; using the reference visuals", path.display()))
        .ok()
}

/// A room's modern scene, fixed up once it has spawned.
#[derive(Component)]
struct RoomScene {
    /// `content://rooms/<room>`.
    dir: String,
    lightmap_exposure: f32,
}

/// Spawn the room's exported scene; the caller parents or tags it.
pub fn spawn_scene(
    commands: &mut Commands,
    assets: &AssetServer,
    room: &Path,
    manifest: &RoomManifest,
) -> Entity {
    let dir = format!("{SOURCE}://rooms/{}", room_stem(room));
    let scene =
        assets.load(GltfAssetLabel::Scene(0).from_asset(format!("{dir}/{}", manifest.visual)));
    info!("{}: modern visuals from {dir}", room.display());
    commands
        .spawn((
            SceneRoot(scene),
            RoomScene {
                dir,
                lightmap_exposure: manifest.lightmap_exposure,
            },
            Transform::default(),
            Visibility::default(),
        ))
        .observe(finish_scene)
        .id()
}

/// The `lightmap` extra of a node, if it has one.
fn lightmap_file(extras: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(extras)
        .ok()?
        .get("lightmap")?
        .as_str()
        .map(str::to_owned)
}

/// Bevy reads a glTF light's colour as sRGB; glTF and Blender mean linear.
fn linear_light_colour(c: &mut Color) {
    let s = c.to_srgba();
    *c = Color::linear_rgb(s.red, s.green, s.blue);
}

/// Once the scene is in the world: each lightmapped node's meshes get their
/// lightmap and their materials the manifest's exposure; lights get their
/// colour corrected and leave the lightmapped surfaces' diffuse to the bake
/// (they still light actors and add specular).
#[allow(clippy::too_many_arguments)]
fn finish_scene(
    ready: On<SceneInstanceReady>,
    mut commands: Commands,
    scenes: Query<&RoomScene>,
    children: Query<&Children>,
    extras: Query<&GltfExtras>,
    mesh_materials: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut points: Query<&mut PointLight>,
    mut spots: Query<&mut SpotLight>,
    assets: Res<AssetServer>,
) {
    let Ok(scene) = scenes.get(ready.entity) else {
        return;
    };
    let mut images: HashMap<String, Handle<Image>> = HashMap::new();
    let (mut lit, mut lights) = (0, 0);
    for e in children.iter_descendants(ready.entity) {
        if let Ok(mut l) = points.get_mut(e) {
            linear_light_colour(&mut l.color);
            l.affects_lightmapped_mesh_diffuse = false;
            lights += 1;
        }
        if let Ok(mut l) = spots.get_mut(e) {
            linear_light_colour(&mut l.color);
            l.affects_lightmapped_mesh_diffuse = false;
            lights += 1;
        }
        let Some(file) = extras.get(e).ok().and_then(|x| lightmap_file(&x.value)) else {
            continue;
        };
        let image = images
            .entry(file.clone())
            .or_insert_with(|| {
                assets.load_with_settings(
                    format!("{}/{file}", scene.dir),
                    |s: &mut ImageLoaderSettings| {
                        s.is_srgb = false;
                        s.sampler = ImageSampler::linear();
                    },
                )
            })
            .clone();
        for mesh in children.get(e).into_iter().flatten() {
            let Ok(material) = mesh_materials.get(*mesh) else {
                continue;
            };
            commands.entity(*mesh).insert(Lightmap {
                image: image.clone(),
                uv_rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                bicubic_sampling: true,
            });
            if let Some(m) = materials.get_mut(&material.0) {
                m.lightmap_exposure = scene.lightmap_exposure;
            }
            lit += 1;
        }
    }
    info!(
        "{}: {lit} lightmapped meshes, {} lightmaps, {lights} lights",
        scene.dir,
        images.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_parse_and_refuse_newer_schemas() {
        let m = RoomManifest::parse(r#"(schema: 1, room: "r100", visual: "visual.glb")"#).unwrap();
        assert_eq!(m.visual, "visual.glb");
        assert_eq!(m.lightmap_exposure, 683.0);
        assert!(RoomManifest::parse(r#"(schema: 2, room: "r100", visual: "v.glb")"#).is_err());
    }

    #[test]
    fn lightmaps_come_from_node_extras() {
        let extras = r#"{"standin": true, "lightmap": "lightmaps/lightmap0.ktx2"}"#;
        assert_eq!(
            lightmap_file(extras).as_deref(),
            Some("lightmaps/lightmap0.ktx2")
        );
        assert_eq!(lightmap_file(r#"{"standin": true}"#), None);
        assert_eq!(room_stem(Path::new("x/R100.fsd")), "r100");
    }
}
