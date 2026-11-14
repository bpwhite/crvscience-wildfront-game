//! Integration tests for `wf_sim` deterministic headless simulation, Turn-1 economy invariants,
//! `.ron` state-hash replay verification, and 2-ply Grandmaster AI (`v3.4.0`).

use std::fs;
use std::path::PathBuf;
use wf_core::{OrganismFaction, OriginAllegiance, RosterManifest, ScenarioId};
use wf_sim::tournament::play_headless_match_to_end;
use wf_sim::{build_tournament_deck, AiDifficulty, HeadlessMatch};

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
