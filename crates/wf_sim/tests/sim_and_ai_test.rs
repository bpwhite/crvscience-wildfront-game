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

//! Integration tests for `wf_sim` deterministic headless simulation, Turn-1 economy invariants,
//! `.ron` state-hash replay verification, and 2-ply Grandmaster AI (`v3.4.0`).

use std::fs;
use std::path::PathBuf;
use hexx::Hex;
use wf_core::mapgen::SplitMix64;
use wf_core::{Morphology, OrganismFaction, OriginAllegiance, RosterManifest, ScenarioId};
use wf_sim::sim_engine::{make_sim_board_unit, SimEntity};
use wf_sim::tournament::play_headless_match_to_end;
use wf_sim::{
    build_tournament_deck, compute_attack_ev, select_best_action, AiDifficulty, HeadlessMatch,
    SimAction,
};

fn load_workspace_roster() -> RosterManifest {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let raw = fs::read_to_string(root.join("assets/compiled/master_roster.ron"))
        .expect("Missing master_roster.ron");
    ron::from_str(&raw).expect("Invalid master_roster.ron")
}

#[test]
fn sim_test_01_turn_1_economy_and_legal_auto_decks() {
    let roster = load_workspace_roster();
    let d1 = build_tournament_deck(
        &roster,
        OriginAllegiance::Australia,
        OrganismFaction::Mammal,
        OrganismFaction::Mammal,
    );
    let d2 = build_tournament_deck(
        &roster,
        OriginAllegiance::Africa,
        OrganismFaction::Fish,
        OrganismFaction::Fish,
    );
    assert!(d1.validate_against(&roster).is_legal, "Auto-built Pure Mammal deck must be 250pt legal");
    assert!(d2.validate_against(&roster).is_legal, "Auto-built Pure Fish deck must be 250pt legal");
    assert!(
        d2.entries.iter().any(|e| e.card_key == "Hydro-Breach Canal" && e.quantity == 2),
        "Pure Fish tournament deck must auto-provision 2x Hydro-Breach Canal"
    );

    let m = HeadlessMatch::new(
        roster,
        [d1, d2],
        ScenarioId::PelagicAtollAssault,
        0x2072_3400,
        false,
    );
    assert_eq!(m.economy.actions_remaining.get(&1).copied(), Some(2));
    assert_eq!(m.economy.actions_remaining.get(&2).copied(), Some(2));
    assert_eq!(m.economy.home_base_munitions.get(&1).copied(), Some(5));
    assert_eq!(m.economy.home_base_munitions.get(&2).copied(), Some(8));
    assert_eq!(m.hands[0].len(), 5);
    assert_eq!(m.hands[1].len(), 6);
}

#[test]
fn sim_test_02_replay_recording_and_state_hash_verification() {
    let roster = load_workspace_roster();
    let d1 = build_tournament_deck(
        &roster,
        OriginAllegiance::Asia,
        OrganismFaction::Plant,
        OrganismFaction::Cnidarian,
    );
    let d2 = build_tournament_deck(
        &roster,
        OriginAllegiance::NorthAmerica,
        OrganismFaction::Reptile,
        OrganismFaction::Amphibian,
    );

    let finished = play_headless_match_to_end(
        &roster,
        d1,
        d2,
        ScenarioId::KingOfTheControlSpire,
        0xCAFE_2072,
        AiDifficulty::Veteran,
        45,
        true,
    );
    assert!(
        finished.winner_team.is_some(),
        "Headless match must terminate decisively within 45 rounds"
    );
    let replay = finished.export_replay();
    assert!(!replay.steps.is_empty());
    HeadlessMatch::verify_replay_data(roster, &replay)
        .expect("Deterministic replay state hashes must match step-by-step!");
}

#[test]
fn sim_test_03_pure_fish_aquatic_siege_completes_decisively() {
    let roster = load_workspace_roster();
    let fish_deck = build_tournament_deck(
        &roster,
        OriginAllegiance::SouthAmerica,
        OrganismFaction::Fish,
        OrganismFaction::Fish,
    );
    let bird_deck = build_tournament_deck(
        &roster,
        OriginAllegiance::Europe,
        OrganismFaction::Bird,
        OrganismFaction::Bird,
    );
    let m = play_headless_match_to_end(
        &roster,
        fish_deck,
        bird_deck,
        ScenarioId::BioRiverConvoy,
        0x7777_2072,
        AiDifficulty::Grandmaster2Ply,
        45,
        false,
    );
    assert!(m.winner_team.is_some(), "Grandmaster Pure Fish vs Pure Bird must reach decisive finish");
}

#[test]
fn sim_test_04_tampered_replay_hash_rejected() {
    let roster = load_workspace_roster();
    let d1 = build_tournament_deck(
        &roster,
        OriginAllegiance::Asia,
        OrganismFaction::Plant,
        OrganismFaction::Cnidarian,
    );
    let d2 = build_tournament_deck(
        &roster,
        OriginAllegiance::NorthAmerica,
        OrganismFaction::Reptile,
        OrganismFaction::Amphibian,
    );
    let finished = play_headless_match_to_end(
        &roster,
        d1,
        d2,
        ScenarioId::KingOfTheControlSpire,
        0xCAFE_2072,
        AiDifficulty::Veteran,
        45,
        true,
    );
    let mut tampered = finished.export_replay();
    assert!(!tampered.steps.is_empty());
    tampered.steps[0].post_state_hash ^= 1;
    assert!(
        HeadlessMatch::verify_replay_data(roster, &tampered).is_err(),
        "Tampered post_state_hash must fail replay verification"
    );
}

#[test]
fn sim_test_05_veteran_ev_planner_matches_exact_analytical_d6_expectation() {
    let roster = load_workspace_roster();
    let d1 = build_tournament_deck(
        &roster,
        OriginAllegiance::Australia,
        OrganismFaction::Mammal,
        OrganismFaction::Mammal,
    );
    let d2 = build_tournament_deck(
        &roster,
        OriginAllegiance::Africa,
        OrganismFaction::Reptile,
        OrganismFaction::Reptile,
    );
    let mut m = HeadlessMatch::new(roster.clone(), [d1.clone(), d2.clone()], ScenarioId::EmergenceSkirmish, 0x2072_E401, false);
    m.entities.retain(|e| e.card.combat_class == wf_core::CombatClass::Installation);
    for e in &mut m.entities {
        // Keep HQs as valid board anchors without acting as extra ranged attackers in this unit EV comparison
        e.card.ranged_distance = 0;
        e.card.melee_range = 0;
    }
    m.hands = [Vec::new(), Vec::new()];
    m.economy.actions_remaining.insert(1, 1);

    let mut attacker_card = roster.units.iter().find(|u| u.ranged_distance >= 2 && u.strength_hp >= 4 && !u.is_laser).unwrap().clone();
    attacker_card.ranged_distance = 3;
    attacker_card.melee_range = 0;
    attacker_card.strength_hp = 4;
    attacker_card.faction = OrganismFaction::Reptile; // avoid Mammal Pack Tactics or Insect modifiers
    let mut attacker_u = make_sim_board_unit(&attacker_card, &d1);
    attacker_u.current_str_hp = 4;
    attacker_u.max_str_hp = 4;
    attacker_u.carried_munitions = 5;
    attacker_u.node_leash_range = 0;

    let mut target_card = attacker_card.clone();
    target_card.strength_hp = 4;
    target_card.armor = 1;
    target_card.evade = 0;
    target_card.shield = 0;
    let mut def_easy_u = make_sim_board_unit(&target_card, &d2);
    def_easy_u.current_str_hp = 4;
    def_easy_u.max_str_hp = 4;
    def_easy_u.current_armor = 1;
    def_easy_u.base_armor = 1;
    def_easy_u.evade = 0;
    def_easy_u.fortified_stacks = 0;
    def_easy_u.node_leash_range = 0;

    let mut def_hard_u = def_easy_u.clone();
    def_hard_u.evade = 1;

    for h in [Hex::new(0, 0), Hex::new(2, 0), Hex::new(-2, 0)] {
        if let Some(t) = m.scenario.map.tiles.get_mut(&h) {
            t.morphology = Morphology::Flat;
            t.elevation = 0;
        }
    }

    m.entities.push(SimEntity {
        uid: 100,
        team_id: 1,
        hex: Hex::new(0, 0),
        card: attacker_card.clone(),
        unit: attacker_u,
    });
    m.entities.push(SimEntity {
        uid: 201,
        team_id: 2,
        hex: Hex::new(2, 0), // dist 2, evade 0 -> hit threshold 3+ (4/6 hit chance)
        card: target_card.clone(),
        unit: def_easy_u,
    });
    m.entities.push(SimEntity {
        uid: 202,
        team_id: 2,
        hex: Hex::new(-2, 0), // dist 2, evade 1 -> hit threshold 4+ (3/6 hit chance)
        card: target_card,
        unit: def_hard_u,
    });

    // Analytical expectation for attacker STR=4 vs defender STR=4, ARM=1, dist=2:
    // dmg_d6 in 1..=6 -> capped=[1,2,3,4,4,4], delta_str=0 -> armor_abs=1, str_loss=[0,1,2,3,3,3]
    // Given a hit: E[arm_abs|hit]=1.0, E[str_dmg|hit]=12/6=2.0, E[total_dmg|hit]=3.0.
    // Easy target (3+ -> P=4/6): E[arm]=4/6, E[str]=8/6, E[total]=2.0.
    // Hard target (4+ -> P=3/6): E[arm]=3/6, E[str]=6/6=1.0, E[total]=1.5.
    let ev_easy = compute_attack_ev(&m, 100, 201).expect("EV for easy target");
    let ev_hard = compute_attack_ev(&m, 100, 202).expect("EV for hard target");

    assert!((ev_easy.hit_probability - (4.0 / 6.0)).abs() < 1e-9);
    assert!((ev_easy.expected_armor_absorbed - (4.0 / 6.0)).abs() < 1e-9);
    assert!((ev_easy.expected_str_damage - (8.0 / 6.0)).abs() < 1e-9);
    assert!((ev_easy.expected_total_damage - 2.0).abs() < 1e-9);

    assert!((ev_hard.hit_probability - 0.5).abs() < 1e-9);
    assert!((ev_hard.expected_armor_absorbed - 0.5).abs() < 1e-9);
    assert!((ev_hard.expected_str_damage - 1.0).abs() < 1e-9);
    assert!((ev_hard.expected_total_damage - 1.5).abs() < 1e-9);

    // Veteran (1-Ply EV Focus) must strictly choose the higher-EV target (uid 201) deterministically
    let mut rng = SplitMix64::new(0xDEAD_BEEF);
    let best = select_best_action(&m, AiDifficulty::Veteran, &mut rng);
    assert_eq!(
        best,
        SimAction::AttackUnit {
            attacker_uid: 100,
            defender_uid: 201,
        },
        "Veteran EV planner must choose the higher-expected-value target (3+ vs 4+)"
    );
}
