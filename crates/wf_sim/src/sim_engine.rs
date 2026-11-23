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

//! Pure-Rust headless deterministic match state (`HeadlessMatch`), 64-bit FNV-1a board
//! state hashing, legal action generator, and `.ron` replay stream verifier (`v3.4.1`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;
use anyhow::{anyhow, Context, Result};
use hexx::Hex;
use ron::ser::{to_string_pretty, PrettyConfig};
use serde::{Deserialize, Serialize};
use wf_core::combat::{resolve_attack, CombatContext, GraveyardTicket};
use wf_core::logistics::{
    begin_player_turn, compute_home_supply_coverage, is_within_control_node, BoardUnit, HexPos,
    PlayerEconomy, PlayerOwner,
};
use wf_core::mapgen::SplitMix64;
use wf_core::{
    generate_scenario, CombatClass, DeckList, GeneratedScenario, GraveyardFacility, Morphology,
    OrganismFaction, OriginAllegiance, RosterManifest, ScenarioId, UnitCardDef, VictoryCondition,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimAction {
    DeployCard {
        hand_idx: usize,
        q: i32,
        r: i32,
    },
    MoveUnit {
        uid: u64,
        to_q: i32,
        to_r: i32,
    },
    AttackUnit {
        attacker_uid: u64,
        defender_uid: u64,
    },
    FieldHeal {
        medic_uid: u64,
        target_uid: u64,
    },
    CommandAbility {
        target_q: i32,
        target_r: i32,
    },
    PrimalSurge {
        target_q: i32,
        target_r: i32,
    },
    RestUnit {
        uid: u64,
    },
    EndTurn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimEntity {
    pub uid: u64,
    pub team_id: u8,
    pub hex: Hex,
    pub card: UnitCardDef,
    pub unit: BoardUnit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnTelemetryPoint {
    pub turn_number: u32,
    pub active_team: u8,
    pub p1_total_str: u16,
    pub p2_total_str: u16,
    pub p1_munitions: u16,
    pub p2_munitions: u16,
    pub p1_units: u8,
    pub p2_units: u8,
    pub p1_obj: u16,
    pub p2_obj: u16,
    pub state_hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedActionStep {
    pub turn_number: u32,
    pub actor_team: u8,
    pub action: SimAction,
    pub post_state_hash: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchReplayFile {
    pub engine_version: String,
    pub scenario_id: ScenarioId,
    pub seed: u64,
    pub decks: [DeckList; 2],
    pub initial_state_hash: u64,
    pub steps: Vec<RecordedActionStep>,
    pub telemetry: Vec<TurnTelemetryPoint>,
    pub winner_team: Option<u8>,
    pub win_reason: String,
    pub total_turns: u32,
}

#[derive(Debug, Clone)]
pub struct HeadlessMatch {
    pub roster: RosterManifest,
    pub decks: [DeckList; 2],
    pub scenario: GeneratedScenario,
    pub seed: u64,
    pub turn_number: u32,
    pub active_team: u8,
    pub economy: PlayerEconomy,
    pub draw_piles: [Vec<String>; 2],
    pub hands: [Vec<String>; 2],
    pub graveyards: [Vec<GraveyardTicket>; 2],
    pub entities: Vec<SimEntity>,
    pub next_uid: u64,
    pub dice_rng: SplitMix64,
    pub obj_progress: [u16; 2],
    pub winner_team: Option<u8>,
    pub win_reason: String,
    pub record_replay: bool,
    pub initial_state_hash: u64,
    pub steps: Vec<RecordedActionStep>,
    pub telemetry: Vec<TurnTelemetryPoint>,
}

fn fnv_mix_u64(mut h: u64, val: u64) -> u64 {
    for b in val.to_le_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

fn fnv_mix_str(mut h: u64, s: &str) -> u64 {
    for &b in s.as_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

pub fn make_sim_board_unit(card: &UnitCardDef, deck: &DeckList) -> BoardUnit {
    let mut u = BoardUnit {
        card_key: card.card_key.clone(),
        faction: card.faction,
        combat_class: card.combat_class,
        graveyard_queue: card.graveyard_queue,
        current_str_hp: card.strength_hp,
        max_str_hp: card.strength_hp,
        current_armor: card.armor,
        base_armor: card.armor,
        evade: card.evade,
        encumbrance_cap: card.encumbrance,
        carried_munitions: card.encumbrance,
        node_leash_range: if card.is_autonomous { 0 } else { card.node_leash_range },
        emits_node: card.emits_node,
        node_emit_radius: card.node_emit_radius,
        supply_range: card.supply_range,
        is_laser: card.is_laser,
        laser_fired_this_turn: false,
        confused: false,
        poisoned_stacks: 0,
        fortified_stacks: card.shield,
        rooted_turns: 0,
        stunned_this_turn: false,
        burrowed: false,
        target_locked: false,
        revealed_until_turn_start: false,
        temp_move_bonus: 0,
        temp_stealth: false,
        blinded_this_turn: false,
    };

    if deck.faction == OriginAllegiance::Europe && card.combat_class.is_building() {
        u.fortified_stacks = (u.fortified_stacks + 1).min(3);
    }
    if deck.faction == OriginAllegiance::Australia
        && (card.combat_class.is_building() || card.emits_node)
    {
        u.node_emit_radius = u.node_emit_radius.saturating_add(1);
    }
    if deck.faction == OriginAllegiance::Asia
        && (card.faction == OrganismFaction::Human || card.card_key == "Drone")
    {
        u.node_leash_range = 0;
    }

    if deck.is_pure_biotype() && card.faction == deck.primary_faction {
        match deck.primary_faction {
            OrganismFaction::Mammal => {
                if card.combat_class == CombatClass::Light {
                    u.current_armor = u.current_armor.saturating_add(1);
                    u.base_armor = u.base_armor.saturating_add(1);
                } else if matches!(card.combat_class, CombatClass::Medium | CombatClass::Heavy) {
                    u.current_str_hp = u.current_str_hp.saturating_add(1);
                    u.max_str_hp = u.max_str_hp.saturating_add(1);
                }
            }
            OrganismFaction::Plant => {
                u.fortified_stacks = (u.fortified_stacks + 1).min(3);
                u.current_str_hp = u.current_str_hp.saturating_add(1);
                u.max_str_hp = u.max_str_hp.saturating_add(1);
            }
            OrganismFaction::Reptile if !card.combat_class.is_building() => {
                u.current_armor = u.current_armor.saturating_add(1);
                u.base_armor = u.base_armor.saturating_add(1);
            }
            OrganismFaction::Bird => {
                u.evade = u.evade.saturating_add(1);
            }
            OrganismFaction::Fish => {
                u.fortified_stacks = (u.fortified_stacks + 1).min(3);
                u.evade = u.evade.saturating_add(1);
                u.temp_move_bonus = u.temp_move_bonus.saturating_add(1);
            }
            OrganismFaction::Arachnid if !card.combat_class.is_building() => {
                u.evade = u.evade.saturating_add(1);
                u.fortified_stacks = (u.fortified_stacks + 1).min(3);
                u.current_armor = u.current_armor.saturating_add(1);
                u.base_armor = u.base_armor.saturating_add(1);
                u.current_str_hp = u.current_str_hp.saturating_add(1);
                u.max_str_hp = u.max_str_hp.saturating_add(1);
            }
            OrganismFaction::Insect if !card.combat_class.is_building() => {
                u.carried_munitions = u.carried_munitions.saturating_add(1);
                u.fortified_stacks = (u.fortified_stacks + 1).min(3);
                u.current_str_hp = u.current_str_hp.saturating_add(1);
                u.max_str_hp = u.max_str_hp.saturating_add(1);
            }
            OrganismFaction::Amphibian if !card.combat_class.is_building() => {
                u.evade = u.evade.saturating_add(1);
                u.current_str_hp = u.current_str_hp.saturating_add(1);
                u.max_str_hp = u.max_str_hp.saturating_add(1);
            }
            OrganismFaction::Cnidarian => {
                u.emits_node = true;
                u.node_emit_radius = u.node_emit_radius.max(1);
                u.node_leash_range = 0;
                u.fortified_stacks = (u.fortified_stacks + 1).min(3);
            }
            _ => {}
        }
    }
    u
}

impl HeadlessMatch {
    pub fn new(
        roster: RosterManifest,
        decks: [DeckList; 2],
        scenario_id: ScenarioId,
        seed: u64,
        record_replay: bool,
    ) -> Self {
        let scenario = generate_scenario(scenario_id, seed);
        let mut dice_rng = SplitMix64::new(seed ^ 0xD1CE_2072);

        let mut draw_piles = [Vec::new(), Vec::new()];
        let mut hands = [Vec::new(), Vec::new()];
        for (idx, opening_n) in [(0usize, 5usize), (1usize, 6usize)] {
            let mut pile = Vec::new();
            for entry in &decks[idx].entries {
                if entry.card_key.starts_with("Command Center") {
                    continue;
                }
                for _ in 0..entry.quantity {
                    pile.push(entry.card_key.clone());
                }
            }
            for i in (1..pile.len()).rev() {
                let j = (dice_rng.next_u64() as usize) % (i + 1);
                pile.swap(i, j);
            }
            let mut hand = Vec::with_capacity(opening_n);
            for _ in 0..opening_n {
                if let Some(ck) = pile.pop() {
                    hand.push(ck);
                }
            }
            draw_piles[idx] = pile;
            hands[idx] = hand;
        }

        let mut m = Self {
            roster,
            decks,
            scenario,
            seed,
            turn_number: 1,
            active_team: 1,
            economy: PlayerEconomy::default(),
            draw_piles,
            hands,
            graveyards: [Vec::new(), Vec::new()],
            entities: Vec::new(),
            next_uid: 1,
            dice_rng,
            obj_progress: [0, 0],
            winner_team: None,
            win_reason: String::new(),
            record_replay,
            initial_state_hash: 0,
            steps: Vec::new(),
            telemetry: Vec::new(),
        };

        // Spawn ONLY the two Continental Command Centers on their base anchors (strict 250-pt tournament start)
        let lookup = m.roster.index_by_key();
        let mut hq_anchors = [(Hex::new(-4, 2), 1u8), (Hex::new(4, -2), 2u8)];
        for pl in &m.scenario.placements {
            if pl.card_key.starts_with("Command Center") && (pl.team_id == 1 || pl.team_id == 2) {
                hq_anchors[(pl.team_id - 1) as usize] = (Hex::new(pl.q, pl.r), pl.team_id);
            }
        }
        for (hq_hex, tid) in hq_anchors {
            let deck = &m.decks[(tid - 1) as usize];
            let hq_key = deck
                .entries
                .iter()
                .find(|e| e.card_key.starts_with("Command Center"))
                .map(|e| e.card_key.clone())
                .unwrap_or_else(|| "Command Center (Australia)".to_string());
            if let Some(&card) = lookup.get(hq_key.as_str()) {
                let uid = m.next_uid;
                m.next_uid += 1;
                let unit = make_sim_board_unit(card, deck);
                if let Some(tile) = m.scenario.map.tiles.get_mut(&hq_hex) {
                    if matches!(
                        tile.morphology,
                        Morphology::MountainPeak
                            | Morphology::RavineChasm
                            | Morphology::DeepRiver
                            | Morphology::OpenOcean
                    ) {
                        tile.morphology = Morphology::Flat;
                        tile.elevation = 0;
                    }
                }
                // Ensure pure-aquatic Fish decks have at least 2 adjacent ShallowFord launch aprons next to HQ
                if deck.active_biotypes().contains(&OrganismFaction::Fish) {
                    let mut apron = 0;
                    for nb in hq_hex.all_neighbors() {
                        if let Some(nt) = m.scenario.map.tiles.get_mut(&nb) {
                            if matches!(nt.morphology, Morphology::Flat | Morphology::RoughCover) {
                                nt.morphology = Morphology::ShallowFord;
                                nt.elevation = 0;
                                apron += 1;
                                if apron >= 2 {
                                    break;
                                }
                            }
                        }
                    }
                }
                m.entities.push(SimEntity {
                    uid,
                    team_id: tid,
                    hex: hq_hex,
                    card: card.clone(),
                    unit,
                });
            }
        }

        // Strict v3.4.1 Turn-1 Economy & Seat Initiative Compensation:
        // Both players start Turn 1 with strictly 2 Base Actions; P1 = 5 Munitions (5 cards), P2 = 8 Munitions (6 cards).
        m.economy.home_base_munitions.insert(1, 5);
        m.economy.home_base_munitions.insert(2, 8);
        m.economy.actions_remaining.insert(1, 2);
        m.economy.actions_remaining.insert(2, 2);
        m.economy.faction_ability_used.insert(1, false);
        m.economy.faction_ability_used.insert(2, false);
        m.economy.primal_surge_used.insert(1, false);
        m.economy.primal_surge_used.insert(2, false);
        m.refresh_confusion_states();

        m.initial_state_hash = m.compute_state_hash();
        m.record_telemetry_point();
        m
    }

    pub fn roll_d6(&mut self) -> u8 {
        ((self.dice_rng.next_u64() % 6) as u8) + 1
    }

    pub fn compute_state_hash(&self) -> u64 {
        let mut h: u64 = 0xCBF2_9CE4_8422_2325;
        h = fnv_mix_u64(h, u64::from(self.turn_number));
        h = fnv_mix_u64(h, u64::from(self.active_team));
        h = fnv_mix_u64(h, u64::from(self.obj_progress[0]));
        h = fnv_mix_u64(h, u64::from(self.obj_progress[1]));
        for tid in [1u8, 2u8] {
            h = fnv_mix_u64(h, u64::from(self.economy.home_base_munitions.get(&tid).copied().unwrap_or(0)));
            h = fnv_mix_u64(h, u64::from(self.economy.actions_remaining.get(&tid).copied().unwrap_or(0)));
        }
        for p in 0..2 {
            for ck in &self.hands[p] {
                h = fnv_mix_str(h, ck);
            }
            h = fnv_mix_u64(h, self.draw_piles[p].len() as u64);
        }
        let mut sorted_ents: Vec<&SimEntity> = self
            .entities
            .iter()
            .filter(|e| e.unit.current_str_hp > 0)
            .collect();
        sorted_ents.sort_by_key(|e| e.uid);
        for e in sorted_ents {
            h = fnv_mix_u64(h, e.uid);
            h = fnv_mix_u64(h, u64::from(e.team_id));
            h = fnv_mix_u64(h, (e.hex.x as i64) as u64);
            h = fnv_mix_u64(h, (e.hex.y as i64) as u64);
            h = fnv_mix_u64(h, u64::from(e.unit.current_str_hp));
            h = fnv_mix_u64(h, u64::from(e.unit.current_armor));
            h = fnv_mix_u64(h, u64::from(e.unit.fortified_stacks));
            h = fnv_mix_u64(h, u64::from(e.unit.carried_munitions));
            h = fnv_mix_u64(h, u64::from(e.unit.poisoned_stacks));
            h = fnv_mix_u64(h, u64::from(e.unit.rooted_turns));
        }
        let mut tiles: Vec<_> = self.scenario.map.tiles.iter().collect();
        tiles.sort_by_key(|(hx, _)| (hx.x, hx.y));
        for (hx, t) in tiles {
            h = fnv_mix_u64(h, ((hx.x as i64) << 16 ^ (hx.y as i64)) as u64);
            h = fnv_mix_u64(h, t.morphology as u64);
        }
        h
    }

    pub fn record_telemetry_point(&mut self) {
        let mut p1_str = 0u16;
        let mut p2_str = 0u16;
        let mut p1_u = 0u8;
        let mut p2_u = 0u8;
        for e in &self.entities {
            if e.unit.current_str_hp > 0 {
                if e.team_id == 1 {
                    p1_str += u16::from(e.unit.current_str_hp + e.unit.current_armor + e.unit.fortified_stacks);
                    p1_u = p1_u.saturating_add(1);
                } else if e.team_id == 2 {
                    p2_str += u16::from(e.unit.current_str_hp + e.unit.current_armor + e.unit.fortified_stacks);
                    p2_u = p2_u.saturating_add(1);
                }
            }
        }
        let hash = self.compute_state_hash();
        self.telemetry.push(TurnTelemetryPoint {
            turn_number: self.turn_number,
            active_team: self.active_team,
            p1_total_str: p1_str,
            p2_total_str: p2_str,
            p1_munitions: self.economy.home_base_munitions.get(&1).copied().unwrap_or(0),
            p2_munitions: self.economy.home_base_munitions.get(&2).copied().unwrap_or(0),
            p1_units: p1_u,
            p2_units: p2_u,
            p1_obj: self.obj_progress[0],
            p2_obj: self.obj_progress[1],
            state_hash: hash,
        });
    }

    pub fn entity_at(&self, hex: Hex) -> Option<usize> {
        self.entities
            .iter()
            .position(|e| e.unit.current_str_hp > 0 && e.hex == hex)
    }

    pub fn entity_by_uid(&self, uid: u64) -> Option<usize> {
        self.entities
            .iter()
            .position(|e| e.unit.current_str_hp > 0 && e.uid == uid)
    }

    pub fn team_supply_hexes(&self, team_id: u8) -> HashSet<Hex> {
        let refs: Vec<(PlayerOwner, HexPos, &BoardUnit)> = self
            .entities
            .iter()
            .filter(|e| e.unit.current_str_hp > 0)
            .map(|e| (PlayerOwner(e.team_id), HexPos(e.hex), &e.unit))
            .collect();
        compute_home_supply_coverage(team_id, &refs)
    }

    pub fn refresh_confusion_states(&mut self) {
        let snapshot: Vec<(PlayerOwner, HexPos, BoardUnit)> = self
            .entities
            .iter()
            .filter(|e| e.unit.current_str_hp > 0)
            .map(|e| (PlayerOwner(e.team_id), HexPos(e.hex), e.unit.clone()))
            .collect();
        let refs: Vec<(PlayerOwner, HexPos, &BoardUnit)> = snapshot
            .iter()
            .map(|(o, p, u)| (*o, *p, u))
            .collect();
        for ent in &mut self.entities {
            if ent.unit.current_str_hp == 0 || ent.team_id == 0 {
                continue;
            }
            ent.unit.confused = !is_within_control_node(
                ent.team_id,
                ent.unit.node_leash_range,
                ent.hex,
                &refs,
            );
        }
    }

    pub fn card_deploy_cost(&self, team_id: u8, card: &UnitCardDef, hex: Hex) -> u16 {
        let deck = &self.decks[(team_id - 1).min(1) as usize];
        if card.card_key == "Cargo Drop" {
            return 0;
        }
        if deck.is_pure_biotype()
            && deck.primary_faction == OrganismFaction::Plant
            && card.combat_class == CombatClass::Trap
        {
            return 0;
        }
        let mut cost = u16::from(card.encumbrance);
        if card.faction == OrganismFaction::Insect {
            if deck.is_pure_biotype()
                && deck.primary_faction == OrganismFaction::Insect
                && card.combat_class == CombatClass::Light
            {
                cost = cost.saturating_sub(2);
            } else if self.entities.iter().any(|e| {
                e.team_id == team_id
                    && e.unit.current_str_hp > 0
                    && e.card.faction == OrganismFaction::Insect
                    && e.hex.unsigned_distance_to(hex) == 1
            }) {
                cost = cost.saturating_sub(1).max(1);
            }
        }
        cost
    }

    pub fn valid_deploy_hexes(&self, team_id: u8, card: &UnitCardDef) -> Vec<Hex> {
        let mut out = Vec::new();
        if card.combat_class == CombatClass::TacticalDeployment {
            if card.card_key == "Hydro-Breach Canal" {
                let water: HashSet<Hex> = self
                    .scenario
                    .map
                    .tiles
                    .iter()
                    .filter(|(_, t)| {
                        matches!(
                            t.morphology,
                            Morphology::DeepRiver | Morphology::OpenOcean | Morphology::ShallowFord
                        )
                    })
                    .map(|(&h, _)| h)
                    .collect();
                for (&h, t) in &self.scenario.map.tiles {
                    if matches!(
                        t.morphology,
                        Morphology::Flat | Morphology::RoughCover | Morphology::RadiationFallout
                    ) && self.entity_at(h).is_none()
                        && (water.is_empty() || h.all_neighbors().iter().any(|nb| water.contains(nb)))
                    {
                        out.push(h);
                    }
                }
            } else if card.heal_repair_range > 0 || card.shield > 0 || card.action_bonus > 0 {
                for e in &self.entities {
                    if e.team_id == team_id && e.unit.current_str_hp > 0 {
                        out.push(e.hex);
                    }
                }
            } else {
                for e in &self.entities {
                    if e.team_id != team_id && e.team_id != 0 && e.unit.current_str_hp > 0 {
                        out.push(e.hex);
                    }
                }
            }
            out.sort_by_key(|h| (h.x, h.y));
            return out;
        }

        let supply = self.team_supply_hexes(team_id);
        for (&h, tile) in &self.scenario.map.tiles {
            if self.entity_at(h).is_some() {
                continue;
            }
            let terrain_ok = if card.combat_class.is_building() {
                !matches!(
                    tile.morphology,
                    Morphology::MountainPeak
                        | Morphology::RavineChasm
                        | Morphology::DeepRiver
                        | Morphology::OpenOcean
                        | Morphology::ShallowFord
                )
            } else if card.can_fly {
                tile.morphology != Morphology::MountainPeak
            } else if card.amphibious_tier == 2 {
                matches!(
                    tile.morphology,
                    Morphology::DeepRiver
                        | Morphology::OpenOcean
                        | Morphology::ShallowFord
                        | Morphology::ArmoredDam
                )
            } else if card.amphibious_tier == 1 {
                !matches!(tile.morphology, Morphology::MountainPeak | Morphology::RavineChasm)
            } else {
                !matches!(
                    tile.morphology,
                    Morphology::MountainPeak
                        | Morphology::RavineChasm
                        | Morphology::DeepRiver
                        | Morphology::OpenOcean
                )
            };
            if !terrain_ok {
                continue;
            }

            let mut zone_ok = supply.contains(&h);
            if !zone_ok && card.card_key == "Cargo Drop" {
                zone_ok = self.entities.iter().any(|e| {
                    e.team_id == team_id
                        && e.unit.current_str_hp > 0
                        && e.hex.unsigned_distance_to(h) <= 4
                });
            }
            if !zone_ok && card.amphibious_tier == 2 {
                zone_ok = self.entities.iter().any(|e| {
                    e.team_id == team_id
                        && e.unit.current_str_hp > 0
                        && ((e.card.combat_class.is_building() && e.hex.unsigned_distance_to(h) <= 3)
                            || (e.card.faction == OrganismFaction::Fish
                                && e.hex.unsigned_distance_to(h) <= 1))
                });
            }
            if !zone_ok && card.faction == OrganismFaction::Plant {
                zone_ok = self.entities.iter().any(|e| {
                    e.team_id == team_id
                        && e.unit.current_str_hp > 0
                        && e.card.faction == OrganismFaction::Plant
                        && e.hex.unsigned_distance_to(h) <= 2
                });
            }
            if zone_ok {
                out.push(h);
            }
        }
        out.sort_by_key(|h| (h.x, h.y));
        out
    }

    pub fn valid_move_hexes(&self, ent_idx: usize) -> Vec<Hex> {
        let mut out = Vec::new();
        let Some(ent) = self.entities.get(ent_idx) else {
            return out;
        };
        if ent.unit.current_str_hp == 0
            || ent.unit.confused
            || ent.unit.rooted_turns > 0
            || ent.card.movement == 0
        {
            return out;
        }
        let max_steps = ent.card.movement.saturating_add(ent.unit.temp_move_bonus);
        let refs: Vec<(PlayerOwner, HexPos, &BoardUnit)> = self
            .entities
            .iter()
            .filter(|e| e.unit.current_str_hp > 0)
            .map(|e| (PlayerOwner(e.team_id), HexPos(e.hex), &e.unit))
            .collect();

        let mut visited: HashMap<Hex, u8> = HashMap::new();
        let mut q = VecDeque::new();
        visited.insert(ent.hex, 0);
        q.push_back((ent.hex, 0u8));

        while let Some((curr, dist)) = q.pop_front() {
            if dist >= max_steps {
                continue;
            }
            for nb in curr.all_neighbors() {
                let next_d = dist + 1;
                if visited.get(&nb).map_or(false, |&prev| prev <= next_d) {
                    continue;
                }
                if !self.scenario.map.can_traverse(
                    curr,
                    nb,
                    ent.card.can_fly,
                    ent.unit.burrowed,
                    ent.card.amphibious_tier,
                ) {
                    continue;
                }
                let occ = self.entity_at(nb);
                let friendly_pass = occ.map_or(false, |oi| {
                    let oe = &self.entities[oi];
                    oe.team_id == ent.team_id
                        && (oe.card.card_key == "Cargo Drop"
                            || ent.card.faction == OrganismFaction::Fish
                            || ent.card.amphibious_tier == 2
                            || ent.card.faction == OrganismFaction::Insect)
                });
                if occ.is_some()
                    && !(ent.card.can_fly || ent.card.stealth || ent.unit.temp_stealth || friendly_pass)
                {
                    continue;
                }
                visited.insert(nb, next_d);
                q.push_back((nb, next_d));
                if occ.is_none()
                    && is_within_control_node(ent.team_id, ent.unit.node_leash_range, nb, &refs)
                {
                    out.push(nb);
                }
            }
        }
        out.sort_by_key(|h| (h.x, h.y));
        out
    }

    pub fn effective_attack_range(&self, ent: &SimEntity) -> u8 {
        let deck = &self.decks[(ent.team_id - 1).min(1) as usize];
        let mut r = ent.card.ranged_distance;
        if deck.is_pure_biotype() && ent.card.faction == deck.primary_faction {
            if matches!(deck.primary_faction, OrganismFaction::Bird | OrganismFaction::Fish) && r > 0 {
                r = r.saturating_add(1);
            }
        }
        r.max(ent.card.melee_range)
    }

    pub fn can_spend_munition(&self, ent_idx: usize, cost: u8) -> bool {
        if cost == 0 {
            return true;
        }
        let ent = &self.entities[ent_idx];
        let home = self.economy.home_base_munitions.get(&ent.team_id).copied().unwrap_or(0);
        if self.team_supply_hexes(ent.team_id).contains(&ent.hex) && home >= u16::from(cost) {
            return true;
        }
        if self.entities.iter().any(|c| {
            c.team_id == ent.team_id
                && c.card.card_key == "Cargo Drop"
                && c.unit.current_str_hp > 0
                && c.unit.carried_munitions >= cost
                && c.hex.unsigned_distance_to(ent.hex) <= 2
        }) {
            return true;
        }
        ent.unit.carried_munitions >= cost
    }

    pub(crate) fn spend_munition_for_unit(&mut self, ent_idx: usize, cost: u8) -> bool {
        if cost == 0 {
            return true;
        }
        let team = self.entities[ent_idx].team_id;
        let hex = self.entities[ent_idx].hex;
        if self.team_supply_hexes(team).contains(&hex) {
            let home = self.economy.home_base_munitions.entry(team).or_insert(0);
            if *home >= u16::from(cost) {
                *home -= u16::from(cost);
                return true;
            }
        }
        if let Some(ci) = self.entities.iter().position(|c| {
            c.team_id == team
                && c.card.card_key == "Cargo Drop"
                && c.unit.current_str_hp > 0
                && c.unit.carried_munitions >= cost
                && c.hex.unsigned_distance_to(hex) <= 2
        }) {
            self.entities[ci].unit.carried_munitions -= cost;
            if self.entities[ci].unit.carried_munitions == 0 {
                self.entities[ci].unit.current_str_hp = 0;
            }
            return true;
        }
        if self.entities[ent_idx].unit.carried_munitions >= cost {
            self.entities[ent_idx].unit.carried_munitions -= cost;
            return true;
        }
        false
    }

    pub fn legal_actions(&self) -> Vec<SimAction> {
        if self.winner_team.is_some() {
            return Vec::new();
        }
        let actor = self.active_team;
        let ap = self.economy.actions_remaining.get(&actor).copied().unwrap_or(0);
        if ap == 0 {
            return vec![SimAction::EndTurn];
        }
        let mut actions = Vec::new();
        let lookup = self.roster.index_by_key();
        let home_mun = self.economy.home_base_munitions.get(&actor).copied().unwrap_or(0);

        // 1. Attacks & Field Heals
        for (idx, ent) in self.entities.iter().enumerate() {
            if ent.team_id != actor || ent.unit.current_str_hp == 0 || ent.unit.confused {
                continue;
            }
            let max_rng = self.effective_attack_range(ent);
            let m_cost = u8::from(!ent.unit.is_laser);
            if max_rng > 0
                && !(ent.unit.is_laser && ent.unit.laser_fired_this_turn)
                && self.can_spend_munition(idx, m_cost)
            {
                for other in &self.entities {
                    if other.team_id != actor
                        && other.team_id != 0
                        && other.unit.current_str_hp > 0
                        && !(other.card.can_fly && !ent.card.can_fly && ent.card.ranged_distance == 0)
                    {
                        let d = ent.hex.unsigned_distance_to(other.hex);
                        if d >= 1 && d <= u32::from(max_rng) {
                            actions.push(SimAction::AttackUnit {
                                attacker_uid: ent.uid,
                                defender_uid: other.uid,
                            });
                        }
                    }
                }
            }
            if ent.card.heal_repair_range > 0 && self.can_spend_munition(idx, 1) {
                for ally in &self.entities {
                    if ally.team_id == actor
                        && ally.unit.current_str_hp > 0
                        && !ally.card.combat_class.is_building()
                        && (ally.unit.current_str_hp < ally.unit.max_str_hp
                            || ally.unit.poisoned_stacks > 0)
                        && ent.hex.unsigned_distance_to(ally.hex)
                            <= u32::from(ent.card.heal_repair_range)
                    {
                        actions.push(SimAction::FieldHeal {
                            medic_uid: ent.uid,
                            target_uid: ally.uid,
                        });
                    }
                }
            }
        }

        // 2. Card Deployments (sample up to 4 best candidate hexes per hand slot toward enemy HQ / objective)
        let enemy_hq = self
            .entities
            .iter()
            .find(|e| e.team_id != actor && e.card.combat_class == CombatClass::Installation)
            .map(|e| e.hex)
            .unwrap_or(Hex::ZERO);
        let hand = &self.hands[(actor - 1) as usize];
        for (h_idx, ck) in hand.iter().enumerate() {
            if let Some(&card) = lookup.get(ck.as_str()) {
                let mut valid = self.valid_deploy_hexes(actor, card);
                valid.sort_by_key(|h| h.unsigned_distance_to(enemy_hq));
                for h in valid.into_iter().take(4) {
                    if self.card_deploy_cost(actor, card, h) <= home_mun {
                        actions.push(SimAction::DeployCard {
                            hand_idx: h_idx,
                            q: h.x,
                            r: h.y,
                        });
                    }
                }
            }
        }

        // 3. Continental Command Ability (1/turn)
        if !self.economy.faction_ability_used.get(&actor).copied().unwrap_or(false) {
            let fcard = self.decks[(actor - 1) as usize].faction_card();
            if home_mun >= u16::from(fcard.active_munition_cost) {
                actions.push(SimAction::CommandAbility {
                    target_q: enemy_hq.x,
                    target_r: enemy_hq.y,
                });
            }
        }

        // 4. Unit Movements (up to 4 forward/objective hexes per mobile unit)
        let focal = self
            .scenario
            .objective_hexes
            .first()
            .map(|&(q, r)| Hex::new(q, r))
            .unwrap_or(enemy_hq);
        for (idx, ent) in self.entities.iter().enumerate() {
            if ent.team_id != actor || ent.unit.current_str_hp == 0 {
                continue;
            }
            let mut moves = self.valid_move_hexes(idx);
            moves.sort_by_key(|h| {
                h.unsigned_distance_to(enemy_hq)
                    .min(h.unsigned_distance_to(focal))
            });
            for dst in moves.into_iter().take(4) {
                actions.push(SimAction::MoveUnit {
                    uid: ent.uid,
                    to_q: dst.x,
                    to_r: dst.y,
                });
            }
            if ent.unit.current_str_hp + 1 < ent.unit.max_str_hp && !ent.card.combat_class.is_building() {
                actions.push(SimAction::RestUnit { uid: ent.uid });
            }
        }

        actions.push(SimAction::EndTurn);
        actions
    }

    pub fn apply_action(&mut self, action: SimAction) -> bool {
        if self.winner_team.is_some() {
            return false;
        }
        let actor = self.active_team;
        let ap = self.economy.actions_remaining.get(&actor).copied().unwrap_or(0);

        let applied = match &action {
            SimAction::EndTurn => {
                self.advance_turn();
                true
            }
            SimAction::DeployCard { hand_idx, q, r } => {
                if ap == 0 {
                    return false;
                }
                let p = (actor - 1) as usize;
                let Some(ck) = self.hands[p].get(*hand_idx).cloned() else {
                    return false;
                };
                let Some(card) = self.roster.index_by_key().get(ck.as_str()).cloned().cloned() else {
                    return false;
                };
                let target_hex = Hex::new(*q, *r);
                let cost = self.card_deploy_cost(actor, &card, target_hex);
                let home = self.economy.home_base_munitions.get(&actor).copied().unwrap_or(0);
                if cost > home || !self.valid_deploy_hexes(actor, &card).contains(&target_hex) {
                    return false;
                }
                *self.economy.home_base_munitions.entry(actor).or_insert(0) -= cost;
                *self.economy.actions_remaining.entry(actor).or_insert(1) -= 1;
                self.hands[p].remove(*hand_idx);

                if card.combat_class == CombatClass::TacticalDeployment {
                    self.resolve_tactical_card(actor, &card, target_hex);
                } else if card.card_key == "Cargo Drop" {
                    let uid = self.next_uid;
                    self.next_uid += 1;
                    let mut u = make_sim_board_unit(&card, &self.decks[p]);
                    u.carried_munitions = 5;
                    self.entities.push(SimEntity {
                        uid,
                        team_id: actor,
                        hex: target_hex,
                        card,
                        unit: u,
                    });
                } else {
                    let uid = self.next_uid;
                    self.next_uid += 1;
                    let deck_clone = self.decks[p].clone();
                    let u = make_sim_board_unit(&card, &deck_clone);
                    let is_fish = card.faction == OrganismFaction::Fish;
                    self.entities.push(SimEntity {
                        uid,
                        team_id: actor,
                        hex: target_hex,
                        card,
                        unit: u,
                    });
                    if is_fish {
                        let cap = if deck_clone.is_pure_biotype()
                            && deck_clone.primary_faction == OrganismFaction::Fish
                        {
                            2
                        } else {
                            1
                        };
                        let enemy_hq = self
                            .entities
                            .iter()
                            .find(|e| e.team_id != actor && e.card.combat_class == CombatClass::Installation)
                            .map(|e| e.hex)
                            .unwrap_or(Hex::ZERO);
                        let mut nbs = target_hex.all_neighbors().to_vec();
                        nbs.sort_by_key(|nb| nb.unsigned_distance_to(enemy_hq));
                        let mut carved = 0;
                        for nb in nbs {
                            if let Some(nt) = self.scenario.map.tiles.get_mut(&nb) {
                                if matches!(nt.morphology, Morphology::Flat | Morphology::RoughCover) {
                                    nt.morphology = Morphology::ShallowFord;
                                    nt.elevation = 0;
                                    carved += 1;
                                    if carved >= cap {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                self.refresh_confusion_states();
                self.check_immediate_hq_victory();
                true
            }
            SimAction::MoveUnit { uid, to_q, to_r } => {
                if ap == 0 {
                    return false;
                }
                let Some(idx) = self.entity_by_uid(*uid) else {
                    return false;
                };
                if self.entities[idx].team_id != actor {
                    return false;
                }
                let dst = Hex::new(*to_q, *to_r);
                if !self.valid_move_hexes(idx).contains(&dst) {
                    return false;
                }
                self.entities[idx].hex = dst;
                self.entities[idx].unit.target_locked = false;
                *self.economy.actions_remaining.entry(actor).or_insert(1) -= 1;
                self.refresh_confusion_states();
                true
            }
            SimAction::AttackUnit {
                attacker_uid,
                defender_uid,
            } => {
                if ap == 0 {
                    return false;
                }
                let (Some(att_i), Some(def_i)) = (
                    self.entity_by_uid(*attacker_uid),
                    self.entity_by_uid(*defender_uid),
                ) else {
                    return false;
                };
                if att_i == def_i || self.entities[att_i].team_id != actor {
                    return false;
                }
                let dist = self.entities[att_i]
                    .hex
                    .unsigned_distance_to(self.entities[def_i].hex) as u8;
                let max_rng = self.effective_attack_range(&self.entities[att_i]);
                if dist == 0 || dist > max_rng {
                    return false;
                }
                let m_cost = u8::from(!self.entities[att_i].unit.is_laser);
                if !self.spend_munition_for_unit(att_i, m_cost) {
                    return false;
                }
                *self.economy.actions_remaining.entry(actor).or_insert(1) -= 1;
                if self.entities[att_i].unit.is_laser {
                    self.entities[att_i].unit.laser_fired_this_turn = true;
                }

                let is_melee = dist <= self.entities[att_i].card.melee_range;
                let def_hex = self.entities[def_i].hex;
                let att_hex = self.entities[att_i].hex;
                let att_fac = self.entities[att_i].card.faction;
                let in_cover = self
                    .scenario
                    .map
                    .tiles
                    .get(&def_hex)
                    .map_or(false, |t| t.morphology.grants_ranged_cover())
                    && !matches!(att_fac, OrganismFaction::Arachnid | OrganismFaction::Insect);
                let adj_cmd = self.entities.iter().any(|e| {
                    e.team_id == actor
                        && e.unit.current_str_hp > 0
                        && e.card.combat_class == CombatClass::Commander
                        && e.hex.unsigned_distance_to(att_hex) == 1
                });
                let pack_bonus = att_fac == OrganismFaction::Mammal
                    && self.entities.iter().any(|e| {
                        e.team_id == actor
                            && e.uid != *attacker_uid
                            && e.unit.current_str_hp > 0
                            && e.card.faction == OrganismFaction::Mammal
                            && e.hex.unsigned_distance_to(att_hex) == 1
                    });

                let ctx = CombatContext {
                    hex_distance: dist,
                    is_melee,
                    attacker_can_fly: self.entities[att_i].card.can_fly,
                    attacker_stealth_ambush: self.entities[att_i].card.stealth,
                    attacker_adjacent_commander: adj_cmd,
                    attacker_pack_tactics_bonus: pack_bonus,
                    defender_in_rough_cover: in_cover,
                };
                let hit_d6 = self.roll_d6();
                let dmg_d6 = self.roll_d6();

                let (mut a_unit, mut d_unit) = (
                    self.entities[att_i].unit.clone(),
                    self.entities[def_i].unit.clone(),
                );
                let out = resolve_attack(&mut a_unit, &mut d_unit, ctx, hit_d6, dmg_d6);

                // North America natural-6 Laser bonus
                if out.hit_succeeded
                    && a_unit.is_laser
                    && hit_d6 == 6
                    && self.decks[(actor - 1) as usize].faction == OriginAllegiance::NorthAmerica
                {
                    d_unit.current_str_hp = d_unit.current_str_hp.saturating_sub(1);
                }
                if out.hit_succeeded && self.entities[att_i].card.poison_stacks > 0 && !self.entities[def_i].card.poison_immune {
                    d_unit.poisoned_stacks = (d_unit.poisoned_stacks + self.entities[att_i].card.poison_stacks).min(3);
                }
                if out.hit_succeeded && self.entities[att_i].card.root_stacks > 0 {
                    d_unit.rooted_turns = d_unit.rooted_turns.max(1);
                }
                if out.triggers_melee_retaliation && d_unit.current_str_hp > 0 {
                    let ret_d6 = self.roll_d6();
                    let ret_raw = ret_d6.min(d_unit.current_str_hp);
                    let abs = ret_raw.min(a_unit.current_armor);
                    a_unit.current_armor -= abs;
                    a_unit.current_str_hp = a_unit.current_str_hp.saturating_sub(ret_raw - abs);
                }

                self.entities[att_i].unit = a_unit;
                self.entities[def_i].unit = d_unit;
                self.enqueue_destroyed_units();
                self.refresh_confusion_states();
                self.check_immediate_hq_victory();
                true
            }
            SimAction::FieldHeal {
                medic_uid,
                target_uid,
            } => {
                if ap == 0 {
                    return false;
                }
                let (Some(m_i), Some(t_i)) =
                    (self.entity_by_uid(*medic_uid), self.entity_by_uid(*target_uid))
                else {
                    return false;
                };
                if !self.spend_munition_for_unit(m_i, 1) {
                    return false;
                }
                *self.economy.actions_remaining.entry(actor).or_insert(1) -= 1;
                let t = &mut self.entities[t_i].unit;
                t.current_str_hp = (t.current_str_hp + 2).min(t.max_str_hp);
                t.poisoned_stacks = 0;
                t.rooted_turns = 0;
                true
            }
            SimAction::CommandAbility { target_q, target_r } => {
                if ap == 0 || self.economy.faction_ability_used.get(&actor).copied().unwrap_or(false) {
                    return false;
                }
                let fcard = self.decks[(actor - 1) as usize].faction_card();
                let m_cost = u16::from(fcard.active_munition_cost);
                let home = self.economy.home_base_munitions.get(&actor).copied().unwrap_or(0);
                if home < m_cost {
                    return false;
                }
                *self.economy.home_base_munitions.entry(actor).or_insert(0) -= m_cost;
                *self.economy.actions_remaining.entry(actor).or_insert(1) -= 1;
                self.economy.faction_ability_used.insert(actor, true);

                let center = Hex::new(*target_q, *target_r);
                match self.decks[(actor - 1) as usize].faction {
                    OriginAllegiance::Australia => {
                        if let Some(ally) = self.entities.iter_mut().find(|e| {
                            e.team_id == actor
                                && e.unit.current_str_hp > 0
                                && e.unit.current_str_hp < e.unit.max_str_hp
                        }) {
                            let boost = if ally.card.combat_class.is_building() { 2 } else { 1 };
                            ally.unit.current_str_hp =
                                (ally.unit.current_str_hp + boost).min(ally.unit.max_str_hp);
                        }
                    }
                    OriginAllegiance::Africa | OriginAllegiance::Asia => {
                        for e in &mut self.entities {
                            if e.team_id != actor
                                && e.unit.current_str_hp > 0
                                && e.hex.unsigned_distance_to(center) <= 2
                            {
                                e.unit.target_locked = true;
                            }
                        }
                    }
                    OriginAllegiance::Europe | OriginAllegiance::NorthAmerica => {
                        if let Some(ally) = self.entities.iter_mut().find(|e| {
                            e.team_id == actor
                                && e.unit.current_str_hp > 0
                                && !e.card.combat_class.is_building()
                        }) {
                            ally.unit.fortified_stacks = (ally.unit.fortified_stacks + 1).min(3);
                            ally.unit.current_str_hp =
                                (ally.unit.current_str_hp + 1).min(ally.unit.max_str_hp);
                        }
                    }
                    OriginAllegiance::SouthAmerica => {
                        for e in &mut self.entities {
                            if e.team_id == actor && e.unit.current_str_hp > 0 {
                                e.unit.poisoned_stacks = 0;
                                e.unit.rooted_turns = 0;
                                if !e.card.combat_class.is_building() {
                                    e.unit.current_str_hp =
                                        (e.unit.current_str_hp + 1).min(e.unit.max_str_hp);
                                }
                            }
                        }
                    }
                    OriginAllegiance::Neutral => {}
                }
                true
            }
            SimAction::PrimalSurge { .. } => {
                if ap == 0 || self.economy.primal_surge_used.get(&actor).copied().unwrap_or(false) {
                    return false;
                }
                let bcard = self.decks[(actor - 1) as usize].primary_biotype_card();
                let m_cost = u16::from(bcard.primal_surge_cost.1);
                let home = self.economy.home_base_munitions.get(&actor).copied().unwrap_or(0);
                if home < m_cost {
                    return false;
                }
                *self.economy.home_base_munitions.entry(actor).or_insert(0) -= m_cost;
                *self.economy.actions_remaining.entry(actor).or_insert(1) -= 1;
                self.economy.primal_surge_used.insert(actor, true);
                for e in &mut self.entities {
                    if e.team_id == actor && e.unit.current_str_hp > 0 && !e.card.combat_class.is_building() {
                        e.unit.temp_move_bonus = e.unit.temp_move_bonus.saturating_add(1);
                    }
                }
                true
            }
            SimAction::RestUnit { uid } => {
                if ap == 0 {
                    return false;
                }
                let Some(idx) = self.entity_by_uid(*uid) else {
                    return false;
                };
                if self.entities[idx].team_id != actor
                    || self.entities[idx].card.combat_class.is_building()
                    || self.entities[idx].unit.current_str_hp >= self.entities[idx].unit.max_str_hp
                {
                    return false;
                }
                *self.economy.actions_remaining.entry(actor).or_insert(1) -= 1;
                let deck = &self.decks[(actor - 1) as usize];
                let mend = if deck.is_pure_biotype()
                    && deck.primary_faction == OrganismFaction::Reptile
                    && self.entities[idx].card.faction == OrganismFaction::Reptile
                {
                    2
                } else {
                    1
                };
                let u = &mut self.entities[idx].unit;
                u.current_str_hp = (u.current_str_hp + mend).min(u.max_str_hp);
                true
            }
        };

        if applied && self.record_replay {
            let post_hash = self.compute_state_hash();
            self.steps.push(RecordedActionStep {
                turn_number: self.turn_number,
                actor_team: actor,
                action,
                post_state_hash: post_hash,
            });
        }
        applied
    }

    fn resolve_tactical_card(&mut self, actor: u8, card: &UnitCardDef, target_hex: Hex) {
        if card.card_key == "Hydro-Breach Canal" {
            let enemy_hq = self
                .entities
                .iter()
                .find(|e| e.team_id != actor && e.card.combat_class == CombatClass::Installation)
                .map(|e| e.hex)
                .unwrap_or(Hex::ZERO);
            if let Some(t) = self.scenario.map.tiles.get_mut(&target_hex) {
                t.morphology = Morphology::ShallowFord;
                t.elevation = 0;
            }
            let mut nbs = target_hex.all_neighbors().to_vec();
            nbs.sort_by_key(|nb| nb.unsigned_distance_to(enemy_hq));
            for nb in nbs {
                if let Some(nt) = self.scenario.map.tiles.get_mut(&nb) {
                    if matches!(nt.morphology, Morphology::Flat | Morphology::RoughCover | Morphology::RadiationFallout) {
                        nt.morphology = Morphology::ShallowFord;
                        nt.elevation = 0;
                        break;
                    }
                }
            }
            return;
        }
        if card.action_bonus > 0 {
            *self.economy.actions_remaining.entry(actor).or_insert(0) += card.action_bonus;
        }
        for e in &mut self.entities {
            if e.unit.current_str_hp == 0 || e.hex.unsigned_distance_to(target_hex) > 1 {
                continue;
            }
            if (card.heal_repair_range > 0 || card.shield > 0) && e.team_id == actor {
                if card.heal_repair_range > 0 && !e.card.combat_class.is_building() {
                    e.unit.current_str_hp = (e.unit.current_str_hp + 2).min(e.unit.max_str_hp);
                    e.unit.poisoned_stacks = 0;
                }
                if card.shield > 0 {
                    e.unit.fortified_stacks = (e.unit.fortified_stacks + card.shield).min(3);
                }
            } else if card.heal_repair_range == 0 && card.shield == 0 && e.team_id != actor && e.team_id != 0 {
                let dmg = if e.card.combat_class.is_building() {
                    1
                } else {
                    card.strength_hp.min(3)
                };
                if e.unit.fortified_stacks > 0 {
                    e.unit.fortified_stacks -= 1;
                } else {
                    let abs = dmg.min(e.unit.current_armor);
                    e.unit.current_armor -= abs;
                    e.unit.current_str_hp = e.unit.current_str_hp.saturating_sub(dmg - abs);
                }
                if card.poison_stacks > 0 && !e.card.poison_immune {
                    e.unit.poisoned_stacks = (e.unit.poisoned_stacks + card.poison_stacks).min(3);
                }
                if card.root_stacks > 0 {
                    e.unit.rooted_turns = e.unit.rooted_turns.max(1);
                }
            }
        }
        self.enqueue_destroyed_units();
        self.check_immediate_hq_victory();
    }

    pub(crate) fn enqueue_destroyed_units(&mut self) {
        for e in &mut self.entities {
            if e.unit.current_str_hp == 0
                && !matches!(
                    e.card.graveyard_queue,
                    GraveyardFacility::BaseCore | GraveyardFacility::Consumable
                )
            {
                let tid = e.team_id;
                if tid == 1 || tid == 2 {
                    let deck = &self.decks[(tid - 1) as usize];
                    let copies = deck
                        .entries
                        .iter()
                        .find(|en| en.card_key == e.card.card_key)
                        .map(|en| en.quantity)
                        .unwrap_or(1);
                    let per_copy = if deck.is_pure_biotype()
                        && deck.primary_faction == OrganismFaction::Insect
                        && matches!(e.card.combat_class, CombatClass::Light | CombatClass::Medium)
                    {
                        2
                    } else {
                        1
                    };
                    let cap = copies.saturating_mul(per_copy);
                    let rk = format!("{}:{}", tid, e.card.card_key);
                    let used = self.economy.revives_used.get(&rk).copied().unwrap_or(0);
                    if used < cap {
                        self.economy.revives_used.insert(rk, used + 1);
                        self.graveyards[(tid - 1) as usize].push(GraveyardTicket {
                            card_key: e.card.card_key.clone(),
                            facility: e.card.graveyard_queue,
                            recovered_str: 0,
                            target_str: e.card.strength_hp,
                        });
                    }
                }
            }
        }
        self.entities.retain(|e| e.unit.current_str_hp > 0);
    }

    pub(crate) fn check_immediate_hq_victory(&mut self) {
        let p1_hq_alive = self
            .entities
            .iter()
            .any(|e| e.team_id == 1 && e.unit.current_str_hp > 0 && e.card.combat_class == CombatClass::Installation);
        let p2_hq_alive = self
            .entities
            .iter()
            .any(|e| e.team_id == 2 && e.unit.current_str_hp > 0 && e.card.combat_class == CombatClass::Installation);

        if !p2_hq_alive && p1_hq_alive {
            self.winner_team = Some(1);
            self.win_reason = "Player 1 razed Enemy Command Center".into();
        } else if !p1_hq_alive && p2_hq_alive {
            self.winner_team = Some(2);
            self.win_reason = "Player 2 razed Enemy Command Center".into();
        } else if !p1_hq_alive && !p2_hq_alive {
            self.winner_team = Some(self.active_team);
            self.win_reason = "Simultaneous HQ collapse resolved to active attacker".into();
        }
    }

    fn advance_turn(&mut self) {
        if self.active_team == 1 {
            self.active_team = 2;
            if self.turn_number == 1 {
                // Player 2's very first turn keeps pre-seeded 8 Munitions & strict 2 Actions!
                self.economy.actions_remaining.insert(2, 2);
                self.economy.faction_ability_used.insert(2, false);
                self.economy.primal_surge_used.insert(2, false);
            } else {
                self.start_team_upkeep(2);
            }
        } else {
            // Equal-Turn End-of-Full-Round Objective Evaluation + Turn 30+ Orbital Fallout Storm!
            self.evaluate_end_of_round();
            if self.winner_team.is_none() {
                self.turn_number += 1;
                self.active_team = 1;
                self.start_team_upkeep(1);
            }
        }
        self.record_telemetry_point();
    }

    fn start_team_upkeep(&mut self, team_id: u8) {
        {
            let mut pairs: Vec<(PlayerOwner, &mut BoardUnit)> = self
                .entities
                .iter_mut()
                .map(|e| (PlayerOwner(e.team_id), &mut e.unit))
                .collect();
            begin_player_turn(team_id, &mut self.economy, &mut pairs);
        }
        // Africa / Asia every-3rd-turn bonus Munition
        let origin = self.decks[(team_id - 1) as usize].faction;
        if matches!(origin, OriginAllegiance::Africa | OriginAllegiance::Asia)
            && self.turn_number % 3 == 0
        {
            *self.economy.home_base_munitions.entry(team_id).or_insert(0) += 1;
        }
        self.enqueue_destroyed_units();
        self.check_immediate_hq_victory();
        if self.winner_team.is_some() {
            return;
        }

        let p = (team_id - 1) as usize;
        if self.hands[p].len() < 7 {
            if let Some(ck) = self.draw_piles[p].pop() {
                self.hands[p].push(ck);
            }
        }
        let supply = self.team_supply_hexes(team_id);
        let has_hosp = self.entities.iter().any(|e| {
            e.team_id == team_id && e.card.card_key == "Hospital" && supply.contains(&e.hex)
        });
        let has_fact = self.entities.iter().any(|e| {
            e.team_id == team_id && e.card.card_key == "Factory" && supply.contains(&e.hex)
        });

        let mut rem = Vec::new();
        let mut bays = 0u8;
        for mut t in self.graveyards[p].drain(..) {
            let can_bay = (t.facility == GraveyardFacility::HospitalBio && has_hosp)
                || (t.facility == GraveyardFacility::FactoryMech && has_fact);
            let cur_m = self.economy.home_base_munitions.get(&team_id).copied().unwrap_or(0);
            if can_bay && bays < 2 && cur_m >= 1 {
                *self.economy.home_base_munitions.entry(team_id).or_insert(0) -= 1;
                bays += 1;
                if t.tick_recovery(has_hosp, has_fact) {
                    self.hands[p].push(t.card_key);
                } else {
                    rem.push(t);
                }
            } else {
                rem.push(t);
            }
        }
        self.graveyards[p] = rem;
        self.refresh_confusion_states();
    }

    fn weighted_hex_control(&self, obj_hex: Hex) -> Option<u8> {
        let mut w = [0u16; 2];
        for e in &self.entities {
            if e.unit.current_str_hp == 0 || e.card.combat_class.is_building() || e.card.card_key == "Cargo Drop" {
                continue;
            }
            let d = e.hex.unsigned_distance_to(obj_hex);
            if d == 0 && (e.team_id == 1 || e.team_id == 2) {
                w[(e.team_id - 1) as usize] += 2;
            } else if d == 1 && (e.team_id == 1 || e.team_id == 2) {
                w[(e.team_id - 1) as usize] += 1;
            }
        }
        if w[0] > w[1] {
            Some(1)
        } else if w[1] > w[0] {
            Some(2)
        } else {
            None
        }
    }

    fn evaluate_end_of_round(&mut self) {
        self.check_immediate_hq_victory();
        if self.winner_team.is_some() {
            return;
        }
        let obj_hexes: Vec<Hex> = self
            .scenario
            .objective_hexes
            .iter()
            .map(|&(q, r)| Hex::new(q, r))
            .collect();

        match self.scenario.victory.clone() {
            VictoryCondition::DestroyEnemyCommandCenter => {}
            VictoryCondition::HoldVaultTurns { target_turns } => {
                if let Some(&vh) = obj_hexes.first() {
                    if let Some(ctrl) = self.weighted_hex_control(vh) {
                        let slot = (ctrl - 1) as usize;
                        self.obj_progress[slot] += 1;
                        if self.obj_progress[slot] >= u16::from(target_turns) {
                            self.winner_team = Some(ctrl);
                            self.win_reason = format!("Player {} held Central Vault for {} rounds", ctrl, target_turns);
                            return;
                        }
                    }
                }
            }
            VictoryCondition::HoldCentralControlSpire { target_turns } => {
                if let Some(&sh) = obj_hexes.first() {
                    if let Some(ctrl) = self.weighted_hex_control(sh) {
                        let slot = (ctrl - 1) as usize;
                        self.obj_progress[slot] += 1;
                        if self.obj_progress[slot] >= u16::from(target_turns) {
                            self.winner_team = Some(ctrl);
                            self.win_reason = format!("Player {} controlled Central Spire for {} rounds", ctrl, target_turns);
                            return;
                        }
                    }
                }
            }
            VictoryCondition::SalvageMunitionsOrRaze { target_munitions } => {
                for &ch in &obj_hexes {
                    if let Some(ctrl) = self.weighted_hex_control(ch) {
                        self.obj_progress[(ctrl - 1) as usize] += 2;
                    }
                }
                if self.obj_progress[0] >= target_munitions || self.obj_progress[1] >= target_munitions {
                    let w = if self.obj_progress[0] >= self.obj_progress[1] { 1 } else { 2 };
                    self.winner_team = Some(w);
                    self.win_reason = format!("Player {} harvested {} Salvage Munitions", w, target_munitions);
                    return;
                }
            }
            VictoryCondition::ControlIslandRelays {
                required_relays,
                target_turns,
            } => {
                let mut r_cnt = [0u8; 2];
                for &rh in &obj_hexes {
                    if let Some(ctrl) = self.weighted_hex_control(rh) {
                        r_cnt[(ctrl - 1) as usize] += 1;
                    }
                }
                for tid in [1u8, 2u8] {
                    let slot = (tid - 1) as usize;
                    if r_cnt[slot] >= required_relays {
                        self.obj_progress[slot] += 1;
                        if self.obj_progress[slot] >= u16::from(target_turns) {
                            self.winner_team = Some(tid);
                            self.win_reason = format!("Player {} held Atoll Relays for {} rounds", tid, target_turns);
                            return;
                        }
                    }
                }
            }
        }

        // Round 30+ Orbital Fallout Storm Sudden-Death Clock
        if self.turn_number >= 30 {
            let storm_dmg: u8 = if self.turn_number >= 36 { 2 } else { 1 };
            for e in &mut self.entities {
                if e.card.combat_class == CombatClass::Installation && e.unit.current_str_hp > 0 {
                    e.unit.current_str_hp = e.unit.current_str_hp.saturating_sub(storm_dmg);
                }
            }
            let p1_hq = self
                .entities
                .iter()
                .find(|e| e.team_id == 1 && e.card.combat_class == CombatClass::Installation)
                .map(|e| e.unit.current_str_hp)
                .unwrap_or(0);
            let p2_hq = self
                .entities
                .iter()
                .find(|e| e.team_id == 2 && e.card.combat_class == CombatClass::Installation)
                .map(|e| e.unit.current_str_hp)
                .unwrap_or(0);

            if p1_hq == 0 || p2_hq == 0 {
                if p1_hq > p2_hq {
                    self.winner_team = Some(1);
                    self.win_reason = "Orbital Fallout Storm razed Player 2 Command Center".into();
                } else if p2_hq > p1_hq {
                    self.winner_team = Some(2);
                    self.win_reason = "Orbital Fallout Storm razed Player 1 Command Center".into();
                } else {
                    // Tiebreak on objective progress -> total mobile army STR -> P2 tiebreaker
                    let p1_army: u16 = self
                        .entities
                        .iter()
                        .filter(|e| e.team_id == 1 && !e.card.combat_class.is_building())
                        .map(|e| u16::from(e.unit.current_str_hp))
                        .sum();
                    let p2_army: u16 = self
                        .entities
                        .iter()
                        .filter(|e| e.team_id == 2 && !e.card.combat_class.is_building())
                        .map(|e| u16::from(e.unit.current_str_hp))
                        .sum();
                    let w = if (self.obj_progress[0], p1_army) > (self.obj_progress[1], p2_army) {
                        1
                    } else {
                        2
                    };
                    self.winner_team = Some(w);
                    self.win_reason = format!("Orbital Fallout Storm tiebreak won by Player {}", w);
                }
            }
        }
    }

    pub fn export_replay(&self) -> MatchReplayFile {
        MatchReplayFile {
            engine_version: "3.4.1".into(),
            scenario_id: self.scenario.scenario_id,
            seed: self.seed,
            decks: self.decks.clone(),
            initial_state_hash: self.initial_state_hash,
            steps: self.steps.clone(),
            telemetry: self.telemetry.clone(),
            winner_team: self.winner_team,
            win_reason: self.win_reason.clone(),
            total_turns: self.turn_number,
        }
    }

    pub fn save_replay_ron(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let replay = self.export_replay();
        let pretty = PrettyConfig::new().depth_limit(4).separate_tuple_members(true);
        let raw = to_string_pretty(&replay, pretty)?;
        fs::write(path, raw)?;
        Ok(())
    }

    pub fn verify_replay_file(roster: RosterManifest, path: &Path) -> Result<MatchReplayFile> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("Failed reading replay file {}", path.display()))?;
        let replay: MatchReplayFile = ron::from_str(&raw)?;
        Self::verify_replay_data(roster, &replay)?;
        Ok(replay)
    }

    pub fn verify_replay_data(roster: RosterManifest, replay: &MatchReplayFile) -> Result<()> {
        let mut sim = HeadlessMatch::new(
            roster,
            replay.decks.clone(),
            replay.scenario_id,
            replay.seed,
            false,
        );
        if sim.initial_state_hash != replay.initial_state_hash {
            return Err(anyhow!(
                "Initial state hash mismatch: sim=0x{:016X}, replay=0x{:016X}",
                sim.initial_state_hash,
                replay.initial_state_hash
            ));
        }
        for (idx, step) in replay.steps.iter().enumerate() {
            if !sim.apply_action(step.action.clone()) {
                return Err(anyhow!(
                    "Step {} ({:?}) rejected during deterministic replay verification",
                    idx,
                    step.action
                ));
            }
            let cur_hash = sim.compute_state_hash();
            if cur_hash != step.post_state_hash {
                return Err(anyhow!(
                    "State desync at step {} (turn {}): sim=0x{:016X}, replay=0x{:016X}",
                    idx,
                    step.turn_number,
                    cur_hash,
                    step.post_state_hash
                ));
            }
        }
        Ok(())
    }
}
