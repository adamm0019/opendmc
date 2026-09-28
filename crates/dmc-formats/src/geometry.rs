//! Geometry section of a DMC1 model: objects, meshes, skinning, skeleton.
//! Layout: `docs/formats/README.md` §4–5.

use crate::bytes::{Endian, Reader, Writer};
use crate::error::{FormatError, Result};
use serde::Serialize;

pub const STRIP_BREAK: u16 = 0x8000;
/// Alignment filler in the PC build's widened records.
pub const PC_PAD: u8 = 0xCC;

/// Record layout of the geometry section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Variant {
    /// PS3 notes: 32-bit offsets, 16-byte objects, 32-byte mesh descriptors,
    /// attribute-major packing, skeleton offsets relative to the section.
    Ps3,
    /// PC build: the same records widened for 64-bit (u64 offsets with
    /// `0xCC` padding), 24-byte objects, 56-byte mesh descriptors, mesh-major
    /// packing, skeleton offsets relative to the skeleton header.
    Pc64,
    /// PC room props (`.fsd` section 19): [`Variant::Pc64`] with 80-byte mesh
    /// descriptors that add a sixth array, 4 bytes of vertex colour per
    /// vertex (baked lighting, as in the room geometry).
    Pc64Prop,
}

impl Variant {
    fn header(self) -> usize {
        match self {
            Variant::Ps3 => 8,
            Variant::Pc64 | Variant::Pc64Prop => 16,
        }
    }

    fn object_stride(self) -> usize {
        match self {
            Variant::Ps3 => 16,
            Variant::Pc64 | Variant::Pc64Prop => 24,
        }
    }

    fn mesh_stride(self) -> usize {
        match self {
            Variant::Ps3 => 32,
            Variant::Pc64 => 56,
            Variant::Pc64Prop => 80,
        }
    }

    /// Per-vertex arrays a mesh descriptor points at.
    fn arrays(self) -> usize {
        match self {
            Variant::Ps3 | Variant::Pc64 => 5,
            Variant::Pc64Prop => 6,
        }
    }

    fn is_pc(self) -> bool {
        self != Variant::Ps3
    }

    fn ptr_size(self) -> usize {
        match self {
            Variant::Ps3 => 4,
            Variant::Pc64 | Variant::Pc64Prop => 8,
        }
    }

    /// Where the first offset of a record sits (after the small fields and,
    /// on PC, the 4 padding bytes).
    fn ptr_start(self) -> usize {
        self.ptr_size()
    }

    /// V as used for sampling, from V as stored (and back: it is its own
    /// inverse). PS3 notes: flipped. PC: the DDS images are stored top-down
    /// and V is used as is (checked by eye on Dante's texture atlas; props
    /// on the room's images, §4f).
    pub fn texture_v(self, v: f32) -> f32 {
        match self {
            Variant::Ps3 => 1.0 - v,
            Variant::Pc64 | Variant::Pc64Prop => v,
        }
    }

    fn ptr(self, r: &Reader, at: usize) -> Result<usize> {
        match self {
            Variant::Ps3 => Ok(r.u32(at)? as usize),
            Variant::Pc64 | Variant::Pc64Prop => r.offset64(at),
        }
    }
}

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
    /// Vertex colours as stored ([`Variant::Pc64Prop`] only, else empty);
    /// channel order and scale unconfirmed, as for rooms.
    pub colours: Vec<[u8; 4]>,
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
    pub variant: Variant,
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
    pub fn parse(section: &[u8], endian: Endian, variant: Variant) -> Result<Self> {
        let r = Reader::new(section, endian);
        let object_count = r.u8(0)? as usize;
        let bone_count = r.u8(1)?;
        let tex_count = r.u8(2)?;
        let skel_off = variant.ptr(&r, variant.ptr_start())?;
        if object_count == 0 {
            return Err(FormatError::invalid("geometry", "no objects"));
        }
        if variant.is_pc() && r.bytes(4, 4)? != [PC_PAD; 4] {
            return Err(FormatError::invalid("geometry", "no PC padding in header"));
        }

        let mut objects = Vec::with_capacity(object_count);
        for oi in 0..object_count {
            let o = variant.header() + oi * variant.object_stride();
            let mesh_count = r.u8(o)? as usize;
            let total_verts = r.u16(o + 2)?;
            let desc_off = variant.ptr(&r, o + variant.ptr_start())?;
            if mesh_count == 0 {
                return Err(FormatError::invalid(
                    "geometry",
                    format!("object {oi} has no meshes"),
                ));
            }
            let mut meshes = Vec::with_capacity(mesh_count);
            for mi in 0..mesh_count {
                meshes.push(parse_mesh(
                    &r,
                    variant,
                    desc_off + mi * variant.mesh_stride(),
                )?);
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
            let base = if variant.is_pc() { skel_off } else { 0 };
            Some(parse_skeleton(&r, skel_off, base, bone_count as usize)?)
        } else {
            None
        };
        Ok(Geometry {
            variant,
            bone_count,
            tex_count,
            objects,
            skeleton,
        })
    }
}

fn parse_mesh(r: &Reader, variant: Variant, d: usize) -> Result<Mesh> {
    let n = r.u16(d)? as usize;
    let tex_index = r.u16(d + 2)?;
    let at = |k: usize| variant.ptr(r, d + variant.ptr_start() + k * variant.ptr_size());
    let (pos, nrm, uv, bone, weight) = (at(0)?, at(1)?, at(2)?, at(3)?, at(4)?);
    let colour = if variant.arrays() > 5 {
        Some(at(5)?)
    } else {
        None
    };
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
    if let Some(c) = colour {
        r.bytes(c, n * 4)?;
    }

    let mut m = Mesh {
        tex_index,
        positions: Vec::with_capacity(n),
        normals: Vec::with_capacity(n),
        uvs: Vec::with_capacity(n),
        joints: Vec::with_capacity(n),
        weights: Vec::with_capacity(n),
        strip_break: Vec::with_capacity(n),
        colours: Vec::new(),
    };
    for k in 0..n {
        m.positions.push(r.vec3(pos + 12 * k)?);
        m.normals.push(r.vec3(nrm + 12 * k)?);
        let u = r.i16(uv + 4 * k)? as f32 / 4096.0;
        let v = r.i16(uv + 4 * k + 2)? as f32 / 4096.0;
        m.uvs.push([u, variant.texture_v(v)]);
        let b = r.bytes(bone + 4 * k, 4)?;
        m.joints.push([b[1] >> 2, b[2] >> 2, b[3] >> 2]);
        let word = r.u16(weight + 2 * k)?;
        m.strip_break.push(word & STRIP_BREAK != 0);
        m.weights.push(unpack_weights(word));
        if let Some(c) = colour {
            m.colours.push(r.bytes(c + 4 * k, 4)?.try_into().unwrap());
        }
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

/// The skeleton header keeps 32-bit offsets on both builds; they count from
/// `base` (the section on PS3, the skeleton header on PC).
fn parse_skeleton(r: &Reader, off: usize, base: usize, bone_count: usize) -> Result<Skeleton> {
    let rel = |k: usize| -> Result<usize> {
        let v = r.u32(off + k)? as usize;
        Ok(if v == 0 { 0 } else { base + v })
    };
    let (hier, flags, xforms) = (rel(0)?, rel(4)?, rel(8)?);
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

/// Write an offset of the variant's width at `at`.
fn set_ptr(w: &mut Writer, variant: Variant, at: usize, v: usize) {
    match variant {
        Variant::Ps3 => w.set_u32(at, v as u32),
        Variant::Pc64 | Variant::Pc64Prop => w.set_u64(at, v as u64),
    }
}

/// The small fields of a record, then (PC) the padding before its offsets.
fn record_head(w: &mut Writer, variant: Variant, bytes: &[u8]) {
    let start = w.pos();
    w.bytes(bytes);
    if variant.is_pc() {
        w.bytes(&[PC_PAD; 4]);
    }
    w.zeros(variant.ptr_start() - (w.pos() - start));
}

/// Build a geometry section in the documented layout for `variant`. On PS3
/// the meshes of an object share attribute-major arrays; on PC each mesh's
/// arrays are contiguous. Both match the original files. Props get a neutral
/// grey vertex colour.
pub fn build(
    endian: Endian,
    variant: Variant,
    tex_count: u8,
    objects: &[Vec<NewMesh>],
    skeleton: Option<&NewSkeleton>,
) -> Vec<u8> {
    let mut w = Writer::new(endian);
    let bone_count = skeleton.map_or(0, |s| s.parents.len());
    record_head(
        &mut w,
        variant,
        &[objects.len() as u8, bone_count as u8, tex_count, 0],
    );
    w.zeros(variant.ptr_size());
    let obj_table = w.pos();
    w.zeros(objects.len() * variant.object_stride());

    for (oi, meshes) in objects.iter().enumerate() {
        let desc = w.pos();
        w.zeros(meshes.len() * variant.mesh_stride());
        let arrays = variant.arrays();
        let mut starts = vec![[0usize; 6]; meshes.len()];
        let order: Vec<(usize, usize)> = if variant.is_pc() {
            (0..meshes.len())
                .flat_map(|mi| (0..arrays).map(move |a| (a, mi)))
                .collect()
        } else {
            (0..arrays)
                .flat_map(|a| (0..meshes.len()).map(move |mi| (a, mi)))
                .collect()
        };
        for (a, mi) in order {
            let m = &meshes[mi];
            w.pad_to(16, 0xCD);
            starts[mi][a] = w.pos();
            for k in 0..m.positions.len() {
                match a {
                    0 => w.vec3(m.positions[k]),
                    1 => w.vec3(m.normals[k]),
                    2 => w
                        .i16((m.uvs[k][0] * 4096.0) as i16)
                        .i16((variant.texture_v(m.uvs[k][1]) * 4096.0) as i16),
                    3 => w
                        .u8(0)
                        .u8(m.joints[k][0] << 2)
                        .u8(m.joints[k][1] << 2)
                        .u8(m.joints[k][2] << 2),
                    4 => w.u16(m.weight_words[k]),
                    _ => w.bytes(&[0x80; 4]),
                };
            }
        }
        let total: usize = meshes.iter().map(|m| m.positions.len()).sum();
        let rec = obj_table + oi * variant.object_stride();
        let mut head = Writer::new(endian);
        head.u8(meshes.len() as u8).u8(0).u16(total as u16);
        let mut obj = Writer::new(endian);
        record_head(&mut obj, variant, &head.buf);
        w.buf[rec..rec + obj.buf.len()].copy_from_slice(&obj.buf);
        set_ptr(&mut w, variant, rec + variant.ptr_start(), desc);
        for (mi, m) in meshes.iter().enumerate() {
            let d = desc + mi * variant.mesh_stride();
            let mut head = Writer::new(endian);
            head.u16(m.positions.len() as u16).u16(m.tex_index);
            let mut rec = Writer::new(endian);
            record_head(&mut rec, variant, &head.buf);
            w.buf[d..d + rec.buf.len()].copy_from_slice(&rec.buf);
            for (k, s) in starts[mi].into_iter().take(arrays).enumerate() {
                set_ptr(
                    &mut w,
                    variant,
                    d + variant.ptr_start() + k * variant.ptr_size(),
                    s,
                );
            }
        }
    }

    if let Some(s) = skeleton {
        w.pad_to(16, 0);
        let skel = w.pos();
        set_ptr(&mut w, variant, variant.ptr_start(), skel);
        let base = if variant.is_pc() { skel } else { 0 };
        w.zeros(16);
        let hier = w.pos();
        w.bytes(&s.parents).pad_to(4, 0);
        let flags = w.pos();
        w.bytes(&s.ik_flags).pad_to(16, 0);
        let xf = w.pos();
        for o in &s.offsets {
            w.vec3(*o).f32(dot(*o, *o).sqrt());
        }
        for (i, v) in [hier - base, flags - base, xf - base, s.parents.len()]
            .into_iter()
            .enumerate()
        {
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
    fn round_trip_both_orders_and_variants() {
        for (e, v) in [
            (Endian::Big, Variant::Ps3),
            (Endian::Little, Variant::Ps3),
            (Endian::Little, Variant::Pc64),
        ] {
            let bytes = build(e, v, 2, &[quad_object()], Some(&two_bone_skeleton()));
            let g = Geometry::parse(&bytes, e, v).unwrap();
            assert_eq!(g.variant, v);
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
    fn wrong_byte_order_or_variant_fails_cleanly() {
        let bytes = build(Endian::Big, Variant::Ps3, 1, &[quad_object()], None);
        assert!(Geometry::parse(&bytes, Endian::Little, Variant::Ps3).is_err());
        let pc = build(Endian::Little, Variant::Pc64, 1, &[quad_object()], None);
        assert!(Geometry::parse(&pc, Endian::Little, Variant::Ps3).is_err());
        assert!(Geometry::parse(&bytes, Endian::Big, Variant::Pc64).is_err());
    }

    #[test]
    fn pc_records_are_padded_and_widened() {
        let bytes = build(Endian::Little, Variant::Pc64, 1, &[quad_object()], None);
        assert_eq!(&bytes[4..8], &[PC_PAD; 4]);
        // First object record: mesh count, 0, total vertices, padding, u64 offset.
        assert_eq!(
            &bytes[16..24],
            &[2, 0, 7, 0, PC_PAD, PC_PAD, PC_PAD, PC_PAD]
        );
        let desc = u64::from_le_bytes(bytes[24..32].try_into().unwrap()) as usize;
        assert_eq!(desc, 16 + 24);
    }

    #[test]
    fn truncation_fails_cleanly() {
        for (e, v) in [(Endian::Big, Variant::Ps3), (Endian::Little, Variant::Pc64)] {
            let bytes = build(e, v, 1, &[quad_object()], Some(&two_bone_skeleton()));
            for cut in (0..bytes.len()).step_by(7) {
                let _ = Geometry::parse(&bytes[..cut], e, v);
            }
        }
    }
}
