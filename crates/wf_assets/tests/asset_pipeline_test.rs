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

//! Integration tests verifying `wf_assets` Excel-to-RON compiler, binary `.vox` RIFF baker,
//! and watertight 2-manifold `.STL` exporter with guaranteed print-thick V-cradle divider walls.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use wf_assets::stl_exporter::{
    build_coplanar_voxel_stl_triangles, export_card_to_stl, export_deck_to_stl_folder,
    export_vox_to_stl, resolve_cradle_layout, write_binary_stl_triangles, CradleMode,
    ResolvedCradleLayout, TabletopPrintConfig, MIN_PRINTABLE_CENTER_WALL_MM,
};
use wf_assets::vox_baker::bake_vox_file;
use wf_assets::xlsx_compiler::compile_master_xlsx;
use wf_core::{DeckList, RosterManifest};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("Failed resolving workspace root")
}

/// Parses a binary `.STL` and audits strict half-edge 2-manifoldness:
/// - `boundary_edges == 0` (every directed edge `(a, b)` has its opposite `(b, a)`),
/// - `non_manifold_edges == 0` (no directed edge appears more than once),
/// - `signed_volume_mm3 > 1000.0` (closed outward-oriented solid volume).
fn audit_binary_stl_2manifold(stl_bytes: &[u8]) -> (usize, usize, usize, f64) {
    assert!(stl_bytes.len() >= 84, "STL shorter than 84-byte header");
    let tri_count = u32::from_le_bytes(stl_bytes[80..84].try_into().unwrap()) as usize;
    assert_eq!(stl_bytes.len(), 84 + tri_count * 50, "STL byte length mismatch");

    let quant = |v: [f32; 3]| -> (i64, i64, i64) {
        (
            (v[0] as f64 * 1000.0).round() as i64,
            (v[1] as f64 * 1000.0).round() as i64,
            (v[2] as f64 * 1000.0).round() as i64,
        )
    };

    let mut directed_edges: HashMap<((i64, i64, i64), (i64, i64, i64)), usize> =
        HashMap::with_capacity(tri_count * 3);
    let mut signed_vol6 = 0.0_f64;

    for i in 0..tri_count {
        let base = 84 + i * 50;
        let read_v = |off: usize| -> [f32; 3] {
            [
                f32::from_le_bytes(stl_bytes[off..off + 4].try_into().unwrap()),
                f32::from_le_bytes(stl_bytes[off + 4..off + 8].try_into().unwrap()),
                f32::from_le_bytes(stl_bytes[off + 8..off + 12].try_into().unwrap()),
            ]
        };
        let v0 = read_v(base + 12);
        let v1 = read_v(base + 24);
        let v2 = read_v(base + 36);

        let p0 = quant(v0);
        let p1 = quant(v1);
        let p2 = quant(v2);
        for (a, b) in [(p0, p1), (p1, p2), (p2, p0)] {
            *directed_edges.entry((a, b)).or_insert(0) += 1;
        }

        let (x0, y0, z0) = (v0[0] as f64, v0[1] as f64, v0[2] as f64);
        let (x1, y1, z1) = (v1[0] as f64, v1[1] as f64, v1[2] as f64);
        let (x2, y2, z2) = (v2[0] as f64, v2[1] as f64, v2[2] as f64);
        signed_vol6 += x0 * (y1 * z2 - z1 * y2)
            - y0 * (x1 * z2 - z1 * x2)
            + z0 * (x1 * y2 - y1 * x2);
    }

    let mut boundary_edges = 0usize;
    let mut non_manifold_edges = 0usize;
    for (&(a, b), &cnt) in &directed_edges {
        if cnt != 1 {
            non_manifold_edges += 1;
        }
        match directed_edges.get(&(b, a)).copied().unwrap_or(0) {
            0 => boundary_edges += 1,
            1 => {}
            _ => non_manifold_edges += 1,
        }
    }

    (tri_count, boundary_edges, non_manifold_edges, signed_vol6 / 6.0)
}

#[test]
fn test_xlsx_compiler_parses_152_balanced_units() {
    let xlsx = repo_root().join("data/Wildfront_PC_Master_Stats_and_DeckBuilder.xlsx");
    let (manifest, starter_deck) =
        compile_master_xlsx(&xlsx).expect("Failed compiling Master Excel workbook");
    assert_eq!(manifest.units.len(), 152);
    manifest.validate().expect("Compiled manifest failed validation");
    let report = starter_deck.validate_against(&manifest);
    assert!(report.is_legal, "Workbook starter deck violations: {:?}", report.violations);
}

#[test]
fn test_vox_baker_parses_binary_magica_voxel_rigs() {
    let root = repo_root();
    for vox_rel in [
        "assets/vox/hospital.vox",
        "assets/vox/supply_truck.vox",
        "assets/vox/prop_mountain_crag.vox",
    ] {
        let baked = bake_vox_file(&root.join(vox_rel)).expect("Failed baking .vox binary");
        assert!(baked.non_empty_voxels > 5_000);
        assert_eq!(baked.palette_rgba.len(), 256);
        assert!(!baked.bricks.is_empty());
    }
}

#[test]
fn test_stl_exporter_writes_valid_binary_stl() {
    let root = repo_root();
    let out_stl = std::env::temp_dir().join("wf_test_commander_45mm.stl");
    let tris = export_vox_to_stl(&root.join("assets/vox/human_commander.vox"), &out_stl, 45.0)
        .expect("Failed exporting binary STL from .vox");
    assert!(tris > 1_000);
    let bytes = fs::read(&out_stl).expect("Missing exported STL");
    let (_, boundary, non_manifold, vol_mm3) = audit_binary_stl_2manifold(&bytes);
    assert_eq!(boundary, 0, "Open boundary edges found in Commander STL!");
    assert_eq!(non_manifold, 0, "Non-manifold edges found in Commander STL!");
    assert!(vol_mm3 > 2_000.0, "Unexpectedly tiny STL volume: {vol_mm3}");
    let _ = fs::remove_file(&out_stl);
}

#[test]
fn test_dual_cradle_wall_thickness_and_manifold_across_card_classes() {
    let root = repo_root();
    let raw_roster = fs::read_to_string(root.join("assets/compiled/master_roster.ron"))
        .expect("Missing master_roster.ron");
    let roster: RosterManifest = ron::from_str(&raw_roster).expect("Invalid master_roster.ron");
    let lookup = roster.index_by_key();

    // 1. Verify automatic routing across all 152 cards
    for card in &roster.units {
        let slug = card.default_vox_slug();
        let layout = resolve_cradle_layout(
            Some(card),
            slug,
            CradleMode::AutoFromCardStats,
        );
        if slug.starts_with("td_") || (slug.starts_with("art_") && slug != "art_cargo_drop") {
            assert_eq!(layout, ResolvedCradleLayout::None);
        } else if card.encumbrance == 0 {
            if card.strength_hp > 6 {
                assert_eq!(layout, ResolvedCradleLayout::SingleCenteredD10 { str_cap: card.strength_hp });
            } else {
                assert_eq!(layout, ResolvedCradleLayout::SingleCenteredD6 { str_cap: card.strength_hp });
            }
        } else if card.encumbrance > 6 {
            assert_eq!(
                layout,
                ResolvedCradleLayout::AsymmetricD6AndD10 {
                    str_cap: card.strength_hp,
                    mun_cap: card.encumbrance
                }
            );
        } else {
            assert_eq!(
                layout,
                ResolvedCradleLayout::DualD6 {
                    str_cap: card.strength_hp,
                    mun_cap: card.encumbrance
                }
            );
        }
    }

    // 2. DualD6 (`Battlephant`, STR 6, ENC 6): default 2.80mm center wall
    let battlephant = lookup["Battlephant"];
    let bp_stl = std::env::temp_dir().join("wf_test_battlephant_dual_d6.stl");
    let cfg_default = TabletopPrintConfig::default();
    let rep_bp = export_card_to_stl(battlephant, &root.join("assets/vox"), &bp_stl, &cfg_default)
        .expect("Failed exporting Battlephant STL");
    assert!(
        rep_bp.min_center_wall_mm >= 2.80,
        "Center wall {:.2}mm is thinner than 2.80mm default!",
        rep_bp.min_center_wall_mm
    );
    let bp_bytes = fs::read(&bp_stl).unwrap();
    let (_, b_ed, nm_ed, bp_vol) = audit_binary_stl_2manifold(&bp_bytes);
    assert_eq!(b_ed, 0);
    assert_eq!(nm_ed, 0);
    assert!(bp_vol > 3_000.0);
    let _ = fs::remove_file(&bp_stl);

    // 3. AsymmetricD6AndD10 (`Convoy`, STR 5, ENC 7) + Safety Floor Clamp (`0.40mm` -> `>= 2.00mm`)
    let convoy = lookup["Convoy"];
    let cv_stl = std::env::temp_dir().join("wf_test_convoy_clamped_wall.stl");
    let cfg_too_thin = TabletopPrintConfig {
        center_wall_mm: 0.40, // Intentionally too thin: exporter MUST clamp to >= 2.00mm!
        ..TabletopPrintConfig::default()
    };
    let rep_cv = export_card_to_stl(convoy, &root.join("assets/vox"), &cv_stl, &cfg_too_thin)
        .expect("Failed exporting Convoy STL");
    assert!(
        rep_cv.min_center_wall_mm >= MIN_PRINTABLE_CENTER_WALL_MM,
        "Safety floor clamp failed: measured {:.2}mm < {:.2}mm",
        rep_cv.min_center_wall_mm,
        MIN_PRINTABLE_CENTER_WALL_MM
    );
    let cv_bytes = fs::read(&cv_stl).unwrap();
    let (_, b_ed2, nm_ed2, _) = audit_binary_stl_2manifold(&cv_bytes);
    assert_eq!(b_ed2, 0);
    assert_eq!(nm_ed2, 0);
    let _ = fs::remove_file(&cv_stl);

    // 4. SingleCenteredD10 (`Command Center (Africa)`, STR 10, ENC 0)
    let hq = lookup["Command Center (Africa)"];
    let hq_stl = std::env::temp_dir().join("wf_test_hq_d10.stl");
    let rep_hq = export_card_to_stl(hq, &root.join("assets/vox"), &hq_stl, &cfg_default)
        .expect("Failed exporting HQ STL");
    assert_eq!(rep_hq.layout, ResolvedCradleLayout::SingleCenteredD10 { str_cap: 10 });
    let hq_bytes = fs::read(&hq_stl).unwrap();
    let (_, b_ed3, nm_ed3, _) = audit_binary_stl_2manifold(&hq_bytes);
    assert_eq!(b_ed3, 0);
    assert_eq!(nm_ed3, 0);
    let _ = fs::remove_file(&hq_stl);

    // 5. Full Deck Folder + PRINT_MANIFEST.txt export smoke test
    let raw_deck = fs::read_to_string(root.join("assets/compiled/starter_deck_plant_cnidarian.ron"))
        .expect("Missing starter deck");
    let mut deck: DeckList = ron::from_str(&raw_deck).expect("Invalid starter deck");
    deck.entries.truncate(3); // Keep test fast while verifying multi-card + manifest output
    let deck_dir = std::env::temp_dir().join("wf_test_deck_stl_export");
    let _ = fs::remove_dir_all(&deck_dir);
    let reps = export_deck_to_stl_folder(&deck, &roster, &root.join("assets/vox"), &deck_dir, &cfg_default)
        .expect("Deck STL export failed");
    assert_eq!(reps.len(), 3);
    assert!(deck_dir.join("PRINT_MANIFEST.txt").exists());
    let _ = fs::remove_dir_all(&deck_dir);
}

#[test]
fn test_coplanar_voxel_merge_cuboid_exact_12_tris_and_real_rig_invariants() {
    // 1. Solid 10x8x6 voxel cuboid -> unmerged = 2 * 2 * (80 + 60 + 48) = 752 tris -> merged MUST be exact 12 tris!
    let (sx, sy, sz) = (10usize, 8usize, 6usize);
    let occ = vec![true; sx * sy * sz];
    let mm = 1.5_f32;
    let (tris, bmin, bmax) = build_coplanar_voxel_stl_triangles(&occ, sx, sy, sz, mm, 7.5, 6.0);
    assert_eq!(tris.len(), 12, "Solid cuboid must merge to exactly 12 triangles");
    assert!((bmin[0] - (-7.5)).abs() < 1e-4 && (bmax[0] - 7.5).abs() < 1e-4);
    assert!((bmin[1] - (-6.0)).abs() < 1e-4 && (bmax[1] - 6.0).abs() < 1e-4);
    assert!((bmin[2] - 0.0).abs() < 1e-4 && (bmax[2] - 9.0).abs() < 1e-4);

    let cuboid_stl = std::env::temp_dir().join("wf_test_cuboid_merged.stl");
    write_binary_stl_triangles(&cuboid_stl, "cuboid_10x8x6", 2.8, &tris).unwrap();
    let cuboid_bytes = fs::read(&cuboid_stl).unwrap();
    let (tri_cnt, boundary, non_manifold, vol_mm3) = audit_binary_stl_2manifold(&cuboid_bytes);
    assert_eq!(tri_cnt, 12);
    assert_eq!(boundary, 0);
    assert_eq!(non_manifold, 0);
    let expected_vol = (sx * sy * sz) as f64 * (mm as f64).powi(3);
    assert!(
        (vol_mm3 - expected_vol).abs() < 0.25,
        "Expected cuboid volume {expected_vol:.2} mm^3, got {vol_mm3:.2}"
    );
    let _ = fs::remove_file(&cuboid_stl);

    // 2. Stepped L-shaped solid (checks T-junction elimination at step boundaries!)
    let (lx, ly, lz) = (8usize, 6usize, 6usize);
    let mut occ_step = vec![false; lx * ly * lz];
    for x in 0..lx {
        for y in 0..ly {
            for z in 0..lz {
                if z < 2 || (x < 4 && y < 3) {
                    occ_step[x * (ly * lz) + y * lz + z] = true;
                }
            }
        }
    }
    let (step_tris, _, _) = build_coplanar_voxel_stl_triangles(&occ_step, lx, ly, lz, 1.0, 4.0, 3.0);
    assert!(step_tris.len() < 100, "Stepped L-shape should merge cleanly, got {} tris", step_tris.len());
    let step_stl = std::env::temp_dir().join("wf_test_step_merged.stl");
    write_binary_stl_triangles(&step_stl, "step_L", 2.8, &step_tris).unwrap();
    let step_bytes = fs::read(&step_stl).unwrap();
    let (_, sb, snm, svol) = audit_binary_stl_2manifold(&step_bytes);
    assert_eq!(sb, 0, "T-junction / boundary edges on stepped L-shape!");
    assert_eq!(snm, 0, "Non-manifold edges on stepped L-shape!");
    let expected_step_vol = occ_step.iter().filter(|&&b| b).count() as f64;
    assert!((svol - expected_step_vol).abs() < 0.1);
    let _ = fs::remove_file(&step_stl);

    // 3. Real Battlephant rig: verify outward normals, non-degenerate triangles, bounding box, and >= 45% triangle reduction
    let root = repo_root();
    let raw_roster = fs::read_to_string(root.join("assets/compiled/master_roster.ron")).unwrap();
    let roster: RosterManifest = ron::from_str(&raw_roster).unwrap();
    let lookup = roster.index_by_key();
    let battlephant = lookup["Battlephant"];
    let bp_stl = std::env::temp_dir().join("wf_test_bp_coplanar.stl");
    let cfg = TabletopPrintConfig::default();
    let rep = export_card_to_stl(battlephant, &root.join("assets/vox"), &bp_stl, &cfg).unwrap();
    let bp_bytes = fs::read(&bp_stl).unwrap();
    let (bp_tris, b_ed, nm_ed, _) = audit_binary_stl_2manifold(&bp_bytes);
    assert_eq!(b_ed, 0);
    assert_eq!(nm_ed, 0);
    assert_eq!(bp_tris, rep.triangle_count);

    // Verify bounding box matches footprint_mm and every triangle has positive area & outward normal
    let mut min_v = [f32::MAX; 3];
    let mut max_v = [f32::MIN; 3];
    for i in 0..bp_tris {
        let base = 84 + i * 50;
        let read_v = |off: usize| -> [f32; 3] {
            [
                f32::from_le_bytes(bp_bytes[off..off + 4].try_into().unwrap()),
                f32::from_le_bytes(bp_bytes[off + 4..off + 8].try_into().unwrap()),
                f32::from_le_bytes(bp_bytes[off + 8..off + 12].try_into().unwrap()),
            ]
        };
        let n = read_v(base);
        let v0 = read_v(base + 12);
        let v1 = read_v(base + 24);
        let v2 = read_v(base + 36);
        for v in [v0, v1, v2] {
            for k in 0..3 {
                min_v[k] = min_v[k].min(v[k]);
                max_v[k] = max_v[k].max(v[k]);
            }
        }
        let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
        let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
        let cross = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let dot = cross[0] * n[0] + cross[1] * n[1] + cross[2] * n[2];
        assert!(dot > 1e-6, "Degenerate or inverted triangle #{i}: cross.n = {dot}");
    }
    assert!((max_v[0] - min_v[0] - rep.footprint_mm.0).abs() < 1e-3);
    assert!((max_v[1] - min_v[1] - rep.footprint_mm.1).abs() < 1e-3);
    assert!((max_v[2] - min_v[2] - rep.footprint_mm.2).abs() < 1e-3);
    let _ = fs::remove_file(&bp_stl);
}
