//! Room export: `.fsd` → glTF for inspection, one node per room object with
//! its transform. Output stays on the user's machine (DECISIONS.md, rule 6).

use crate::archive::Source;
use crate::export::encode_png;
use crate::gltf::Glb;
use anyhow::{Context, Result, bail};
use dmc_formats::room::{Room, RoomObject};
use serde_json::{Value, json};
use std::fmt;
use std::fs;
use std::path::Path;

/// How far (in room units) an object's transformed vertices may stray from
/// the bounds stored in its record. The bounds are whole numbers.
const BOUNDS_SLACK: f32 = 2.0;
/// About 2% of objects in the PC rooms have bounds that don't match their
/// vertices for reasons not yet understood (docs/formats/README.md §4b), so
/// a room passes when nearly all of its objects agree.
const BOUNDS_PASS: f32 = 0.95;

#[derive(Debug, Default, Clone, Copy)]
pub struct RoomStats {
    pub objects: usize,
    pub meshes: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub textures: usize,
    pub mean_normal_length: f32,
    /// Objects whose transformed vertices fill their stored bounds.
    pub bounds_ok: usize,
}

impl RoomStats {
    pub fn looks_valid(&self) -> bool {
        self.triangles > 0
            && (0.9..=1.1).contains(&self.mean_normal_length)
            && self.bounds_ok as f32 >= BOUNDS_PASS * self.objects as f32
    }
}

impl fmt::Display for RoomStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "objects={:3} meshes={:4} verts={:6} tris={:6} tex={:2} nrmlen={:.4} bounds={}/{}{}",
            self.objects,
            self.meshes,
            self.vertices,
            self.triangles,
            self.textures,
            self.mean_normal_length,
            self.bounds_ok,
            self.objects,
            if self.looks_valid() {
                ""
            } else {
                "  <-- CHECK"
            }
        )
    }
}

/// Do the object's vertices, moved into the room, span its stored bounds?
/// This checks the transform, the vertex arrays and the bounds against each
/// other.
pub fn bounds_match(obj: &RoomObject) -> bool {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in obj.meshes.iter().flat_map(|m| &m.positions) {
        let w = obj.to_room(*p);
        for a in 0..3 {
            min[a] = min[a].min(w[a]);
            max[a] = max[a].max(w[a]);
        }
    }
    (0..3).all(|a| {
        // The stored bounds are i16 and wrap in rooms larger than ±32k units.
        // An axis whose range crosses a wrap point can't be checked.
        let wraps = |v: f32| ((v + 32768.0) / 65536.0).floor();
        if wraps(min[a]) != wraps(max[a]) {
            return true;
        }
        let near = |actual: f32, stored: i16| {
            (actual - stored as f32)
                .rem_euclid(65536.0)
                .min((stored as f32 - actual).rem_euclid(65536.0))
                <= BOUNDS_SLACK
        };
        near(max[a], obj.bounds[2 * a]) && near(min[a], obj.bounds[2 * a + 1])
    })
}

/// glTF wants unit normals. A few rooms store them scaled far down (`r40b`)
/// or as NaN (`r305`); rescale the first and point the second up.
fn unit_normals(normals: &[[f32; 3]]) -> Vec<[f32; 3]> {
    normals
        .iter()
        .map(|n| {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len.is_finite() && len > 0.0 {
                n.map(|v| v / len)
            } else {
                [0.0, 1.0, 0.0]
            }
        })
        .collect()
}

pub fn room_to_glb(data: &[u8], out: &Path) -> Result<RoomStats> {
    let room = Room::parse(data).context("not a room file")?;
    let geo = &room.geometry;
    let mut glb = Glb::default();
    let mut stats = RoomStats {
        objects: geo.objects.len(),
        ..Default::default()
    };
    let mut doc = json!({ "scene": 0, "scenes": [{ "nodes": [] }], "nodes": [], "meshes": [] });

    // Textures -> one material per image of section 34.
    let mut materials = Vec::new();
    if let Some((bytes, set)) = room.textures(data) {
        let mut images = Vec::new();
        for img in &set.images {
            let rgba = set
                .decode_rgba(bytes, img)?
                .unwrap_or_else(|| vec![255; img.width as usize * img.height as usize * 4]);
            let png = encode_png(img.width as u32, img.height as u32, &rgba)?;
            let view = glb.view(&png, None);
            images.push(json!({ "bufferView": view, "mimeType": "image/png" }));
            materials.push(json!({
                "pbrMetallicRoughness": { "baseColorTexture": { "index": materials.len() }, "metallicFactor": 0.0, "roughnessFactor": 1.0 },
                "alphaMode": "MASK"
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
    materials.push(json!({ "pbrMetallicRoughness": { "baseColorFactor": [0.7, 0.7, 0.7, 1.0] } }));
    doc["materials"] = Value::Array(materials);

    let mut roots = Vec::new();
    let mut nrm_sum = 0.0;
    for (oi, obj) in geo.objects.iter().enumerate() {
        stats.bounds_ok += bounds_match(obj) as usize;
        let mut prims = Vec::new();
        for m in &obj.meshes {
            let tris = m.triangles();
            if tris.is_empty() {
                continue;
            }
            // Vertex lighting; channel order and scale are provisional (§4b).
            let colours: Vec<[f32; 3]> = m
                .colours
                .iter()
                .map(|c| c.map(|v| v as f32 / 255.0))
                .collect();
            let attrs = json!({
                "POSITION": glb.floats(&m.positions, "VEC3", true),
                "NORMAL": glb.floats(&unit_normals(&m.normals), "VEC3", false),
                "TEXCOORD_0": glb.floats(&m.uvs, "VEC2", false),
                "COLOR_0": glb.floats(&colours, "VEC3", false),
            });
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
        meshes.push(json!({ "name": format!("obj{oi:03}"), "primitives": prims }));
        let mesh = meshes.len() - 1;
        // glTF matrices are column-major; ours are row-major.
        let t = obj.transform;
        let matrix: Vec<f32> = (0..4).flat_map(|c| (0..4).map(move |r| t[r][c])).collect();
        let nodes = doc["nodes"].as_array_mut().unwrap();
        nodes.push(json!({ "name": format!("obj{oi:03}"), "mesh": mesh, "matrix": matrix }));
        roots.push(nodes.len() - 1);
    }
    if stats.meshes == 0 {
        bail!("room has no drawable meshes");
    }
    stats.mean_normal_length = nrm_sum / stats.vertices as f32;
    doc["scenes"][0]["nodes"] = json!(roots);

    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(out, glb.finish(doc))?;
    Ok(stats)
}

/// `dmc room`: one file (`path` or `archive.nbz::Fsd/r002.fsd`) to one
/// `.glb`, or every `.fsd` in a directory or archive into a directory.
pub fn run(source: &Path, out: &Path, pattern: Option<&str>) -> Result<()> {
    let batch = source.is_dir()
        || fs::File::open(source)
            .and_then(|mut f| {
                use std::io::Read;
                let mut magic = [0; 4];
                f.read_exact(&mut magic).map(|_| magic)
            })
            .is_ok_and(|m| m == crate::archive::ZIP_MAGIC);
    if !batch {
        let stats = room_to_glb(&crate::archive::read_spec(source)?, out)?;
        println!("{}  {stats}", out.display());
        return Ok(());
    }

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
            None => name.ends_with(".fsd"),
        };
        if !wanted {
            continue;
        }
        let dest = out.join(&entry.name).with_extension("glb");
        match src.read(&entry.name).and_then(|d| room_to_glb(&d, &dest)) {
            Ok(s) => {
                println!("OK   {:<24} {s}", entry.name);
                ok += 1;
                suspicious += (!s.looks_valid()) as usize;
            }
            Err(e) => {
                println!("FAIL {:<24} {e:#}", entry.name);
                failed.push(entry.name);
            }
        }
    }
    println!(
        "\n{ok} rooms converted ({suspicious} flagged for review), {} failed",
        failed.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmc_formats::room::{self, NewRoomMesh, NewRoomObject, flag};

    fn tri(offset: f32) -> NewRoomObject {
        let mut transform = [[0.0; 4]; 4];
        for (i, row) in transform.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        transform[1][3] = offset;
        let f = flag::ALWAYS;
        NewRoomObject {
            transform,
            bounds: [1, 0, 1 + offset as i16, offset as i16, 0, 0],
            meshes: vec![NewRoomMesh {
                tex_index: 0,
                positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
                normals: vec![[0., 0., 1.]; 3],
                uvs: vec![[0., 0.]; 3],
                flags: vec![f | flag::RESTART, f | flag::RESTART, f | flag::FRONT],
                colours: vec![[255, 128, 0]; 3],
            }],
        }
    }

    #[test]
    fn bounds_wrap_at_i16() {
        let mut far = tri(0.0);
        far.transform[2][3] = 50_000.0;
        far.bounds[4] = (50_000i32 - 65_536) as i16;
        far.bounds[5] = (50_000i32 - 65_536) as i16;
        let data = room::build(&room::build_geometry(0, &[far]), None);
        let r = Room::parse(&data).unwrap();
        assert!(bounds_match(&r.geometry.objects[0]));
    }

    #[test]
    fn normals_are_made_unit_length() {
        let n = unit_normals(&[[0.0, 2e-5, 0.0], [f32::NAN, 0.0, 0.0], [3.0, 4.0, 0.0]]);
        assert_eq!(n, vec![[0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.6, 0.8, 0.0]]);
    }

    #[test]
    fn exports_a_valid_glb_with_placed_objects() {
        let mut wrong = tri(50.0);
        wrong.bounds[2] = 90; // y max is really 51
        let data = room::build(
            &room::build_geometry(0, &[tri(0.0), tri(50.0), wrong]),
            None,
        );
        let out = std::env::temp_dir().join(format!("dmc-room-test-{}.glb", std::process::id()));
        let stats = room_to_glb(&data, &out).unwrap();
        assert_eq!((stats.objects, stats.meshes, stats.triangles), (3, 3, 3));
        assert_eq!(stats.bounds_ok, 2, "the third object's bounds disagree");
        assert!(
            !stats.looks_valid(),
            "2 of 3 objects is below the pass rate"
        );

        let (doc, _, _) = gltf::import(&out).expect("valid glTF");
        let node = doc.nodes().nth(1).unwrap();
        let (t, _, _) = node.transform().decomposed();
        assert_eq!(t, [0.0, 50.0, 0.0]);
        let _ = fs::remove_file(out);
    }
}
