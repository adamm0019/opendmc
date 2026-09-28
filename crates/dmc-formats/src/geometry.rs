//! Geometry section of a DMC1 model: objects, meshes, skinning, skeleton.
//! Layout: `docs/formats/README.md` §4–5.

use crate::bytes::{Endian, Reader, Writer};
use crate::error::{FormatError, Result};
use serde::Serialize;

const HEADER: usize = 8;
const OBJECT_STRIDE: usize = 16;
const MESH_STRIDE: usize = 32;
pub const STRIP_BREAK: u16 = 0x8000;

#[derive(Debug, Clone, Serialize)]
pub struct Mesh {
    pub tex_index: u16,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Up to three bone indices per vertex.
    pub joints: Vec<[u8; 3]>,
    /// Normalised weights matching `joints`.
    pub weights: Vec<[f32; 3]>,
    /// `true` where the vertex does not close a triangle (strip restart).
    pub strip_break: Vec<bool>,
}

impl Mesh {
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Triangle list from the strip. Each face is wound so that its geometric
    /// normal agrees with the sum of its vertex normals, which avoids depending
    /// on strip parity across restarts.
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        let mut tris = Vec::new();
        for i in 2..self.positions.len() {
            if self.strip_break[i] {
                continue;
            }
            let (a, b, c) = (i - 2, i - 1, i);
            if a == b || b == c || a == c {
                continue;
            }
            let (pa, pb, pc) = (self.positions[a], self.positions[b], self.positions[c]);
            let n = cross(sub(pb, pa), sub(pc, pa));
            let vn = add(add(self.normals[a], self.normals[b]), self.normals[c]);
            if dot(n, vn) >= 0.0 {
                tris.push([a as u32, b as u32, c as u32]);
            } else {
                tris.push([a as u32, c as u32, b as u32]);
            }
        }
        tris
    }

    /// Mean length of the normals; ≈1.0 for correctly read geometry.
    pub fn mean_normal_length(&self) -> f32 {
        if self.normals.is_empty() {
            return 0.0;
        }
        self.normals.iter().map(|n| dot(*n, *n).sqrt()).sum::<f32>() / self.normals.len() as f32
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Object {
    pub total_verts: u16,
    pub meshes: Vec<Mesh>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Skeleton {
    pub parents: Vec<Option<u8>>,
    pub ik_flags: Vec<u8>,
    /// Bind-pose offset of each bone from its parent (no rotations in bind pose).
    pub offsets: Vec<[f32; 3]>,
    pub lengths: Vec<f32>,
}

impl Skeleton {
    pub fn bone_count(&self) -> usize {
        self.parents.len()
    }

    /// Model-space bind position of every bone.
    pub fn bind_positions(&self) -> Vec<[f32; 3]> {
        let mut out = vec![[0.0; 3]; self.parents.len()];
        for i in 0..self.parents.len() {
            let base = match self.parents[i] {
                Some(p) if (p as usize) < i => out[p as usize],
                _ => [0.0; 3],
            };
            out[i] = add(base, self.offsets[i]);
        }
        out
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Geometry {
    pub bone_count: u8,
    pub tex_count: u8,
    pub objects: Vec<Object>,
    pub skeleton: Option<Skeleton>,
}

impl Geometry {
    pub fn mesh_count(&self) -> usize {
        self.objects.iter().map(|o| o.meshes.len()).sum()
    }

    pub fn vertex_count(&self) -> usize {
        self.objects
            .iter()
            .flat_map(|o| &o.meshes)
            .map(Mesh::vertex_count)
            .sum()
    }

    /// Parse a geometry section; `section` starts at the section base.
    pub fn parse(section: &[u8], endian: Endian) -> Result<Self> {
        let r = Reader::new(section, endian);
        let object_count = r.u8(0)? as usize;
        let bone_count = r.u8(1)?;
        let tex_count = r.u8(2)?;
        let skel_off = r.u32(4)? as usize;
        if object_count == 0 {
            return Err(FormatError::invalid("geometry", "no objects"));
        }

        let mut objects = Vec::with_capacity(object_count);
        for oi in 0..object_count {
            let o = HEADER + oi * OBJECT_STRIDE;
            let mesh_count = r.u8(o)? as usize;
            let total_verts = r.u16(o + 2)?;
            let desc_off = r.u32(o + 4)? as usize;
            let mut meshes = Vec::with_capacity(mesh_count);
            for mi in 0..mesh_count {
                meshes.push(parse_mesh(&r, desc_off + mi * MESH_STRIDE)?);
            }
            let sum: usize = meshes.iter().map(Mesh::vertex_count).sum();
            if sum != total_verts as usize {
                return Err(FormatError::invalid(
                    "geometry",
                    format!("object {oi}: meshes hold {sum} vertices, record says {total_verts}"),
                ));
            }
            objects.push(Object {
                total_verts,
                meshes,
            });
        }

        let skeleton = if skel_off != 0 && bone_count > 0 {
            Some(parse_skeleton(&r, skel_off, bone_count as usize)?)
        } else {
            None
        };
        Ok(Geometry {
            bone_count,
            tex_count,
            objects,
            skeleton,
        })
    }
}

fn parse_mesh(r: &Reader, d: usize) -> Result<Mesh> {
    let n = r.u16(d)? as usize;
    let tex_index = r.u16(d + 2)?;
    let [pos, nrm, uv, bone, weight] = [4, 8, 12, 16, 20].map(|k| r.u32(d + k).map(|v| v as usize));
    let (pos, nrm, uv, bone, weight) = (pos?, nrm?, uv?, bone?, weight?);
    if n < 3 {
        return Err(FormatError::invalid(
            "mesh",
            format!("{n} vertices at 0x{d:x}"),
        ));
    }
    // Validate every array fits before decoding any of them.
    r.bytes(pos, n * 12)?;
    r.bytes(nrm, n * 12)?;
    r.bytes(uv, n * 4)?;
    r.bytes(bone, n * 4)?;
    r.bytes(weight, n * 2)?;

    let mut m = Mesh {
        tex_index,
        positions: Vec::with_capacity(n),
        normals: Vec::with_capacity(n),
        uvs: Vec::with_capacity(n),
        joints: Vec::with_capacity(n),
        weights: Vec::with_capacity(n),
        strip_break: Vec::with_capacity(n),
    };
    for k in 0..n {
        m.positions.push(r.vec3(pos + 12 * k)?);
        m.normals.push(r.vec3(nrm + 12 * k)?);
        let u = r.i16(uv + 4 * k)? as f32 / 4096.0;
        let v = r.i16(uv + 4 * k + 2)? as f32 / 4096.0;
        m.uvs.push([u, 1.0 - v]);
        let b = r.bytes(bone + 4 * k, 4)?;
        m.joints.push([b[1] >> 2, b[2] >> 2, b[3] >> 2]);
        let word = r.u16(weight + 2 * k)?;
        m.strip_break.push(word & STRIP_BREAK != 0);
        m.weights.push(unpack_weights(word));
    }
    Ok(m)
}

/// Three 5-bit weights in bits 0–14; all-zero means fully bound to the first joint.
pub fn unpack_weights(word: u16) -> [f32; 3] {
    let w = [word & 31, (word >> 5) & 31, (word >> 10) & 31];
    let total: u16 = w.iter().sum();
    if total == 0 {
        return [1.0, 0.0, 0.0];
    }
    w.map(|x| x as f32 / total as f32)
}

fn parse_skeleton(r: &Reader, off: usize, bone_count: usize) -> Result<Skeleton> {
    let hier = r.u32(off)? as usize;
    let flags = r.u32(off + 4)? as usize;
    let xforms = r.u32(off + 8)? as usize;
    let count = r.u32(off + 12)? as usize;
    if count != bone_count {
        return Err(FormatError::invalid(
            "skeleton",
            format!("{count} bones, header says {bone_count}"),
        ));
    }
    let parents = r
        .bytes(hier, count)?
        .iter()
        .map(|&p| (p != 0xFF).then_some(p))
        .collect::<Vec<_>>();
    let ik_flags = if flags != 0 {
        r.bytes(flags, count)?.to_vec()
    } else {
        vec![0; count]
    };
    let mut offsets = Vec::with_capacity(count);
    let mut lengths = Vec::with_capacity(count);
    for i in 0..count {
        offsets.push(r.vec3(xforms + 16 * i)?);
        lengths.push(r.f32(xforms + 16 * i + 12)?);
    }
    Ok(Skeleton {
        parents,
        ik_flags,
        offsets,
        lengths,
    })
}

// ---------------------------------------------------------------- writer

pub struct NewMesh {
    pub tex_index: u16,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub joints: Vec<[u8; 3]>,
    /// Raw weight words, including the strip-break bit.
    pub weight_words: Vec<u16>,
}

pub struct NewSkeleton {
    pub parents: Vec<u8>,
    pub ik_flags: Vec<u8>,
    pub offsets: Vec<[f32; 3]>,
}

/// Build a geometry section in the documented layout. Meshes of an object
/// share packed attribute arrays, as in the original files.
pub fn build(
    endian: Endian,
    tex_count: u8,
    objects: &[Vec<NewMesh>],
    skeleton: Option<&NewSkeleton>,
) -> Vec<u8> {
    let mut w = Writer::new(endian);
    let bone_count = skeleton.map_or(0, |s| s.parents.len());
    w.u8(objects.len() as u8)
        .u8(bone_count as u8)
        .u8(tex_count)
        .u8(0)
        .u32(0);
    let obj_table = w.pos();
    w.zeros(objects.len() * OBJECT_STRIDE);

    for (oi, meshes) in objects.iter().enumerate() {
        let desc = w.pos();
        w.zeros(meshes.len() * MESH_STRIDE);
        // Attribute-major packing: every mesh's positions, then every mesh's
        // normals, and so on.
        let mut starts = vec![[0usize; 5]; meshes.len()];
        #[allow(clippy::needless_range_loop)] // `a` selects the attribute, not just an index
        for a in 0..5 {
            for (mi, m) in meshes.iter().enumerate() {
                w.pad_to(4, 0xCD);
                starts[mi][a] = w.pos();
                for k in 0..m.positions.len() {
                    match a {
                        0 => w.vec3(m.positions[k]),
                        1 => w.vec3(m.normals[k]),
                        2 => w
                            .i16((m.uvs[k][0] * 4096.0) as i16)
                            .i16(((1.0 - m.uvs[k][1]) * 4096.0) as i16),
                        3 => w
                            .u8(0)
                            .u8(m.joints[k][0] << 2)
                            .u8(m.joints[k][1] << 2)
                            .u8(m.joints[k][2] << 2),
                        _ => w.u16(m.weight_words[k]),
                    };
                }
            }
        }
        let total: usize = meshes.iter().map(|m| m.positions.len()).sum();
        let rec = obj_table + oi * OBJECT_STRIDE;
        w.buf[rec] = meshes.len() as u8;
        let tv = match endian {
            Endian::Big => (total as u16).to_be_bytes(),
            Endian::Little => (total as u16).to_le_bytes(),
        };
        w.buf[rec + 2..rec + 4].copy_from_slice(&tv);
        w.set_u32(rec + 4, desc as u32);
        for (mi, m) in meshes.iter().enumerate() {
            let d = desc + mi * MESH_STRIDE;
            let mut head = Writer::new(endian);
            head.u16(m.positions.len() as u16).u16(m.tex_index);
            for s in starts[mi] {
                head.u32(s as u32);
            }
            w.buf[d..d + head.buf.len()].copy_from_slice(&head.buf);
        }
    }

    if let Some(s) = skeleton {
        w.pad_to(16, 0);
        let skel = w.pos();
        w.set_u32(4, skel as u32);
        w.zeros(16);
        let hier = w.pos();
        w.bytes(&s.parents).pad_to(4, 0);
        let flags = w.pos();
        w.bytes(&s.ik_flags).pad_to(16, 0);
        let xf = w.pos();
        for o in &s.offsets {
            w.vec3(*o).f32(dot(*o, *o).sqrt());
        }
        for (i, v) in [hier, flags, xf, s.parents.len()].into_iter().enumerate() {
            w.set_u32(skel + 4 * i, v as u32);
        }
    }
    w.finish()
}

// ---------------------------------------------------------------- vec helpers

pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(crate) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A unit quad as a 4-vertex strip facing +Z, plus a second 3-vertex mesh.
    pub fn quad_object() -> Vec<NewMesh> {
        let quad = NewMesh {
            tex_index: 0,
            positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]],
            normals: vec![[0., 0., 1.]; 4],
            uvs: vec![[0., 0.], [1., 0.], [0., 1.], [1., 1.]],
            joints: vec![[0, 1, 0]; 4],
            weight_words: vec![STRIP_BREAK | 31, STRIP_BREAK | 31, 31, 31 | (31 << 5)],
        };
        let tri = NewMesh {
            tex_index: 1,
            positions: vec![[0., 0., 1.], [1., 0., 1.], [0., 1., 1.]],
            normals: vec![[0., 0., -1.]; 3],
            uvs: vec![[0.5, 0.5]; 3],
            joints: vec![[1, 0, 0]; 3],
            weight_words: vec![STRIP_BREAK, STRIP_BREAK, 0],
        };
        vec![quad, tri]
    }

    pub fn two_bone_skeleton() -> NewSkeleton {
        NewSkeleton {
            parents: vec![0xFF, 0],
            ik_flags: vec![0, 0],
            offsets: vec![[0., 1., 0.], [0., 0.5, 0.]],
        }
    }

    #[test]
    fn round_trip_both_orders() {
        for e in [Endian::Big, Endian::Little] {
            let bytes = build(e, 2, &[quad_object()], Some(&two_bone_skeleton()));
            let g = Geometry::parse(&bytes, e).unwrap();
            assert_eq!(g.objects.len(), 1);
            assert_eq!(g.mesh_count(), 2);
            assert_eq!(g.vertex_count(), 7);
            let quad = &g.objects[0].meshes[0];
            assert_eq!(quad.tex_index, 0);
            assert_eq!(quad.positions[3], [1., 1., 0.]);
            assert_eq!(quad.joints[0], [0, 1, 0]);
            assert_eq!(quad.weights[3], [0.5, 0.5, 0.0]);
            assert!((quad.uvs[1][0] - 1.0).abs() < 1e-3);
            assert!((quad.mean_normal_length() - 1.0).abs() < 1e-6);

            let tris = quad.triangles();
            assert_eq!(tris.len(), 2);
            for t in &tris {
                let [a, b, c] = t.map(|i| quad.positions[i as usize]);
                assert!(cross(sub(b, a), sub(c, a))[2] > 0.0, "faces +Z");
            }
            let tri = &g.objects[0].meshes[1];
            assert_eq!(tri.weights[0], [1.0, 0.0, 0.0]);
            let [a, b, c] = tri.triangles()[0].map(|i| tri.positions[i as usize]);
            assert!(cross(sub(b, a), sub(c, a))[2] < 0.0, "faces -Z");

            let s = g.skeleton.unwrap();
            assert_eq!(s.parents, vec![None, Some(0)]);
            assert_eq!(s.bind_positions()[1], [0., 1.5, 0.]);
            assert!((s.lengths[1] - 0.5).abs() < 1e-6);
        }
    }

    #[test]
    fn wrong_byte_order_fails_cleanly() {
        let bytes = build(Endian::Big, 1, &[quad_object()], None);
        assert!(Geometry::parse(&bytes, Endian::Little).is_err());
    }

    #[test]
    fn truncation_fails_cleanly() {
        let bytes = build(Endian::Big, 1, &[quad_object()], Some(&two_bone_skeleton()));
        for cut in (0..bytes.len()).step_by(7) {
            let _ = Geometry::parse(&bytes[..cut], Endian::Big);
        }
    }
}
