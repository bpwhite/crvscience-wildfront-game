// Copyright (C) 2026 CRV Science Network (https://crvscience.com)
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://gnu.org>.

use std::env;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use wf_assets::vox_baker::bake_vox_file;
use wf_core::{compile_multi_asset_64tree, BakedVoxelAsset, SparseBrick4};

fn make_fallback_box_rig(shared_palette: Vec<(u8, u8, u8, u8)>) -> BakedVoxelAsset {
    use std::collections::BTreeMap;
    let palette_rgba = if shared_palette.is_empty() {
        vec![(140, 175, 200, 255), (210, 160, 90, 255)]
    } else {
        shared_palette
    };
    let mut bricks_map: BTreeMap<(u8, u8, u8), [[u8; 16]; 4]> = BTreeMap::new();
    let mut non_empty_voxels = 0usize;
    for vz in 0u8..28 {
        let half = if vz < 4 { 10 } else if vz < 20 { 6 } else { 4 };
        let mat = if vz < 4 || vz >= 20 { 2u8 } else { 1u8 };
        for vy in (12 - half)..=(12 + half) {
            for vx in (12 - half)..=(12 + half) {
                let b_pos = (vx / 4, vy / 4, vz / 4);
                let lx = (vx % 4) as usize;
                let ly = (vy % 4) as usize;
                let lz = (vz % 4) as usize;
                let entry = bricks_map.entry(b_pos).or_insert([[0u8; 16]; 4]);
                entry[lz][lx + ly * 4] = mat;
                non_empty_voxels += 1;
            }
        }
    }
    let bricks = bricks_map
        .into_iter()
        .map(|(brick_pos, voxels_z_yx)| SparseBrick4 {
            brick_pos,
            voxels_z_yx,
        })
        .collect();
    BakedVoxelAsset {
        model_name: String::from("__fallback_box_rig"),
        dimensions: (24, 24, 28),
        non_empty_voxels,
        palette_rgba,
        bricks,
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.join("../..");
    let vox_dir = workspace_root.join("assets/vox");
    let compiled_dir = workspace_root.join("assets/compiled");
    let sfx_dir = workspace_root.join("assets/sfx");

    println!("cargo:rerun-if-changed={}", vox_dir.display());
    println!(
        "cargo:rerun-if-changed={}",
        compiled_dir.join("master_roster.ron").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        compiled_dir.join("starter_deck_plant_cnidarian.ron").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        compiled_dir.join("starter_deck_mammal_reptile.ron").display()
    );
    println!("cargo:rerun-if-changed={}", sfx_dir.display());

    let mut vox_paths: Vec<PathBuf> = fs::read_dir(&vox_dir)
        .unwrap_or_else(|e| panic!("Failed reading '{}': {e}", vox_dir.display()))
        .filter_map(|ent| ent.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("vox"))
        .collect();
    vox_paths.sort();

    let mut baked_assets = Vec::with_capacity(vox_paths.len());
    for path in &vox_paths {
        println!("cargo:rerun-if-changed={}", path.display());
        let baked = bake_vox_file(path)
            .unwrap_or_else(|e| panic!("Failed baking '{}': {e}", path.display()));
        baked_assets.push(baked);
    }

    let shared_palette = baked_assets
        .first()
        .map(|a| a.palette_rgba.clone())
        .unwrap_or_default();
    baked_assets.push(make_fallback_box_rig(shared_palette));

    let vox_gpu = compile_multi_asset_64tree(&baked_assets);

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let out_file_path = out_dir.join("precompiled_vox_64tree.bin");
    let file = File::create(&out_file_path)
        .unwrap_or_else(|e| panic!("Failed creating '{}': {e}", out_file_path.display()));
    let mut enc = ZlibEncoder::new(BufWriter::new(file), Compression::default());

    enc.write_all(b"WF64TR01").expect("write magic");
    for rgba in &vox_gpu.palette_linear_rgba {
        for ch in rgba {
            enc.write_all(&ch.to_le_bytes()).expect("write palette");
        }
    }

    let node_count = u32::try_from(vox_gpu.nodes.len()).expect("node count fits u32");
    enc.write_all(&node_count.to_le_bytes()).expect("write node count");
    for n in &vox_gpu.nodes {
        enc.write_all(&n.mask_lo.to_le_bytes()).unwrap();
        enc.write_all(&n.mask_hi.to_le_bytes()).unwrap();
        enc.write_all(&n.child_base_ptr.to_le_bytes()).unwrap();
        enc.write_all(&n.flags_and_lod_rgba.to_le_bytes()).unwrap();
    }

    let brick_count = u32::try_from(vox_gpu.bricks.len()).expect("brick count fits u32");
    enc.write_all(&brick_count.to_le_bytes()).expect("write brick count");
    for b in &vox_gpu.bricks {
        enc.write_all(&b.mask_lo.to_le_bytes()).unwrap();
        enc.write_all(&b.mask_hi.to_le_bytes()).unwrap();
        enc.write_all(&b.lod_rgba.to_le_bytes()).unwrap();
        enc.write_all(&b._pad.to_le_bytes()).unwrap();
        for m in &b.packed_mats {
            enc.write_all(&m.to_le_bytes()).unwrap();
        }
    }

    let handle_count = u32::try_from(vox_gpu.handles.len()).expect("handle count fits u32");
    enc.write_all(&handle_count.to_le_bytes()).expect("write handle count");
    for (slug, h) in &vox_gpu.handles {
        let slug_bytes = slug.as_bytes();
        let slug_len = u32::try_from(slug_bytes.len()).expect("slug length fits u32");
        enc.write_all(&slug_len.to_le_bytes()).unwrap();
        enc.write_all(slug_bytes).unwrap();
        enc.write_all(&h.root_node_index.to_le_bytes()).unwrap();
        for s in h.size_voxels {
            enc.write_all(&s.to_le_bytes()).unwrap();
        }
    }

    enc.finish().expect("finish zlib stream");
}
