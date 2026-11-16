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

//! End-to-end integration tests for `wf_core` rules, combat math, logistics graphs,
//! procedural scenarios, deck validation, and Sparse 64-Tree GPU voxel compilation.

use std::fs;
use std::path::{Path, PathBuf};
use hexx::Hex;
use wf_core::abilities::{execute_field_heal, execute_venom_spit, execute_web_snare, AbilityCatalog};
use wf_core::combat::{required_ranged_hit_roll, resolve_attack, CombatContext, GraveyardTicket};
use wf_core::logistics::{
    begin_player_turn, compute_home_supply_coverage, is_within_control_node, BoardUnit, HexPos,
    PlayerEconomy, PlayerOwner,
};
use wf_core::{
    compile_multi_asset_64tree, generate_scenario, BakedVoxelAsset, CombatClass, DeckEntry,
    DeckList, GraveyardFacility, OrganismFaction, RosterManifest, ScenarioId,
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("Failed resolving workspace root")
}

fn sample_board_unit(
    key: &str,
    cls: CombatClass,
    str_hp: u8,
    armor: u8,
    evade: u8,
    leash: u8,
    emits_node: bool,
    node_rad: u8,
    supply_rng: u8,
) -> BoardUnit {
    BoardUnit {
        card_key: key.to_string(),
        faction: OrganismFaction::Mammal,
        combat_class: cls,
        graveyard_queue: GraveyardFacility::HospitalBio,
        current_str_hp: str_hp,
        max_str_hp: str_hp,
        current_armor: armor,
        base_armor: armor,
        evade,
        encumbrance_cap: 3,
        carried_munitions: 3,
        node_leash_range: leash,
        emits_node,
        node_emit_radius: node_rad,
        supply_range: supply_rng,
        is_laser: false,
        laser_fired_this_turn: false,
        confused: false,
        poisoned_stacks: 0,
        fortified_stacks: 0,
        rooted_turns: 0,
        stunned_this_turn: false,
        burrowed: false,
        target_locked: false,
        revealed_until_turn_start: false,
        temp_move_bonus: 0,
        temp_stealth: false,
        blinded_this_turn: false,
    }
}

#[test]
fn test_01_master_roster_152_cards_and_default_vox_slugs() {
    let root = repo_root();
    let raw = fs::read_to_string(root.join("assets/compiled/master_roster.ron"))
        .expect("Missing master_roster.ron");
    let roster: RosterManifest = ron::from_str(&raw).expect("Invalid master_roster.ron");
    assert_eq!(roster.units.len(), 152);
    roster.validate().expect("Master roster validation failed");

    for fac in OrganismFaction::ALL {
        assert!(
            roster.units.iter().any(|u| u.faction == fac),
            "Missing cards for faction {:?}",
            fac
        );
    }

    for card in &roster.units {
        let slug = card.default_vox_slug();
        let vox_file = root.join("assets/vox").join(format!("{slug}.vox"));
        let ron_file = root.join("assets/compiled/vox").join(format!("{slug}_vox.ron"));
        assert!(vox_file.exists(), "Missing .vox file for {} -> {}", card.card_key, vox_file.display());
        assert!(ron_file.exists(), "Missing _vox.ron file for {} -> {}", card.card_key, ron_file.display());
    }
}

#[test]
fn test_02_starter_decks_validate_under_250_pt_two_faction_rules() {
    let root = repo_root();
    let roster: RosterManifest =
        ron::from_str(&fs::read_to_string(root.join("assets/compiled/master_roster.ron")).unwrap())
            .unwrap();

    for deck_file in [
        "assets/compiled/starter_deck_plant_cnidarian.ron",
        "assets/compiled/starter_deck_mammal_reptile.ron",
    ] {
        let deck: DeckList =
            ron::from_str(&fs::read_to_string(root.join(deck_file)).unwrap()).unwrap();
        let rep = deck.validate_against(&roster);
        assert!(
            rep.is_legal,
            "Starter deck {} failed legality: {:?}",
            deck.deck_name, rep.violations
        );
        assert!(rep.points_used <= 250);
    }
}

#[test]
fn test_03_deck_validator_rejects_third_bio_faction_and_budget_overflow() {
    let root = repo_root();
    let roster: RosterManifest =
        ron::from_str(&fs::read_to_string(root.join("assets/compiled/master_roster.ron")).unwrap())
            .unwrap();

    let illegal_deck = DeckList {
        deck_name: "Illegal 3-Faction Overflow".into(),
        faction: wf_core::OriginAllegiance::Australia,
        primary_faction: OrganismFaction::Mammal,
        secondary_faction: OrganismFaction::Reptile,
        is_pure_biotype: false,
        point_budget: 250,
        entries: vec![
            DeckEntry { card_key: "Command Center (Australia)".into(), quantity: 1 },
            DeckEntry { card_key: "Supply Depot".into(), quantity: 1 },
            DeckEntry { card_key: "Mecharoo (Kanga Mech)".into(), quantity: 3 }, // >2 Heavy & high pts
            DeckEntry { card_key: "Jelly Zeppelin".into(), quantity: 2 },        // Illegal 3rd Cnidarian faction!
            DeckEntry { card_key: "Battlephant".into(), quantity: 2 },
        ],
    };
    let rep = illegal_deck.validate_against(&roster);
    assert!(!rep.is_legal);
    assert!(!rep.violations.is_empty());
}

#[test]
fn test_04_contiguous_supply_chain_bfs_and_severance() {
    let hq = sample_board_unit("Command Center", CombatClass::Installation, 10, 2, 0, 0, true, 2, 2);
    let depot = sample_board_unit("Supply Depot", CombatClass::SupplyDepot, 6, 1, 0, 0, false, 0, 3);
    let hosp = sample_board_unit("Hospital", CombatClass::SupportInstallation, 6, 1, 0, 0, false, 0, 2);

    let full_chain = vec![
        (PlayerOwner(1), HexPos(Hex::new(0, 0)), &hq),
        (PlayerOwner(1), HexPos(Hex::new(1, 0)), &depot),
        (PlayerOwner(1), HexPos(Hex::new(2, 0)), &hosp),
    ];
    let cov_intact = compute_home_supply_coverage(1, &full_chain);
    assert!(cov_intact.contains(&Hex::new(4, 0)), "Contiguous chain should reach (4, 0)");

    // Sever middle Supply Depot at (1, 0) -> Hospital at (2, 0) is no longer touching HQ!
    let mut dead_depot = depot.clone();
    dead_depot.current_str_hp = 0;
    let severed_chain = vec![
        (PlayerOwner(1), HexPos(Hex::new(0, 0)), &hq),
        (PlayerOwner(1), HexPos(Hex::new(1, 0)), &dead_depot),
        (PlayerOwner(1), HexPos(Hex::new(2, 0)), &hosp),
    ];
    let cov_severed = compute_home_supply_coverage(1, &severed_chain);
    assert!(!cov_severed.contains(&Hex::new(4, 0)), "Severed Hospital must not project Home Supply");
}

#[test]
fn test_05_psychic_control_node_leash_and_autonomous_units() {
    let node = sample_board_unit("Control Node", CombatClass::ControlNode, 5, 1, 0, 0, true, 3, 1);
    let refs = vec![(PlayerOwner(1), HexPos(Hex::new(0, 0)), &node)];

    assert!(is_within_control_node(1, 2, Hex::new(3, 0), &refs));
    assert!(!is_within_control_node(1, 2, Hex::new(4, 0), &refs));
    // Autonomous (`leash = 0`) operates anywhere regardless of nodes
    assert!(is_within_control_node(1, 0, Hex::new(9, -4), &refs));
}

#[test]
fn test_06_action_economy_scales_with_static_control_nodes_and_poisons_tick() {
    let mut econ = PlayerEconomy::default();
    let mut n1 = sample_board_unit("Control Node", CombatClass::ControlNode, 5, 1, 0, 0, true, 3, 1);
    let mut n2 = sample_board_unit("Control Node", CombatClass::ControlNode, 5, 1, 0, 0, true, 3, 1);
    let mut koala = sample_board_unit("Combat Koala", CombatClass::Medium, 5, 2, 1, 2, false, 0, 0);
    koala.current_armor = 0;
    koala.poisoned_stacks = 1;
    koala.revealed_until_turn_start = true;

    let mut pairs = vec![
        (PlayerOwner(1), &mut n1),
        (PlayerOwner(1), &mut n2),
        (PlayerOwner(1), &mut koala),
    ];
    begin_player_turn(1, &mut econ, &mut pairs);

    assert_eq!(econ.home_base_munitions[&1], 5);
    assert_eq!(econ.actions_remaining[&1], 4); // 2 base + 2 Control Nodes
    assert_eq!(koala.current_armor, 2);         // Armor refreshed
    assert_eq!(koala.current_str_hp, 4);        // Poison dealt 1 STR damage
    assert!(!koala.revealed_until_turn_start);  // Stealth reveal cleared
}

#[test]
fn test_07_distance_evade_cover_hit_roll_and_strength_invariant_damage() {
    let mut att = sample_board_unit("Mecharoo", CombatClass::Heavy, 6, 2, 0, 2, false, 0, 0);
    let mut def = sample_board_unit("Koala", CombatClass::Medium, 4, 1, 1, 2, false, 0, 0);

    let ctx = CombatContext {
        hex_distance: 2,
        is_melee: false,
        attacker_can_fly: false,
        attacker_stealth_ambush: false,
        attacker_adjacent_commander: false,
        attacker_pack_tactics_bonus: false,
        defender_in_rough_cover: true,
    };
    // 1 + dist(2) + evade(1) + cover(1) = 5+
    assert_eq!(required_ranged_hit_roll(&def, ctx), 5);

    // Hit roll 5 succeeds; dmg_d6 = 4, delta_str = (6 - 4) = +2 -> raw 6; armor 1 absorbs 1 -> 5 net -> capped at 4 STR KO!
    let out = resolve_attack(&mut att, &mut def, ctx, 5, 4);
    assert!(out.hit_succeeded);
    assert_eq!(out.armor_absorbed, 1);
    assert_eq!(out.strength_damage_dealt, 4);
    assert!(out.defender_destroyed);
    assert_eq!(def.current_str_hp, 0);
}

#[test]
fn test_08_hospital_bio_and_factory_mech_graveyard_recovery() {
    let mut bio_ticket = GraveyardTicket {
        card_key: "Combat Koala".into(),
        facility: GraveyardFacility::HospitalBio,
        recovered_str: 0,
        target_str: 4,
    };
    assert!(!bio_ticket.tick_recovery(false, true)); // Factory does NOT heal Bio units!
    assert_eq!(bio_ticket.recovered_str, 0);
    assert!(!bio_ticket.tick_recovery(true, false)); // +2/4 STR
    assert!(bio_ticket.tick_recovery(true, false));  // +4/4 STR -> Restored!

    let mut mech_ticket = GraveyardTicket {
        card_key: "Mecharoo (Kanga Mech)".into(),
        facility: GraveyardFacility::FactoryMech,
        recovered_str: 0,
        target_str: 4,
    };
    assert!(!mech_ticket.tick_recovery(true, false)); // Hospital does NOT repair Mechs!
    assert!(!mech_ticket.tick_recovery(false, true));
    assert!(mech_ticket.tick_recovery(false, true));
}

#[test]
fn test_09_abilities_field_heal_venom_spit_and_web_snare() {
    let cat = AbilityCatalog::canonical();
    assert_eq!(cat.abilities.len(), 18);
    assert_eq!(cat.faction_passives.len(), 10);

    let mut target = sample_board_unit("Target", CombatClass::Medium, 6, 1, 1, 2, false, 0, 0);
    target.current_str_hp = 3;
    execute_venom_spit(OrganismFaction::Arachnid, &mut target, false);
    execute_web_snare(OrganismFaction::Arachnid, &mut target, false);
    assert!(target.poisoned_stacks > 0);
    assert!(target.rooted_turns > 0);

    execute_field_heal(&mut target);
    assert!(target.current_str_hp > 3);
    assert_eq!(target.poisoned_stacks, 0);
}

#[test]
fn test_10_procedural_mapgen_and_all_five_scenarios_guarantee_connectivity() {
    for sid in ScenarioId::ALL {
        let scen = generate_scenario(sid, 0x2072_0814);
        assert!(scen.map.tiles.len() >= 90);
        assert!(scen.placements.len() >= 2);
        // Both Team 1 and Team 2 Command Centers must exist on valid passable tiles
        for team in [1u8, 2u8] {
            let hq = scen
                .placements
                .iter()
                .find(|p| p.team_id == team && p.card_key.starts_with("Command Center"))
                .expect("Missing HQ placement");
            assert!(scen.map.tiles.contains_key(&Hex::new(hq.q, hq.r)));
        }
    }
}

#[test]
fn test_11_baked_vox_ron_rigs_compile_into_gpu_64tree() {
    let root = repo_root();
    let mut assets = Vec::new();
    for slug in [
        "hospital_vox.ron",
        "factory_vox.ron",
        "mecharoo_vox.ron",
        "prop_mountain_crag_vox.ron",
        "prop_ford_bridge_vox.ron",
    ] {
        let raw = fs::read_to_string(root.join("assets/compiled/vox").join(slug))
            .expect("Missing baked vox RON");
        let parsed: BakedVoxelAsset = ron::from_str(&raw).expect("Invalid BakedVoxelAsset RON");
        assets.push(parsed);
    }
    let gpu = compile_multi_asset_64tree(&assets);
    assert_eq!(gpu.handles.len(), 5);
    assert!(!gpu.nodes.is_empty());
    assert!(!gpu.bricks.is_empty());
}

#[test]
fn test_12_high_density_micro_voxel_rigs_and_compact_hex_scales() {
    let root = repo_root();
    let vox_ron_dir = root.join("assets/compiled/vox");
    let mut assets = Vec::new();
    for entry in fs::read_dir(&vox_ron_dir).expect("Missing compiled vox dir") {
        let p = entry.unwrap().path();
        if p.extension().and_then(|s| s.to_str()) == Some("ron") {
            let raw = fs::read_to_string(&p).unwrap();
            let parsed: BakedVoxelAsset =
                ron::from_str(&raw).unwrap_or_else(|e| panic!("Invalid RON in {}: {e}", p.display()));
            assert!(
                parsed.dimensions.0 <= 256 && parsed.dimensions.1 <= 256 && parsed.dimensions.2 <= 256,
                "Model {} exceeds 256^3 Sparse 64-Tree bound: {:?}",
                parsed.model_name,
                parsed.dimensions
            );
            assert!(
                parsed.non_empty_voxels >= 1_000,
                "Model {} has unexpectedly low micro-voxel density: {}",
                parsed.model_name,
                parsed.non_empty_voxels
            );
            assets.push(parsed);
        }
    }
    assert_eq!(assets.len(), 170, "Expected all 170 bespoke card, terrain hex, and prop rigs");
    let gpu = compile_multi_asset_64tree(&assets);
    assert_eq!(gpu.handles.len(), 170);
    assert!(gpu.bricks.len() > 25_000);
}

#[test]
fn test_13_static_building_structural_armor_does_not_refresh_on_turn_start() {
    let mut econ = PlayerEconomy::default();
    let mut hq = sample_board_unit("Command Center", CombatClass::Installation, 10, 2, 0, 0, true, 2, 2);
    hq.current_armor = 0; // Structural armor cracked in prior siege!
    let mut pairs = vec![(PlayerOwner(1), &mut hq)];
    begin_player_turn(1, &mut econ, &mut pairs);
    assert_eq!(hq.current_armor, 0, "Static building structural armor must not auto-refresh!");
}
