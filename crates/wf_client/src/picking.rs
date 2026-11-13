//! Exact 3D cursor ray-picking against stepped hexagonal terrain and unit voxel AABBs.

use glam::{Mat4, Vec2, Vec3, Vec4};
use hexx::Hex;
use wf_core::BattleHexMap;
use crate::scene::{axial_to_world_xz, tile_surface_height, HEX_WORLD_RADIUS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexPickResult {
    pub hex: Hex,
    pub hit_unit_slot: Option<usize>,
}

/// Converts physical window cursor `(px, py)` into a world-space ray `(origin, dir)`
/// matching the exact NDC convention used in `raymarch_64tree.wgsl` (`cs_main`).
pub fn screen_cursor_to_world_ray(
    cursor_px: (f64, f64),
    viewport_wh: (u32, u32),
    eye_pos: Vec3,
    inv_view_proj: Mat4,
) -> (Vec3, Vec3) {
    let w = viewport_wh.0.max(1) as f32;
    let h = viewport_wh.1.max(1) as f32;
    let uv = Vec2::new(
        (cursor_px.0 as f32 / w) * 2.0 - 1.0,
        (cursor_px.1 as f32 / h) * 2.0 - 1.0,
    );
    let ndc = Vec4::new(uv.x, -uv.y, 1.0, 1.0);
    let world_far = inv_view_proj * ndc;
    let far_pt = world_far.truncate() / world_far.w;
    let rd = (far_pt - eye_pos).normalize_or_zero();
    (eye_pos, rd)
}

/// Converts world `(x, z)` coordinates into fractional-rounded axial `Hex(q, r)`.
pub fn world_xz_to_axial_hex(x: f32, z: f32, radius: f32) -> Hex {
    let qf = (1.732_050_8 / 3.0 * x - 1.0 / 3.0 * z) / radius;
    let rf = (2.0 / 3.0 * z) / radius;
    let sf = -qf - rf;

    let mut rq = qf.round();
    let mut rr = rf.round();
    let rs = sf.round();

    let dq = (rq - qf).abs();
    let dr = (rr - rf).abs();
    let ds = (rs - sf).abs();

    if dq > dr && dq > ds {
        rq = -rr - rs;
    } else if dr > ds {
        rr = -rq - rs;
    }
    Hex::new(rq as i32, rr as i32)
}

fn ray_aabb_hit(ro: Vec3, rd: Vec3, bmin: Vec3, bmax: Vec3) -> Option<f32> {
    let inv = Vec3::new(
        if rd.x.abs() < 1e-6 { 1e6 } else { 1.0 / rd.x },
        if rd.y.abs() < 1e-6 { 1e6 } else { 1.0 / rd.y },
        if rd.z.abs() < 1e-6 { 1e6 } else { 1.0 / rd.z },
    );
    let t0 = (bmin - ro) * inv;
    let t1 = (bmax - ro) * inv;
    let tmin_v = t0.min(t1);
    let tmax_v = t0.max(t1);
    let tmin = tmin_v.x.max(tmin_v.y).max(tmin_v.z).max(0.0);
    let tmax = tmax_v.x.min(tmax_v.y).min(tmax_v.z);
    if tmax >= tmin {
        Some(tmin)
    } else {
        None
    }
}

/// Picks either the closest intersected live unit AABB (mapping directly to its occupied hex)
/// or the closest stepped hex column surface on `map`.
pub fn pick_hex_from_ray(
    ro: Vec3,
    rd: Vec3,
    map: &BattleHexMap,
    unit_pick_boxes: &[(usize, Hex, Vec3, Vec3)],
) -> Option<HexPickResult> {
    if rd.length_squared() < 0.5 {
        return None;
    }

    // 1. Check unit oriented/world bounding boxes first so tall units & flying drones pick cleanly
    let mut best_unit: Option<(f32, usize, Hex)> = None;
    for &(slot_idx, u_hex, bmin, bmax) in unit_pick_boxes {
        if let Some(t_hit) = ray_aabb_hit(ro, rd, bmin, bmax) {
            if best_unit.map_or(true, |(bt, _, _)| t_hit < bt) {
                best_unit = Some((t_hit, slot_idx, u_hex));
            }
        }
    }

    // 2. Raymarch stepped hex terrain slab `y in [-0.75, 3.6]`
    let mut terrain_hit: Option<(f32, Hex)> = None;
    let inv_y = if rd.y.abs() < 1e-5 { -1e5 } else { 1.0 / rd.y };
    let t_a = (3.6 - ro.y) * inv_y;
    let t_b = (-0.75 - ro.y) * inv_y;
    let t_enter = t_a.min(t_b).max(0.0);
    let t_exit = t_a.max(t_b).min(240.0);

    if !(rd.y >= 0.0 && ro.y > 3.6) && t_exit > t_enter {
        let mut t = t_enter;
        while t <= t_exit {
            let p = ro + rd * t;
            let h = world_xz_to_axial_hex(p.x, p.z, HEX_WORLD_RADIUS);
            if let Some(tile) = map.tiles.get(&h) {
                let (_, surf_y) = tile_surface_height(tile.morphology, tile.elevation);
                if p.y <= surf_y {
                    terrain_hit = Some((t, h));
                    break;
                }
            }
            t += 0.04;
        }
    }

    match (best_unit, terrain_hit) {
        (Some((tu, slot, uhex)), Some((tt, thex))) => {
            if tu <= tt + 0.35 {
                Some(HexPickResult {
                    hex: uhex,
                    hit_unit_slot: Some(slot),
                })
            } else {
                Some(HexPickResult {
                    hex: thex,
                    hit_unit_slot: None,
                })
            }
        }
        (Some((_, slot, uhex)), None) => Some(HexPickResult {
            hex: uhex,
            hit_unit_slot: Some(slot),
        }),
        (None, Some((_, thex))) => Some(HexPickResult {
            hex: thex,
            hit_unit_slot: None,
        }),
        (None, None) => None,
    }
}

/// World-space muzzle/chest anchor above a given `hex` on `map`.
pub fn hex_world_anchor(map: &BattleHexMap, hex: Hex, height_offset: f32) -> Vec3 {
    let (wx, wz) = axial_to_world_xz(hex);
    let base_y = map
        .tiles
        .get(&hex)
        .map(|t| tile_surface_height(t.morphology, t.elevation).1)
        .unwrap_or(0.0);
    Vec3::new(wx, base_y + height_offset, wz)
}
