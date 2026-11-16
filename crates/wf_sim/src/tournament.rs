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

//! Automated 250-Pt Tournament Deck Builder & Symmetric 102-Match Round-Robin Benchmark (`v3.4.1`).

use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use wf_core::mapgen::SplitMix64;
use wf_core::{
    BioTypeCardDef, CombatClass, DeckEntry, DeckList, OrganismFaction, OriginAllegiance,
    RosterManifest, ScenarioId,
};
use crate::ai_planner::{select_best_action, AiDifficulty};
use crate::sim_engine::HeadlessMatch;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchSummaryRow {
    pub match_index: usize,
    pub category: String,
    pub scenario: ScenarioId,
    pub seed: u64,
    pub p1_label: String,
    pub p2_label: String,
    pub winner_team: u8,
    pub turns_played: u32,
    pub win_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchetypeStanding {
    pub name: String,
    pub category: String,
    pub games: u32,
    pub wins: u32,
    pub win_rate_pct: f32,
    pub avg_turns: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentReport {
    pub engine_version: String,
    pub ai_difficulty: String,
    pub total_matches: usize,
    pub decisive_matches: usize,
    pub p1_wins: usize,
    pub p2_wins: usize,
    pub median_turns: f32,
    pub standings: Vec<ArchetypeStanding>,
    pub matches: Vec<MatchSummaryRow>,
}

fn origin_hq_and_commander(origin: OriginAllegiance) -> (&'static str, &'static str) {
    match origin {
        OriginAllegiance::Australia | OriginAllegiance::Neutral => {
            ("Command Center (Australia)", "Commander (Australia)")
        }
        OriginAllegiance::Africa => ("Command Center (Africa)", "Commander (Africa)"),
        OriginAllegiance::Asia => ("Command Center (Asia)", "Commander (Asia)"),
        OriginAllegiance::Europe => ("Command Center (Europe)", "Commander (Europe)"),
        OriginAllegiance::NorthAmerica => {
            ("Command Center (North America)", "Commander (North America)")
        }
        OriginAllegiance::SouthAmerica => {
            ("Command Center (South America)", "Commander (South America)")
        }
    }
}

/// Builds a tournament-legal `<= 250`-point deck for any Continental + Pure/Hybrid Bio-Type pairing.
pub fn build_tournament_deck(
    roster: &RosterManifest,
    origin: OriginAllegiance,
    primary: OrganismFaction,
    secondary: OrganismFaction,
) -> DeckList {
    let lookup = roster.index_by_key();
    let is_pure = primary == secondary;
    let (hq_key, cmd_key) = origin_hq_and_commander(origin);

    let mut counts: BTreeMap<String, u8> = BTreeMap::new();
    let mut points_used: u16 = 0;

    let mut try_add = |key: &str, qty: u8, max_cap: u8| {
        if let Some(&card) = lookup.get(key) {
            for _ in 0..qty {
                let cur = counts.get(key).copied().unwrap_or(0);
                if cur >= max_cap || points_used + card.deck_points > 250 {
                    break;
                }
                counts.insert(key.to_string(), cur + 1);
                points_used += card.deck_points;
            }
        }
    };

    // Mandatory Core Infrastructure
    try_add(hq_key, 1, 1);
    try_add("Supply Depot", 1, 1);
    try_add(cmd_key, 1, 1);
    try_add("Control Node", 1, 2);
    try_add("Hospital", 1, 1);
    try_add("Cargo Drop", 1, 2);

    // Pure / Hybrid Fish decks always carry 2x Hydro-Breach Canal for guaranteed inland HQ reach
    if primary == OrganismFaction::Fish || secondary == OrganismFaction::Fish {
        try_add("Hydro-Breach Canal", 2, 2);
    }

    // Gather allowed Organism Bio-Type combat & Tactical Deployment cards sorted by efficiency
    let mut clade_cards: Vec<_> = roster
        .units
        .iter()
        .filter(|u| {
            if is_pure {
                u.faction == primary
            } else {
                u.faction == primary || u.faction == secondary
            }
        })
        .collect();
    clade_cards.sort_by_key(|u| {
        let role_rank = match u.combat_class {
            CombatClass::Heavy => 0,
            CombatClass::Medium => 1,
            CombatClass::Light => 2,
            CombatClass::Medic | CombatClass::Support => 3,
            CombatClass::TacticalDeployment => 4,
            _ => 5,
        };
        (role_rank, std::cmp::Reverse(u.combat_ehp_tenths), u.deck_points)
    });

    for pass_cap in [1u8, 2u8] {
        for card in &clade_cards {
            let cap = if card.combat_class == CombatClass::TacticalDeployment {
                1
            } else {
                pass_cap
            };
            try_add(&card.card_key, 1, cap);
        }
    }
    try_add("Combat Medic", 1, 2);
    try_add("Control Node", 1, 2);

    let entries = counts
        .into_iter()
        .map(|(card_key, quantity)| DeckEntry { card_key, quantity })
        .collect();

    let deck_name = if is_pure {
        format!("{:?} Pure {:?} (250pt)", origin, primary)
    } else {
        format!("{:?} {:?}+{:?} (250pt)", origin, primary, secondary)
    };

    DeckList {
        deck_name,
        faction: origin,
        primary_faction: primary,
        secondary_faction: secondary,
        is_pure_biotype: is_pure,
        point_budget: 250,
        entries,
    }
}

pub fn play_headless_match_to_end(
    roster: &RosterManifest,
    p1_deck: DeckList,
    p2_deck: DeckList,
    scenario: ScenarioId,
    seed: u64,
    difficulty: AiDifficulty,
    max_rounds: u32,
    record_replay: bool,
) -> HeadlessMatch {
    let mut m = HeadlessMatch::new(
        roster.clone(),
        [p1_deck, p2_deck],
        scenario,
        seed,
        record_replay,
    );
    let mut ai_rng = SplitMix64::new(seed ^ 0x9988_7766);
    let mut safety_steps = 0u32;

    while m.winner_team.is_none() && m.turn_number <= max_rounds && safety_steps < 4_000 {
        safety_steps += 1;
        let act = select_best_action(&m, difficulty, &mut ai_rng);
        if !m.apply_action(act) {
            let _ = m.apply_action( crate::sim_engine::SimAction::EndTurn );
        }
    }
    m
}

/// Runs the symmetric 102-match tournament (`72` Pure Bio-Type home-and-away + `30` Continental mirrors).
pub fn run_102_round_robin(
    roster: &RosterManifest,
    difficulty: AiDifficulty,
    max_rounds: u32,
) -> TournamentReport {
    let mut matches = Vec::with_capacity(102);
    let mut stats: BTreeMap<(String, String), (u32, u32, u32)> = BTreeMap::new();
    let scenarios = ScenarioId::ALL;
    let mut match_idx = 0usize;

    // 1. 9x8 = 72 Symmetric Pure Bio-Type Matches
    for (i, &b1) in BioTypeCardDef::ALL.iter().enumerate() {
        for (j, &b2) in BioTypeCardDef::ALL.iter().enumerate() {
            if i == j {
                continue;
            }
            match_idx += 1;
            let scen = scenarios[(match_idx - 1) % scenarios.len()];
            let seed = 0x2072_0000u64 + (match_idx as u64) * 0x9E37;
            let d1 = build_tournament_deck(roster, OriginAllegiance::Australia, b1, b1);
            let d2 = build_tournament_deck(roster, OriginAllegiance::Australia, b2, b2);
            let res = play_headless_match_to_end(roster, d1, d2, scen, seed, difficulty, max_rounds, false);
            let w = res.winner_team.unwrap_or(0);
            let n1 = format!("Pure {:?}", b1);
            let n2 = format!("Pure {:?}", b2);

            let e1 = stats.entry((n1.clone(), "Pure Bio-Type".into())).or_insert((0, 0, 0));
            e1.0 += 1;
            if w == 1 { e1.1 += 1; }
            e1.2 += res.turn_number;

            let e2 = stats.entry((n2.clone(), "Pure Bio-Type".into())).or_insert((0, 0, 0));
            e2.0 += 1;
            if w == 2 { e2.1 += 1; }
            e2.2 += res.turn_number;

            matches.push(MatchSummaryRow {
                match_index: match_idx,
                category: "Pure Bio-Type".into(),
                scenario: scen,
                seed,
                p1_label: n1,
                p2_label: n2,
                winner_team: w,
                turns_played: res.turn_number,
                win_reason: res.win_reason,
            });
        }
    }

    // 2. 6x5 = 30 Symmetric Continental Command Matches
    let continents = [
        OriginAllegiance::Australia,
        OriginAllegiance::Africa,
        OriginAllegiance::Asia,
        OriginAllegiance::Europe,
        OriginAllegiance::NorthAmerica,
        OriginAllegiance::SouthAmerica,
    ];
    for (i, &c1) in continents.iter().enumerate() {
        for (j, &c2) in continents.iter().enumerate() {
            if i == j {
                continue;
            }
            match_idx += 1;
            let scen = scenarios[(match_idx - 1) % scenarios.len()];
            let seed = 0x3072_0000u64 + (match_idx as u64) * 0x7F4A;
            let d1 = build_tournament_deck(roster, c1, OrganismFaction::Mammal, OrganismFaction::Reptile);
            let d2 = build_tournament_deck(roster, c2, OrganismFaction::Mammal, OrganismFaction::Reptile);
            let res = play_headless_match_to_end(roster, d1, d2, scen, seed, difficulty, max_rounds, false);
            let w = res.winner_team.unwrap_or(0);
            let n1 = format!("{:?}", c1);
            let n2 = format!("{:?}", c2);

            let e1 = stats.entry((n1.clone(), "Continent".into())).or_insert((0, 0, 0));
            e1.0 += 1;
            if w == 1 { e1.1 += 1; }
            e1.2 += res.turn_number;

            let e2 = stats.entry((n2.clone(), "Continent".into())).or_insert((0, 0, 0));
            e2.0 += 1;
            if w == 2 { e2.1 += 1; }
            e2.2 += res.turn_number;

            matches.push(MatchSummaryRow {
                match_index: match_idx,
                category: "Continent".into(),
                scenario: scen,
                seed,
                p1_label: n1,
                p2_label: n2,
                winner_team: w,
                turns_played: res.turn_number,
                win_reason: res.win_reason,
            });
        }
    }

    let p1_wins = matches.iter().filter(|m| m.winner_team == 1).count();
    let p2_wins = matches.iter().filter(|m| m.winner_team == 2).count();
    let decisive = p1_wins + p2_wins;

    let mut turn_list: Vec<u32> = matches.iter().map(|m| m.turns_played).collect();
    turn_list.sort_unstable();
    let median_turns = if turn_list.is_empty() {
        0.0
    } else {
        turn_list[turn_list.len() / 2] as f32
    };

    let mut standings: Vec<ArchetypeStanding> = stats
        .into_iter()
        .map(|((name, category), (games, wins, sum_t))| ArchetypeStanding {
            name,
            category,
            games,
            wins,
            win_rate_pct: if games > 0 { (wins as f32 * 100.0) / (games as f32) } else { 0.0 },
            avg_turns: if games > 0 { (sum_t as f32) / (games as f32) } else { 0.0 },
        })
        .collect();
    standings.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| b.win_rate_pct.partial_cmp(&a.win_rate_pct).unwrap_or(std::cmp::Ordering::Equal))
    });

    TournamentReport {
        engine_version: "3.4.1".into(),
        ai_difficulty: difficulty.label().into(),
        total_matches: matches.len(),
        decisive_matches: decisive,
        p1_wins,
        p2_wins,
        median_turns,
        standings,
        matches,
    }
}
