//! Local exports for inspection in Blender or any glTF viewer. Output stays on
//! the user's machine (DECISIONS.md, rule 6).

use crate::archive::Source;
use crate::gltf::Glb;
use anyhow::{Context, Result, bail};
use dmc_formats::geometry::{Geometry, Skeleton};
use dmc_formats::model::ModelFile;
use dmc_formats::motion::{FPS, MotionBank};
use dmc_formats::pose;
use dmc_formats::texture::{self, TextureSet};
use serde_json::{Value, json};
use std::fmt;
use std::fs;
use std::path::Path;

/// Room files (`.fsd`) use their own geometry records (docs/formats/README.md §4b)
/// and are exported by the room pipeline instead.
const MODEL_EXTS: &[&str] = &["pld", "pws", "pwd", "emd"];

pub fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(out)
}

/// Write every decodable image in every container as
/// `<stem>_s<set>_<image>.png`; returns how many were written.
pub fn textures_to_png(data: &[u8], out: &Path, stem: &str) -> Result<usize> {
    fs::create_dir_all(out)?;
    let mut written = 0;
    for (si, (off, set)) in texture::scan(data).into_iter().enumerate() {
        for (ii, img) in set.images.iter().enumerate() {
            if let Ok(Some(rgba)) = set.decode_rgba(&data[off..], img) {
                let png = encode_png(img.width as u32, img.height as u32, &rgba)?;
                fs::write(out.join(format!("{stem}_s{si}_{ii}.png")), png)?;
                written += 1;
            }
        }
    }
    Ok(written)
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    pub meshes: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub bones: usize,
    pub textures: usize,
    pub motions: usize,
    pub mean_normal_length: f32,
}

impl Stats {
    /// Geometry read with the right layout has unit normals (PS2-era data can
    /// run a few percent short).
    pub fn looks_valid(&self) -> bool {
        self.triangles > 0 && (0.9..=1.1).contains(&self.mean_normal_length)
    }
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "meshes={:3} verts={:6} tris={:6} bones={:3} tex={:2} anims={:3} nrmlen={:.4}{}",
            self.meshes,
            self.vertices,
            self.triangles,
            self.bones,
            self.textures,
            self.motions,
            self.mean_normal_length,
            if self.looks_valid() {
                ""
            } else {
                "  <-- CHECK"
            }
        )
    }
}

/// Pick the texture container that a mesh group indexes into: the first one
/// holding enough images for every slot used. A heuristic until the model's
/// own texture references are understood (COVERAGE.md).
fn pick_textures(data: &[u8], geo: &Geometry) -> Option<(usize, TextureSet)> {
    let need = geo
        .objects
        .iter()
        .flat_map(|o| &o.meshes)
        .map(|m| m.tex_index as usize)
        .max()?
        + 1;
    texture::scan(data)
        .into_iter()
        .find(|(_, s)| s.images.len() >= need)
}

pub fn model_to_glb(data: &[u8], out: &Path, animations: bool) -> Result<Stats> {
    let (model, geo) = ModelFile::detect(data).context("not a recognised model file")?;
    let mut glb = Glb::default();
    let mut stats = Stats::default();
    let mut doc = json!({ "scene": 0, "scenes": [{ "nodes": [] }], "nodes": [], "meshes": [] });

    // Textures -> images/textures/materials, one material per texture slot.
    let mut materials = Vec::new();
    if let Some((off, set)) = pick_textures(data, &geo) {
        let mut images = Vec::new();
        for img in &set.images {
            let rgba = set
                .decode_rgba(&data[off..], img)?
                .unwrap_or_else(|| vec![255; img.width as usize * img.height as usize * 4]);
            let png = encode_png(img.width as u32, img.height as u32, &rgba)?;
            let view = glb.view(&png, None);
            images.push(json!({ "bufferView": view, "mimeType": "image/png" }));
            materials.push(json!({
                "pbrMetallicRoughness": { "baseColorTexture": { "index": materials.len() }, "metallicFactor": 0.0, "roughnessFactor": 1.0 },
                "alphaMode": "MASK", "doubleSided": true
            }));
        }
        stats.textures = images.len();
        doc["textures"] = json!(
            (0..images.len())
                .map(|i| json!({ "source": i, "sampler": 0 }))
                .collect::<Vec<_>>()
        );
        doc["samplers"] =
            json!([{ "magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497 }]);
        doc["images"] = Value::Array(images);
    }
    let fallback_material = materials.len();
    materials.push(json!({ "pbrMetallicRoughness": { "baseColorFactor": [0.7, 0.7, 0.7, 1.0] }, "doubleSided": true }));
    doc["materials"] = Value::Array(materials);

    // Skeleton -> joint nodes + skin.
    let skeleton = geo.skeleton.as_ref();
    let bone_count = skeleton.map_or(0, |s| s.bone_count());
    let mut roots = Vec::new();
    if let Some(s) = skeleton {
        let nodes = doc["nodes"].as_array_mut().unwrap();
        for i in 0..bone_count {
            let children: Vec<usize> = (0..bone_count)
                .filter(|&c| s.parents[c] == Some(i as u8))
                .collect();
            let mut n = json!({ "name": format!("bone{i:02}"), "translation": s.offsets[i] });
            if !children.is_empty() {
                n["children"] = json!(children);
            }
            nodes.push(n);
            if s.parents[i].is_none() {
                roots.push(i);
            }
        }
        let ibm: Vec<[f32; 16]> = s
            .bind_positions()
            .iter()
            .map(|p| {
                [
                    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., -p[0], -p[1], -p[2], 1.,
                ]
            })
            .collect();
        let acc = glb.floats(&ibm, "MAT4", false);
        doc["skins"] = json!([{ "joints": (0..bone_count).collect::<Vec<_>>(), "inverseBindMatrices": acc, "skeleton": roots.first() }]);
        stats.bones = bone_count;
        if animations {
            let anims = motion_animations(data, &model, s, &mut glb);
            stats.motions = anims.len();
            if !anims.is_empty() {
                doc["animations"] = Value::Array(anims);
            }
        }
    }

    // One glTF mesh per object, one primitive per DMC mesh.
    let mut nrm_sum = 0.0;
    for (oi, obj) in geo.objects.iter().enumerate() {
        let mut prims = Vec::new();
        for m in &obj.meshes {
            let tris = m.triangles();
            if tris.is_empty() {
                continue;
            }
            let mut attrs = json!({
                "POSITION": glb.floats(&m.positions, "VEC3", true),
                "NORMAL": glb.floats(&m.normals, "VEC3", false),
                "TEXCOORD_0": glb.floats(&m.uvs, "VEC2", false),
            });
            if bone_count > 0 {
                let max = (bone_count - 1) as u8;
                let joints: Vec<[u8; 4]> = m
                    .joints
                    .iter()
                    .map(|j| [j[0].min(max), j[1].min(max), j[2].min(max), 0])
                    .collect();
                let weights: Vec<[f32; 4]> =
                    m.weights.iter().map(|w| [w[0], w[1], w[2], 0.0]).collect();
                attrs["JOINTS_0"] = json!(glb.joints(&joints));
                attrs["WEIGHTS_0"] = json!(glb.floats(&weights, "VEC4", false));
            }
            let material = if (m.tex_index as usize) < stats.textures {
                m.tex_index as usize
            } else {
                fallback_material
            };
            prims.push(
                json!({ "attributes": attrs, "indices": glb.indices(&tris), "material": material }),
            );
            stats.meshes += 1;
            stats.vertices += m.vertex_count();
            stats.triangles += tris.len();
            nrm_sum += m.mean_normal_length() * m.vertex_count() as f32;
        }
        if prims.is_empty() {
            continue;
        }
        let meshes = doc["meshes"].as_array_mut().unwrap();
        meshes.push(json!({ "name": format!("obj{oi:02}"), "primitives": prims }));
        let mut node = json!({ "name": format!("obj{oi:02}"), "mesh": meshes.len() - 1 });
        if bone_count > 0 {
            node["skin"] = json!(0);
        }
        let nodes = doc["nodes"].as_array_mut().unwrap();
        nodes.push(node);
        roots.push(nodes.len() - 1);
    }
    if stats.meshes == 0 {
        bail!("model has no drawable meshes");
    }
    stats.mean_normal_length = nrm_sum / stats.vertices as f32;
    doc["scenes"][0]["nodes"] = json!(roots);

    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(out, glb.finish(doc))?;
    Ok(stats)
}

/// Every motion in every motion bank of the model, as glTF animations on the
/// joint nodes (0..bones), sampled once per 60 fps frame through
/// [`pose::sample`] so the export shows exactly what the engine plays.
fn motion_animations(
    data: &[u8],
    model: &ModelFile,
    skeleton: &Skeleton,
    glb: &mut Glb,
) -> Vec<Value> {
    let mut out = Vec::new();
    for sec in &model.sections {
        if sec.index == model.layout.geometry_section() || Some(sec.index) == coat_bank(model) {
            continue;
        }
        let Some(bytes) = model.section(data, sec.index) else {
            continue;
        };
        let Ok(bank) = MotionBank::parse(bytes, model.endian) else {
            continue;
        };
        for (mi, motion) in bank.motions.iter().enumerate() {
            let Some(motion) = motion else { continue };
            let frames = motion.frames.max(1) as usize;
            let poses: Vec<_> = (0..frames)
                .map(|f| pose::sample(skeleton, motion, f as f32))
                .collect();
            let times: Vec<[f32; 1]> = (0..frames).map(|f| [f as f32 / FPS]).collect();
            let input = glb.samples(&times, "SCALAR", true);
            let (mut samplers, mut channels) = (Vec::new(), Vec::new());
            for joint in 0..skeleton.bone_count() {
                let local = |f: fn(&pose::Local) -> Vec<f32>| -> Vec<Vec<f32>> {
                    poses.iter().map(|p| f(&p.locals[joint])).collect()
                };
                let t = local(|l| l.translation.to_array().to_vec());
                let r = local(|l| l.rotation.to_array().to_vec());
                let s = local(|l| l.scale.to_array().to_vec());
                let mut add = |path: &str, output: usize| {
                    channels.push(json!({ "sampler": samplers.len(), "target": { "node": joint, "path": path } }));
                    samplers.push(
                        json!({ "input": input, "output": output, "interpolation": "LINEAR" }),
                    );
                };
                add("translation", glb.samples(&arrays::<3>(&t), "VEC3", false));
                add("rotation", glb.samples(&arrays::<4>(&r), "VEC4", false));
                if s.iter().any(|v| v.iter().any(|&x| x != 1.0)) {
                    add("scale", glb.samples(&arrays::<3>(&s), "VEC3", false));
                }
            }
            out.push(json!({
                "name": format!("s{}_m{mi:03}", sec.index),
                "samplers": samplers,
                "channels": channels,
            }));
        }
    }
    out
}

/// Player bodies (11 sections on PC) keep the coat's motions in section 7;
/// they drive the separate coat skeleton in section 1, not the body.
fn coat_bank(model: &ModelFile) -> Option<usize> {
    (model.layout == dmc_formats::model::Layout::Counted && model.sections.len() == 11).then_some(7)
}

fn arrays<const N: usize>(v: &[Vec<f32>]) -> Vec<[f32; N]> {
    v.iter().map(|x| std::array::from_fn(|i| x[i])).collect()
}

pub fn batch(source: &Path, out: &Path, pattern: Option<&str>, animations: bool) -> Result<()> {
    let mut src = Source::open(source)?;
    let (mut ok, mut suspicious, mut failed) = (0, 0, Vec::new());
    for entry in src.entries()? {
        let name = entry
            .name
            .rsplit('/')
            .next()
            .unwrap_or(&entry.name)
            .to_ascii_lowercase();
        let wanted = match pattern {
            Some(p) => crate::wildcard::matches(&p.to_ascii_lowercase(), &name),
            None => name
                .rsplit_once('.')
                .is_some_and(|(_, e)| MODEL_EXTS.contains(&e)),
        };
        if !wanted {
            continue;
        }
        let dest = out.join(&entry.name).with_extension("glb");
        match src
            .read(&entry.name)
            .and_then(|d| model_to_glb(&d, &dest, animations))
        {
            Ok(s) => {
                println!("OK   {:<40} {s}", entry.name);
                ok += 1;
                suspicious += (!s.looks_valid()) as usize;
            }
            Err(e) => {
                println!("FAIL {:<40} {e:#}", entry.name);
                failed.push(entry.name);
            }
        }
    }
    println!(
        "\n{ok} converted ({suspicious} flagged for review), {} failed",
        failed.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmc_formats::Endian;
    use dmc_formats::bytes::Writer;
    use dmc_formats::geometry::{self, NewMesh, NewSkeleton, STRIP_BREAK};
    use dmc_formats::motion;

    /// A synthetic model file: section list, geometry, then a texture set.
    fn synthetic_model() -> Vec<u8> {
        let e = Endian::Big;
        let mesh = NewMesh {
            tex_index: 0,
            positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]],
            normals: vec![[0., 0., 1.]; 4],
            uvs: vec![[0., 0.], [1., 0.], [0., 1.], [1., 1.]],
            joints: vec![[1, 0, 0]; 4],
            weight_words: vec![STRIP_BREAK, STRIP_BREAK, 0, 0],
        };
        let skel = NewSkeleton {
            parents: vec![0xFF, 0],
            ik_flags: vec![0, 0],
            offsets: vec![[0., 0., 0.], [0., 1., 0.]],
        };
        let geo = geometry::build(e, geometry::Variant::Ps3, 1, &[vec![mesh]], Some(&skel));
        let px = [0x00, 0xF8, 0, 0, 0, 0, 0, 0];
        let tex = texture::build(
            texture::Kind::T32,
            e,
            &[texture::NewImage {
                format_code: 0x26,
                width: 4,
                height: 4,
                pixels: &px,
            }],
        );
        let key = |frame, value| motion::Key {
            frame,
            value,
            in_tangent: 0.0,
            out_tangent: 0.0,
        };
        let swing = motion::Channel {
            tracks: [
                motion::Track {
                    keys: vec![key(0, 0.0), key(9, 1.0)],
                },
                motion::Track::default(),
                motion::Track::default(),
            ],
        };
        let bank = motion::build(
            e,
            &[motion::NewMotion {
                frames: 10,
                channel_ids: vec![1],
                channels: vec![
                    motion::Channel::default(),
                    motion::Channel::default(),
                    swing,
                ],
                frame_words: Vec::new(),
            }],
        );
        let mut w = Writer::new(e);
        let s0 = 16;
        let s1 = s0 + geo.len().next_multiple_of(16);
        let s2 = s1 + tex.len().next_multiple_of(16);
        w.u32(s0 as u32).u32(s1 as u32).u32(s2 as u32).u32(0);
        w.bytes(&geo)
            .pad_to(16, 0)
            .bytes(&tex)
            .pad_to(16, 0)
            .bytes(&bank);
        w.finish()
    }

    #[test]
    fn exports_a_valid_glb() {
        let dir = std::env::temp_dir().join(format!("dmc-cli-test-{}", std::process::id()));
        let out = dir.join("m.glb");
        let stats = model_to_glb(&synthetic_model(), &out, true).unwrap();
        assert_eq!(
            (
                stats.meshes,
                stats.vertices,
                stats.triangles,
                stats.bones,
                stats.textures
            ),
            (1, 4, 2, 2, 1)
        );
        assert!(stats.looks_valid());
        assert_eq!(stats.motions, 1);

        let bytes = fs::read(&out).unwrap();
        let parsed = ::gltf::Gltf::from_slice(&bytes).expect("an independent reader accepts it");
        let anim = parsed.animations().next().expect("one animation");
        assert_eq!(
            anim.channels().count(),
            4,
            "translation + rotation per joint"
        );
        assert_eq!(&bytes[..4], b"glTF");
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
            bytes.len()
        );
        let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let doc: Value = serde_json::from_slice(&bytes[20..20 + json_len]).unwrap();
        assert_eq!(doc["skins"][0]["joints"].as_array().unwrap().len(), 2);
        assert_eq!(doc["images"].as_array().unwrap().len(), 1);
        assert!(doc["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"].is_number());

        assert_eq!(
            textures_to_png(&synthetic_model(), &dir.join("tex"), "m").unwrap(),
            1
        );
        let _ = fs::remove_dir_all(dir);
    }
}
