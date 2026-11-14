//! Shared Sparse 64-Tree (`4x4x4` branching factor) voxel representation and
//! flattened GPU buffer builder used by `wf_assets` and `wf_client`.
//!
//! Coordinate convention:
//! - Source `.vox` files use MagicaVoxel Z-up `(vx, vy, vz)` where `-vy` faces forward.
//! - On upload, `compile_multi_asset_64tree` remaps `(vx, vz, vy)` -> local `(x, y_up, z)`
//!   inside a `[0..256]^3` Sparse 64-Tree (`Level 2: 64^3` -> `Level 1: 16^3` -> `Level 0: 4^3` -> `Leaf Brick: 1^3`).

use std::collections::BTreeMap;
use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

fn deserialize_voxels_z_yx<'de, D>(deserializer: D) -> Result<[[u8; 16]; 4], D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Row16([u8; 16]);
    impl<'de> serde::Deserialize<'de> for Row16 {
        fn deserialize<D2>(d: D2) -> Result<Self, D2::Error>
        where
            D2: serde::Deserializer<'de>,
        {
            struct RowVisitor;
            impl<'de> serde::de::Visitor<'de> for RowVisitor {
                type Value = [u8; 16];
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("a sequence or tuple of 16 u8 voxel indices")
                }
                fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                where
                    A: serde::de::SeqAccess<'de>,
                {
                    let mut out = [0u8; 16];
                    for (i, slot) in out.iter_mut().enumerate() {
                        *slot = seq
                            .next_element()?
                            .ok_or_else(|| serde::de::Error::invalid_length(i, &self))?;
                    }
                    if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                        return Err(serde::de::Error::invalid_length(17, &self));
                    }
                    Ok(out)
                }
            }
            d.deserialize_any(RowVisitor).map(Row16)
        }
    }

    struct OuterVisitor;
    impl<'de> serde::de::Visitor<'de> for OuterVisitor {
        type Value = [[u8; 16]; 4];
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a sequence or tuple of 4 brick Z-layers")
        }
        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut out = [[0u8; 16]; 4];
            for (i, slot) in out.iter_mut().enumerate() {
                let row: Row16 = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(i, &self))?;
                *slot = row.0;
            }
            if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::invalid_length(5, &self));
            }
            Ok(out)
        }
    }

    deserializer.deserialize_any(OuterVisitor)
}

/// One sparse `4x4x4` brick in MagicaVoxel Z-up coordinates `(bx, by, bz)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SparseBrick4 {
    pub brick_pos: (u8, u8, u8),
    #[serde(deserialize_with = "deserialize_voxels_z_yx")]
    pub voxels_z_yx: [[u8; 16]; 4],
}

/// Serialized `.ron` voxel asset shared between `wf_assets` (baker) and `wf_client` (GPU loader).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BakedVoxelAsset {
    pub model_name: String,
    pub dimensions: (u32, u32, u32),
    pub non_empty_voxels: usize,
    pub palette_rgba: Vec<(u8, u8, u8, u8)>,
    pub bricks: Vec<SparseBrick4>,
}

/// 16-byte GPU Sparse 64-Tree node (`4x4x4` children).
/// - `mask_lo` / `mask_hi`: 64-bit child occupancy bitmask (`bit = cx + cy * 4 + cz * 16`).
/// - `child_base_ptr`: index of first populated child in `nodes` (or `bricks` if `flags & 1 != 0`).
///   Populated child offset = `countOneBits` of lower bits in the 64-bit mask!
/// - `flags_and_lod_rgba`: bit 0 = `is_leaf_parent` (children are `GpuLeafBrick`),
///   bits 8..31 = packed RGB average color for far-zoom cone LOD termination.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct GpuNode64 {
    pub mask_lo: u32,
    pub mask_hi: u32,
    pub child_base_ptr: u32,
    pub flags_and_lod_rgba: u32,
}

/// 80-byte GPU `4x4x4` leaf brick (`16-byte` aligned):
/// - `mask_lo` / `mask_hi`: 64-bit solid voxel occupancy mask (`bit = lx + ly * 4 + lz * 16`).
/// - `lod_rgba`: packed average RGBA color of this `4x4x4` brick.
/// - `packed_mats`: 64 x `u8` palette indices (`1..=255`) packed 4-per-`u32` (`16` x `u32`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct GpuLeafBrick {
    pub mask_lo: u32,
    pub mask_hi: u32,
    pub lod_rgba: u32,
    pub _pad: u32,
    pub packed_mats: [u32; 16],
}

#[derive(Debug, Clone, Copy)]
pub struct CompiledModelHandle {
    pub root_node_index: u32,
    /// Model space dimensions in engine Y-up coordinates `(sx, sy_up, sz)`.
    pub size_voxels: [u32; 3],
}

pub struct MultiAssetGpuBuffers {
    pub nodes: Vec<GpuNode64>,
    pub bricks: Vec<GpuLeafBrick>,
    pub palette_linear_rgba: [[f32; 4]; 256],
    pub handles: BTreeMap<String, CompiledModelHandle>,
}

fn pack_rgb_flag(r: u8, g: u8, b: u8, is_twig: bool) -> u32 {
    let flag = u32::from(is_twig);
    flag | (u32::from(r) << 8) | (u32::from(g) << 16) | (u32::from(b) << 24)
}

fn unpack_rgb(packed: u32) -> (u32, u32, u32) {
    ((packed >> 8) & 0xFF, (packed >> 16) & 0xFF, (packed >> 24) & 0xFF)
}

fn set_bit64(lo: &mut u32, hi: &mut u32, bit: usize) {
    if bit < 32 {
        *lo |= 1u32 << bit;
    } else {
        *hi |= 1u32 << (bit - 32);
    }
}

/// Compiles multiple `BakedVoxelAsset` rigs into one unified GPU `Node64` + `GpuLeafBrick` buffer pair.
/// Supports models up to `256x256x256` voxels (`4^4`: Root[64s] -> L1[16s] -> Twig[4s] -> LeafBrick[1s]).
pub fn compile_multi_asset_64tree(assets: &[BakedVoxelAsset]) -> MultiAssetGpuBuffers {
    let mut palette_linear_rgba = [[0.0f32; 4]; 256];
    if let Some(first) = assets.first() {
        for (idx, &(r, g, b, a)) in first.palette_rgba.iter().take(256).enumerate() {
            // Index 0 in shader is unused air; palette index `c` (1..=255) reads slot `c - 1`
            palette_linear_rgba[idx] = [
                (f32::from(r) / 255.0).powf(2.2),
                (f32::from(g) / 255.0).powf(2.2),
                (f32::from(b) / 255.0).powf(2.2),
                f32::from(a) / 255.0,
            ];
        }
    }

    let mut all_nodes: Vec<GpuNode64> = Vec::new();
    let mut all_bricks: Vec<GpuLeafBrick> = Vec::new();
    let mut handles = BTreeMap::new();

    for asset in assets {
        let pal = &asset.palette_rgba;
        // Remap MagicaVoxel Z-up `(vx, vy, vz)` -> Engine Y-up `(x = vx, y = vz, z = vy)`
        let mut y_up_bricks: BTreeMap<(u8, u8, u8), GpuLeafBrick> = BTreeMap::new();

        for sb in &asset.bricks {
            let (bx_v, by_v, bz_v) = sb.brick_pos;
            for lz_v in 0..4usize {
                for ly_v in 0..4usize {
                    for lx_v in 0..4usize {
                        let mat = sb.voxels_z_yx[lz_v][lx_v + ly_v * 4];
                        if mat == 0 {
                            continue;
                        }
                        let wx = usize::from(bx_v) * 4 + lx_v;
                        let wy = usize::from(bz_v) * 4 + lz_v; // Z-up -> Y-up!
                        let wz = usize::from(by_v) * 4 + ly_v;
                        if wx >= 256 || wy >= 256 || wz >= 256 {
                            continue;
                        }
                        let b_coord = ((wx / 4) as u8, (wy / 4) as u8, (wz / 4) as u8);
                        let bit = (wx % 4) + (wy % 4) * 4 + (wz % 4) * 16;
                        let entry = y_up_bricks.entry(b_coord).or_default();
                        set_bit64(&mut entry.mask_lo, &mut entry.mask_hi, bit);
                        let word = bit / 4;
                        let shift = (bit % 4) * 8;
                        entry.packed_mats[word] |= u32::from(mat) << shift;
                    }
                }
            }
        }

        // Compute average RGB per leaf brick for LOD mipmapping
        for brick in y_up_bricks.values_mut() {
            let (mut sr, mut sg, mut sb_c, mut cnt) = (0u32, 0u32, 0u32, 0u32);
            for idx in 0..64usize {
                let m = ((brick.packed_mats[idx / 4] >> ((idx % 4) * 8)) & 0xFF) as usize;
                if m > 0 && m <= pal.len() {
                    let (r, g, b, _) = pal[m - 1];
                    sr += u32::from(r);
                    sg += u32::from(g);
                    sb_c += u32::from(b);
                    cnt += 1;
                }
            }
            if cnt > 0 {
                brick.lod_rgba = pack_rgb_flag(
                    (sr / cnt) as u8,
                    (sg / cnt) as u8,
                    (sb_c / cnt) as u8,
                    true,
                );
            }
        }

        // Group Leaf Bricks (4^3) into Twig Nodes (16^3, coords 0..16)
        let mut twig_groups: BTreeMap<(u8, u8, u8), Vec<((u8, u8, u8), GpuLeafBrick)>> = BTreeMap::new();
        for (&(bx, by, bz), &brick) in &y_up_bricks {
            twig_groups
                .entry((bx / 4, by / 4, bz / 4))
                .or_default()
                .push(((bx % 4, by % 4, bz % 4), brick));
        }

        // Group Twigs (16^3) into Mid L1 Nodes (64^3, coords 0..4)
        let mut l1_groups: BTreeMap<(u8, u8, u8), Vec<((u8, u8, u8), GpuNode64)>> = BTreeMap::new();
        for ((tx, ty, tz), mut children) in twig_groups {
            // Sort children in canonical bit order `cx + cy*4 + cz*16`!
            children.sort_by_key(|&((cx, cy, cz), _)| usize::from(cx) + usize::from(cy) * 4 + usize::from(cz) * 16);
            let brick_base = all_bricks.len() as u32;
            let (mut lo, mut hi) = (0u32, 0u32);
            let (mut sr, mut sg, mut sb_c) = (0u32, 0u32, 0u32);

            for &((cx, cy, cz), br) in &children {
                let bit = usize::from(cx) + usize::from(cy) * 4 + usize::from(cz) * 16;
                set_bit64(&mut lo, &mut hi, bit);
                let (r, g, b) = unpack_rgb(br.lod_rgba);
                sr += r;
                sg += g;
                sb_c += b;
                all_bricks.push(br);
            }
            let n_ch = children.len().max(1) as u32;
            let twig_node = GpuNode64 {
                mask_lo: lo,
                mask_hi: hi,
                child_base_ptr: brick_base,
                flags_and_lod_rgba: pack_rgb_flag(
                    (sr / n_ch) as u8,
                    (sg / n_ch) as u8,
                    (sb_c / n_ch) as u8,
                    true, // Twig node -> children are in `bricks`!
                ),
            };
            l1_groups
                .entry((tx / 4, ty / 4, tz / 4))
                .or_default()
                .push(((tx % 4, ty % 4, tz % 4), twig_node));
        }

        // Reserve root slot first so root_node_index is stable
        let root_idx = all_nodes.len() as u32;
        all_nodes.push(GpuNode64::default());

        // Emit L1 nodes (64^3) and build Root Node (256^3)
        let mut root_entries: Vec<((u8, u8, u8), GpuNode64)> = Vec::new();
        for ((mx, my, mz), mut twigs) in l1_groups {
            twigs.sort_by_key(|&((cx, cy, cz), _)| usize::from(cx) + usize::from(cy) * 4 + usize::from(cz) * 16);
            let twig_base = all_nodes.len() as u32;
            let (mut lo, mut hi) = (0u32, 0u32);
            let (mut sr, mut sg, mut sb_c) = (0u32, 0u32, 0u32);

            for &((cx, cy, cz), tnode) in &twigs {
                let bit = usize::from(cx) + usize::from(cy) * 4 + usize::from(cz) * 16;
                set_bit64(&mut lo, &mut hi, bit);
                let (r, g, b) = unpack_rgb(tnode.flags_and_lod_rgba);
                sr += r;
                sg += g;
                sb_c += b;
                all_nodes.push(tnode);
            }
            let n_ch = twigs.len().max(1) as u32;
            let l1_node = GpuNode64 {
                mask_lo: lo,
                mask_hi: hi,
                child_base_ptr: twig_base,
                flags_and_lod_rgba: pack_rgb_flag(
                    (sr / n_ch) as u8,
                    (sg / n_ch) as u8,
                    (sb_c / n_ch) as u8,
                    false, // Interior node -> children are in `nodes`!
                ),
            };
            root_entries.push(((mx, my, mz), l1_node));
        }

        root_entries.sort_by_key(|&((cx, cy, cz), _)| usize::from(cx) + usize::from(cy) * 4 + usize::from(cz) * 16);
        let l1_base = all_nodes.len() as u32;
        let (mut r_lo, mut r_hi) = (0u32, 0u32);
        let (mut sr, mut sg, mut sb_c) = (0u32, 0u32, 0u32);
        for &((cx, cy, cz), l1_node) in &root_entries {
            let bit = usize::from(cx) + usize::from(cy) * 4 + usize::from(cz) * 16;
            set_bit64(&mut r_lo, &mut r_hi, bit);
            let (r, g, b) = unpack_rgb(l1_node.flags_and_lod_rgba);
            sr += r;
            sg += g;
            sb_c += b;
            all_nodes.push(l1_node);
        }
        let n_r = root_entries.len().max(1) as u32;
        all_nodes[root_idx as usize] = GpuNode64 {
            mask_lo: r_lo,
            mask_hi: r_hi,
            child_base_ptr: l1_base,
            flags_and_lod_rgba: pack_rgb_flag(
                (sr / n_r) as u8,
                (sg / n_r) as u8,
                (sb_c / n_r) as u8,
                false,
            ),
        };

        handles.insert(
            asset.model_name.clone(),
            CompiledModelHandle {
                root_node_index: root_idx,
                size_voxels: [asset.dimensions.0, asset.dimensions.2, asset.dimensions.1],
            },
        );
    }

    MultiAssetGpuBuffers {
        nodes: all_nodes,
        bricks: all_bricks,
        palette_linear_rgba,
        handles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_valid_4_level_64tree_and_popcnt_offsets() {
        let mut vox = [[0u8; 16]; 4];
        vox[2][1 + 3 * 4] = 17; // (vx=1, vy=3, vz=2) -> Y-up (x=1, y=2, z=3)
        let asset = BakedVoxelAsset {
            model_name: "test_rig".to_string(),
            dimensions: (16, 16, 16),
            non_empty_voxels: 1,
            palette_rgba: vec![(200, 50, 40, 255); 256],
            bricks: vec![SparseBrick4 {
                brick_pos: (0, 0, 0),
                voxels_z_yx: vox,
            }],
        };

        let gpu = compile_multi_asset_64tree(&[asset]);
        let h = gpu.handles["test_rig"];
        assert_eq!(h.root_node_index, 0);
        assert_eq!(gpu.bricks.len(), 1);
        // Bit index inside Y-up brick: lx=1, ly=2, lz=3 -> 1 + 8 + 48 = 57 (in mask_hi bit 25)
        assert_eq!(gpu.bricks[0].mask_hi, 1u32 << (57 - 32));
    }

    #[test]
    fn deserializes_sparse_brick4_from_both_list_and_tuple_ron_syntax() {
        let mixed_list_tuple = r#"(
            brick_pos: (1, 2, 3),
            voxels_z_yx: (
                [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                [16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255],
            ),
        )"#;
        let parsed_a: SparseBrick4 = ron::from_str(mixed_list_tuple).expect("list-in-tuple RON must parse");
        assert_eq!(parsed_a.brick_pos, (1, 2, 3));
        assert_eq!(parsed_a.voxels_z_yx[0][15], 15);
        assert_eq!(parsed_a.voxels_z_yx[3][15], 255);

        let tuple_in_tuple = r#"(
            brick_pos: (0, 0, 0),
            voxels_z_yx: (
                (0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
                (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                (16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 42),
            ),
        )"#;
        let parsed_b: SparseBrick4 = ron::from_str(tuple_in_tuple).expect("tuple-in-tuple RON must parse");
        assert_eq!(parsed_b.voxels_z_yx[0][5], 5);
        assert_eq!(parsed_b.voxels_z_yx[3][15], 42);
    }
}
