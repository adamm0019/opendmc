//! How the game looks (`docs/REMAKE.md` §3, ADR-012).
//!
//! - `--look modern`, the product: linear HDR, forward PBR, TAA, SSAO,
//!   bloom, per-room exposure and grading, distance fog, volumetric fog lit by
//!   the room's volumetric lights, and moonlight with shadows and shafts.
//!   Screen-space reflections wait for Bevy 0.19.1: in 0.18.1 the deferred
//!   lighting pass fails to compile once an irradiance volume exists, so the
//!   stack renders forward (docs/REMAKE.md §3).
//! - `--look reference`: the original's own meshes with their baked vertex
//!   lighting, unlit, for side-by-side checks.
//!
//! A room's look is authored data: `data/looks/default.ron`, replaced per room
//! by `<content>/rooms/<room>/look.ron` from the private content store
//! (ADR-009). Only the camera stack and lights live here; the room director
//! still decides where the camera is.

use crate::Options;
use crate::play::MainCamera;
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::camera::Exposure;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::{CascadeShadowConfigBuilder, FogVolume, VolumetricFog, VolumetricLight};
use bevy::pbr::{DistanceFog, FogFalloff, ScreenSpaceAmbientOcclusion};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::{ColorGrading, ColorGradingGlobal, ColorGradingSection, Hdr};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The newest look schema this build reads.
pub const LOOK_SCHEMA: u32 = 1;

const DEFAULT_LOOK: &str = include_str!("../../../data/looks/default.ron");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Look {
    #[default]
    Reference,
    Modern,
    /// Trial: real-time ray-traced lighting (feature `rt`, `crate::rt`).
    Rt,
}

impl Look {
    pub fn parse(s: &str) -> Option<Look> {
        match s {
            "reference" => Some(Look::Reference),
            "modern" => Some(Look::Modern),
            "rt" => Some(Look::Rt),
            _ => None,
        }
    }

    /// Lit by the modern stack (with a room look), rasterised or ray traced.
    pub fn is_lit(self) -> bool {
        self != Look::Reference
    }
}

#[cfg_attr(not(feature = "rt"), allow(dead_code))]
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// A glowing panel the ray-traced look adds to a room (moonlight at a
/// window, say).
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Emitter {
    /// Centre, in room units.
    pub position: [f32; 3],
    /// Width and height, metres.
    pub size: [f32; 2],
    /// Which way it shines.
    pub facing: [f32; 3],
    /// Linear RGB.
    pub colour: [f32; 3],
    /// Luminance, cd/m².
    pub nits: f32,
}

/// The ray-traced look's lighting: the room's own lights as glowing spheres
/// (scaled, desaturated, pulled from yellow toward amber), plus emitters.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct RtLook {
    pub light_scale: f32,
    /// Metres.
    pub light_radius: f32,
    pub desaturate: f32,
    pub green: f32,
    pub emitters: Vec<Emitter>,
}

impl Default for RtLook {
    fn default() -> Self {
        RtLook {
            light_scale: 1.0,
            light_radius: 0.12,
            desaturate: 0.0,
            green: 1.0,
            emitters: Vec::new(),
        }
    }
}

/// Tone mapping. `BlenderFilmic` is the authoring default: it matches
/// Blender's Filmic view, so a room lit in Blender reads the same here
/// (docs/REMAKE.md §3).
#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Tonemap {
    /// No tone mapping (clipped), for calibrating against Blender's
    /// "Standard" view.
    None,
    AgX,
    TonyMcMapface,
    BlenderFilmic,
    AcesFitted,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Ambient {
    pub colour: [f32; 3],
    /// cd/m².
    pub brightness: f32,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Moon {
    pub colour: [f32; 3],
    /// Lux.
    pub illuminance: f32,
    /// Which way the light travels.
    pub direction: [f32; 3],
    /// Light shafts through the room's haze.
    pub shafts: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Fog {
    pub colour: [f32; 3],
    /// Metres at which things fade to the fog colour.
    pub visibility: f32,
}

/// Volumetric haze filling the room's bounds (lit by the moon's shafts).
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Haze {
    pub colour: [f32; 3],
    pub density: f32,
    pub scattering_asymmetry: f32,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Section {
    pub saturation: f32,
    pub contrast: f32,
    pub gamma: f32,
    pub gain: f32,
    pub lift: f32,
}

impl From<Section> for ColorGradingSection {
    fn from(s: Section) -> Self {
        ColorGradingSection {
            saturation: s.saturation,
            contrast: s.contrast,
            gamma: s.gamma,
            gain: s.gain,
            lift: s.lift,
        }
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Grading {
    pub temperature: f32,
    pub tint: f32,
    pub hue: f32,
    pub post_saturation: f32,
    pub shadows: Section,
    pub midtones: Section,
    pub highlights: Section,
}

/// How much the fog volumes glow without a volumetric light (in-scattered
/// ambient light), and how finely they are marched.
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Volumetric {
    pub ambient_colour: [f32; 3],
    pub ambient_intensity: f32,
    pub step_count: u32,
}

impl Default for Volumetric {
    fn default() -> Self {
        Volumetric {
            ambient_colour: [1.0; 3],
            ambient_intensity: 0.0,
            step_count: 64,
        }
    }
}

/// One room's look (schema [`LOOK_SCHEMA`]).
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct RoomLook {
    pub schema: u32,
    pub exposure_ev100: f32,
    pub tonemapping: Tonemap,
    /// Bloom intensity; 0 turns it off.
    pub bloom: f32,
    pub ambient: Ambient,
    pub moon: Moon,
    pub fog: Fog,
    pub haze: Option<Haze>,
    #[serde(default)]
    pub volumetric: Volumetric,
    #[serde(default)]
    pub rt: RtLook,
    /// Dust motes (`crate::dust`); none when absent.
    #[serde(default)]
    pub dust: Option<crate::dust::Dust>,
    pub grading: Grading,
}

impl RoomLook {
    pub fn parse(text: &str) -> Result<RoomLook, String> {
        let look: RoomLook = ron::from_str(text).map_err(|e| e.to_string())?;
        if look.schema > LOOK_SCHEMA {
            return Err(format!(
                "look schema {} is newer than this build's {LOOK_SCHEMA}",
                look.schema
            ));
        }
        Ok(look)
    }

    pub fn default_look() -> RoomLook {
        RoomLook::parse(DEFAULT_LOOK).expect("data/looks/default.ron")
    }

    /// `<content>/rooms/<room>/look.ron`, or the default look.
    pub fn for_room(content: Option<&Path>, room: &Path) -> RoomLook {
        let Some(path) = content.map(|c| look_path(c, room)) else {
            return RoomLook::default_look();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => RoomLook::parse(&text).unwrap_or_else(|e| {
                warn!("{}: {e}; using the default look", path.display());
                RoomLook::default_look()
            }),
            Err(_) => RoomLook::default_look(),
        }
    }
}

/// Where a room's look lives in the content store.
pub fn look_path(content: &Path, room: &Path) -> PathBuf {
    let stem = room
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    content.join("rooms").join(stem).join("look.ron")
}

/// The look in force, and the bounds of the room it lights (metres), which
/// the haze fills.
#[derive(Resource, Clone, Debug)]
pub struct CurrentLook {
    pub look: RoomLook,
    pub bounds: Option<(Vec3, Vec3)>,
    /// Where the room's own coordinates start, in the world.
    pub origin: Vec3,
}

#[derive(Component)]
struct MoonLight;

#[derive(Component)]
struct RoomHaze;

pub struct RenderPlugin {
    pub look: Look,
}

/// Which lit look is running.
#[derive(Resource, Clone, Copy)]
struct LookMode(Look);

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        if !self.look.is_lit() {
            return;
        }
        if self.look == Look::Rt {
            // Solari reads a G-buffer, so opaque materials render deferred.
            // (No irradiance volumes here, so Bevy 0.18.1's deferred bug is
            // not in play.)
            app.insert_resource(bevy::pbr::DefaultOpaqueRendererMethod::deferred());
            #[cfg(feature = "rt")]
            app.add_plugins(crate::rt::RtPlugin);
        }
        app.insert_resource(LookMode(self.look))
            .insert_resource(CurrentLook {
                look: RoomLook::default_look(),
                bounds: None,
                origin: Vec3::ZERO,
            })
            .add_systems(PostStartup, modern_camera)
            .add_systems(Update, apply_look.run_if(resource_changed::<CurrentLook>))
            .add_plugins(crate::dust::DustPlugin);
    }
}

fn rgb(c: [f32; 3]) -> Color {
    Color::linear_rgb(c[0], c[1], c[2])
}

/// The modern stack on the main camera: conventional, or Solari's ray-traced
/// lighting. The per-room values come from [`apply_look`].
fn modern_camera(
    mode: Res<LookMode>,
    mut commands: Commands,
    cams: Query<Entity, With<MainCamera>>,
) {
    for cam in &cams {
        let mut e = commands.entity(cam);
        e.insert((Hdr, Msaa::Off));
        if mode.0 == Look::Rt {
            // No denoiser without DLSS (NVIDIA only); TAA's accumulation is
            // the one that works everywhere.
            e.insert(TemporalAntiAliasing::default());
            #[cfg(feature = "rt")]
            e.insert((
                bevy::solari::prelude::SolariLighting::default(),
                bevy::camera::CameraMainTextureUsages::default()
                    .with(bevy::render::render_resource::TextureUsages::STORAGE_BINDING),
            ));
        } else {
            e.insert((
                TemporalAntiAliasing::default(),
                ScreenSpaceAmbientOcclusion::default(),
            ));
        }
    }
}

fn apply_look(
    mode: Res<LookMode>,
    current: Res<CurrentLook>,
    mut commands: Commands,
    cams: Query<Entity, With<MainCamera>>,
    moons: Query<Entity, With<MoonLight>>,
    hazes: Query<Entity, With<RoomHaze>>,
) {
    let look = &current.look;
    let g = &look.grading;
    for cam in &cams {
        let mut e = commands.entity(cam);
        e.insert((
            Exposure {
                ev100: look.exposure_ev100,
            },
            match look.tonemapping {
                Tonemap::None => Tonemapping::None,
                Tonemap::AgX => Tonemapping::AgX,
                Tonemap::TonyMcMapface => Tonemapping::TonyMcMapface,
                Tonemap::BlenderFilmic => Tonemapping::BlenderFilmic,
                Tonemap::AcesFitted => Tonemapping::AcesFitted,
            },
            AmbientLight {
                color: rgb(look.ambient.colour),
                brightness: look.ambient.brightness,
                affects_lightmapped_meshes: false,
            },
            DistanceFog {
                color: rgb(look.fog.colour),
                falloff: FogFalloff::from_visibility(look.fog.visibility),
                ..default()
            },
            ColorGrading {
                global: ColorGradingGlobal {
                    temperature: g.temperature,
                    tint: g.tint,
                    hue: g.hue,
                    post_saturation: g.post_saturation,
                    ..default()
                },
                shadows: g.shadows.into(),
                midtones: g.midtones.into(),
                highlights: g.highlights.into(),
            },
        ));
        // Volumetric fog samples shadow maps, which ray tracing replaces.
        if mode.0 != Look::Rt {
            e.insert(VolumetricFog {
                ambient_color: rgb(look.volumetric.ambient_colour),
                ambient_intensity: look.volumetric.ambient_intensity,
                step_count: look.volumetric.step_count,
                ..default()
            });
        }
        if look.bloom > 0.0 {
            e.insert(Bloom {
                intensity: look.bloom,
                ..Bloom::NATURAL
            });
        } else {
            e.remove::<Bloom>();
        }
    }

    for moon in &moons {
        commands.entity(moon).despawn();
    }
    let m = &look.moon;
    let dir = Vec3::from_array(m.direction).normalize_or(Vec3::NEG_Y);
    if m.illuminance <= 0.0 {
        return;
    }
    let rt = mode.0 == Look::Rt;
    let mut moon = commands.spawn((
        MoonLight,
        DirectionalLight {
            color: rgb(m.colour),
            illuminance: m.illuminance,
            // Ray tracing replaces shadow maps.
            shadows_enabled: !rt,
            ..default()
        },
        Transform::default().looking_to(dir, Vec3::Y),
        CascadeShadowConfigBuilder {
            num_cascades: 3,
            first_cascade_far_bound: 6.0,
            maximum_distance: 50.0,
            ..default()
        }
        .build(),
    ));
    if m.shafts && !rt {
        moon.insert(VolumetricLight);
    }

    for haze in &hazes {
        commands.entity(haze).despawn();
    }
    if let (Some(h), Some((lo, hi))) = (&look.haze, current.bounds) {
        commands.spawn((
            RoomHaze,
            FogVolume {
                fog_color: rgb(h.colour),
                density_factor: h.density,
                scattering_asymmetry: h.scattering_asymmetry,
                ..default()
            },
            Transform::from_translation((lo + hi) * 0.5).with_scale(hi - lo),
        ));
    }
}

/// Set the look for a room that has just loaded (a no-op in the reference
/// look, which has no [`CurrentLook`]).
pub fn set_room_look(
    commands: &mut Commands,
    options: &Options,
    room: &Path,
    bounds: (Vec3, Vec3),
) {
    if !options.look.is_lit() {
        return;
    }
    commands.insert_resource(CurrentLook {
        look: RoomLook::for_room(options.content.as_deref(), room),
        bounds: Some(bounds),
        origin: crate::room_view::origin(options),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_look_parses() {
        let look = RoomLook::default_look();
        assert_eq!(look.schema, LOOK_SCHEMA);
        assert!(look.moon.illuminance > 0.0);
    }

    #[test]
    fn newer_schemas_are_refused() {
        let newer = DEFAULT_LOOK.replacen("schema: 1", "schema: 99", 1);
        assert!(RoomLook::parse(&newer).is_err());
    }

    #[test]
    fn looks_live_beside_the_room_in_the_content_store() {
        let p = look_path(Path::new("store"), Path::new("x/R100.fsd"));
        assert_eq!(p, Path::new("store/rooms/r100/look.ron"));
    }
}
