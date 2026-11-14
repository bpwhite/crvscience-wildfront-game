use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use hexx::Hex;
use wf_core::mapgen::SplitMix64;
use wf_core::{
    compile_multi_asset_64tree, generate_scenario, BakedVoxelAsset, CombatClass, GpuNode64,
    MultiAssetGpuBuffers, OrganismFaction, OriginAllegiance, RosterManifest, ScenarioId,
};
use wf_sim::tournament::play_headless_match_to_end;
use wf_sim::{build_tournament_deck, run_102_round_robin, AiDifficulty, HeadlessMatch};

static SUMMARY_LOCK: Mutex<()> = Mutex::new(());

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("Failed resolving crvscience-wildfront-game root")
}

fn validation_dir() -> PathBuf {
    let dir = repo_root().join("validation");
    fs::create_dir_all(&dir).expect("Failed creating validation directory");
    dir
}

fn find_app_preprints_releases() -> Option<PathBuf> {
    let root = repo_root();
    for anc in root.ancestors() {
        let cand = anc.join("app-preprints").join("releases");
        if cand.exists() {
            return Some(cand);
        }
    }
    None
}

fn load_master_roster() -> RosterManifest {
    let raw = fs::read_to_string(repo_root().join("assets/compiled/master_roster.ron"))
        .expect("Missing assets/compiled/master_roster.ron");
    ron::from_str(&raw).expect("Invalid master_roster.ron")
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Persists `wildfront_validation_summary.json` incrementally per test (`t1`, `t2`, `t3`)
/// using a small `.sections/` scratch map converted deterministically into JSON without adding dependencies.
fn merge_summary_section(section_key: &str, json_fragment: &str) {
    let _guard = SUMMARY_LOCK.lock().expect("Summary lock poisoned");
    let vdir = validation_dir();
    let sections_dir = std::env::temp_dir().join("crvscience_wildfront_validation_sections");
    fs::create_dir_all(&sections_dir).expect("Failed creating sections scratch dir");
    fs::write(sections_dir.join(format!("{section_key}.json")), json_fragment.trim())
        .expect("Failed writing summary fragment");

    let mut ordered_keys: Vec<String> = Vec::new();
    for k in [
        "t1_hex_determinism_replay",
        "t2_deck_audit_and_round_robin",
        "t3_svo_64tree_raymarching",
    ] {
        if sections_dir.join(format!("{k}.json")).exists() {
            ordered_keys.push(k.to_string());
        }
    }

    let mut body = String::from("{\n");
    body.push_str("  \"repository\": \"crvscience-wildfront-game\",\n");
    body.push_str(&format!(
        "  \"version\": {},\n",
        json_escape(env!("CARGO_PKG_VERSION"))
    ));
    body.push_str("  \"slug\": \"wildfront-hexagonal-ecological-tactics\",\n");
    body.push_str("  \"preprint_id\": \"APP-PREPRINT-12\",\n");
    body.push_str("  \"concept_doi\": \"10.5281/zenodo.24222396\",\n");
    body.push_str("  \"preprint_url\": \"https://crvscience.com/apps/preprints/wildfront-hexagonal-ecological-tactics\"");
    for k in ordered_keys {
        let frag = fs::read_to_string(sections_dir.join(format!("{k}.json")))
            .expect("Missing section fragment");
        body.push_str(",\n  ");
        body.push_str(&json_escape(&k));
        body.push_str(": ");
        body.push_str(frag.trim());
    }
    body.push_str("\n}\n");

    assert!(
        body.len() < 384 * 1024,
        "wildfront_validation_summary.json exceeded 384 KB limit: {} bytes",
        body.len()
    );

    let local_path = vdir.join("wildfront_validation_summary.json");
    fs::write(&local_path, &body).expect("Failed writing wildfront_validation_summary.json");

    if let Some(rel_dir) = find_app_preprints_releases() {
        let mirror = rel_dir.join("wildfront-hexagonal-ecological-tactics-validation-telemetry.json");
        fs::write(&mirror, &body).expect("Failed mirroring Wildfront telemetry JSON");
    }
}

#[test]
fn t1_deterministic_hex_skirmish_replay_and_terrain_invariants() {
    let roster = load_master_roster();

    // 1. Axial hex geometry symmetry, neighbor metric, and terrain movement rule audit across all ScenarioId maps
    let mut symmetry_checks = 0usize;
    let mut scenario_json_rows = Vec::new();
    let mut cliff_blocks_observed = 0usize;
    let mut water_blocks_terrestrial = 0usize;
    let mut flyer_bypasses_observed = 0usize;
    let mut aquatic_water_passes = 0usize;

    for (s_idx, &scen_id) in ScenarioId::ALL.iter().enumerate() {
        let scen = generate_scenario(scen_id, 0x2072_0001 + s_idx as u64);
        let map = &scen.map;

        let mut coords: Vec<Hex> = map.tiles.keys().copied().collect();
        coords.sort_by_key(|h| (h.x, h.y));

        // Sample pairwise axial distance symmetry d(a,b) == d(b,a) & triangle inequality d(a,c) <= d(a,b)+d(b,c)
        let sample_n = coords.len().min(32);
        for i in 0..sample_n {
            let a = coords[(i * 7) % coords.len()];
            let b = coords[(i * 13 + 3) % coords.len()];
            let c = coords[(i * 19 + 5) % coords.len()];
            let dab = a.unsigned_distance_to(b);
            let dba = b.unsigned_distance_to(a);
            let dac = a.unsigned_distance_to(c);
            let dbc = b.unsigned_distance_to(c);
            assert_eq!(dab, dba, "Axial hex distance must be symmetric");
            assert!(dac <= dab + dbc, "Triangle inequality violated on axial hex map");
            assert_eq!(a.unsigned_distance_to(a), 0);
            for nb in a.all_neighbors() {
                assert_eq!(a.unsigned_distance_to(nb), 1);
            }
            symmetry_checks += 1;
        }

        // Verify movement modifier semantics across ground, flyer, and pure aquatic units
        let mut cover_tiles = 0usize;
        let mut water_tiles = 0usize;
        let mut cliff_edges = 0usize;
        for &from in &coords {
            let tile = &map.tiles[&from];
            if tile.morphology.grants_ranged_cover() {
                cover_tiles += 1;
            }
            if tile.morphology.is_water() {
                water_tiles += 1;
            }
            for (edge_idx, nb) in from.all_neighbors().into_iter().enumerate() {
                if let Some(dst_tile) = map.tiles.get(&nb) {
                    let has_cliff =
                        tile.has_cliff_edge(edge_idx) || tile.elevation.abs_diff(dst_tile.elevation) >= 2;
                    if has_cliff {
                        cliff_edges += 1;
                        let ground_ok = map.can_traverse(from, nb, false, false, 0);
                        assert!(!ground_ok, "Ground unit must not cross cliff without bypass");
                        cliff_blocks_observed += 1;
                        if !matches!(dst_tile.morphology, wf_core::Morphology::MountainPeak) {
                            assert!(map.can_traverse(from, nb, true, false, 0));
                            flyer_bypasses_observed += 1;
                        }
                    }
                    if dst_tile.morphology.is_water() && !has_cliff {
                        if !map.can_traverse(from, nb, false, false, 0) {
                            water_blocks_terrestrial += 1;
                        }
                        if map.can_traverse(from, nb, false, false, 2) {
                            aquatic_water_passes += 1;
                        }
                    }
                }
            }
        }

        scenario_json_rows.push(format!(
            "      {{\"scenario\": {}, \"tiles\": {}, \"cover_tiles\": {}, \"water_tiles\": {}, \"directed_cliff_edges\": {}}}",
            json_escape(scen_id.title()),
            coords.len(),
            cover_tiles,
            water_tiles,
            cliff_edges
        ));
    }

    assert!(symmetry_checks >= ScenarioId::ALL.len() * 16);
    assert!(water_blocks_terrestrial > 0, "Scenario suite must test aquatic boundary rules");

    // 2. Deterministic seeded replay across twin runs verifying identical final hash and step hashes
    let pairs = [
        (
            ScenarioId::KingOfTheControlSpire,
            0xBEEF_2072_0001u64,
            (OriginAllegiance::Australia, OrganismFaction::Mammal, OrganismFaction::Bird),
            (OriginAllegiance::Europe, OrganismFaction::Reptile, OrganismFaction::Insect),
        ),
        (
            ScenarioId::BioRiverConvoy,
            0xBEEF_2072_0002u64,
            (OriginAllegiance::SouthAmerica, OrganismFaction::Fish, OrganismFaction::Amphibian),
            (OriginAllegiance::Asia, OrganismFaction::Plant, OrganismFaction::Cnidarian),
        ),
        (
            ScenarioId::BunkerBreakout,
            0xBEEF_2072_0003u64,
            (OriginAllegiance::NorthAmerica, OrganismFaction::Cnidarian, OrganismFaction::Arachnid),
            (OriginAllegiance::Africa, OrganismFaction::Mammal, OrganismFaction::Reptile),
        ),
    ];

    let mut replay_rows = Vec::new();
    let mut total_verified_actions = 0usize;

    for (scen, seed, (o1, p1f, s1f), (o2, p2f, s2f)) in pairs {
        let d1 = build_tournament_deck(&roster, o1, p1f, s1f);
        let d2 = build_tournament_deck(&roster, o2, p2f, s2f);
        assert!(d1.validate_against(&roster).is_legal);
        assert!(d2.validate_against(&roster).is_legal);

        let run_a = play_headless_match_to_end(
            &roster,
            d1.clone(),
            d2.clone(),
            scen,
            seed,
            AiDifficulty::Veteran,
            45,
            true,
        );
        let run_b = play_headless_match_to_end(
            &roster,
            d1,
            d2,
            scen,
            seed,
            AiDifficulty::Veteran,
            45,
            true,
        );

        let hash_a = run_a.compute_state_hash();
        let hash_b = run_b.compute_state_hash();
        assert_eq!(hash_a, hash_b, "Final state hash mismatch across repeated deterministic runs");
        assert_eq!(run_a.turn_number, run_b.turn_number);
        assert_eq!(run_a.winner_team, run_b.winner_team);

        let replay_a = run_a.export_replay();
        let replay_b = run_b.export_replay();
        assert_eq!(replay_a.steps.len(), replay_b.steps.len());
        for (sa, sb) in replay_a.steps.iter().zip(replay_b.steps.iter()) {
            assert_eq!(sa.post_state_hash, sb.post_state_hash, "Per-step hash divergence");
        }
        HeadlessMatch::verify_replay_data(roster.clone(), &replay_a)
            .expect("Replay hash chain verification failed");

        total_verified_actions += replay_a.steps.len();
        replay_rows.push(format!(
            "      {{\"scenario\": {}, \"seed\": {}, \"winner_team\": {}, \"turns\": {}, \"recorded_steps\": {}, \"final_state_hash_hex\": {}}}",
            json_escape(scen.title()),
            seed,
            run_a.winner_team.unwrap_or(0),
            run_a.turn_number,
            replay_a.steps.len(),
            json_escape(&format!("0x{:016x}", hash_a))
        ));
    }

    let frag = format!(
        "{{\n\
    \"axial_symmetry_checks\": {symmetry_checks},\n\
    \"cliff_blocks_observed\": {cliff_blocks_observed},\n\
    \"water_blocks_terrestrial\": {water_blocks_terrestrial},\n\
    \"flyer_bypasses_observed\": {flyer_bypasses_observed},\n\
    \"aquatic_water_passes\": {aquatic_water_passes},\n\
    \"deterministic_replays_verified\": {},\n\
    \"total_verified_action_steps\": {total_verified_actions},\n\
    \"scenarios\": [\n{}\n    ],\n\
    \"replay_runs\": [\n{}\n    ]\n\
  }}",
        replay_rows.len(),
        scenario_json_rows.join(",\n"),
        replay_rows.join(",\n")
    );
    merge_summary_section("t1_hex_determinism_replay", &frag);
}

#[test]
fn t2_deck_cost_utility_audit_and_faction_balance_matrix() {
    let roster = load_master_roster();
    roster.validate().expect("Master roster validation failed");

    // 1. Audit full card roster vs deployable combat unit subset
    let total_cards = roster.units.len();
    let mut class_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut faction_card_stats: BTreeMap<OrganismFaction, (usize, f64, f64, f64, u16)> =
        BTreeMap::new();

    let mut bio_combat_units = 0usize;
    let mut support_infra_cards = 0usize;
    let mut tactical_and_artifact_cards = 0usize;

    let mut pts_vec = Vec::new();
    let mut weighted_vec = Vec::new();
    let mut ehp_vec = Vec::new();

    for card in &roster.units {
        *class_counts
            .entry(format!("{:?}", card.combat_class))
            .or_insert(0) += 1;

        match card.combat_class {
            CombatClass::TacticalDeployment | CombatClass::Artifact => {
                tactical_and_artifact_cards += 1;
            }
            cls if cls.is_building() => {
                support_infra_cards += 1;
            }
            _ => {
                bio_combat_units += 1;
            }
        }

        let weighted = card.computed_weighted_points();
        let diff = card.deck_points.abs_diff(weighted);
        let ehp = f64::from(card.combat_ehp_tenths) / 10.0;
        let pts = f64::from(card.deck_points);
        let wp = f64::from(weighted);

        pts_vec.push(pts);
        weighted_vec.push(wp);
        ehp_vec.push(ehp);

        let entry = faction_card_stats
            .entry(card.faction)
            .or_insert((0, 0.0, 0.0, 0.0, 0));
        entry.0 += 1;
        entry.1 += pts;
        entry.2 += wp;
        entry.3 += ehp;
        if diff > entry.4 {
            entry.4 = diff;
        }
    }

    let n_f = pts_vec.len() as f64;
    let mean_pts = pts_vec.iter().sum::<f64>() / n_f;
    let mean_wp = weighted_vec.iter().sum::<f64>() / n_f;
    let mut cov = 0.0;
    let mut var_p = 0.0;
    let mut var_w = 0.0;
    for (&p, &w) in pts_vec.iter().zip(weighted_vec.iter()) {
        cov += (p - mean_pts) * (w - mean_wp);
        var_p += (p - mean_pts).powi(2);
        var_w += (w - mean_wp).powi(2);
    }
    let cost_utility_corr = cov / (var_p * var_w).sqrt().max(1e-9);
    assert!(
        cost_utility_corr > 0.90,
        "Deck points vs weighted stat utility correlation too low: {cost_utility_corr:.4}"
    );

    // 2. Run real 102-match round-robin tournament at Veteran difficulty
    let t0 = Instant::now();
    let report = run_102_round_robin(&roster, AiDifficulty::Veteran, 45);
    let wall_ms = t0.elapsed().as_secs_f64() * 1000.0;

    assert_eq!(report.total_matches, 102, "Expected 102 round-robin matches");
    assert!(report.decisive_matches >= 80, "Insufficient decisive matches: {}", report.decisive_matches);

    // 3. Build head-to-head Bio-Type matrix & write validation/faction_balance_matrix.csv
    let mut csv = String::from(
        "record_type,entity_or_p1,category_or_p2,games_or_count,wins_or_sum_pts,win_rate_or_mean_wp,avg_turns_or_mean_ehp,notes\n",
    );

    for (fac, (cnt, sum_pts, sum_wp, sum_ehp, max_diff)) in &faction_card_stats {
        let cf = *cnt as f64;
        csv.push_str(&format!(
            "DECK_AUDIT,{:?},OrganismFaction,{},{:.0},{:.2},{:.2},max_point_delta_{}\n",
            fac,
            cnt,
            sum_pts,
            sum_wp / cf,
            sum_ehp / cf,
            max_diff
        ));
    }

    let mut standing_rows = Vec::new();
    for st in &report.standings {
        csv.push_str(&format!(
            "STANDING,{},{},{},{},{:.2},{:.2},win_rate_pct\n",
            st.name, st.category, st.games, st.wins, st.win_rate_pct, st.avg_turns
        ));
        standing_rows.push(format!(
            "      {{\"name\": {}, \"category\": {}, \"games\": {}, \"wins\": {}, \"win_rate_pct\": {:.2}, \"avg_turns\": {:.2}}}",
            json_escape(&st.name),
            json_escape(&st.category),
            st.games,
            st.wins,
            st.win_rate_pct,
            st.avg_turns
        ));
    }

    for m in &report.matches {
        csv.push_str(&format!(
            "MATCH,{},{},{},{},{},{},{}\n",
            m.p1_label,
            m.p2_label,
            m.match_index,
            m.winner_team,
            if m.winner_team == 1 { 100.0 } else { 0.0 },
            m.turns_played,
            m.scenario.title()
        ));
    }

    assert!(
        csv.len() < 384 * 1024,
        "faction_balance_matrix.csv exceeded 384 KB limit: {} bytes",
        csv.len()
    );
    fs::write(validation_dir().join("faction_balance_matrix.csv"), &csv)
        .expect("Failed writing faction_balance_matrix.csv");

    let class_breakdown: Vec<String> = class_counts
        .iter()
        .map(|(k, v)| format!("{}: {}", json_escape(k), v))
        .collect();

    let frag = format!(
        "{{\n\
    \"total_roster_cards\": {total_cards},\n\
    \"bio_combat_unit_cards\": {bio_combat_units},\n\
    \"support_installation_cards\": {support_infra_cards},\n\
    \"tactical_and_artifact_cards\": {tactical_and_artifact_cards},\n\
    \"unit_plus_infra_cards\": {},\n\
    \"cost_vs_weighted_utility_r\": {cost_utility_corr:.6},\n\
    \"mean_deck_points\": {mean_pts:.2},\n\
    \"mean_weighted_utility_points\": {mean_wp:.2},\n\
    \"tournament_ai_difficulty\": {},\n\
    \"total_matches\": {},\n\
    \"decisive_matches\": {},\n\
    \"p1_wins\": {},\n\
    \"p2_wins\": {},\n\
    \"median_turns\": {:.2},\n\
    \"tournament_wall_ms\": {:.2},\n\
    \"csv_bytes\": {},\n\
    \"combat_class_counts\": {{ {} }},\n\
    \"standings\": [\n{}\n    ]\n\
  }}",
        bio_combat_units + support_infra_cards,
        json_escape(&report.ai_difficulty),
        report.total_matches,
        report.decisive_matches,
        report.p1_wins,
        report.p2_wins,
        report.median_turns,
        wall_ms,
        csv.len(),
        class_breakdown.join(", "),
        standing_rows.join(",\n")
    );
    merge_summary_section("t2_deck_audit_and_round_robin", &frag);
}

#[inline]
fn has_bit64(lo: u32, hi: u32, bit: u32) -> bool {
    if bit < 32 {
        (lo & (1u32 << bit)) != 0
    } else {
        (hi & (1u32 << (bit - 32))) != 0
    }
}

#[inline]
fn popcnt_below64(lo: u32, hi: u32, bit: u32) -> u32 {
    if bit == 0 {
        return 0;
    }
    if bit < 32 {
        (lo & ((1u32 << bit) - 1)).count_ones()
    } else {
        lo.count_ones() + (hi & ((1u32 << (bit - 32)) - 1)).count_ones()
    }
}

fn sample_64tree_cpu(gpu: &MultiAssetGpuBuffers, root_idx: u32, x: i32, y: i32, z: i32) -> u8 {
    if x < 0 || y < 0 || z < 0 || x >= 256 || y >= 256 || z >= 256 {
        return 0;
    }
    let (ux, uy, uz) = (x as u32, y as u32, z as u32);
    let root: &GpuNode64 = &gpu.nodes[root_idx as usize];

    // Level 2: 64^3 quadrants
    let b2 = ((ux >> 6) & 3) + (((uy >> 6) & 3) * 4) + (((uz >> 6) & 3) * 16);
    if !has_bit64(root.mask_lo, root.mask_hi, b2) {
        return 0;
    }
    let l1_idx = root.child_base_ptr + popcnt_below64(root.mask_lo, root.mask_hi, b2);
    let l1 = &gpu.nodes[l1_idx as usize];

    // Level 1: 16^3 sub-blocks
    let b1 = ((ux >> 4) & 3) + (((uy >> 4) & 3) * 4) + (((uz >> 4) & 3) * 16);
    if !has_bit64(l1.mask_lo, l1.mask_hi, b1) {
        return 0;
    }
    let twig_idx = l1.child_base_ptr + popcnt_below64(l1.mask_lo, l1.mask_hi, b1);
    let twig = &gpu.nodes[twig_idx as usize];

    // Level 0 (Twig): 4^3 bricks
    let b0 = ((ux >> 2) & 3) + (((uy >> 2) & 3) * 4) + (((uz >> 2) & 3) * 16);
    if !has_bit64(twig.mask_lo, twig.mask_hi, b0) {
        return 0;
    }
    let brick_idx = twig.child_base_ptr + popcnt_below64(twig.mask_lo, twig.mask_hi, b0);
    let brick = &gpu.bricks[brick_idx as usize];

    // Leaf Voxel (1^3)
    let bv = (ux & 3) + ((uy & 3) * 4) + ((uz & 3) * 16);
    if !has_bit64(brick.mask_lo, brick.mask_hi, bv) {
        return 0;
    }
    let word = brick.packed_mats[(bv >> 2) as usize];
    ((word >> ((bv & 3) * 8)) & 255) as u8
}

fn intersect_aabb(ro: [f32; 3], inv_rd: [f32; 3], bmax: [f32; 3]) -> Option<(f32, f32)> {
    let mut tmin = 0.0_f32;
    let mut tmax = f32::INFINITY;
    for a in 0..3 {
        let t0 = (0.0 - ro[a]) * inv_rd[a];
        let t1 = (bmax[a] - ro[a]) * inv_rd[a];
        let (ts, tb) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
        tmin = tmin.max(ts);
        tmax = tmax.min(tb);
    }
    if tmin <= tmax {
        Some((tmin, tmax))
    } else {
        None
    }
}

fn raymarch_64tree_dda(
    gpu: &MultiAssetGpuBuffers,
    root_idx: u32,
    size_voxels: [u32; 3],
    ro: [f32; 3],
    rd: [f32; 3],
) -> Option<(f32, u8)> {
    let norm = (rd[0] * rd[0] + rd[1] * rd[1] + rd[2] * rd[2]).sqrt().max(1e-6);
    let rdn = [rd[0] / norm, rd[1] / norm, rd[2] / norm];
    let inv_rd = [
        1.0 / if rdn[0].abs() < 1e-6 { 1e-6 } else { rdn[0] },
        1.0 / if rdn[1].abs() < 1e-6 { 1e-6 } else { rdn[1] },
        1.0 / if rdn[2].abs() < 1e-6 { 1e-6 } else { rdn[2] },
    ];
    let bmax = [
        size_voxels[0] as f32,
        size_voxels[1] as f32,
        size_voxels[2] as f32,
    ];
    let (tmin, tmax) = intersect_aabb(ro, inv_rd, bmax)?;

    let t_start = (tmin - 0.002).max(0.0);
    let mut px = [
        (ro[0] + rdn[0] * (t_start + 0.005)).clamp(0.001, bmax[0] - 0.001),
        (ro[1] + rdn[1] * (t_start + 0.005)).clamp(0.001, bmax[1] - 0.001),
        (ro[2] + rdn[2] * (t_start + 0.005)).clamp(0.001, bmax[2] - 0.001),
    ];
    let mut map_pos = [
        px[0].floor() as i32,
        px[1].floor() as i32,
        px[2].floor() as i32,
    ];
    let step_dir = [
        if rdn[0] >= 0.0 { 1.0_f32 } else { -1.0_f32 },
        if rdn[1] >= 0.0 { 1.0_f32 } else { -1.0_f32 },
        if rdn[2] >= 0.0 { 1.0_f32 } else { -1.0_f32 },
    ];
    let step_i = [step_dir[0] as i32, step_dir[1] as i32, step_dir[2] as i32];
    let delta_dist = [inv_rd[0].abs(), inv_rd[1].abs(), inv_rd[2].abs()];
    let mut side_dist = [
        (step_dir[0] * (map_pos[0] as f32 - px[0]) + step_dir[0] * 0.5 + 0.5) * delta_dist[0],
        (step_dir[1] * (map_pos[1] as f32 - px[1]) + step_dir[1] * 0.5 + 0.5) * delta_dist[1],
        (step_dir[2] * (map_pos[2] as f32 - px[2]) + step_dir[2] * 0.5 + 0.5) * delta_dist[2],
    ];
    let _ = &mut px;

    for _ in 0..680 {
        if map_pos[0] < 0
            || map_pos[1] < 0
            || map_pos[2] < 0
            || map_pos[0] >= size_voxels[0] as i32
            || map_pos[1] >= size_voxels[1] as i32
            || map_pos[2] >= size_voxels[2] as i32
        {
            break;
        }
        let mat = sample_64tree_cpu(gpu, root_idx, map_pos[0], map_pos[1], map_pos[2]);
        if mat != 0 {
            let hit_t = (side_dist[0] - delta_dist[0])
                .min(side_dist[1] - delta_dist[1])
                .min(side_dist[2] - delta_dist[2]);
            let final_t = tmin.max(hit_t).min(tmax);
            return Some((final_t, mat));
        }

        if side_dist[0] <= side_dist[1] && side_dist[0] <= side_dist[2] {
            side_dist[0] += delta_dist[0];
            map_pos[0] += step_i[0];
        } else if side_dist[1] <= side_dist[2] {
            side_dist[1] += delta_dist[1];
            map_pos[1] += step_i[1];
        } else {
            side_dist[2] += delta_dist[2];
            map_pos[2] += step_i[2];
        }
    }

    None
}

#[test]
fn t3_sparse_64tree_svo_raymarching_invariant_and_throughput() {
    let root = repo_root();
    let vox_dir = root.join("assets/compiled/vox");
    let mut assets = Vec::new();
    let mut total_non_empty_voxels = 0usize;
    let mut dense_bounding_voxels = 0u64;

    let mut entries: Vec<PathBuf> = fs::read_dir(&vox_dir)
        .expect("Missing assets/compiled/vox")
        .filter_map(|e| e.ok().map(|x| x.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ron"))
        .collect();
    entries.sort();

    for path in entries {
        let raw = fs::read_to_string(&path).expect("Read compiled vox RON");
        let parsed: BakedVoxelAsset =
            ron::from_str(&raw).expect("Parse BakedVoxelAsset RON");
        total_non_empty_voxels += parsed.non_empty_voxels;
        dense_bounding_voxels += u64::from(parsed.dimensions.0)
            * u64::from(parsed.dimensions.1)
            * u64::from(parsed.dimensions.2);
        assets.push(parsed);
    }

    assert_eq!(assets.len(), 170, "Expected 170 compiled voxel rigs");
    let gpu = compile_multi_asset_64tree(&assets);
    assert_eq!(gpu.handles.len(), 170);

    // 1. Verify exact voxel parity & outside-AABB miss invariant on representative rigs
    let mut exterior_miss_rays = 0usize;
    let mut aabb_miss_correct = 0usize;
    let mut interior_rays_cast = 0usize;
    let mut interior_voxel_hits = 0usize;

    for (idx, asset) in assets.iter().enumerate().take(24) {
        let handle = gpu.handles.get(&asset.model_name).expect("Missing handle");

        // Verify sampled palette indices from sampled bricks match raw Y-up remapping (vx, vz, vy)
        if let Some(br) = asset.bricks.first() {
            let (bx, by, bz) = br.brick_pos;
            for lz in 0..4usize {
                for yx in 0..16usize {
                    let mat = br.voxels_z_yx[lz][yx];
                    if mat != 0 {
                        let lx = yx & 3;
                        let ly = yx >> 2;
                        let vx = i32::from(bx) * 4 + lx as i32;
                        let vy_src = i32::from(by) * 4 + ly as i32;
                        let vz_src = i32::from(bz) * 4 + lz as i32;
                        let sampled = sample_64tree_cpu(
                            &gpu,
                            handle.root_node_index,
                            vx,
                            vz_src,
                            vy_src,
                        );
                        assert_eq!(sampled, mat, "64-tree voxel lookup mismatch on {}", asset.model_name);
                        break;
                    }
                }
            }
        }

        // Exterior rays aimed past bounding box must always miss
        let sz = [
            handle.size_voxels[0] as f32,
            handle.size_voxels[1] as f32,
            handle.size_voxels[2] as f32,
        ];
        for side in [-15.0_f32, sz[0] + 15.0] {
            exterior_miss_rays += 1;
            let hit = raymarch_64tree_dda(
                &gpu,
                handle.root_node_index,
                handle.size_voxels,
                [side, sz[1] * 0.5, -20.0],
                [0.0, 0.0, 1.0],
            );
            assert!(hit.is_none(), "Ray strictly outside AABB must never hit");
            aabb_miss_correct += 1;
        }

        // Grid of targeted camera rays through bounding box center must score solid voxel hits
        let steps = 8;
        for gy in 1..steps {
            for gx in 1..steps {
                let u = (gx as f32) / (steps as f32);
                let v = (gy as f32) / (steps as f32);
                let target = [sz[0] * u, sz[1] * v, sz[2] * 0.5];
                let ro = [
                    sz[0] * 0.5 + ((idx % 3) as f32 - 1.0) * 18.0,
                    sz[1] * 0.65,
                    -sz[2].max(16.0) - 8.0,
                ];
                let rd = [target[0] - ro[0], target[1] - ro[1], target[2] - ro[2]];
                interior_rays_cast += 1;
                if let Some((_t, mat)) =
                    raymarch_64tree_dda(&gpu, handle.root_node_index, handle.size_voxels, ro, rd)
                {
                    assert!(mat > 0);
                    interior_voxel_hits += 1;
                }
            }
        }
    }

    assert!(
        interior_voxel_hits > interior_rays_cast / 10,
        "Expected substantial voxel hits through rig AABBs ({interior_voxel_hits}/{interior_rays_cast})"
    );

    // 2. Raymarching CPU DDA throughput benchmark across 16 complex compiled rigs
    let mut rng = SplitMix64::new(0x570_64_73EE);
    let mut bench_handles: Vec<(f32, _)> = gpu
        .handles
        .values()
        .copied()
        .map(|h| {
            let vol = (h.size_voxels[0] as f32) * (h.size_voxels[1] as f32) * (h.size_voxels[2] as f32);
            (vol, h)
        })
        .collect();
    bench_handles.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let top_rigs: Vec<_> = bench_handles.into_iter().take(16).map(|(_, h)| h).collect();

    let n_bench_rays = 24_000usize;
    let mut bench_hits = 0usize;
    let t0 = Instant::now();
    for i in 0..n_bench_rays {
        let h = &top_rigs[i % top_rigs.len()];
        let sx = h.size_voxels[0] as f32;
        let sy = h.size_voxels[1] as f32;
        let sz = h.size_voxels[2] as f32;
        let u = (rng.next_u64() % 1000) as f32 / 1000.0;
        let v = (rng.next_u64() % 1000) as f32 / 1000.0;
        let ro = [sx * 0.5, sy * 0.7, -sz.max(16.0) - 6.0];
        let tgt = [sx * (0.15 + 0.7 * u), sy * (0.1 + 0.8 * v), sz * 0.5];
        let rd = [tgt[0] - ro[0], tgt[1] - ro[1], tgt[2] - ro[2]];
        if raymarch_64tree_dda(&gpu, h.root_node_index, h.size_voxels, ro, rd).is_some() {
            bench_hits += 1;
        }
    }
    let elapsed_s = t0.elapsed().as_secs_f64().max(1e-6);
    let rays_per_sec = (n_bench_rays as f64) / elapsed_s;
    let svo_bytes = gpu.nodes.len() * std::mem::size_of::<GpuNode64>()
        + gpu.bricks.len() * std::mem::size_of::<wf_core::GpuLeafBrick>();
    let compression_ratio = (dense_bounding_voxels as f64) / (svo_bytes.max(1) as f64);

    let frag = format!(
        "{{\n\
    \"compiled_vox_rigs\": {},\n\
    \"total_nodes\": {},\n\
    \"total_leaf_bricks\": {},\n\
    \"total_non_empty_voxels\": {total_non_empty_voxels},\n\
    \"dense_bounding_voxels\": {dense_bounding_voxels},\n\
    \"gpu_svo_buffer_bytes\": {svo_bytes},\n\
    \"dense_to_svo_byte_ratio\": {compression_ratio:.2},\n\
    \"exterior_aabb_miss_rays\": {exterior_miss_rays},\n\
    \"exterior_aabb_misses_verified\": {aabb_miss_correct},\n\
    \"interior_rays_cast\": {interior_rays_cast},\n\
    \"interior_voxel_hits\": {interior_voxel_hits},\n\
    \"benchmark_rays\": {n_bench_rays},\n\
    \"benchmark_hits\": {bench_hits},\n\
    \"benchmark_elapsed_ms\": {:.2},\n\
    \"cpu_debug_rays_per_sec\": {:.1}\n\
  }}",
        assets.len(),
        gpu.nodes.len(),
        gpu.bricks.len(),
        elapsed_s * 1000.0,
        rays_per_sec
    );
    merge_summary_section("t3_svo_64tree_raymarching", &frag);
}
