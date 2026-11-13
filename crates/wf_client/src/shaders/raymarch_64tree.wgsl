// Wildfront Battlegrounds — GPU Sparse 64-Tree Brickmap & Procedural Hex Terrain Raymarcher
// Traverses:
//   1. Multi-instance 4-level Sparse 64-Tree unit rigs (`256^3 -> 64^3 -> 16^3 -> 4^3 -> 1^3`)
//      with zoom-adaptive cone LOD termination, face normals, sun shadows & emissive optics.
//   2. Procedural post-nuclear 2072 hexagonal battlefields (up to Radius 9 / 271 hexes across
//      6 biomes, stepped plateaus, cliffs, rivers, fords, radiation fallout & objective rings).

struct CameraUniforms {
    inv_view_proj: mat4x4<f32>,
    camera_pos_spread: vec4<f32>, // xyz = eye world pos, w = pixel ray spread (tan(fov/2)*2/H)
    sun_dir_flags: vec4<f32>,     // xyz = normalized sun dir, w = show_overlays (1.0 or 0.0)
    resolution: vec2<u32>,
    instance_count: u32,
    hex_radius: f32,
    vfx_beam_count: u32,
    vfx_debris_count: u32,
    time_seconds: f32,
    atmosphere_mode: u32,
};

struct GpuVfxBeam {
    start_and_radius: vec4<f32>, // xyz = start/center, w = radius
    end_and_progress: vec4<f32>, // xyz = end, w = progress (0..1)
    color_and_kind: vec4<f32>,   // xyz = linear RGB, w = kind (1..5)
};

struct GpuVfxDebris {
    pos_and_scale: vec4<f32>,    // xyz = center, w = half_extent
    color_and_emit: vec4<f32>,   // xyz = linear RGB, w = emissive
};

struct GpuNode64 {
    mask_lo: u32,
    mask_hi: u32,
    child_base_ptr: u32,
    flags_and_lod_rgba: u32, // bit 0 = is_twig (children in leaf_bricks), bits 8..31 = RGB avg
};

struct GpuLeafBrick {
    mask_lo: u32,
    mask_hi: u32,
    lod_rgba: u32,
    _pad: u32,
    packed_mats: array<u32, 16>,
};

struct GpuUnitInstance {
    world_to_voxel: mat4x4<f32>, // Maps world pos -> local [0..size_voxels] voxel coordinates
    voxel_to_world_scale: f32,   // World meters per 1.0 voxel
    root_node_index: u32,
    team_id: u32,
    _pad0: u32,
    world_aabb_min: vec4<f32>,
    world_aabb_max: vec4<f32>,
    size_voxels: vec4<f32>,
};

struct GpuHexOverlay {
    q: i32,
    r: i32,
    elevation_tier: i32, // -1 = Navigable River, 0..2 = Stepped Land, 3 = Mountain Peak
    packed_flags: u32,   // bits 0..3 = auras/hazards/objectives, bits 8..15 = biome, bits 16..23 = morphology
};

@group(0) @binding(0) var<uniform> cam: CameraUniforms;
@group(0) @binding(1) var<storage, read> tree_nodes: array<GpuNode64>;
@group(0) @binding(2) var<storage, read> leaf_bricks: array<GpuLeafBrick>;
@group(0) @binding(3) var<storage, read> palette_lin: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read> instances: array<GpuUnitInstance>;
@group(0) @binding(5) var<storage, read> hex_tiles: array<GpuHexOverlay>;
@group(0) @binding(6) var out_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(7) var<storage, read> vfx_beams: array<GpuVfxBeam>;
@group(0) @binding(8) var<storage, read> vfx_debris: array<GpuVfxDebris>;

fn has_bit64(lo: u32, hi: u32, bit: u32) -> bool {
    if (bit < 32u) {
        return ((lo >> bit) & 1u) != 0u;
    }
    return ((hi >> (bit - 32u)) & 1u) != 0u;
}

fn popcnt_below64(lo: u32, hi: u32, bit: u32) -> u32 {
    if (bit == 0u) {
        return 0u;
    }
    if (bit < 32u) {
        let mask = (1u << bit) - 1u;
        return countOneBits(lo & mask);
    }
    if (bit == 32u) {
        return countOneBits(lo);
    }
    let hi_mask = (1u << (bit - 32u)) - 1u;
    return countOneBits(lo) + countOneBits(hi & hi_mask);
}

fn unpack_lod_linear(packed: u32) -> vec3<f32> {
    let r = f32((packed >> 8u) & 255u) / 255.0;
    let g = f32((packed >> 16u) & 255u) / 255.0;
    let b = f32((packed >> 24u) & 255u) / 255.0;
    return pow(vec3<f32>(r, g, b), vec3<f32>(2.2));
}

fn intersect_aabb(ro: vec3<f32>, inv_rd: vec3<f32>, bmin: vec3<f32>, bmax: vec3<f32>) -> vec2<f32> {
    let t0 = (bmin - ro) * inv_rd;
    let t1 = (bmax - ro) * inv_rd;
    let tsmaller = min(t0, t1);
    let tbigger = max(t0, t1);
    let tmin = max(max(tsmaller.x, tsmaller.y), max(tsmaller.z, 0.0));
    let tmax = min(min(tbigger.x, tbigger.y), tbigger.z);
    return vec2<f32>(tmin, tmax);
}

struct VoxelHit {
    hit: bool,
    t_world: f32,
    normal: vec3<f32>,
    albedo: vec3<f32>,
    emissive: f32,
};

// Point-query helper inside a unit's 4-level Sparse 64-Tree (`256^3` domain)
// Returns palette index `1..=255` (`0` = empty air), or `256 + packed_rgb` when LOD-terminated.
fn sample_64tree(root_idx: u32, vp: vec3<i32>, lod_level: u32) -> u32 {
    if (any(vp < vec3<i32>(0)) || any(vp >= vec3<i32>(256))) {
        return 0u;
    }
    let up = vec3<u32>(vp);
    let root = tree_nodes[root_idx];

    // Level 2: 64^3 quadrants
    let c2 = (up >> vec3<u32>(6u)) & vec3<u32>(3u);
    let b2 = c2.x + c2.y * 4u + c2.z * 16u;
    if (!has_bit64(root.mask_lo, root.mask_hi, b2)) {
        return 0u;
    }
    let l1_idx = root.child_base_ptr + popcnt_below64(root.mask_lo, root.mask_hi, b2);
    let l1 = tree_nodes[l1_idx];

    // Level 1: 16^3 sub-blocks
    let c1 = (up >> vec3<u32>(4u)) & vec3<u32>(3u);
    let b1 = c1.x + c1.y * 4u + c1.z * 16u;
    if (!has_bit64(l1.mask_lo, l1.mask_hi, b1)) {
        return 0u;
    }
    let twig_idx = l1.child_base_ptr + popcnt_below64(l1.mask_lo, l1.mask_hi, b1);
    let twig = tree_nodes[twig_idx];
    if (lod_level >= 2u) {
        return 0x80000000u | twig.flags_and_lod_rgba;
    }

    // Level 0 (Twig): 4^3 bricks
    let c0 = (up >> vec3<u32>(2u)) & vec3<u32>(3u);
    let b0 = c0.x + c0.y * 4u + c0.z * 16u;
    if (!has_bit64(twig.mask_lo, twig.mask_hi, b0)) {
        return 0u;
    }
    let brick_idx = twig.child_base_ptr + popcnt_below64(twig.mask_lo, twig.mask_hi, b0);
    let brick = leaf_bricks[brick_idx];
    if (lod_level == 1u) {
        return 0x80000000u | brick.lod_rgba;
    }

    // Leaf Voxel (1^3)
    let lv = up & vec3<u32>(3u);
    let bv = lv.x + lv.y * 4u + lv.z * 16u;
    if (!has_bit64(brick.mask_lo, brick.mask_hi, bv)) {
        return 0u;
    }
    let word = brick.packed_mats[bv >> 2u];
    let shift = (bv & 3u) * 8u;
    return (word >> shift) & 255u;
}

// Fast Empty-Space-Skipping 3D DDA through a unit's Sparse 64-Tree
fn trace_unit_instance(inst_idx: u32, ro_w: vec3<f32>, rd_w: vec3<f32>, max_t_w: f32) -> VoxelHit {
    var result: VoxelHit;
    result.hit = false;
    result.t_world = max_t_w;

    let inst = instances[inst_idx];
    let inv_rd_w = 1.0 / select(rd_w, vec3<f32>(1e-6), abs(rd_w) < vec3<f32>(1e-6));
    let hit_box = intersect_aabb(ro_w, inv_rd_w, inst.world_aabb_min.xyz, inst.world_aabb_max.xyz);
    if (hit_box.x > hit_box.y || hit_box.x >= max_t_w) {
        return result;
    }

    // Transform ray into local voxel grid [0 .. size_voxels]
    let ro_v = (inst.world_to_voxel * vec4<f32>(ro_w, 1.0)).xyz;
    let rd_v_raw = (inst.world_to_voxel * vec4<f32>(rd_w, 0.0)).xyz;
    let v_ray_len = length(rd_v_raw);
    let rd_v = rd_v_raw / max(v_ray_len, 1e-6);
    let inv_rd_v = 1.0 / select(rd_v, vec3<f32>(1e-6), abs(rd_v) < vec3<f32>(1e-6));

    let vbox = intersect_aabb(ro_v, inv_rd_v, vec3<f32>(0.0), inst.size_voxels.xyz);
    if (vbox.x > vbox.y) {
        return result;
    }

    // Determine adaptive LOD tier from projected voxel size on screen
    let approx_world_dist = max(hit_box.x, 1.0);
    let pixel_world_size = approx_world_dist * cam.camera_pos_spread.w;
    var lod_level = 0u;
    if (pixel_world_size > inst.voxel_to_world_scale * 3.4) {
        lod_level = 1u; // Brick-level cone LOD when zoomed far out
    }

    var t = max(vbox.x - 0.002, 0.0);
    let t_end = min(vbox.y, (max_t_w * v_ray_len));
    let step_dir = sign(rd_v);

    var p = clamp(ro_v + rd_v * (t + 0.005), vec3<f32>(0.001), inst.size_voxels.xyz - vec3<f32>(0.001));
    var map_pos = vec3<i32>(floor(p));
    let delta_dist = abs(inv_rd_v);
    var side_dist = (step_dir * (vec3<f32>(map_pos) - p) + (step_dir * 0.5 + 0.5)) * delta_dist;
    var last_mask = vec3<f32>(0.0, 1.0, 0.0);

    // Estimate entry face normal when hitting outer boundary directly
    if (vbox.x > 0.001) {
        let d_min = abs(p - vec3<f32>(0.0));
        let d_max = abs(p - inst.size_voxels.xyz);
        let d_edge = min(d_min, d_max);
        if (d_edge.x <= d_edge.y && d_edge.x <= d_edge.z) {
            last_mask = vec3<f32>(1.0, 0.0, 0.0);
        } else if (d_edge.y <= d_edge.z) {
            last_mask = vec3<f32>(0.0, 1.0, 0.0);
        } else {
            last_mask = vec3<f32>(0.0, 0.0, 1.0);
        }
    }

    for (var iter = 0u; iter < 680u; iter = iter + 1u) {
        if (any(map_pos < vec3<i32>(0)) || any(map_pos >= vec3<i32>(inst.size_voxels.xyz))) {
            break;
        }

        let sample = sample_64tree(inst.root_node_index, map_pos, lod_level);
        if (sample != 0u) {
            let hit_t_v = min(min(side_dist.x - delta_dist.x, side_dist.y - delta_dist.y), side_dist.z - delta_dist.z);
            let final_t_v = max(vbox.x, hit_t_v);
            let final_t_w = final_t_v / v_ray_len;
            if (final_t_w < max_t_w) {
                result.hit = true;
                result.t_world = final_t_w;
                let hit_n = normalize(-step_dir * last_mask);
                result.normal = hit_n;
                // Real-time 4-tap GPU Voxel Crevice Ambient Occlusion on hit tangent plane
                let n_step = vec3<i32>(round(hit_n));
                let front_v = map_pos + n_step;
                var occ_n = 0.0;
                if (abs(n_step.y) > 0) {
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(1, 0, 0), 0u) != 0u) { occ_n += 1.0; }
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(-1, 0, 0), 0u) != 0u) { occ_n += 1.0; }
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(0, 0, 1), 0u) != 0u) { occ_n += 1.0; }
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(0, 0, -1), 0u) != 0u) { occ_n += 1.0; }
                } else if (abs(n_step.x) > 0) {
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(0, 1, 0), 0u) != 0u) { occ_n += 1.0; }
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(0, -1, 0), 0u) != 0u) { occ_n += 1.0; }
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(0, 0, 1), 0u) != 0u) { occ_n += 1.0; }
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(0, 0, -1), 0u) != 0u) { occ_n += 1.0; }
                } else {
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(1, 0, 0), 0u) != 0u) { occ_n += 1.0; }
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(-1, 0, 0), 0u) != 0u) { occ_n += 1.0; }
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(0, 1, 0), 0u) != 0u) { occ_n += 1.0; }
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(0, -1, 0), 0u) != 0u) { occ_n += 1.0; }
                }
                if (abs(n_step.y) > 0) {
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(1, 0, 1), 0u) != 0u) { occ_n += 0.55; }
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(-1, 0, 1), 0u) != 0u) { occ_n += 0.55; }
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(1, 0, -1), 0u) != 0u) { occ_n += 0.55; }
                    if (sample_64tree(inst.root_node_index, front_v + vec3<i32>(-1, 0, -1), 0u) != 0u) { occ_n += 0.55; }
                }
                let vox_ao = clamp(1.0 - occ_n * 0.088, 0.46, 1.0);
                if ((sample & 0x80000000u) != 0u) {
                    result.albedo = unpack_lod_linear(sample) * vox_ao;
                    result.emissive = 0.0;
                } else {
                    let mat_idx = clamp(sample - 1u, 0u, 255u);
                    let raw_alb = palette_lin[mat_idx].xyz;
                    if (sample == 11u || sample == 14u || sample == 15u || sample == 19u || sample == 20u || sample >= 240u) {
                        result.albedo = raw_alb;
                        result.emissive = 1.35;
                    } else {
                        result.albedo = raw_alb * vox_ao;
                        result.emissive = 0.0;
                    }
                }
            }
            return result;
        }

        // Standard Amanatides & Woo 3D voxel DDA step
        if (side_dist.x < side_dist.y) {
            if (side_dist.x < side_dist.z) {
                if (side_dist.x > t_end) { break; }
                side_dist.x += delta_dist.x;
                map_pos.x += i32(step_dir.x);
                last_mask = vec3<f32>(1.0, 0.0, 0.0);
            } else {
                if (side_dist.z > t_end) { break; }
                side_dist.z += delta_dist.z;
                map_pos.z += i32(step_dir.z);
                last_mask = vec3<f32>(0.0, 0.0, 1.0);
            }
        } else {
            if (side_dist.y < side_dist.z) {
                if (side_dist.y > t_end) { break; }
                side_dist.y += delta_dist.y;
                map_pos.y += i32(step_dir.y);
                last_mask = vec3<f32>(0.0, 1.0, 0.0);
            } else {
                if (side_dist.z > t_end) { break; }
                side_dist.z += delta_dist.z;
                map_pos.z += i32(step_dir.z);
                last_mask = vec3<f32>(0.0, 0.0, 1.0);
            }
        }
    }

    return result;
}

// Converts world `(x, z)` to rounded pointy-top axial hex `(q, r)` and hex-edge distance `[0..1]`
fn world_to_axial_hex(xz: vec2<f32>, radius: f32) -> vec3<f32> {
    let qf = (1.7320508 / 3.0 * xz.x - 1.0 / 3.0 * xz.y) / radius;
    let rf = (2.0 / 3.0 * xz.y) / radius;
    let sf = -qf - rf;

    var rq = round(qf);
    var rr = round(rf);
    var rs = round(sf);

    let dq = abs(rq - qf);
    let dr = abs(rr - rf);
    let ds = abs(rs - sf);

    if (dq > dr && dq > ds) {
        rq = -rr - rs;
    } else if (dr > ds) {
        rr = -rq - rs;
    }

    // Hex center back to world XZ
    let cx = radius * 1.7320508 * (rq + rr * 0.5);
    let cz = radius * 1.5 * rr;
    let local = abs(xz - vec2<f32>(cx, cz)) / radius;
    // Pointy-top hex edge metric (1.0 on hex perimeter)
    let hex_d = max(local.x * 1.1547005, local.x * 0.5773503 + local.y);
    return vec3<f32>(rq, rr, hex_d);
}

fn lookup_hex_tile(q: i32, r: i32) -> GpuHexOverlay {
    // Supports procedural boards up to Radius 9 stored in 19x19 dense table `(q + 9) + (r + 9) * 19`
    if (abs(q) > 9 || abs(r) > 9 || abs(q + r) > 9) {
        return GpuHexOverlay(q, r, -2, 0u);
    }
    let idx = u32((q + 9) + (r + 9) * 19);
    return hex_tiles[idx];
}

fn biome_ground_palette(biome_id: u32, morph_id: u32, tier: i32) -> vec3<f32> {
    // Morphologies: 0=Flat, 1=RoughCover, 2=MountainPeak, 3=RavineChasm, 4=DeepRiver,
    //               5=OpenOcean, 6=RadiationFallout, 7=BunkerRuins, 8=ShallowFord, 9=ArmoredDam
    if (morph_id == 4u) {
        return vec3<f32>(0.03, 0.20, 0.28); // DeepRiver teal channel
    }
    if (morph_id == 5u) {
        return vec3<f32>(0.02, 0.11, 0.22); // OpenOcean pelagic trench
    }
    if (morph_id == 3u) {
        return vec3<f32>(0.05, 0.05, 0.06); // RavineChasm abyssal basalt
    }
    if (morph_id == 8u) {
        return vec3<f32>(0.16, 0.26, 0.27); // ShallowFord passable river stones
    }
    if (morph_id == 7u || morph_id == 9u) {
        return vec3<f32>(0.26, 0.27, 0.28); // Pre-War BunkerRuins / ArmoredDam ferrocrete
    }
    if (tier >= 2 || morph_id == 2u) {
        return vec3<f32>(0.21, 0.21, 0.23); // MountainPeak granite
    }

    // Biome ground colors (0=Badlands, 1=Savannah, 2=Ashwood, 3=BioJungle, 4=Atoll, 5=Caldera)
    var base = vec3<f32>(0.22, 0.20, 0.12);
    if (biome_id == 0u) {
        base = vec3<f32>(0.30, 0.16, 0.09); // Irradiated Badlands rust-ochre
    } else if (biome_id == 1u) {
        base = vec3<f32>(0.25, 0.22, 0.12); // Savannah Dustbowl dry gold-olive
    } else if (biome_id == 2u) {
        base = vec3<f32>(0.14, 0.21, 0.16); // Ashwood Basin pine-slate
    } else if (biome_id == 3u) {
        base = vec3<f32>(0.08, 0.24, 0.12); // Overgrown Bio-Jungle lush emerald
    } else if (biome_id == 4u) {
        base = vec3<f32>(0.31, 0.29, 0.21); // Sunken Atoll coral sandbar
    } else if (biome_id == 5u) {
        base = vec3<f32>(0.14, 0.13, 0.15); // Ruined Bunker Caldera volcanic ash
    }

    if (morph_id == 1u) {
        base *= 0.78; // RoughCover darkened brush/boulders
    }
    if (tier == 1) {
        base = mix(base, vec3<f32>(0.26, 0.23, 0.18), 0.35); // Highland plateau tint
    }
    return base;
}

// Raymarches procedural stepped hexagonal terrain (biomes, plateaus, cliffs, rivers & hazards)
fn trace_hex_terrain(ro: vec3<f32>, rd: vec3<f32>, max_t: f32) -> VoxelHit {
    var out: VoxelHit;
    out.hit = false;
    out.t_world = max_t;

    let inv_y = 1.0 / select(rd.y, -1e-5, abs(rd.y) < 1e-5);
    let t_top = (3.4 - ro.y) * inv_y;
    let t_bot = (-0.7 - ro.y) * inv_y;
    let t_enter = max(0.0, min(t_top, t_bot));
    let t_exit = min(max_t, max(t_top, t_bot));
    if (rd.y >= 0.0 && ro.y > 3.4) {
        return out;
    }

    var t = t_enter;
    let dt = 0.06;
    var prev_pos = ro + rd * t;
    for (var i = 0u; i < 420u; i = i + 1u) {
        if (t > t_exit) { break; }
        let p = ro + rd * t;
        let hinfo = world_to_axial_hex(p.xz, cam.hex_radius);
        let tile = lookup_hex_tile(i32(hinfo.x), i32(hinfo.y));

        if (tile.elevation_tier >= -1) {
            let biome_id = (tile.packed_flags >> 8u) & 255u;
            let morph_id = (tile.packed_flags >> 16u) & 255u;

            var surface_y = 0.0;
            if (tile.elevation_tier == -1) {
                surface_y = select(-0.24, -0.46, morph_id == 3u);
            } else if (morph_id == 8u) {
                surface_y = -0.05;
            } else {
                surface_y = f32(tile.elevation_tier) * 0.55;
            }

            if (p.y <= surface_y) {
                out.hit = true;
                out.t_world = t;
                let is_cliff = (prev_pos.y < surface_y - 0.03);
                var surf_n = vec3<f32>(0.0, 1.0, 0.0);
                if (!is_cliff && (morph_id == 4u || morph_id == 5u || morph_id == 8u)) {
                    let wx = cos(p.x * 3.4 + cam.time_seconds * 2.4) * 0.055 + sin(p.z * 2.3 - cam.time_seconds * 1.8) * 0.035;
                    let wz = sin(p.z * 3.1 + cam.time_seconds * 2.1) * 0.055 + cos(p.x * 2.6 + cam.time_seconds * 1.5) * 0.035;
                    surf_n = normalize(vec3<f32>(wx, 1.0, wz));
                }
                out.normal = select(surf_n, normalize(vec3<f32>(-rd.x, 0.25, -rd.z)), is_cliff);

                var base_col = biome_ground_palette(biome_id, morph_id, tile.elevation_tier);
                var emit = 0.0;

                if (is_cliff) {
                    base_col *= 0.55;
                }

                // Radiation Fallout glowing surface veins (bit 2)
                if (!is_cliff && (tile.packed_flags & 4u) != 0u) {
                    let rad_pulse = smoothstep(0.35, 0.78, hinfo.z);
                    base_col = mix(vec3<f32>(0.18, 0.58, 0.08), base_col, rad_pulse * 0.65);
                    emit = 0.38 * (1.0 - rad_pulse * 0.5);
                }

                // Crisp hex grid lines
                let border = smoothstep(0.93, 0.985, hinfo.z);
                base_col = mix(base_col, vec3<f32>(0.04, 0.07, 0.08), border * 0.58);

                // Toggleable tactical overlays: Cyan Node Range, Amber Supply Range, Gold Objective Beacon
                if (cam.sun_dir_flags.w > 0.5 && !is_cliff) {
                    let ring_band = smoothstep(0.76, 0.89, hinfo.z) * (1.0 - smoothstep(0.94, 0.99, hinfo.z));
                    if ((tile.packed_flags & 2u) != 0u) {
                        base_col = mix(base_col, vec3<f32>(0.04, 0.65, 0.82), ring_band * 0.72);
                    }
                    if ((tile.packed_flags & 1u) != 0u) {
                        base_col = mix(base_col, vec3<f32>(0.88, 0.45, 0.05), ring_band * 0.65);
                    }
                    if ((tile.packed_flags & 8u) != 0u) {
                        base_col = mix(base_col, vec3<f32>(0.96, 0.78, 0.14), ring_band * 0.88);
                        emit = max(emit, ring_band * 0.45);
                    }
                }

                // Interactive Hex Picking & Command Highlights (always visible on top face)
                if (!is_cliff) {
                    let inner_fill = 1.0 - smoothstep(0.72, 0.92, hinfo.z);
                    let crisp_ring = smoothstep(0.80, 0.90, hinfo.z) * (1.0 - smoothstep(0.95, 0.995, hinfo.z));
                    // Bit 6: Valid Move / Card Deploy Destination (emerald-green pad)
                    if ((tile.packed_flags & 64u) != 0u) {
                        base_col = mix(base_col, vec3<f32>(0.08, 0.85, 0.36), inner_fill * 0.32 + crisp_ring * 0.82);
                        emit = max(emit, crisp_ring * 0.45);
                    }
                    // Bit 7: Valid Attack / Ability Target (crimson targeting reticle)
                    if ((tile.packed_flags & 128u) != 0u) {
                        base_col = mix(base_col, vec3<f32>(0.96, 0.16, 0.18), inner_fill * 0.28 + crisp_ring * 0.88);
                        emit = max(emit, crisp_ring * 0.55);
                    }
                    // Bit 5: Selected Hex Bracket (bright emerald-cyan ring)
                    if ((tile.packed_flags & 32u) != 0u) {
                        base_col = mix(base_col, vec3<f32>(0.18, 0.98, 0.68), crisp_ring * 0.92);
                        emit = max(emit, crisp_ring * 0.65);
                    }
                    // Bit 4: Cursor Hovered Hex Reticle (pulsing crisp white-mint outline)
                    if ((tile.packed_flags & 16u) != 0u) {
                        let pulse = 0.75 + 0.25 * sin(cam.time_seconds * 7.0);
                        let hov_ring = smoothstep(0.84, 0.92, hinfo.z) * (1.0 - smoothstep(0.96, 0.998, hinfo.z));
                        base_col = mix(base_col, vec3<f32>(0.92, 0.99, 0.96), hov_ring * 0.95 * pulse);
                        emit = max(emit, hov_ring * 0.70 * pulse);
                    }
                }

                // Fog-of-War Visibility States (bits 24..25: 0=Visible, 1=Explored Shroud, 2=Deep Unexplored Shroud)
                let fog_state = (tile.packed_flags >> 24u) & 3u;
                if (fog_state == 1u) {
                    let lum = dot(base_col, vec3<f32>(0.299, 0.587, 0.114));
                    base_col = mix(base_col * 0.46, vec3<f32>(lum * 0.34, lum * 0.44, lum * 0.56), 0.70);
                    emit *= 0.30;
                } else if (fog_state == 2u) {
                    let scan = 0.88 + 0.12 * sin(p.x * 2.1 + p.z * 2.1 + cam.time_seconds * 1.6);
                    base_col = mix(base_col * 0.10, vec3<f32>(0.018, 0.032, 0.044) * scan, 0.88);
                    emit *= 0.15;
                }

                out.albedo = base_col;
                out.emissive = emit;
                return out;
            }
        }
        prev_pos = p;
        t += dt;
    }
    return out;
}

// Traces 3D bouncing micro-voxel shrapnel cubes spawned on weapon impacts & unit destruction
fn trace_vfx_debris(ro: vec3<f32>, rd: vec3<f32>, max_t: f32) -> VoxelHit {
    var best: VoxelHit;
    best.hit = false;
    best.t_world = max_t;
    let inv_rd = 1.0 / select(rd, vec3<f32>(1e-6), abs(rd) < vec3<f32>(1e-6));

    for (var i = 0u; i < cam.vfx_debris_count; i = i + 1u) {
        let d = vfx_debris[i];
        let he = d.pos_and_scale.w;
        if (he <= 0.001) { continue; }
        let bmin = d.pos_and_scale.xyz - vec3<f32>(he);
        let bmax = d.pos_and_scale.xyz + vec3<f32>(he);
        let span = intersect_aabb(ro, inv_rd, bmin, bmax);
        if (span.x <= span.y && span.x < best.t_world && span.y > 0.0) {
            let hp = ro + rd * span.x - d.pos_and_scale.xyz;
            let ap = abs(hp);
            var nrm = vec3<f32>(0.0, 1.0, 0.0);
            if (ap.x >= ap.y && ap.x >= ap.z) {
                nrm = vec3<f32>(sign(hp.x), 0.0, 0.0);
            } else if (ap.y >= ap.x && ap.y >= ap.z) {
                nrm = vec3<f32>(0.0, sign(hp.y), 0.0);
            } else {
                nrm = vec3<f32>(0.0, 0.0, sign(hp.z));
            }
            best.hit = true;
            best.t_world = span.x;
            best.normal = nrm;
            best.albedo = d.color_and_emit.xyz;
            best.emissive = d.color_and_emit.w;
        }
    }
    return best;
}

// Evaluates analytical volumetric glow along camera ray `(ro, rd)` up to opaque hit depth `max_t`
fn eval_volumetric_vfx(ro: vec3<f32>, rd: vec3<f32>, max_t: f32) -> vec3<f32> {
    var glow = vec3<f32>(0.0);
    for (var i = 0u; i < cam.vfx_beam_count; i = i + 1u) {
        let b = vfx_beams[i];
        let kind = u32(round(b.color_and_kind.w));
        if (kind == 0u) { continue; }
        let a = b.start_and_radius.xyz;
        let rad = max(b.start_and_radius.w, 0.04);
        let c_end = b.end_and_progress.xyz;
        let prog = b.end_and_progress.w;
        let col = b.color_and_kind.xyz;

        if (kind == 1u || kind == 5u) {
            // LaserBeam (1) or OrbitalDeployColumn (5): closest approach between ray and 3D segment a..c_end
            let seg = c_end - a;
            let w0 = ro - a;
            let a_dot = dot(rd, rd);
            let b_dot = dot(rd, seg);
            let c_dot = dot(seg, seg);
            let d_dot = dot(rd, w0);
            let e_dot = dot(seg, w0);
            let denom = max(a_dot * c_dot - b_dot * b_dot, 1e-5);
            let s_ray = clamp((b_dot * e_dot - c_dot * d_dot) / denom, 0.0, max_t);
            let t_seg = clamp((a_dot * e_dot - b_dot * d_dot) / max(c_dot, 1e-5), 0.0, 1.0);
            let p_ray = ro + rd * s_ray;
            let p_seg = a + seg * t_seg;
            let dist = length(p_ray - p_seg);
            if (s_ray < max_t) {
                let core = exp(-pow(dist / (rad * 0.42), 2.0)) * 2.2;
                let halo = exp(-dist / rad) * 0.75;
                glow += col * (core + halo);
            }
        } else if (kind == 2u) {
            // BallisticTracer (2): glowing slug moving from `a` to `c_end` at `prog`
            let slug_pos = mix(a, c_end, prog) + vec3<f32>(0.0, sin(prog * 3.14159) * 0.55, 0.0);
            let t_proj = clamp(dot(slug_pos - ro, rd), 0.0, max_t);
            if (t_proj < max_t) {
                let d_slug = length((ro + rd * t_proj) - slug_pos);
                glow += col * (exp(-pow(d_slug / (rad * 0.65), 2.0)) * 2.6 + exp(-d_slug / (rad * 1.8)) * 0.7);
            }
        } else if (kind == 3u || kind == 4u) {
            // ExplosionSphere (3) or BioAbilityRing (4) centered at `a`
            let t_proj = clamp(dot(a - ro, rd), 0.0, max_t);
            if (t_proj < max_t) {
                let p_closest = ro + rd * t_proj;
                let d_center = length(p_closest - a);
                if (kind == 3u) {
                    let shell = exp(-pow((d_center - rad * 0.72) / max(rad * 0.32, 0.08), 2.0));
                    let core = exp(-d_center / max(rad * 0.65, 0.08));
                    glow += col * (shell * 1.65 + core * 1.15);
                } else {
                    // Horizontal ring wave near ground plane
                    let inv_ry = 1.0 / select(rd.y, -1e-5, abs(rd.y) < 1e-5);
                    let t_plane = (a.y - ro.y) * inv_ry;
                    if (t_plane > 0.0 && t_plane < max_t) {
                        let hp = ro + rd * t_plane;
                        let r_xz = length(hp.xz - a.xz);
                        let ring_hit = exp(-pow((r_xz - rad) / 0.22, 2.0));
                        glow += col * ring_hit * 2.1;
                    }
                }
            }
        }
    }
    return glow;
}

var<workgroup> wg_tile_count: atomic<u32>;
var<workgroup> wg_tile_insts: array<u32, 128>;

fn pixel_ray_dir(px: vec2<f32>, res: vec2<f32>, ro: vec3<f32>) -> vec3<f32> {
    let uv = (px / max(res, vec2<f32>(1.0, 1.0))) * 2.0 - 1.0;
    let ndc = vec4<f32>(uv.x, -uv.y, 1.0, 1.0);
    let wf = cam.inv_view_proj * ndc;
    return normalize(wf.xyz / wf.w - ro);
}

@compute @workgroup_size(8, 8, 1)
fn cs_main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
    @builtin(workgroup_id) wid: vec3<u32>,
) {
    // 0. Cooperative 8x8 Workgroup Ray-Cone Instance Culling (strictly uniform control flow!)
    if (lid == 0u) {
        atomicStore(&wg_tile_count, 0u);
    }
    workgroupBarrier();

    let ro = cam.camera_pos_spread.xyz;
    let res_f = vec2<f32>(cam.resolution);
    let tile_min = vec2<f32>(f32(wid.x * 8u), f32(wid.y * 8u));
    let tile_max = min(tile_min + vec2<f32>(8.0, 8.0), res_f);
    let tile_mid = (tile_min + tile_max) * 0.5;

    let cone_axis = pixel_ray_dir(tile_mid, res_f, ro);
    let r00 = pixel_ray_dir(tile_min, res_f, ro);
    let r11 = pixel_ray_dir(tile_max, res_f, ro);
    let cos_half = min(dot(cone_axis, r00), dot(cone_axis, r11));
    let sin_half = sqrt(max(1.0 - cos_half * cos_half, 0.0));
    let tan_half = sin_half / max(cos_half, 0.15);

    let inst_total = min(cam.instance_count, 256u);
    for (var idx = lid; idx < inst_total; idx = idx + 64u) {
        let inst = instances[idx];
        let bmin = inst.world_aabb_min.xyz;
        let bmax = inst.world_aabb_max.xyz;
        let center = (bmin + bmax) * 0.5;
        let radius = length(bmax - bmin) * 0.52 + 0.08;
        let to_c = center - ro;
        let along = dot(to_c, cone_axis);
        if (along > -radius) {
            let perp = length(to_c - cone_axis * along);
            if (perp <= radius + max(along, 0.0) * tan_half) {
                let slot = atomicAdd(&wg_tile_count, 1u);
                if (slot < 128u) {
                    wg_tile_insts[slot] = idx;
                }
            }
        }
    }
    workgroupBarrier();

    if (gid.x >= cam.resolution.x || gid.y >= cam.resolution.y) {
        return;
    }

    let rd = pixel_ray_dir(vec2<f32>(f32(gid.x) + 0.5, f32(gid.y) + 0.5), res_f, ro);

    // 1. Trace terrain + ONLY the tile-culled Sparse 64-Tree instances + 3D voxel shrapnel debris
    var best = trace_hex_terrain(ro, rd, 250.0);
    let culled_n = min(atomicLoad(&wg_tile_count), 128u);
    for (var k = 0u; k < culled_n; k = k + 1u) {
        let i = wg_tile_insts[k];
        let uhit = trace_unit_instance(i, ro, rd, best.t_world);
        if (uhit.hit && uhit.t_world < best.t_world) {
            best = uhit;
            let tid = instances[i].team_id;
            if (tid == 1u) {
                best.albedo = mix(best.albedo, vec3<f32>(0.08, 0.72, 0.88), 0.08);
            } else if (tid == 2u) {
                best.albedo = mix(best.albedo, vec3<f32>(0.92, 0.22, 0.16), 0.12);
            }
        }
    }
    let dhit = trace_vfx_debris(ro, rd, best.t_world);
    if (dhit.hit && dhit.t_world < best.t_world) {
        best = dhit;
    }

    // 2. Dynamic Atmosphere Presets (0=ClearDay, 1=DuskTactical, 2=OrbitalFalloutStorm, 3=NightBioGlow)
    var sun_dir = normalize(cam.sun_dir_flags.xyz);
    var sky_low = vec3<f32>(0.05, 0.10, 0.12);
    var sky_high = vec3<f32>(0.18, 0.26, 0.28);
    var haze_col = vec3<f32>(0.10, 0.16, 0.18);
    var sun_tint = vec3<f32>(1.15, 1.13, 1.08);
    var emit_boost = 1.0;

    if (cam.atmosphere_mode == 1u) {
        sun_dir = normalize(vec3<f32>(0.72, 0.34, 0.44));
        sky_low = vec3<f32>(0.18, 0.08, 0.06);
        sky_high = vec3<f32>(0.06, 0.11, 0.20);
        haze_col = vec3<f32>(0.20, 0.11, 0.09);
        sun_tint = vec3<f32>(1.35, 0.88, 0.56);
        emit_boost = 1.22;
    } else if (cam.atmosphere_mode == 2u) {
        let storm_pulse = 0.90 + 0.10 * sin(cam.time_seconds * 2.8);
        sun_dir = normalize(vec3<f32>(-0.42, 0.58, 0.52));
        sky_low = vec3<f32>(0.24, 0.05, 0.03) * storm_pulse;
        sky_high = vec3<f32>(0.12, 0.09, 0.06);
        haze_col = vec3<f32>(0.22, 0.07, 0.04);
        sun_tint = vec3<f32>(1.28, 0.64, 0.42) * storm_pulse;
        emit_boost = 1.40;
    } else if (cam.atmosphere_mode == 3u) {
        sun_dir = normalize(vec3<f32>(-0.35, 0.76, -0.45));
        sky_low = vec3<f32>(0.015, 0.025, 0.05);
        sky_high = vec3<f32>(0.03, 0.06, 0.12);
        haze_col = vec3<f32>(0.02, 0.04, 0.08);
        sun_tint = vec3<f32>(0.42, 0.56, 0.82);
        emit_boost = 1.80;
    }

    var linear_rgb = mix(sky_low, sky_high, clamp(rd.y * 2.0 + 0.2, 0.0, 1.0));

    if (best.hit) {
        let hit_pos = ro + rd * best.t_world;
        let ndotl = max(dot(best.normal, sun_dir), 0.0);

        // Cast shadow ray against voxel unit instances
        var shadow = 1.0;
        if (ndotl > 0.01) {
            let s_ro = hit_pos + best.normal * 0.03;
            for (var si = 0u; si < cam.instance_count; si = si + 1u) {
                let sh = trace_unit_instance(si, s_ro, sun_dir, 40.0);
                if (sh.hit) {
                    shadow = 0.32;
                    break;
                }
            }
        }

        let sky_hemi = 0.26 + 0.18 * (best.normal.y * 0.5 + 0.5);
        let half_v = normalize(sun_dir - rd);
        let spec = pow(max(dot(best.normal, half_v), 0.0), 42.0) * shadow * 0.24;
        linear_rgb = best.albedo * (vec3<f32>(sky_hemi) + sun_tint * (ndotl * shadow))
            + sun_tint * spec
            + best.albedo * (best.emissive * emit_boost);

        let fog = clamp((best.t_world - 35.0) / 95.0, 0.0, 0.65);
        linear_rgb = mix(linear_rgb, haze_col, fog);
    }

    // 3. Add volumetric laser beams, tracers, shockwaves, bio-rings & orbital deploy columns
    linear_rgb += eval_volumetric_vfx(ro, rd, best.t_world);

    // Reinhard highlight compression + Gamma encode (linear -> sRGB) for rgba8unorm
    let mapped = linear_rgb / (vec3<f32>(1.0) + linear_rgb * 0.32);
    let srgb = pow(clamp(mapped, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2));
    textureStore(out_tex, vec2<i32>(gid.xy), vec4<f32>(srgb, 1.0));
}

// =============================================================================
// FULLSCREEN TRIANGLE BLIT PASS (Copies compute texture to swapchain surface)
// =============================================================================
struct BlitVaryings {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var blit_tex: texture_2d<f32>;
@group(0) @binding(1) var blit_samp: sampler;

@vertex
fn vs_blit(@builtin(vertex_index) vid: u32) -> BlitVaryings {
    var out: BlitVaryings;
    let x = f32(i32(vid & 1u) * 4 - 1);
    let y = f32(i32(vid & 2u) * 2 - 1);
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@fragment
fn fs_blit(in: BlitVaryings) -> @location(0) vec4<f32> {
    let sampled = textureSample(blit_tex, blit_samp, in.uv);
    // Decode sRGB back to linear so an sRGB swapchain hardware-encodes once cleanly
    let lin = pow(sampled.rgb, vec3<f32>(2.2));
    return vec4<f32>(lin, 1.0);
}
