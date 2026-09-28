//! Room collision (`.fsd` section 9, PC build): a box tree whose leaves hold
//! quads and triangles with 16-bit vertices. The rest of the section (an
//! object culling tree and a trailing table) is not parsed yet.
//! Layout: `docs/formats/README.md` §4d.

use crate::bytes::{Endian, Reader, Writer};
use crate::error::{FormatError, Result};
use serde::Serialize;

pub const SECTION: usize = 9;

const HEADER: usize = 16;
const NODE: usize = 32;
const POLY: usize = 48;
const POLY_MARK: u32 = 0xFFFF_FFFF;
/// Deep enough for every room (the deepest is far shallower); guards
/// against cycles in corrupt data.
const MAX_DEPTH: usize = 64;

/// Surface bits of [`Poly::flags`] (the rest are not understood yet).
pub mod surface {
    /// Floors only.
    pub const GROUND_A: u32 = 0x1;
    /// Floors only.
    pub const GROUND_B: u32 = 0x2;
    /// Walls and slopes.
    pub const WALL: u32 = 0x10;
    /// Ceilings only.
    pub const CEILING: u32 = 0x4_0000;
    /// Set on most polygons of every kind.
    pub const COMMON: u32 = 0x1000;
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Poly {
    /// Room-space vertices: 4 for a quad, 3 for a triangle (stored as a
    /// quad whose last vertex repeats the third).
    pub vertices: Vec<[f32; 3]>,
    pub flags: u32,
    /// Unit normal; `(v0, v1, v2)` winds counter-clockwise around it.
    pub normal: [f32; 3],
    /// The fourth float after the normal (usually 0; meaning unknown).
    pub extra: f32,
}

impl Poly {
    /// Triangles over [`Poly::vertices`]: `(0, 1, 2)` and, for a quad,
    /// `(2, 1, 3)`.
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        if self.vertices.len() == 4 {
            vec![[0, 1, 2], [2, 1, 3]]
        } else {
            vec![[0, 1, 2]]
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Node {
    pub centre: [i32; 3],
    pub half_extents: [i16; 3],
    pub children: Vec<Node>,
    /// Indices into [`Collision::polys`] (leaves only).
    pub polys: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Collision {
    pub tree: Vec<Node>,
    /// In index order.
    pub polys: Vec<Poly>,
    /// Bytes of section 9 the tree occupies.
    pub size: usize,
}

impl Collision {
    pub fn parse(section: &[u8]) -> Result<Self> {
        let r = Reader::new(section, Endian::Little);
        let top = r.u32(0)? as usize;
        let poly_count = r.u32(4)? as usize;
        if top == 0 || poly_count == 0 || poly_count > section.len() / POLY {
            return Err(FormatError::invalid(
                "collision",
                format!("{top} top nodes, {poly_count} polygons"),
            ));
        }
        let mut polys: Vec<Option<Poly>> = vec![None; poly_count];
        let mut at = HEADER;
        let mut tree = Vec::with_capacity(top);
        for _ in 0..top {
            tree.push(parse_node(&r, &mut at, &mut polys, 0)?);
        }
        let polys = polys
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                p.ok_or_else(|| FormatError::invalid("collision", format!("polygon {i} missing")))
            })
            .collect::<Result<_>>()?;
        Ok(Collision {
            tree,
            polys,
            size: at,
        })
    }

    /// Every polygon as room-space triangles, each with its polygon's flags.
    pub fn triangles(&self) -> impl Iterator<Item = ([[f32; 3]; 3], u32)> + '_ {
        self.polys.iter().flat_map(|p| {
            p.triangles()
                .into_iter()
                .map(move |t| (t.map(|i| p.vertices[i as usize]), p.flags))
        })
    }
}

fn parse_node(
    r: &Reader,
    at: &mut usize,
    polys: &mut [Option<Poly>],
    depth: usize,
) -> Result<Node> {
    let o = *at;
    r.bytes(o, NODE)?;
    // No check against the polygon mark here: a node centred at x = -1 starts
    // with the same bytes (r200, r211 and others).
    if depth > MAX_DEPTH {
        return Err(FormatError::invalid(
            "collision",
            format!("tree deeper than {MAX_DEPTH} at 0x{o:x}"),
        ));
    }
    let centre = [r.u32(o)? as i32, r.u32(o + 4)? as i32, r.u32(o + 8)? as i32];
    let half_extents = [r.i16(o + 12)?, r.i16(o + 14)?, r.i16(o + 16)?];
    let child_count = r.u16(o + 18)? as usize;
    let count = r.u32(o + 20)? as usize;
    let skip = r.u32(o + 24)? as usize;
    let index = r.u32(o + 28)?;
    *at += NODE;

    let mut node = Node {
        centre,
        half_extents,
        children: Vec::new(),
        polys: Vec::new(),
    };
    if child_count == 0 {
        // A leaf: `count` polygons follow, numbered from `index`.
        for k in 0..count {
            let p = *at;
            if r.u32(p)? != POLY_MARK {
                return Err(FormatError::invalid(
                    "collision",
                    format!("no polygon at 0x{p:x}"),
                ));
            }
            let i = index as usize + k;
            let slot = polys
                .get_mut(i)
                .ok_or_else(|| FormatError::invalid("collision", format!("polygon index {i}")))?;
            if slot.is_some() {
                return Err(FormatError::invalid(
                    "collision",
                    format!("polygon {i} twice"),
                ));
            }
            let vertex = |v: usize| -> Result<[f32; 3]> {
                Ok(std::array::from_fn(|a| {
                    centre[a] as f32 + r.i16(p + 4 + 6 * v + 2 * a).unwrap_or(0) as f32
                }))
            };
            r.bytes(p, POLY)?;
            let mut vertices = vec![vertex(0)?, vertex(1)?, vertex(2)?, vertex(3)?];
            if vertices[3] == vertices[2] {
                vertices.pop();
            }
            *slot = Some(Poly {
                vertices,
                flags: r.u32(p + 28)?,
                normal: [r.f32(p + 32)?, r.f32(p + 36)?, r.f32(p + 40)?],
                extra: r.f32(p + 44)?,
            });
            node.polys.push(i as u32);
            *at += POLY;
        }
    } else {
        for _ in 0..child_count {
            node.children.push(parse_node(r, at, polys, depth + 1)?);
        }
    }
    // `skip` spans the node and everything under it (0 on a last sibling).
    if skip != 0 && o + skip != *at {
        return Err(FormatError::invalid(
            "collision",
            format!("node at 0x{o:x} spans 0x{skip:x}, contents 0x{:x}", *at - o),
        ));
    }
    Ok(node)
}

// ---------------------------------------------------------------- writer

/// A leaf for [`build`]: one polygon, vertices relative to `centre`.
pub struct NewLeaf {
    pub centre: [i32; 3],
    pub half_extents: [i16; 3],
    /// Three or four vertices, relative to `centre`.
    pub vertices: Vec<[i16; 3]>,
    pub flags: u32,
    pub normal: [f32; 3],
}

/// Build a collision section with one top node per leaf (fixtures).
pub fn build(leaves: &[NewLeaf]) -> Vec<u8> {
    let mut w = Writer::new(Endian::Little);
    w.u32(leaves.len() as u32)
        .u32(leaves.len() as u32)
        .u32(1)
        .u32(0);
    for (i, leaf) in leaves.iter().enumerate() {
        for c in leaf.centre {
            w.u32(c as u32);
        }
        for h in leaf.half_extents {
            w.i16(h);
        }
        let last = i + 1 == leaves.len();
        w.u16(0).u32(1);
        w.u32(if last { 0 } else { (NODE + POLY) as u32 });
        w.u32(i as u32);
        w.u32(POLY_MARK);
        for k in 0..4 {
            let v = leaf.vertices[k.min(leaf.vertices.len() - 1)];
            for c in v {
                w.i16(c);
            }
        }
        w.u32(leaf.flags);
        for n in leaf.normal {
            w.f32(n);
        }
        w.f32(0.0);
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor_and_wall() -> Vec<NewLeaf> {
        vec![
            NewLeaf {
                centre: [-3075, 0, 0],
                half_extents: [2325, 0, 1924],
                vertices: vec![
                    [-2325, 0, -1924],
                    [-2325, 0, 1923],
                    [2325, 0, -1924],
                    [2325, 0, 1923],
                ],
                flags: surface::COMMON | surface::GROUND_A,
                normal: [0.0, 1.0, 0.0],
            },
            NewLeaf {
                centre: [0, 850, 1000],
                half_extents: [100, 850, 0],
                vertices: vec![[100, 850, 0], [-100, 850, 0], [100, -850, 0]],
                flags: surface::COMMON | surface::WALL,
                normal: [0.0, 0.0, 1.0],
            },
        ]
    }

    #[test]
    fn round_trip() {
        let bytes = build(&floor_and_wall());
        let c = Collision::parse(&bytes).unwrap();
        assert_eq!(c.size, bytes.len());
        assert_eq!((c.tree.len(), c.polys.len()), (2, 2));
        let floor = &c.polys[0];
        assert_eq!(floor.vertices[0], [-5400.0, 0.0, -1924.0]);
        assert_eq!(floor.triangles().len(), 2);
        assert_eq!(floor.flags & surface::GROUND_A, surface::GROUND_A);
        let wall = &c.polys[1];
        assert_eq!(
            wall.vertices.len(),
            3,
            "a repeated last vertex is a triangle"
        );
        assert_eq!(wall.vertices[1], [-100.0, 1700.0, 1000.0]);
        assert_eq!(c.tree[1].polys, vec![1]);
        let tris: Vec<_> = c.triangles().collect();
        assert_eq!(tris.len(), 3);
        assert_eq!(tris[1].0[0], floor.vertices[2]);
        assert_eq!(tris[2].1, wall.flags);
    }

    #[test]
    fn rejects_inconsistent_data() {
        let good = build(&floor_and_wall());
        for cut in (0..good.len()).step_by(3) {
            assert!(Collision::parse(&good[..cut]).is_err());
        }
        let mut bad = good.clone();
        bad[HEADER + 24] = 0x40; // wrong span on the first node
        assert!(Collision::parse(&bad).is_err());
        let mut bad = good;
        bad[4] = 3; // three polygons announced, two stored
        assert!(Collision::parse(&bad).is_err());
    }
}
