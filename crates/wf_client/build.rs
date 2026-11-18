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

    write_wildfront_icons_and_resources(&out_dir);
}

fn hex_sdf(px: f32, py: f32, r: f32) -> f32 {
    // Pointy-top regular hexagon SDF centered at (0,0) with circumradius r (flat apothem = r * sqrt(3)/2)
    let kx = -0.866_025_4_f32;
    let ky = 0.5_f32;
    let kz = 0.577_350_26_f32;
    let mut x = px.abs();
    let mut y = py.abs();
    let dot = 2.0 * (kx * x + ky * y).min(0.0);
    x -= dot * kx;
    y -= dot * ky;
    let clamped_x = x.clamp(-kz * r, kz * r);
    let dx = x - clamped_x;
    let dy = y - r * 0.866_025_4;
    dx.hypot(dy) * dy.signum()
}

fn blend_rgba(dst: &mut [u8; 4], src: [u8; 4], cov: f32) {
    let a = (cov.clamp(0.0, 1.0) * (src[3] as f32 / 255.0)).clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let inv = 1.0 - a;
    dst[0] = (src[0] as f32 * a + dst[0] as f32 * inv).round().clamp(0.0, 255.0) as u8;
    dst[1] = (src[1] as f32 * a + dst[1] as f32 * inv).round().clamp(0.0, 255.0) as u8;
    dst[2] = (src[2] as f32 * a + dst[2] as f32 * inv).round().clamp(0.0, 255.0) as u8;
    let out_a = a + (dst[3] as f32 / 255.0) * inv;
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

fn render_wildfront_icon_rgba(size: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;
    let aa = 1.5 / s;

    for py in 0..size {
        for px in 0..size {
            let nx = ((px as f32 + 0.5) / s - 0.5) * 2.0;
            let ny = ((py as f32 + 0.5) / s - 0.5) * 2.0;
            let mut col = [0u8, 0, 0, 0];

            // 1. Outer armored slate hexagon
            let d_outer = hex_sdf(nx, ny, 0.94);
            let cov_outer = ((-d_outer) / aa + 0.5).clamp(0.0, 1.0);
            blend_rgba(&mut col, [15, 26, 34, 255], cov_outer);

            // 2. Cyan tactical perimeter hex border
            let ring_outer = d_outer.abs() - 0.055;
            let cov_cyan = ((-ring_outer) / aa + 0.5).clamp(0.0, 1.0);
            blend_rgba(&mut col, [68, 206, 232, 255], cov_cyan);

            // 3. Inner emerald bio-hex ring
            let d_inner = hex_sdf(nx, ny, 0.62);
            let ring_inner = d_inner.abs() - 0.045;
            let cov_green = ((-ring_inner) / aa + 0.5).clamp(0.0, 1.0);
            blend_rgba(&mut col, [76, 242, 162, 255], cov_green);

            // 4. Central amber tactical chevron / bio-core diamond
            let chev_y = ny + 0.06;
            let d_chev_outer = (nx.abs() * 0.85 + (chev_y - 0.14).abs() * 0.72) - 0.30;
            let d_chev_inner = (nx.abs() * 0.85 + (chev_y - 0.25).abs() * 0.72) - 0.16;
            let d_chev = if chev_y > 0.10 {
                d_chev_outer.max(-d_chev_inner)
            } else {
                d_chev_outer
            };
            let cov_amber = ((-d_chev) / aa + 0.5).clamp(0.0, 1.0);
            blend_rgba(&mut col, [246, 178, 54, 255], cov_amber);

            // 5. Center emerald node pip
            let d_pip = nx.hypot(ny + 0.02) - 0.10;
            let cov_pip = ((-d_pip) / aa + 0.5).clamp(0.0, 1.0);
            blend_rgba(&mut col, [112, 248, 255, 255], cov_pip);

            let idx = ((py * size + px) * 4) as usize;
            out[idx..idx + 4].copy_from_slice(&col);
        }
    }
    out
}

fn encode_ico_dib_entry(size: u32, rgba: &[u8]) -> Vec<u8> {
    let w = size as usize;
    let h = size as usize;
    let xor_bytes = w * h * 4;
    let and_row_stride = ((w + 31) / 32) * 4;
    let and_bytes = and_row_stride * h;
    let mut buf = Vec::with_capacity(40 + xor_bytes + and_bytes);

    // BITMAPINFOHEADER (40 bytes)
    buf.extend_from_slice(&40u32.to_le_bytes());
    buf.extend_from_slice(&(size as i32).to_le_bytes());
    buf.extend_from_slice(&((size * 2) as i32).to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&32u16.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&(xor_bytes as u32).to_le_bytes());
    buf.extend_from_slice(&[0u8; 16]);

    // Bottom-up BGRA pixels
    for y in (0..h).rev() {
        for x in 0..w {
            let idx = (y * w + x) * 4;
            buf.push(rgba[idx + 2]);
            buf.push(rgba[idx + 1]);
            buf.push(rgba[idx]);
            buf.push(rgba[idx + 3]);
        }
    }
    // Bottom-up 1-bit AND mask
    for y in (0..h).rev() {
        let mut row = vec![0u8; and_row_stride];
        for x in 0..w {
            let idx = (y * w + x) * 4;
            if rgba[idx + 3] < 128 {
                row[x / 8] |= 1 << (7 - (x % 8));
            }
        }
        buf.extend_from_slice(&row);
    }
    buf
}

fn build_multi_size_ico(sizes: &[u32]) -> Vec<u8> {
    let entries: Vec<(u32, Vec<u8>)> = sizes
        .iter()
        .map(|&s| {
            let rgba = render_wildfront_icon_rgba(s);
            (s, encode_ico_dib_entry(s, &rgba))
        })
        .collect();

    let count = entries.len() as u16;
    let mut ico = Vec::new();
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&count.to_le_bytes());

    let mut offset = 6u32 + u32::from(count) * 16u32;
    for (s, payload) in &entries {
        let dim = if *s >= 256 { 0u8 } else { *s as u8 };
        ico.push(dim);
        ico.push(dim);
        ico.push(0);
        ico.push(0);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += payload.len() as u32;
    }
    for (_, payload) in entries {
        ico.extend_from_slice(&payload);
    }
    ico
}

fn find_windows_sdk_rc_exe() -> Option<PathBuf> {
    if let Ok(status) = std::process::Command::new("rc.exe").arg("/?").output() {
        if status.status.success() || !status.stdout.is_empty() || !status.stderr.is_empty() {
            return Some(PathBuf::from("rc.exe"));
        }
    }
    let kits_bin = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin");
    if let Ok(entries) = fs::read_dir(&kits_bin) {
        let mut versions: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|x| x.path())).collect();
        versions.sort();
        versions.reverse();
        for ver_dir in versions {
            let candidate = ver_dir.join("x64/rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn write_wildfront_icons_and_resources(out_dir: &std::path::Path) {
    let rgba64 = render_wildfront_icon_rgba(64);
    fs::write(out_dir.join("wildfront_icon_64.rgba"), &rgba64)
        .expect("Failed writing wildfront_icon_64.rgba");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let ico_path = out_dir.join("wildfront.ico");
    let ico_bytes = build_multi_size_ico(&[16, 32, 48, 64]);
    fs::write(&ico_path, &ico_bytes).expect("Failed writing wildfront.ico");

    let ico_rc_path = ico_path.to_string_lossy().replace('\\', "/");
    let rc_content = format!(
        r#"1 ICON "{ico_rc_path}"
1 VERSIONINFO
FILEVERSION 3,4,1,0
PRODUCTVERSION 3,4,1,0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "CRV Science Network"
      VALUE "FileDescription", "WildFront BattleGrounds - Voxel Hex Strategy Client"
      VALUE "FileVersion", "3.4.1"
      VALUE "InternalName", "wf_client"
      VALUE "LegalCopyright", "Copyright CRV Science Network (GPL-3.0-or-later)"
      VALUE "OriginalFilename", "2_WildFront.exe"
      VALUE "ProductName", "WildFront BattleGrounds"
      VALUE "ProductVersion", "3.4.1"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let rc_path = out_dir.join("wildfront.rc");
    fs::write(&rc_path, rc_content).expect("Failed writing wildfront.rc");

    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_env == "gnu" {
        let windres = env::var("WINDRES").unwrap_or_else(|_| "windres".to_string());
        let obj_path = out_dir.join("wildfront_res.o");
        match std::process::Command::new(&windres)
            .args([
                "-i",
                &rc_path.to_string_lossy(),
                "-o",
                &obj_path.to_string_lossy(),
                "-O",
                "coff",
            ])
            .status()
        {
            Ok(st) if st.success() => {
                println!("cargo:rustc-link-arg={}", obj_path.display());
            }
            Ok(st) => {
                println!("cargo:warning=windres exited with {st}; skipping PE icon resource.");
            }
            Err(e) => {
                println!("cargo:warning=windres not available ({e}); skipping PE icon resource.");
            }
        }
    } else if target_env == "msvc" {
        let res_path = out_dir.join("wildfront_res.res");
        if let Some(rc_exe) = find_windows_sdk_rc_exe() {
            match std::process::Command::new(&rc_exe)
                .args([
                    "/nologo",
                    "/fo",
                    &res_path.to_string_lossy(),
                    &rc_path.to_string_lossy(),
                ])
                .status()
            {
                Ok(st) if st.success() => {
                    println!("cargo:rustc-link-arg={}", res_path.display());
                }
                Ok(st) => {
                    println!("cargo:warning=rc.exe exited with {st}; skipping PE icon resource.");
                }
                Err(e) => {
                    println!("cargo:warning=rc.exe failed ({e}); skipping PE icon resource.");
                }
            }
        } else {
            println!("cargo:warning=rc.exe not found; skipping PE icon resource.");
        }
    }
}
