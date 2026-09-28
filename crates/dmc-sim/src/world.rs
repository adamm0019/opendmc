//! Static room collision for the sim: triangles in sim units, a uniform grid
//! over X/Z, and the queries the character controller needs (the ground
//! under a point, and pushing a body out of walls).
//!
//! The sim never reads files: callers build a [`World`] from plain
//! triangles (the viewer converts a room's collision polygons, dividing room
//! units by [`ROOM_UNITS_PER_SIM_UNIT`]). Every query visits triangles in a
//! fixed order and uses only `+ - * /` and `sqrt`, so results are
//! bit-identical across builds.

use crate::math::V3;

/// Rooms measure Dante at about 900 units; the sim at 2.
pub const ROOM_UNITS_PER_SIM_UNIT: f32 = 450.0;

/// Surfaces whose normal points at least this far up are ground (about 45°).
pub const GROUND_MIN_NORMAL_Y: f32 = 0.7;

/// A body's collision shape: a vertical capsule-like column. Placeholder
/// values in sim units, to be measured (ADR-004).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    pub radius: f32,
    pub height: f32,
    /// Ledges up to this high are stepped onto rather than blocking.
    pub step_up: f32,
    /// A grounded body follows ground this far below it (stairs down, slopes).
    pub snap_down: f32,
}

impl Body {
    pub const PLAYER: Body = Body {
        radius: 0.35,
        height: 2.0,
        step_up: 0.35,
        snap_down: 0.5,
    };
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tri {
    pub a: V3,
    pub b: V3,
    pub c: V3,
    /// Unit normal from the winding (`(b - a) × (c - a)`).
    pub normal: V3,
    /// The source polygon's flags (see `dmc_formats::collision::surface`).
    pub flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ground {
    pub height: f32,
    pub normal: V3,
    pub flags: u32,
}

#[derive(Debug, Clone)]
pub struct World {
    tris: Vec<Tri>,
    cell: f32,
    min_x: f32,
    min_z: f32,
    nx: usize,
    nz: usize,
    /// Triangle indices per cell, row-major over (z, x), ascending.
    cells: Vec<Vec<u32>>,
}

fn cross(a: V3, b: V3) -> V3 {
    V3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

impl World {
    /// Build from triangles and their flags. Degenerate triangles are
    /// dropped. `cell` is the grid spacing in sim units.
    pub fn new(triangles: impl IntoIterator<Item = ([V3; 3], u32)>, cell: f32) -> Self {
        let tris: Vec<Tri> = triangles
            .into_iter()
            .filter_map(|([a, b, c], flags)| {
                let n = cross(b - a, c - a);
                let len = n.length();
                (len > 1e-9).then(|| Tri {
                    a,
                    b,
                    c,
                    normal: n * (1.0 / len),
                    flags,
                })
            })
            .collect();
        let cell = cell.max(1e-3);
        let (mut min_x, mut min_z) = (f32::INFINITY, f32::INFINITY);
        let (mut max_x, mut max_z) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for t in &tris {
            for p in [t.a, t.b, t.c] {
                min_x = min_x.min(p.x);
                min_z = min_z.min(p.z);
                max_x = max_x.max(p.x);
                max_z = max_z.max(p.z);
            }
        }
        if tris.is_empty() {
            (min_x, min_z, max_x, max_z) = (0.0, 0.0, 0.0, 0.0);
        }
        let nx = (((max_x - min_x) / cell) as usize + 1).min(4096);
        let nz = (((max_z - min_z) / cell) as usize + 1).min(4096);
        let mut w = World {
            tris: Vec::new(),
            cell,
            min_x,
            min_z,
            nx,
            nz,
            cells: vec![Vec::new(); nx * nz],
        };
        for (i, t) in tris.iter().enumerate() {
            let lo_x = t.a.x.min(t.b.x).min(t.c.x);
            let hi_x = t.a.x.max(t.b.x).max(t.c.x);
            let lo_z = t.a.z.min(t.b.z).min(t.c.z);
            let hi_z = t.a.z.max(t.b.z).max(t.c.z);
            let (x0, z0) = w.cell_of(lo_x, lo_z);
            let (x1, z1) = w.cell_of(hi_x, hi_z);
            for z in z0..=z1 {
                for x in x0..=x1 {
                    w.cells[z * nx + x].push(i as u32);
                }
            }
        }
        w.tris = tris;
        w
    }

    pub fn triangles(&self) -> &[Tri] {
        &self.tris
    }

    fn cell_of(&self, x: f32, z: f32) -> (usize, usize) {
        let cx = ((x - self.min_x) / self.cell).max(0.0) as usize;
        let cz = ((z - self.min_z) / self.cell).max(0.0) as usize;
        (cx.min(self.nx - 1), cz.min(self.nz - 1))
    }

    /// Triangles whose grid cells overlap the square `centre ± r`, each once,
    /// in ascending index order.
    fn near(&self, centre: V3, r: f32) -> Vec<u32> {
        let (x0, z0) = self.cell_of(centre.x - r, centre.z - r);
        let (x1, z1) = self.cell_of(centre.x + r, centre.z + r);
        let mut out = Vec::new();
        for z in z0..=z1 {
            for x in x0..=x1 {
                out.extend_from_slice(&self.cells[z * self.nx + x]);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The highest ground surface under `p` whose height lies in
    /// `[p.y - below, p.y + above]`.
    pub fn ground(&self, p: V3, above: f32, below: f32) -> Option<Ground> {
        let mut best: Option<Ground> = None;
        for i in self.near(p, 0.0) {
            let t = &self.tris[i as usize];
            if t.normal.y < GROUND_MIN_NORMAL_Y {
                continue;
            }
            let Some(h) = height_at(t, p.x, p.z) else {
                continue;
            };
            if h > p.y + above || h < p.y - below {
                continue;
            }
            if best.is_none_or(|b| h > b.height) {
                best = Some(Ground {
                    height: h,
                    normal: t.normal,
                    flags: t.flags,
                });
            }
        }
        best
    }

    /// Move a body standing at `feet` horizontally out of any wall it
    /// overlaps. Only the part of the body above `step_up` collides, so low
    /// ledges and stair risers are left to [`World::ground`].
    pub fn push_out(&self, feet: V3, body: Body) -> V3 {
        let mut p = feet;
        let samples = 3;
        for _ in 0..4 {
            let mut moved = false;
            for i in self.near(p, body.radius) {
                let t = &self.tris[i as usize];
                // Walls and steep slopes only: ground is handled by `ground`,
                // ceilings don't push sideways.
                if t.normal.y.abs() >= GROUND_MIN_NORMAL_Y {
                    continue;
                }
                for s in 0..samples {
                    let lo = body.step_up + body.radius;
                    let hi = (body.height - body.radius).max(lo);
                    let y = lo + (hi - lo) * s as f32 / (samples - 1) as f32;
                    let centre = V3::new(p.x, p.y + y, p.z);
                    let q = closest_point(centre, t);
                    let d = (centre - q).flat();
                    let dist = d.length();
                    // Only vertical-ish overlap counts: the closest point must
                    // be within the body's radius in height too.
                    if (centre.y - q.y).abs() > body.radius || dist >= body.radius {
                        continue;
                    }
                    let dir = if dist > 1e-6 {
                        d * (1.0 / dist)
                    } else {
                        t.normal.flat().normalize_or(V3::FORWARD)
                    };
                    p += dir * (body.radius - dist);
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        p
    }
}

/// Height of the triangle's plane at (x, z) if (x, z) lies inside the
/// triangle's X/Z projection.
fn height_at(t: &Tri, x: f32, z: f32) -> Option<f32> {
    let (ax, az, bx, bz, cx, cz) = (t.a.x, t.a.z, t.b.x, t.b.z, t.c.x, t.c.z);
    let det = (bz - cz) * (ax - cx) + (cx - bx) * (az - cz);
    if det.abs() < 1e-12 {
        return None;
    }
    let l1 = ((bz - cz) * (x - cx) + (cx - bx) * (z - cz)) / det;
    let l2 = ((cz - az) * (x - cx) + (ax - cx) * (z - cz)) / det;
    let l3 = 1.0 - l1 - l2;
    let eps = -1e-5;
    (l1 >= eps && l2 >= eps && l3 >= eps).then_some(l1 * t.a.y + l2 * t.b.y + l3 * t.c.y)
}

/// Closest point on a triangle to `p` (Ericson, *Real-Time Collision
/// Detection*, §5.1.5).
fn closest_point(p: V3, t: &Tri) -> V3 {
    let (a, b, c) = (t.a, t.b, t.c);
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Two triangles of a quad, wound so the normal is `up` or towards the
    /// inside of a room.
    fn quad(a: V3, b: V3, c: V3, d: V3, flags: u32) -> [([V3; 3], u32); 2] {
        [([a, b, c], flags), ([c, b, d], flags)]
    }

    /// A 20×20 room with a floor at 0, walls 4 high, and a 0.25-high step
    /// covering x in [2, 4].
    pub fn box_room() -> World {
        let v = V3::new;
        let mut t = Vec::new();
        t.extend(quad(
            v(-10., 0., -10.),
            v(-10., 0., 10.),
            v(10., 0., -10.),
            v(10., 0., 10.),
            1,
        ));
        // Step top at y = 0.25 for x in [2, 4].
        t.extend(quad(
            v(2., 0.25, -10.),
            v(2., 0.25, 10.),
            v(4., 0.25, -10.),
            v(4., 0.25, 10.),
            1,
        ));
        // Walls facing inward: the wall at x = 10 faces -X, and so on.
        t.extend(quad(
            v(10., 0., -10.),
            v(10., 0., 10.),
            v(10., 4., -10.),
            v(10., 4., 10.),
            0x10,
        ));
        t.extend(quad(
            v(-10., 0., 10.),
            v(-10., 0., -10.),
            v(-10., 4., 10.),
            v(-10., 4., -10.),
            0x10,
        ));
        t.extend(quad(
            v(10., 0., 10.),
            v(-10., 0., 10.),
            v(10., 4., 10.),
            v(-10., 4., 10.),
            0x10,
        ));
        t.extend(quad(
            v(-10., 0., -10.),
            v(10., 0., -10.),
            v(-10., 4., -10.),
            v(10., 4., -10.),
            0x10,
        ));
        // A ceiling at 4, facing down.
        t.extend(quad(
            v(-10., 4., -10.),
            v(10., 4., -10.),
            v(-10., 4., 10.),
            v(10., 4., 10.),
            0x40000,
        ));
        World::new(t, 2.0)
    }

    #[test]
    fn walls_face_inward() {
        let w = box_room();
        let t = w.triangles();
        assert!(t[4].normal.x < -0.99, "{:?}", t[4].normal);
        assert!(t[6].normal.x > 0.99 && t[8].normal.z < -0.99 && t[10].normal.z > 0.99);
        assert!(t[0].normal.y > 0.99 && t[12].normal.y < -0.99);
    }

    #[test]
    fn ground_finds_floor_and_step() {
        let w = box_room();
        let g = w.ground(V3::new(0.0, 0.1, 0.0), 0.35, 0.5).unwrap();
        assert_eq!((g.height, g.flags), (0.0, 1));
        let step = w.ground(V3::new(3.0, 0.1, 0.0), 0.35, 0.5).unwrap();
        assert_eq!(step.height, 0.25);
        // Out of reach below: nothing.
        assert!(w.ground(V3::new(0.0, 3.0, 0.0), 0.35, 0.5).is_none());
        // Outside the room: nothing.
        assert!(w.ground(V3::new(30.0, 0.0, 0.0), 0.35, 0.5).is_none());
    }

    #[test]
    fn walls_push_bodies_back() {
        let w = box_room();
        let p = w.push_out(V3::new(9.9, 0.0, 0.0), Body::PLAYER);
        assert!((p.x - (10.0 - Body::PLAYER.radius)).abs() < 1e-4, "{p:?}");
        assert_eq!((p.y, p.z), (0.0, 0.0));
        // Into a corner: pushed out of both walls.
        let c = w.push_out(V3::new(9.9, 0.0, 9.9), Body::PLAYER);
        assert!(
            c.x <= 10.0 - Body::PLAYER.radius + 1e-4 && c.z <= 10.0 - Body::PLAYER.radius + 1e-4,
            "{c:?}"
        );
        // Free space is untouched (the ceiling doesn't shove a jumping body
        // sideways), and the low step does not block.
        assert_eq!(
            w.push_out(V3::new(0.0, 2.5, 0.0), Body::PLAYER),
            V3::new(0.0, 2.5, 0.0)
        );
        assert_eq!(w.push_out(V3::new(0.0, 0.0, 0.0), Body::PLAYER), V3::ZERO);
        assert_eq!(
            w.push_out(V3::new(2.1, 0.0, 0.0), Body::PLAYER),
            V3::new(2.1, 0.0, 0.0)
        );
    }

    #[test]
    fn closest_point_regions() {
        let t = Tri {
            a: V3::ZERO,
            b: V3::new(1.0, 0.0, 0.0),
            c: V3::new(0.0, 0.0, 1.0),
            normal: V3::UP,
            flags: 0,
        };
        assert_eq!(closest_point(V3::new(-1.0, 0.0, -1.0), &t), t.a);
        assert_eq!(
            closest_point(V3::new(0.25, 5.0, 0.25), &t),
            V3::new(0.25, 0.0, 0.25)
        );
        assert_eq!(
            closest_point(V3::new(1.0, 0.0, 1.0), &t),
            V3::new(0.5, 0.0, 0.5)
        );
    }
}
