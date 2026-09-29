//! DMC1 room files (`.fsd`, PC build): 35 sections behind a `Counted64`
//! table, room geometry in section 14, its textures in section 34.
//! Layout: `docs/formats/README.md` §3.2, §4b and §4c.

use crate::bytes::{Endian, Reader, Writer};
use crate::camera::{self, Cameras};
use crate::collision::{self, Collision};
use crate::error::{FormatError, Result};
use crate::geometry::PC_PAD;
use crate::lights::LightSet;
use crate::model::{Layout, ModelFile};
use crate::props::{self, Props};
use crate::texture::TextureSet;
use crate::triggers::{self, Triggers};
use serde::Serialize;

pub const SECTION_COUNT: usize = 35;
pub const GEOMETRY_SECTION: usize = 14;
pub const TEXTURE_SECTION: usize = 34;
/// Cutscene actors, `.ecd` event scripts and `.fcv` curves (§4c).
pub const EVENT_SECTION: usize = 30;

const HEADER: usize = 16;
const OBJECT_STRIDE: usize = 40;
const MESH_HEADER: usize = 32;
/// Mesh descriptor offsets count in 16-byte units from the descriptor.
const UNIT: usize = 16;
const MATRIX: usize = 64;
const FILL: u8 = 0xCD;

/// Bits of the per-vertex flag byte (the first byte of each colour entry).
pub mod flag {
    /// Triangle `(i-2, i-1, i)` is front-facing as written; clear means reversed.
    pub const FRONT: u8 = 1;
    /// This vertex does not close a triangle.
    pub const RESTART: u8 = 2;
    /// Set on every vertex seen.
    pub const ALWAYS: u8 = 4;
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RoomMesh {
    pub tex_index: u16,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Strip flags, see [`flag`].
    pub flags: Vec<u8>,
    /// Vertex lighting as RGB. The file stores the flag byte, then blue,
    /// green and red (a byte-reversed RGBA word); 0x80 is full brightness
    /// (docs/formats/README.md §4b).
    pub colours: Vec<[u8; 3]>,
}

impl RoomMesh {
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Triangle list from the strip, wound by the per-vertex flag so that
    /// front faces are counter-clockwise.
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        (2..self.positions.len())
            .filter(|&i| self.flags[i] & flag::RESTART == 0)
            .map(|i| {
                let (a, b, c) = (i as u32 - 2, i as u32 - 1, i as u32);
                if self.flags[i] & flag::FRONT != 0 {
                    [a, b, c]
                } else {
                    [a, c, b]
                }
            })
            .collect()
    }

    /// Mean length of the normals; ≈1.0 for correctly read geometry.
    pub fn mean_normal_length(&self) -> f32 {
        if self.normals.is_empty() {
            return 0.0;
        }
        self.normals
            .iter()
            .map(|n| (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt())
            .sum::<f32>()
            / self.normals.len() as f32
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RoomObject {
    /// Eight bytes of unknown flags.
    pub flags: [u8; 8],
    /// World-space bounds as stored: x max, x min, y max, y min, z max, z min.
    pub bounds: [i16; 6],
    /// Object-to-room transform, row-major: `world = transform · [p, 1]`
    /// (translation in the last column).
    pub transform: [[f32; 4]; 4],
    pub meshes: Vec<RoomMesh>,
}

impl RoomObject {
    pub fn to_room(&self, p: [f32; 3]) -> [f32; 3] {
        let m = &self.transform;
        [0, 1, 2].map(|r| m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3])
    }

    pub fn vertex_count(&self) -> usize {
        self.meshes.iter().map(RoomMesh::vertex_count).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RoomGeometry {
    pub tex_count: u8,
    pub objects: Vec<RoomObject>,
}

impl RoomGeometry {
    pub fn mesh_count(&self) -> usize {
        self.objects.iter().map(|o| o.meshes.len()).sum()
    }

    pub fn vertex_count(&self) -> usize {
        self.objects.iter().map(RoomObject::vertex_count).sum()
    }

    /// Parse section 14; offsets are relative to the section start.
    pub fn parse(section: &[u8]) -> Result<Self> {
        let r = Reader::new(section, Endian::Little);
        // Unlike §4, the object count is 16-bit: 29 rooms have over 255.
        let object_count = r.u16(0)? as usize;
        let tex_count = r.u8(2)?;
        if object_count == 0 {
            return Err(FormatError::invalid("room", "no objects"));
        }
        if r.bytes(4, 4)? != [PC_PAD; 4] {
            return Err(FormatError::invalid("room", "no padding in header"));
        }
        let matrices = r.offset64(8)?;

        let mut objects = Vec::with_capacity(object_count);
        for oi in 0..object_count {
            let o = HEADER + oi * OBJECT_STRIDE;
            r.bytes(o, OBJECT_STRIDE)?;
            let mesh_count = r.u8(o)? as usize;
            let total_verts = r.u16(o + 2)? as usize;
            if mesh_count == 0 || r.bytes(o + 4, 4)? != [PC_PAD; 4] {
                return Err(FormatError::invalid(
                    "room",
                    format!("object {oi}: bad record"),
                ));
            }
            let mut at = r.offset64(o + 8)?;
            let mut meshes = Vec::with_capacity(mesh_count);
            for mi in 0..mesh_count {
                let (mesh, next) = parse_mesh(&r, at).map_err(|e| {
                    FormatError::invalid("room", format!("object {oi} mesh {mi}: {e}"))
                })?;
                meshes.push(mesh);
                at = next;
            }
            let transform = matrix(&r, matrices + oi * MATRIX)?;
            let object = RoomObject {
                flags: r.bytes(o + 16, 8)?.try_into().unwrap(),
                bounds: std::array::from_fn(|k| r.i16(o + 24 + 2 * k).unwrap_or(0)),
                transform,
                meshes,
            };
            if object.vertex_count() != total_verts {
                return Err(FormatError::invalid(
                    "room",
                    format!(
                        "object {oi}: meshes hold {} vertices, record says {total_verts}",
                        object.vertex_count()
                    ),
                ));
            }
            objects.push(object);
        }
        Ok(RoomGeometry { tex_count, objects })
    }
}

fn matrix(r: &Reader, at: usize) -> Result<[[f32; 4]; 4]> {
    r.bytes(at, MATRIX)?;
    Ok(std::array::from_fn(|row| {
        std::array::from_fn(|col| r.f32(at + 16 * row + 4 * col).unwrap_or(0.0))
    }))
}

/// One mesh of a chain; returns it and the offset of the next descriptor.
fn parse_mesh(r: &Reader, d: usize) -> Result<(RoomMesh, usize)> {
    let field = |k: usize| r.u16(d + 2 * k).map(|v| v as usize);
    let n = field(0)?;
    let tex_index = field(1)? as u16;
    let (pos, nrm, uv, col, next) = (field(2)?, field(3)?, field(4)?, field(5)?, field(7)?);
    let header_units = MESH_HEADER / UNIT;
    if n == 0 || pos < header_units || next <= col || [nrm, uv, col].iter().any(|&f| f < pos) {
        return Err(FormatError::invalid(
            "room mesh",
            format!("descriptor at 0x{d:x}: {n} vertices, fields {pos} {nrm} {uv} {col} {next}"),
        ));
    }
    let (pos, nrm, uv, col, next) = (
        d + pos * UNIT,
        d + nrm * UNIT,
        d + uv * UNIT,
        d + col * UNIT,
        d + next * UNIT,
    );
    // Validate every array fits before decoding any of them.
    r.bytes(pos, n * 12)?;
    r.bytes(nrm, n * 12)?;
    r.bytes(uv, n * 4)?;
    let colour_bytes = r.bytes(col, n * 4)?;

    let mut m = RoomMesh {
        tex_index,
        positions: Vec::with_capacity(n),
        normals: Vec::with_capacity(n),
        uvs: Vec::with_capacity(n),
        flags: Vec::with_capacity(n),
        colours: Vec::with_capacity(n),
    };
    for k in 0..n {
        m.positions.push(r.vec3(pos + 12 * k)?);
        m.normals.push(r.vec3(nrm + 12 * k)?);
        let u = r.i16(uv + 4 * k)? as f32 / 4096.0;
        let v = r.i16(uv + 4 * k + 2)? as f32 / 4096.0;
        m.uvs.push([u, 1.0 - v]);
        let c = &colour_bytes[4 * k..4 * k + 4];
        m.flags.push(c[0]);
        m.colours.push([c[3], c[2], c[1]]);
    }
    Ok((m, next))
}

/// A parsed room file.
#[derive(Debug, Clone, Serialize)]
pub struct Room {
    pub file: ModelFile,
    pub geometry: RoomGeometry,
}

impl Room {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = ModelFile::parse_with(data, Endian::Little, Layout::Counted64)?;
        if file.sections.len() != SECTION_COUNT {
            return Err(FormatError::invalid(
                "room",
                format!(
                    "{} sections, rooms have {SECTION_COUNT}",
                    file.sections.len()
                ),
            ));
        }
        let section = file
            .section(data, GEOMETRY_SECTION)
            .ok_or_else(|| FormatError::invalid("room", "geometry section is empty"))?;
        let geometry = RoomGeometry::parse(section)?;
        Ok(Room { file, geometry })
    }

    pub fn section<'a>(&self, data: &'a [u8], index: usize) -> Option<&'a [u8]> {
        self.file.section(data, index)
    }

    /// The cameras of section 2 (version 2 records only).
    pub fn cameras(&self, data: &[u8]) -> Result<Cameras> {
        let section = self
            .section(data, camera::SECTION)
            .ok_or_else(|| FormatError::invalid("room", "camera section is empty"))?;
        Cameras::parse(section)
    }

    /// The triggers of section 3 (doors among them).
    pub fn triggers(&self, data: &[u8]) -> Result<Triggers> {
        let section = self
            .section(data, triggers::SECTION)
            .ok_or_else(|| FormatError::invalid("room", "trigger section is empty"))?;
        Triggers::parse(section)
    }

    /// The props of section 19.
    pub fn props(&self, data: &[u8]) -> Result<Props> {
        let section = self
            .section(data, props::SECTION)
            .ok_or_else(|| FormatError::invalid("room", "props section is empty"))?;
        Props::parse(section)
    }

    /// The light sets of lighting section 4 or 5 ([`crate::lights::SECTIONS`]).
    pub fn light_sets(&self, data: &[u8], section: usize) -> Result<Vec<LightSet>> {
        let bytes = self
            .section(data, section)
            .ok_or_else(|| FormatError::invalid("room", "light section is empty"))?;
        LightSet::parse_all(bytes)
    }

    /// The collision tree and polygons of section 9.
    pub fn collision(&self, data: &[u8]) -> Result<Collision> {
        let section = self
            .section(data, collision::SECTION)
            .ok_or_else(|| FormatError::invalid("room", "collision section is empty"))?;
        Collision::parse(section)
    }

    /// The texture container the geometry's `tex_index` values point into.
    /// Returns the container's bytes along with it.
    pub fn textures<'a>(&self, data: &'a [u8]) -> Option<(&'a [u8], TextureSet)> {
        let bytes = self.section(data, TEXTURE_SECTION)?;
        TextureSet::parse(bytes).ok().map(|set| (bytes, set))
    }
}

// ---------------------------------------------------------------- writer

/// A mesh for [`build`]; `flags` as stored, `colours` as RGB.
pub struct NewRoomMesh {
    pub tex_index: u16,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub flags: Vec<u8>,
    pub colours: Vec<[u8; 3]>,
}

pub struct NewRoomObject {
    pub transform: [[f32; 4]; 4],
    pub bounds: [i16; 6],
    pub meshes: Vec<NewRoomMesh>,
}

/// Build a room geometry section in the documented layout (fixtures).
pub fn build_geometry(tex_count: u8, objects: &[NewRoomObject]) -> Vec<u8> {
    let mut w = Writer::new(Endian::Little);
    w.u16(objects.len() as u16).u8(tex_count).u8(0);
    w.bytes(&[PC_PAD; 4]).u64(0);
    let table = w.pos();
    w.zeros(objects.len() * OBJECT_STRIDE);
    for (oi, obj) in objects.iter().enumerate() {
        w.pad_to(UNIT, 0);
        let first = w.pos();
        for m in &obj.meshes {
            let d = w.pos();
            let n = m.positions.len();
            // Array starts in 16-byte units: each array is padded to 16.
            let units = |bytes: usize| bytes.div_ceil(UNIT);
            let pos = MESH_HEADER / UNIT;
            let nrm = pos + units(n * 12);
            let uv = nrm + units(n * 12);
            let col = uv + units(n * 4);
            let next = col + units(n * 4);
            for f in [n, m.tex_index as usize, pos, nrm, uv, col, 0, next] {
                w.u16(f as u16);
            }
            w.zeros(8).bytes(&[FILL; 8]);
            for p in &m.positions {
                w.vec3(*p);
            }
            w.pad_to(UNIT, FILL);
            for p in &m.normals {
                w.vec3(*p);
            }
            w.pad_to(UNIT, FILL);
            for t in &m.uvs {
                w.i16((t[0] * 4096.0) as i16)
                    .i16(((1.0 - t[1]) * 4096.0) as i16);
            }
            w.pad_to(UNIT, FILL);
            for (f, c) in m.flags.iter().zip(&m.colours) {
                w.u8(*f).bytes(&[c[2], c[1], c[0]]);
            }
            w.pad_to(UNIT, FILL);
            debug_assert_eq!(w.pos(), d + next * UNIT);
        }
        let total: usize = obj.meshes.iter().map(|m| m.positions.len()).sum();
        let mut rec = Writer::new(Endian::Little);
        rec.u8(obj.meshes.len() as u8).u8(0).u16(total as u16);
        rec.bytes(&[PC_PAD; 4]).u64(first as u64).zeros(8);
        for b in obj.bounds {
            rec.i16(b);
        }
        rec.bytes(&[FILL; 4]);
        let at = table + oi * OBJECT_STRIDE;
        w.buf[at..at + OBJECT_STRIDE].copy_from_slice(&rec.buf);
    }
    w.pad_to(UNIT, 0);
    w.set_u64(8, w.pos() as u64);
    for obj in objects {
        for row in obj.transform {
            for v in row {
                w.f32(v);
            }
        }
    }
    w.finish()
}

/// Build a room file: 35 section slots, `geometry` in section 14 and
/// `textures` (a texture container) in section 34 when given.
pub fn build(geometry: &[u8], textures: Option<&[u8]>) -> Vec<u8> {
    build_with(geometry, textures, None)
}

/// [`build`], plus a collision section (section 9) when given.
pub fn build_with(geometry: &[u8], textures: Option<&[u8]>, collision: Option<&[u8]>) -> Vec<u8> {
    build_sections(&[
        (collision::SECTION, collision),
        (GEOMETRY_SECTION, Some(geometry)),
        (TEXTURE_SECTION, textures),
    ])
}

/// A room file with the given `(section, bytes)` payloads (fixtures).
pub fn build_sections(sections: &[(usize, Option<&[u8]>)]) -> Vec<u8> {
    let mut w = Writer::new(Endian::Little);
    w.u32(SECTION_COUNT as u32).bytes(&[PC_PAD; 4]);
    let table = w.pos();
    w.zeros(SECTION_COUNT * 8);
    w.pad_to(UNIT, 0);
    for &(index, payload) in sections {
        let Some(payload) = payload else { continue };
        let at = w.pos();
        w.set_u64(table + 8 * index, at as u64);
        w.bytes(payload).pad_to(UNIT, 0);
    }
    w.finish()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{dds, texture};

    pub const IDENTITY: [[f32; 4]; 4] = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ];

    /// A quad strip facing +Z, restarted into a triangle facing -Z whose
    /// winding bit says "reversed".
    pub fn quad_then_tri() -> NewRoomMesh {
        let f = flag::ALWAYS;
        NewRoomMesh {
            tex_index: 1,
            positions: vec![
                [0., 0., 0.],
                [1., 0., 0.],
                [0., 1., 0.],
                [1., 1., 0.],
                [0., 0., 1.],
                [1., 0., 1.],
                [0., 1., 1.],
            ],
            normals: [[0., 0., 1.]; 4]
                .into_iter()
                .chain([[0., 0., -1.]; 3])
                .collect(),
            uvs: vec![[0.25, 0.5]; 7],
            // Front, reversed, then a restart and a reversed triangle.
            flags: vec![
                f | flag::RESTART,
                f | flag::RESTART,
                f | flag::FRONT,
                f,
                f | flag::RESTART,
                f | flag::RESTART,
                f,
            ],
            colours: vec![[10, 20, 30]; 7],
        }
    }

    pub fn two_objects() -> Vec<NewRoomObject> {
        let mut moved = IDENTITY;
        moved[0][3] = 100.0;
        vec![
            NewRoomObject {
                transform: IDENTITY,
                bounds: [1, 0, 1, 0, 1, 0],
                meshes: vec![quad_then_tri()],
            },
            NewRoomObject {
                transform: moved,
                bounds: [101, 100, 1, 0, 1, 0],
                meshes: vec![quad_then_tri(), quad_then_tri()],
            },
        ]
    }

    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    #[test]
    fn round_trip_and_winding() {
        let geo = build_geometry(2, &two_objects());
        let g = RoomGeometry::parse(&geo).unwrap();
        assert_eq!((g.tex_count, g.objects.len(), g.mesh_count()), (2, 2, 3));
        assert_eq!(g.vertex_count(), 21);
        let m = &g.objects[0].meshes[0];
        assert_eq!(m.tex_index, 1);
        assert_eq!(m.colours[0], [10, 20, 30]);
        assert!((m.uvs[0][0] - 0.25).abs() < 1e-3 && (m.uvs[0][1] - 0.5).abs() < 1e-3);
        assert!((m.mean_normal_length() - 1.0).abs() < 1e-6);

        let tris = m.triangles();
        assert_eq!(tris.len(), 3, "two from the quad, one after the restart");
        for (t, facing) in tris.iter().zip([1.0, 1.0, -1.0]) {
            let [a, b, c] = t.map(|i| m.positions[i as usize]);
            let n = cross(
                [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
            );
            assert!(n[2] * facing > 0.0, "{t:?} faces {facing}");
        }

        let moved = &g.objects[1];
        assert_eq!(moved.meshes.len(), 2);
        assert_eq!(moved.to_room([1., 2., 3.]), [101., 2., 3.]);
        assert_eq!(moved.bounds, [101, 100, 1, 0, 1, 0]);
    }

    #[test]
    fn whole_file_with_textures() {
        let geo = build_geometry(1, &two_objects());
        let red = [0x00, 0xF8, 0, 0, 0, 0, 0, 0];
        let tex = texture::build_pc(
            texture::Kind::T32,
            &[dds::build(4, 4, dds::Format::Dxt1, &[&red])],
        );
        let data = build(&geo, Some(&tex));
        let room = Room::parse(&data).unwrap();
        assert_eq!(room.file.sections.len(), SECTION_COUNT);
        assert_eq!(room.geometry.objects.len(), 2);
        let (bytes, set) = room.textures(&data).unwrap();
        assert_eq!(set.images.len(), 1);
        let rgba = set.decode_rgba(bytes, &set.images[0]).unwrap().unwrap();
        assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
        // Rooms are not character models.
        assert!(ModelFile::detect(&data).is_err());
    }

    #[test]
    fn rejects_bad_input_without_panicking() {
        let geo = build_geometry(1, &two_objects());
        let data = build(&geo, None);
        for cut in (0..data.len()).step_by(5) {
            let _ = Room::parse(&data[..cut]);
        }
        let mut broken = geo.clone();
        broken[HEADER + 2] ^= 1; // total vertex count
        assert!(RoomGeometry::parse(&broken).is_err());
        let mut broken = geo;
        broken[4] = 0;
        assert!(RoomGeometry::parse(&broken).is_err());
    }
}
