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

//! Playable turn-based hexagonal tactical session supporting:
//! - Solo vs Tactical Deck-Playing AI AND 1v1 Local Hotseat Mode (`M`),
//! - Sight, MountainPeak LoS, `Radar Range` True-Sight & `Stealth` Fog-of-War (`F`),
//! - Card deployment, BFS movement, Control Node & Supply Chain validation, D6 combat,
//! - 5-Scenario Victory/Defeat evaluation & `F5`/`F9` `.ron` Quicksave/Quickload.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;
use hexx::Hex;
use ron::ser::{to_string_pretty, PrettyConfig};
use serde::{Deserialize, Serialize};
use wf_core::abilities::{execute_field_heal, execute_venom_spit, execute_web_snare};
use wf_core::combat::{required_ranged_hit_roll, resolve_attack, CombatContext, GraveyardTicket};
use wf_core::logistics::{
    begin_player_turn, compute_home_supply_coverage, is_within_control_node, BoardUnit, HexPos,
    PlayerEconomy, PlayerOwner,
};
use wf_core::mapgen::SplitMix64;
use wf_core::{OriginAllegiance, 
    generate_scenario, CombatClass, DeckList, GeneratedScenario, GraveyardFacility, Morphology,
    OrganismFaction, RosterManifest, ScenarioId, UnitCardDef, VictoryCondition,
};
use crate::audio::{SfxCue, SpatialAudioEngine};
use crate::net_lockstep::{LockstepCommand, StampedLockstepCommand};
use crate::picking::hex_world_anchor;
use crate::scene::axial_to_world_xz;
use crate::vfx::VfxManager;

pub const DEFAULT_QUICKSAVE_PATH: &str = "saves/quicksave.ron";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandMode {
    Inspect,
    DeployHandCard(usize),
    MoveSelectedUnit,
    AttackWithSelectedUnit,
    AbilityWithSelectedUnit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveEntity {
    pub uid: u64,
    pub team_id: u8,
    pub hex: Hex,
    pub vox_slug: String,
    pub yaw_radians: f32,
    pub card: UnitCardDef,
    pub unit: BoardUnit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchOutcome {
    pub winner_team: u8,
    pub headline: String,
    pub reason: String,
    pub turns_taken: u32,
    pub units_deployed: u32,
    pub enemies_destroyed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedMatchSnapshot {
    pub active_seed: u64,
    pub active_deck_preset: usize,
    pub decks: [DeckList; 2],
    pub scenario: GeneratedScenario,
    pub turn_number: u32,
    pub active_team: u8,
    pub hotseat_mode: bool,
    pub fog_enabled: bool,
    pub explored_hexes: [HashSet<Hex>; 2],
    #[serde(default)]
    pub breached_canal_hexes: HashSet<Hex>,
    pub economy: PlayerEconomy,
    pub draw_pile: Vec<String>,
    pub hand: Vec<String>,
    pub graveyard: Vec<GraveyardTicket>,
    pub ai_draw_pile: Vec<String>,
    pub ai_hand: Vec<String>,
    pub ai_graveyard: Vec<GraveyardTicket>,
    pub entities: Vec<LiveEntity>,
    pub next_uid: u64,
    pub combat_log: Vec<String>,
    pub dice_rng: SplitMix64,
    pub p1_obj_progress: u16,
    pub p2_obj_progress: u16,
    pub stats_units_deployed: u32,
    pub stats_enemies_destroyed: u32,
    pub match_outcome: Option<MatchOutcome>,
}

pub struct ClientMatchState {
    pub roster: RosterManifest,
    pub active_deck_preset: usize,
    pub decks: [DeckList; 2],
    pub scenario: GeneratedScenario,
    pub turn_number: u32,
    pub active_team: u8,
    pub online_local_team: Option<u8>,
    pub outbound_net_cmds: Vec<StampedLockstepCommand>,
    pub hotseat_mode: bool,
    pub hotseat_handoff_pending: bool,
    pub fog_enabled: bool,
    pub explored_hexes: [HashSet<Hex>; 2],
    pub breached_canal_hexes: HashSet<Hex>,
    pub economy: PlayerEconomy,
    pub draw_pile: Vec<String>,
    pub hand: Vec<String>,
    pub graveyard: Vec<GraveyardTicket>,
    pub ai_draw_pile: Vec<String>,
    pub ai_hand: Vec<String>,
    pub ai_graveyard: Vec<GraveyardTicket>,
    pub entities: Vec<LiveEntity>,
    pub next_uid: u64,
    pub hovered_hex: Option<Hex>,
    pub selected_hex: Option<Hex>,
    pub mode: CommandMode,
    pub combat_log: Vec<String>,
    pub dice_rng: SplitMix64,
    pub p1_obj_progress: u16,
    pub p2_obj_progress: u16,
    pub stats_units_deployed: u32,
    pub stats_enemies_destroyed: u32,
    pub match_outcome: Option<MatchOutcome>,
    pub outcome_modal_dismissed: bool,
    pub status_toast: Option<(String, f32)>,
    pub ai_difficulty: wf_sim::AiDifficulty,
    pub timeline_frames: Vec<SavedMatchSnapshot>,
}

pub fn card_to_vox_slug(card: &UnitCardDef) -> &'static str {
    card.default_vox_slug()
}

pub fn make_board_unit(card: &UnitCardDef) -> BoardUnit {
    BoardUnit {
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
    }
}

pub fn make_board_unit_for_deck(card: &UnitCardDef, deck: &DeckList) -> BoardUnit {
    let mut u = make_board_unit(card);
    if deck.faction == OriginAllegiance::Europe && card.combat_class.is_building() {
        u.fortified_stacks = (u.fortified_stacks + 1).min(3);
    }
    if deck.faction == OriginAllegiance::Australia && (card.combat_class.is_building() || card.emits_node) {
        u.node_emit_radius = u.node_emit_radius.saturating_add(1);
    }
    if deck.faction == OriginAllegiance::Asia && (card.faction == OrganismFaction::Human || card.card_key == "Drone") {
        u.node_leash_range = 0;
    }
    if deck.is_pure_biotype() {
        match deck.primary_faction {
            OrganismFaction::Mammal if card.faction == OrganismFaction::Mammal => {
                if card.combat_class == CombatClass::Light {
                    u.current_armor = u.current_armor.saturating_add(1);
                    u.base_armor = u.base_armor.saturating_add(1);
                } else if matches!(card.combat_class, CombatClass::Medium | CombatClass::Heavy) {
                    u.current_str_hp = u.current_str_hp.saturating_add(1);
                    u.max_str_hp = u.max_str_hp.saturating_add(1);
                }
            }
            OrganismFaction::Plant if card.faction == OrganismFaction::Plant => {
                u.fortified_stacks = (u.fortified_stacks + 1).min(3);
                u.current_str_hp = u.current_str_hp.saturating_add(1);
                u.max_str_hp = u.max_str_hp.saturating_add(1);
            }
            OrganismFaction::Reptile if card.faction == OrganismFaction::Reptile => {
                if !card.combat_class.is_building() {
                    u.current_armor = u.current_armor.saturating_add(1);
                    u.base_armor = u.base_armor.saturating_add(1);
                }
            }
            OrganismFaction::Bird if card.faction == OrganismFaction::Bird => {
                u.evade = u.evade.saturating_add(1);
            }
            OrganismFaction::Fish if card.faction == OrganismFaction::Fish => {
                // +1 Swim MOVE while submerged water grants natural cover evade & +1 STR gill recovery!
                u.temp_move_bonus = u.temp_move_bonus.saturating_add(1);
            }
            OrganismFaction::Arachnid if card.faction == OrganismFaction::Arachnid => {
                u.evade = u.evade.saturating_add(1);
                u.fortified_stacks = (u.fortified_stacks + 1).min(3);
                if !card.combat_class.is_building() {
                    u.current_armor = u.current_armor.saturating_add(1);
                    u.base_armor = u.base_armor.saturating_add(1);
                    u.current_str_hp = u.current_str_hp.saturating_add(1);
                    u.max_str_hp = u.max_str_hp.saturating_add(1);
                    u.temp_move_bonus = u.temp_move_bonus.saturating_add(1);
                }
            }
            OrganismFaction::Insect if card.faction == OrganismFaction::Insect => {
                u.carried_munitions = u.carried_munitions.saturating_add(1);
                u.fortified_stacks = (u.fortified_stacks + 1).min(3);
                if !card.combat_class.is_building() {
                    u.current_armor = u.current_armor.saturating_add(1);
                    u.base_armor = u.base_armor.saturating_add(1);
                    u.current_str_hp = u.current_str_hp.saturating_add(1);
                    u.max_str_hp = u.max_str_hp.saturating_add(1);
                    u.temp_move_bonus = u.temp_move_bonus.saturating_add(1);
                }
            }
            OrganismFaction::Amphibian if card.faction == OrganismFaction::Amphibian => {
                u.evade = u.evade.saturating_add(1);
                if !card.combat_class.is_building() {
                    u.current_str_hp = u.current_str_hp.saturating_add(1);
                    u.max_str_hp = u.max_str_hp.saturating_add(1);
                }
            }
            OrganismFaction::Cnidarian if card.faction == OrganismFaction::Cnidarian => {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatOddsPreview {
    pub dist: u8,
    pub ctx: CombatContext,
    pub required_d6: Option<u8>,
    pub hit_summary: String,
    pub delta_str: i16,
    pub effective_armor: u8,
    pub pure_mammal_apex: bool,
    pub aegis_bastion: bool,
}

impl ClientMatchState {
    /// Builds authoritative `CombatContext` from live board state (shared between combat resolution and HUD hover preview).
    pub fn build_combat_context(&self, att_idx: usize, def_idx: usize) -> CombatContext {
        let att = &self.entities[att_idx];
        let def = &self.entities[def_idx];
        let att_team = att.team_id;
        let att_hex = att.hex;
        let def_hex = def.hex;
        let dist = att_hex.unsigned_distance_to(def_hex) as u8;
        let is_laser = att.unit.is_laser;
        let is_melee = (!def.card.can_fly || att.card.can_fly)
            && dist <= att.card.melee_range
            && (att.card.ranged_distance == 0 || (dist == 1 && !is_laser));

        let adj_cmd = self.entities.iter().any(|e| {
            e.team_id == att_team
                && e.unit.current_str_hp > 0
                && e.card.combat_class == CombatClass::Commander
                && e.hex.unsigned_distance_to(att_hex) == 1
        });
        let pack_bonus = att.card.faction == OrganismFaction::Mammal
            && self.entities.iter().any(|e| {
                e.team_id == att_team
                    && e.uid != att.uid
                    && e.unit.current_str_hp > 0
                    && e.card.faction == OrganismFaction::Mammal
                    && e.hex.unsigned_distance_to(att_hex) == 1
            });
        let brush_bypass = matches!(
            att.card.faction,
            OrganismFaction::Arachnid | OrganismFaction::Insect
        ) && dist <= 2;
        let def_cover = self
            .scenario
            .map
            .tiles
            .get(&def_hex)
            .map_or(false, |t| {
                (t.morphology.grants_ranged_cover()
                    && !(brush_bypass && t.morphology == Morphology::RoughCover))
                    || (def.card.faction == OrganismFaction::Fish && t.morphology.is_water())
            })
            && !def.card.can_fly;

        CombatContext {
            hex_distance: dist,
            is_melee,
            attacker_can_fly: att.card.can_fly,
            attacker_stealth_ambush: att.card.stealth,
            attacker_adjacent_commander: adj_cmd,
            attacker_pack_tactics_bonus: pack_bonus,
            defender_in_rough_cover: def_cover,
        }
    }

    /// Computes live hover hit-odds & damage modifier preview matching `execute_combat_pair` rules.
    pub fn preview_combat_odds(&self, att_idx: usize, def_idx: usize) -> CombatOddsPreview {
        let att = &self.entities[att_idx];
        let def = &self.entities[def_idx];
        let ctx = self.build_combat_context(att_idx, def_idx);
        let dist = ctx.hex_distance;

        let att_deck = &self.decks[(att.team_id.saturating_sub(1)).min(1) as usize];
        let def_deck = &self.decks[(def.team_id.saturating_sub(1)).min(1) as usize];

        let pure_mammal_apex = att_deck.is_pure_biotype()
            && att_deck.primary_faction == OrganismFaction::Mammal
            && ctx.attacker_pack_tactics_bonus;
        let aegis_bastion = def_deck.faction == OriginAllegiance::Europe
            && dist > 2
            && matches!(
                def.card.combat_class,
                CombatClass::Heavy | CombatClass::Commander
            );
        let effective_armor = def
            .unit
            .current_armor
            .saturating_add(u8::from(aegis_bastion));

        let mut d_str =
            (i16::from(att.unit.current_str_hp) - i16::from(def.unit.current_str_hp)).clamp(-2, 3);
        if ctx.attacker_can_fly && d_str < 0 {
            d_str = 0;
        }

        let (required_d6, hit_summary) = if def.unit.burrowed && !ctx.is_melee && dist >= 2 {
            (None, "IMMUNE (Burrowed)".to_string())
        } else if def.unit.fortified_stacks > 0 {
            (
                None,
                format!("BLOCKED (Shield x{})", def.unit.fortified_stacks),
            )
        } else if ctx.is_melee {
            (Some(1), "AUTO-HIT (Melee)".to_string())
        } else {
            let mut req = required_ranged_hit_roll(&def.unit, ctx);
            if pure_mammal_apex {
                req = req.saturating_sub(1).max(2);
            }
            (Some(req), format!("Hit {}+ on D6", req))
        };

        CombatOddsPreview {
            dist,
            ctx,
            required_d6,
            hit_summary,
            delta_str: d_str,
            effective_armor,
            pure_mammal_apex,
            aegis_bastion,
        }
    }
}

impl ClientMatchState {
    pub fn new(
        roster: RosterManifest,
        decks: [DeckList; 2],
        scenario_id: ScenarioId,
        seed: u64,
        deck_preset: usize,
    ) -> Self {
        let scenario = generate_scenario(scenario_id, seed);
        let mut state = Self {
            roster,
            active_deck_preset: deck_preset % 2,
            decks,
            scenario,
            turn_number: 1,
            active_team: 1,
            online_local_team: None,
            outbound_net_cmds: Vec::new(),
            hotseat_mode: false,
            hotseat_handoff_pending: false,
            fog_enabled: true,
            explored_hexes: [HashSet::new(), HashSet::new()],
            breached_canal_hexes: HashSet::new(),
            economy: PlayerEconomy::default(),
            draw_pile: Vec::new(),
            hand: Vec::new(),
            graveyard: Vec::new(),
            ai_draw_pile: Vec::new(),
            ai_hand: Vec::new(),
            ai_graveyard: Vec::new(),
            entities: Vec::new(),
            next_uid: 1,
            hovered_hex: None,
            selected_hex: None,
            mode: CommandMode::Inspect,
            combat_log: Vec::new(),
            dice_rng: SplitMix64::new(seed ^ 0xD1CE_2072),
            p1_obj_progress: 0,
            p2_obj_progress: 0,
            stats_units_deployed: 0,
            stats_enemies_destroyed: 0,
            match_outcome: None,
            outcome_modal_dismissed: false,
            status_toast: None,
            ai_difficulty: wf_sim::AiDifficulty::Grandmaster2Ply,
            timeline_frames: Vec::new(),
        };
        state.reset_from_current_scenario();
        state
    }

    /// Which team's camera / Fog-of-War / Card Hand perspective is currently rendered.
    pub fn viewing_team(&self) -> u8 {
        if let Some(net_team) = self.online_local_team {
            net_team
        } else if self.hotseat_mode {
            self.active_team
        } else {
            1
        }
    }

    pub fn can_local_player_act(&self) -> bool {
        if self.match_outcome.is_some() || self.hotseat_handoff_pending {
            return false;
        }
        if let Some(net_team) = self.online_local_team {
            self.active_team == net_team
        } else if self.hotseat_mode {
            true
        } else {
            self.active_team == 1
        }
    }

    pub fn active_hand(&self) -> &Vec<String> {
        if self.viewing_team() == 2 {
            &self.ai_hand
        } else {
            &self.hand
        }
    }

    pub fn active_draw_pile(&self) -> &Vec<String> {
        if self.viewing_team() == 2 {
            &self.ai_draw_pile
        } else {
            &self.draw_pile
        }
    }

    pub fn active_graveyard(&self) -> &Vec<GraveyardTicket> {
        if self.viewing_team() == 2 {
            &self.ai_graveyard
        } else {
            &self.graveyard
        }
    }

    pub fn opponent_hand_len(&self) -> usize {
        if self.viewing_team() == 2 {
            self.hand.len()
        } else {
            self.ai_hand.len()
        }
    }

    pub fn opponent_draw_len(&self) -> usize {
        if self.viewing_team() == 2 {
            self.draw_pile.len()
        } else {
            self.ai_draw_pile.len()
        }
    }

    pub fn tick_toast(&mut self, dt: f32) {
        if let Some((_, rem)) = self.status_toast.as_mut() {
            *rem -= dt;
            if *rem <= 0.0 {
                self.status_toast = None;
            }
        }
    }

    pub fn push_toast(&mut self, msg: impl Into<String>) {
        self.status_toast = Some((msg.into(), 3.8));
    }

    pub fn capture_snapshot(&self, active_seed: u64) -> SavedMatchSnapshot {
        SavedMatchSnapshot {
            active_seed,
            active_deck_preset: self.active_deck_preset,
            decks: self.decks.clone(),
            scenario: self.scenario.clone(),
            turn_number: self.turn_number,
            active_team: self.active_team,
            hotseat_mode: self.hotseat_mode,
            fog_enabled: self.fog_enabled,
            explored_hexes: self.explored_hexes.clone(),
            breached_canal_hexes: self.breached_canal_hexes.clone(),
            economy: self.economy.clone(),
            draw_pile: self.draw_pile.clone(),
            hand: self.hand.clone(),
            graveyard: self.graveyard.clone(),
            ai_draw_pile: self.ai_draw_pile.clone(),
            ai_hand: self.ai_hand.clone(),
            ai_graveyard: self.ai_graveyard.clone(),
            entities: self.entities.clone(),
            next_uid: self.next_uid,
            combat_log: self.combat_log.clone(),
            dice_rng: self.dice_rng.clone(),
            p1_obj_progress: self.p1_obj_progress,
            p2_obj_progress: self.p2_obj_progress,
            stats_units_deployed: self.stats_units_deployed,
            stats_enemies_destroyed: self.stats_enemies_destroyed,
            match_outcome: self.match_outcome.clone(),
        }
    }

    pub fn push_timeline_keyframe(&mut self) {
        let seed = self.scenario.seed;
        if self.timeline_frames.len() >= 128 {
            self.timeline_frames.remove(0);
        }
        self.timeline_frames.push(self.capture_snapshot(seed));
    }

    pub fn restore_snapshot_preserving_timeline(&mut self, snap: SavedMatchSnapshot) {
        self.active_deck_preset = snap.active_deck_preset;
        self.decks = snap.decks;
        self.scenario = snap.scenario;
        self.turn_number = snap.turn_number;
        self.active_team = snap.active_team;
        self.hotseat_mode = snap.hotseat_mode;
        self.hotseat_handoff_pending = false;
        self.fog_enabled = snap.fog_enabled;
        self.explored_hexes = snap.explored_hexes;
        self.breached_canal_hexes = snap.breached_canal_hexes;
        self.economy = snap.economy;
        self.draw_pile = snap.draw_pile;
        self.hand = snap.hand;
        self.graveyard = snap.graveyard;
        self.ai_draw_pile = snap.ai_draw_pile;
        self.ai_hand = snap.ai_hand;
        self.ai_graveyard = snap.ai_graveyard;
        self.entities = snap.entities;
        self.next_uid = snap.next_uid;
        self.combat_log = snap.combat_log;
        self.dice_rng = snap.dice_rng;
        self.p1_obj_progress = snap.p1_obj_progress;
        self.p2_obj_progress = snap.p2_obj_progress;
        self.stats_units_deployed = snap.stats_units_deployed;
        self.stats_enemies_destroyed = snap.stats_enemies_destroyed;
        self.match_outcome = snap.match_outcome;
        self.outcome_modal_dismissed = false;
        self.mode = CommandMode::Inspect;
        self.selected_hex = None;
        self.refresh_confusion_states();
        self.update_explored_fog();
    }

    pub fn load_custom_generated_scenario(&mut self, custom: GeneratedScenario) {
        let seed = custom.seed;
        self.scenario = custom;
        self.dice_rng = SplitMix64::new(seed ^ 0xD1CE_2072);
        self.reset_from_current_scenario();
    }

    pub fn load_scenario(&mut self, scenario_id: ScenarioId, seed: u64) {
        self.scenario = generate_scenario(scenario_id, seed);
        self.dice_rng = SplitMix64::new(seed ^ 0xD1CE_2072);
        self.reset_from_current_scenario();
    }

    pub fn switch_deck_preset(&mut self, preset_idx: usize) {
        self.active_deck_preset = preset_idx % 2;
        self.reset_from_current_scenario();
    }

    pub fn apply_custom_deck(&mut self, slot_idx: usize, custom: DeckList) {
        let target = slot_idx % 2;
        self.decks[target] = custom;
        if target == 0 {
            self.active_deck_preset = 0;
        }
        self.reset_from_current_scenario();
    }

    fn build_shuffled_deck_zone(deck: &DeckList, rng: &mut SplitMix64) -> (Vec<String>, Vec<String>) {
        let mut pile = Vec::new();
        for entry in &deck.entries {
            if entry.card_key.starts_with("Command Center") {
                continue;
            }
            for _ in 0..entry.quantity {
                pile.push(entry.card_key.clone());
            }
        }
        for i in (1..pile.len()).rev() {
            let j = (rng.next_u64() as usize) % (i + 1);
            pile.swap(i, j);
        }
        let mut hand = Vec::with_capacity(5);
        for _ in 0..5 {
            if let Some(c) = pile.pop() {
                hand.push(c);
            }
        }
        (pile, hand)
    }

    fn reset_from_current_scenario(&mut self) {
        self.timeline_frames.clear();
        self.entities.clear();
        self.next_uid = 1;
        self.graveyard.clear();
        self.ai_graveyard.clear();
        self.explored_hexes = [HashSet::new(), HashSet::new()];
        self.breached_canal_hexes.clear();
        self.combat_log.clear();
        self.turn_number = 1;
        self.active_team = 1;
        self.hotseat_handoff_pending = false;
        self.mode = CommandMode::Inspect;
        self.selected_hex = None;
        self.p1_obj_progress = 0;
        self.p2_obj_progress = 0;
        self.stats_units_deployed = 0;
        self.stats_enemies_destroyed = 0;
        self.match_outcome = None;
        self.outcome_modal_dismissed = false;

        let lookup: std::collections::BTreeMap<String, UnitCardDef> = self
            .roster
            .units
            .iter()
            .map(|u| (u.card_key.clone(), u.clone()))
            .collect();

        for p in &self.scenario.placements {
            if let Some(card) = lookup.get(&p.card_key) {
                let uid = self.next_uid;
                self.next_uid += 1;
                let p_deck = &self.decks[(p.team_id - 1).min(1) as usize];
                let u = make_board_unit_for_deck(card, p_deck);

                self.entities.push(LiveEntity {
                    uid,
                    team_id: p.team_id,
                    hex: Hex::new(p.q, p.r),
                    vox_slug: p.vox_slug.clone(),
                    yaw_radians: p.yaw_radians,
                    card: card.clone(),
                    unit: u,
                });
            }
        }

        let p1_deck = self.decks[self.active_deck_preset % 2].clone();
        let p2_deck = self.decks[1 - (self.active_deck_preset % 2)].clone();
        let (p1_pile, p1_hand) = Self::build_shuffled_deck_zone(&p1_deck, &mut self.dice_rng);
        let (p2_pile, p2_hand) = Self::build_shuffled_deck_zone(&p2_deck, &mut self.dice_rng);
        self.draw_pile = p1_pile;
        self.hand = p1_hand;
        self.ai_draw_pile = p2_pile;
        self.ai_hand = p2_hand;
        if let Some(extra_c) = self.ai_draw_pile.pop() {
            self.ai_hand.push(extra_c);
        }

        self.economy = PlayerEconomy::default();
        self.economy.home_base_munitions.insert(1, 5);
        self.economy.home_base_munitions.insert(2, 8);
        self.start_team_turn(1);
        self.update_explored_fog();
        self.combat_log.push(format!(
            "Loaded {} | P1: {} vs P2: {}.",
            self.scenario.title, p1_deck.deck_name, p2_deck.deck_name
        ));
        self.combat_log.push(format!(
            "P1 Command: [{:?}] • {}",
            p1_deck.faction,
            if p1_deck.is_pure_biotype() { format!("Pure {:?}", p1_deck.primary_faction) } else { format!("{:?}+{:?}", p1_deck.primary_faction, p1_deck.secondary_faction) }
        ));
        self.combat_log.push(format!(
            "P2 Command: [{:?}] • {}",
            p2_deck.faction,
            if p2_deck.is_pure_biotype() { format!("Pure {:?}", p2_deck.primary_faction) } else { format!("{:?}+{:?}", p2_deck.primary_faction, p2_deck.secondary_faction) }
        ));
    }

    // =========================================================================
    // FOG-OF-WAR, MOUNTAIN LINE-OF-SIGHT, `RADAR RANGE` & `STEALTH` DETECTION
    // =========================================================================

    /// Computes effective Fog-of-War sight radius in hexes for a single unit/building.
    pub fn unit_vision_radius(&self, ent: &LiveEntity) -> u8 {
        if ent.unit.confused {
            return 1;
        }
        let base: u8 = match ent.card.combat_class {
            CombatClass::Installation => 3,
            CombatClass::ControlNode
            | CombatClass::SupportInstallation
            | CombatClass::SupplyDepot => 2,
            _ if ent.card.can_fly => 4,
            _ => 2u8.max(ent.card.ranged_distance),
        };
        let elev_bonus = if !ent.card.can_fly {
            self.scenario
                .map
                .tiles
                .get(&ent.hex)
                .map_or(0, |t| u8::from(t.elevation >= 1))
        } else {
            0
        };
        (base + elev_bonus).max(ent.card.radar_range).min(8)
    }

    /// Ground units without `can_fly` or `radar_range > 0` have direct sight blocked by
    /// intermediate `MountainPeak` hexes along the world segment `from -> target`.
    fn has_clear_sight_line(&self, observer: &LiveEntity, target: Hex) -> bool {
        let dist = observer.hex.unsigned_distance_to(target);
        if dist <= 1 || observer.card.can_fly || observer.card.radar_range > 0 {
            return true;
        }
        let (ax, az) = axial_to_world_xz(observer.hex);
        let (bx, bz) = axial_to_world_xz(target);
        let steps = (dist * 2).max(2);
        for s in 1..steps {
            let t = (s as f32) / (steps as f32);
            let mx = ax + (bx - ax) * t;
            let mz = az + (bz - az) * t;
            let mid_hex = crate::picking::world_xz_to_axial_hex(mx, mz, crate::scene::HEX_WORLD_RADIUS);
            if mid_hex != observer.hex && mid_hex != target {
                if let Some(tile) = self.scenario.map.tiles.get(&mid_hex) {
                    if tile.morphology == Morphology::MountainPeak {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Returns all hexes actively illuminated by `team_id`'s live units and radar installations.
    pub fn compute_visible_hexes(&self, team_id: u8) -> HashSet<Hex> {
        let mut vis = HashSet::new();
        for ent in &self.entities {
            if ent.team_id != team_id || ent.unit.current_str_hp == 0 {
                continue;
            }
            let r = self.unit_vision_radius(ent);
            for cand in ent.hex.range(u32::from(r)) {
                if self.scenario.map.tiles.contains_key(&cand)
                    && self.has_clear_sight_line(ent, cand)
                {
                    vis.insert(cand);
                }
            }
        }
        vis
    }

    /// Returns all hexes where `team_id` possesses True-Sight anti-Stealth detection:
    /// - Adjacent hexes (`distance <= 1`) around any friendly unit/building, OR
    /// - Within `radar_range` of any unconfused friendly unit/installation with `radar_range > 0`.
    pub fn compute_true_sight_hexes(&self, team_id: u8) -> HashSet<Hex> {
        let mut det = HashSet::new();
        for ent in &self.entities {
            if ent.team_id != team_id || ent.unit.current_str_hp == 0 {
                continue;
            }
            for nb in ent.hex.range(1) {
                det.insert(nb);
            }
            if !ent.unit.confused && ent.card.radar_range > 0 {
                for rh in ent.hex.range(u32::from(ent.card.radar_range)) {
                    det.insert(rh);
                }
            }
        }
        det
    }

    pub fn update_explored_fog(&mut self) {
        let v1 = self.compute_visible_hexes(1);
        let v2 = self.compute_visible_hexes(2);
        self.explored_hexes[0].extend(v1);
        self.explored_hexes[1].extend(v2);
    }

    /// Evaluates whether `entities[ent_idx]` is visible to `viewer_team` under Fog-of-War & Stealth rules.
    pub fn is_entity_visible_to(
        &self,
        ent_idx: usize,
        viewer_team: u8,
        vis_hexes: &HashSet<Hex>,
        true_sight: &HashSet<Hex>,
    ) -> bool {
        let Some(ent) = self.entities.get(ent_idx) else {
            return false;
        };
        if ent.unit.current_str_hp == 0 {
            return false;
        }
        if !self.fog_enabled {
            return true;
        }
        if self.hotseat_handoff_pending {
            return false;
        }
        if ent.team_id == viewer_team || ent.team_id == 0 {
            return true;
        }
        if !vis_hexes.contains(&ent.hex) {
            return false;
        }
        let is_cloaked = (ent.card.stealth || ent.unit.burrowed)
            && !ent.unit.target_locked
            && !ent.unit.revealed_until_turn_start;
        if is_cloaked {
            return true_sight.contains(&ent.hex);
        }
        true
    }

    pub fn save_quicksave(&mut self, path: &Path, active_seed: u64) -> Result<String, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed creating save dir: {e}"))?;
        }
        let snap = SavedMatchSnapshot {
            active_seed,
            active_deck_preset: self.active_deck_preset,
            decks: self.decks.clone(),
            scenario: self.scenario.clone(),
            turn_number: self.turn_number,
            active_team: self.active_team,
            hotseat_mode: self.hotseat_mode,
            fog_enabled: self.fog_enabled,
            explored_hexes: self.explored_hexes.clone(),
            breached_canal_hexes: self.breached_canal_hexes.clone(),
            economy: self.economy.clone(),
            draw_pile: self.draw_pile.clone(),
            hand: self.hand.clone(),
            graveyard: self.graveyard.clone(),
            ai_draw_pile: self.ai_draw_pile.clone(),
            ai_hand: self.ai_hand.clone(),
            ai_graveyard: self.ai_graveyard.clone(),
            entities: self.entities.clone(),
            next_uid: self.next_uid,
            combat_log: self.combat_log.clone(),
            dice_rng: self.dice_rng.clone(),
            p1_obj_progress: self.p1_obj_progress,
            p2_obj_progress: self.p2_obj_progress,
            stats_units_deployed: self.stats_units_deployed,
            stats_enemies_destroyed: self.stats_enemies_destroyed,
            match_outcome: self.match_outcome.clone(),
        };
        let pretty = PrettyConfig::new().depth_limit(4).separate_tuple_members(true);
        let serialized =
            to_string_pretty(&snap, pretty).map_err(|e| format!("RON serialize error: {e}"))?;
        fs::write(path, serialized).map_err(|e| format!("Failed writing {}: {e}", path.display()))?;
        let msg = format!("💾 Quicksaved Turn {} to {}", self.turn_number, path.display());
        self.combat_log.push(msg.clone());
        self.push_toast(msg.clone());
        Ok(msg)
    }

    pub fn load_quicksave(&mut self, path: &Path) -> Result<u64, String> {
        let raw = fs::read_to_string(path)
            .map_err(|e| format!("No quicksave at {} ({e})", path.display()))?;
        let snap: SavedMatchSnapshot =
            ron::from_str(&raw).map_err(|e| format!("Invalid quicksave RON: {e}"))?;
        self.active_deck_preset = snap.active_deck_preset;
        self.decks = snap.decks;
        self.scenario = snap.scenario;
        self.turn_number = snap.turn_number;
        self.active_team = snap.active_team;
        self.hotseat_mode = snap.hotseat_mode;
        self.hotseat_handoff_pending = false;
        self.fog_enabled = snap.fog_enabled;
        self.explored_hexes = snap.explored_hexes;
        self.breached_canal_hexes = snap.breached_canal_hexes;
        self.economy = snap.economy;
        self.draw_pile = snap.draw_pile;
        self.hand = snap.hand;
        self.graveyard = snap.graveyard;
        self.ai_draw_pile = snap.ai_draw_pile;
        self.ai_hand = snap.ai_hand;
        self.ai_graveyard = snap.ai_graveyard;
        self.entities = snap.entities;
        self.next_uid = snap.next_uid;
        self.combat_log = snap.combat_log;
        self.dice_rng = snap.dice_rng;
        self.p1_obj_progress = snap.p1_obj_progress;
        self.p2_obj_progress = snap.p2_obj_progress;
        self.stats_units_deployed = snap.stats_units_deployed;
        self.stats_enemies_destroyed = snap.stats_enemies_destroyed;
        self.match_outcome = snap.match_outcome;
        self.outcome_modal_dismissed = false;
        self.mode = CommandMode::Inspect;
        self.selected_hex = None;
        self.refresh_confusion_states();
        self.update_explored_fog();

        let msg = format!("📂 Quickloaded Turn {} ({})", self.turn_number, self.scenario.title);
        self.combat_log.push(msg.clone());
        self.push_toast(msg);
        Ok(snap.active_seed)
    }

    pub fn objective_summary_banner(&self) -> String {
        match &self.scenario.victory {
            VictoryCondition::DestroyEnemyCommandCenter => {
                "🏆 Raze Enemy Command Center".to_string()
            }
            VictoryCondition::HoldVaultTurns { target_turns } => format!(
                "🏆 Vault: P1 {}/{}t | P2 {}/{}t",
                self.p1_obj_progress, target_turns, self.p2_obj_progress, target_turns
            ),
            VictoryCondition::SalvageMunitionsOrRaze { target_munitions } => format!(
                "🏆 Salvage: P1 {}/{} | P2 {}/{}",
                self.p1_obj_progress, target_munitions, self.p2_obj_progress, target_munitions
            ),
            VictoryCondition::ControlIslandRelays {
                required_relays,
                target_turns,
            } => format!(
                "🏆 {}+ Relays: P1 {}/{}t | P2 {}/{}t",
                required_relays, self.p1_obj_progress, target_turns, self.p2_obj_progress, target_turns
            ),
            VictoryCondition::HoldCentralControlSpire { target_turns } => format!(
                "🏆 Spire: P1 {}/{}t | P2 {}/{}t",
                self.p1_obj_progress, target_turns, self.p2_obj_progress, target_turns
            ),
        }
    }

    pub fn roll_d6(&mut self) -> u8 {
        ((self.dice_rng.next_u64() % 6) + 1) as u8
    }

    pub fn card_def(&self, key: &str) -> Option<&UnitCardDef> {
        self.roster.units.iter().find(|u| u.card_key == key)
    }

    pub fn entity_at(&self, hex: Hex) -> Option<usize> {
        self.entities
            .iter()
            .position(|e| e.hex == hex && e.unit.current_str_hp > 0)
    }

    pub fn visible_entity_at_for_viewer(&self, hex: Hex) -> Option<usize> {
        let viewer = self.viewing_team();
        let vis = self.compute_visible_hexes(viewer);
        let det = self.compute_true_sight_hexes(viewer);
        self.entities
            .iter()
            .enumerate()
            .find(|&(idx, e)| {
                e.hex == hex && self.is_entity_visible_to(idx, viewer, &vis, &det)
            })
            .map(|(idx, _)| idx)
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

    pub fn team_node_hexes(&self, team_id: u8) -> HashSet<Hex> {
        let mut out = HashSet::new();
        for e in &self.entities {
            if e.team_id == team_id && e.unit.current_str_hp > 0 && e.unit.emits_node {
                for h in e.hex.range(u32::from(e.unit.node_emit_radius.max(1))) {
                    out.insert(h);
                }
            }
        }
        out
    }

    pub fn refresh_confusion_states(&mut self) {
        let refs: Vec<(PlayerOwner, HexPos, BoardUnit)> = self
            .entities
            .iter()
            .filter(|e| e.unit.current_str_hp > 0)
            .map(|e| (PlayerOwner(e.team_id), HexPos(e.hex), e.unit.clone()))
            .collect();
        let borrow_refs: Vec<(PlayerOwner, HexPos, &BoardUnit)> =
            refs.iter().map(|(o, p, u)| (*o, *p, u)).collect();

        for ent in &mut self.entities {
            if ent.unit.current_str_hp == 0 || ent.team_id == 0 {
                continue;
            }
            let in_node = is_within_control_node(
                ent.team_id,
                ent.unit.node_leash_range,
                ent.hex,
                &borrow_refs,
            );
            ent.unit.confused = !in_node;
        }
    }

    pub fn valid_deploy_hexes_for_team(&self, team_id: u8, card_key: &str) -> HashSet<Hex> {
        let mut valid = HashSet::new();
        let Some(card) = self.card_def(card_key) else {
            return valid;
        };
        let supply = self.team_supply_hexes(team_id);
        if card.combat_class == CombatClass::TacticalDeployment || (card.combat_class == CombatClass::Artifact && card.card_key != "Cargo Drop") {
            let vis = self.compute_visible_hexes(team_id);
            for &h in self.scenario.map.tiles.keys() {
                if !self.fog_enabled || vis.contains(&h) || supply.contains(&h) {
                    valid.insert(h);
                }
            }
            return valid;
        }
        for (&h, tile) in &self.scenario.map.tiles {
            if self.entity_at(h).is_some() {
                continue;
            }
            let terrain_ok = if card.combat_class.is_building() {
                matches!(tile.morphology, Morphology::Flat | Morphology::RoughCover | Morphology::BunkerRuins)
            } else if card.can_fly {
                tile.morphology != Morphology::MountainPeak
            } else if card.amphibious_tier == 2 {
                matches!(tile.morphology, Morphology::DeepRiver | Morphology::OpenOcean | Morphology::ShallowFord | Morphology::ArmoredDam)
            } else if card.amphibious_tier == 1 {
                !matches!(
                    tile.morphology,
                    Morphology::MountainPeak | Morphology::RavineChasm
                )
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
                let vis = self.compute_visible_hexes(team_id);
                zone_ok = vis.contains(&h);
            }
            if !zone_ok && card.amphibious_tier == 2 {
                // Pelagic Launch: Pure Aquatic units can launch into any water/ford channel within 3 hexes of a friendly building or adjacent to a friendly Fish!
                zone_ok = self.entities.iter().any(|e| {
                    e.team_id == team_id
                        && e.unit.current_str_hp > 0
                        && ((e.card.combat_class.is_building() && e.hex.unsigned_distance_to(h) <= 3)
                            || (e.card.faction == OrganismFaction::Fish && e.hex.unsigned_distance_to(h) <= 1))
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
                valid.insert(h);
            }
        }
        valid
    }

    pub fn valid_deploy_hexes(&self, card_key: &str) -> HashSet<Hex> {
        self.valid_deploy_hexes_for_team(self.viewing_team(), card_key)
    }

    pub fn valid_move_hexes_for_team(&self, ent_idx: usize, expected_team: u8) -> HashSet<Hex> {
        let mut reachable = HashSet::new();
        let Some(ent) = self.entities.get(ent_idx) else {
            return reachable;
        };
        if ent.team_id != expected_team
            || ent.unit.confused
            || ent.unit.rooted_turns > 0
            || ent.card.movement == 0
        {
            return reachable;
        }
        let max_steps = ent.card.movement.saturating_add(ent.unit.temp_move_bonus);
        let refs: Vec<(PlayerOwner, HexPos, &BoardUnit)> = self
            .entities
            .iter()
            .filter(|e| e.unit.current_str_hp > 0)
            .map(|e| (PlayerOwner(e.team_id), HexPos(e.hex), &e.unit))
            .collect();

        let mut visited: HashMap<Hex, u8> = HashMap::new();
        let mut queue = VecDeque::new();
        visited.insert(ent.hex, 0);
        queue.push_back((ent.hex, 0u8));

        while let Some((curr, dist)) = queue.pop_front() {
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
                let occ_idx = self.entity_at(nb);
                let occupied = occ_idx.is_some();
                let friendly_pass = occ_idx.map_or(false, |oi| {
                    let oe = &self.entities[oi];
                    oe.team_id == expected_team
                        && (oe.card.card_key == "Cargo Drop"
                            || ent.card.faction == OrganismFaction::Fish
                            || ent.card.amphibious_tier == 2
                            || ent.card.faction == OrganismFaction::Insect)
                });
                if occupied && !(ent.card.can_fly || ent.card.stealth || ent.unit.temp_stealth || friendly_pass) {
                    continue;
                }
                visited.insert(nb, next_d);
                queue.push_back((nb, next_d));
                if !occupied
                    && is_within_control_node(expected_team, ent.unit.node_leash_range, nb, &refs)
                {
                    reachable.insert(nb);
                }
            }
        }
        reachable
    }

    pub fn valid_move_hexes(&self, ent_idx: usize) -> HashSet<Hex> {
        self.valid_move_hexes_for_team(ent_idx, self.viewing_team())
    }

    pub fn valid_attack_hexes(&self, ent_idx: usize) -> HashSet<Hex> {
        let mut targets = HashSet::new();
        let actor = self.viewing_team();
        let Some(ent) = self.entities.get(ent_idx) else {
            return targets;
        };
        if ent.team_id != actor || ent.unit.confused {
            return targets;
        }
        if ent.unit.is_laser && ent.unit.laser_fired_this_turn {
            return targets;
        }
        let team_deck = &self.decks[(actor - 1).min(1) as usize];
        let mut eff_ranged = ent.card.ranged_distance;
        if team_deck.is_pure_biotype() && team_deck.primary_faction == OrganismFaction::Bird && ent.card.faction == OrganismFaction::Bird {
            eff_ranged = eff_ranged.saturating_add(1);
        }
        if team_deck.is_pure_biotype() && team_deck.primary_faction == OrganismFaction::Fish && ent.card.faction == OrganismFaction::Fish && eff_ranged > 0 {
            eff_ranged = eff_ranged.saturating_add(1);
        }
        if team_deck.is_pure_biotype() && team_deck.primary_faction == OrganismFaction::Arachnid && ent.card.faction == OrganismFaction::Arachnid && (ent.card.root_stacks > 0 || ent.card.poison_stacks > 0) {
            eff_ranged = eff_ranged.saturating_add(1);
        }
        let max_rng = eff_ranged.max(ent.card.melee_range);
        if max_rng == 0 {
            return targets;
        }
        let vis = self.compute_visible_hexes(actor);
        let det = self.compute_true_sight_hexes(actor);
        for (o_idx, other) in self.entities.iter().enumerate() {
            if other.unit.current_str_hp > 0
                && other.team_id != actor
                && other.team_id != 0
                && self.is_entity_visible_to(o_idx, actor, &vis, &det)
            {
                // Anti-Air Rule: Ground melee-only units (`eff_ranged == 0 && !can_fly`) cannot hit airborne Flyers!
                if other.card.can_fly && !ent.card.can_fly && eff_ranged == 0 {
                    continue;
                }
                let asia_bonus = u8::from(team_deck.faction == OriginAllegiance::Asia && eff_ranged > 0 && (other.unit.target_locked || det.contains(&other.hex)));
                let allowed_rng = max_rng.saturating_add(asia_bonus);
                let d = ent.hex.unsigned_distance_to(other.hex);
                if d >= 1 && d <= u32::from(allowed_rng) {
                    targets.insert(other.hex);
                }
            }
        }
        targets
    }

    pub fn valid_ability_hexes(&self, ent_idx: usize) -> HashSet<Hex> {
        let mut targets = HashSet::new();
        let actor = self.viewing_team();
        let Some(ent) = self.entities.get(ent_idx) else {
            return targets;
        };
        if ent.team_id != actor || ent.unit.confused {
            return targets;
        }
        if ent.card.heal_repair_range > 0 {
            for other in &self.entities {
                if other.team_id == actor
                    && other.unit.current_str_hp > 0
                    && ent.hex.unsigned_distance_to(other.hex) <= u32::from(ent.card.heal_repair_range)
                {
                    targets.insert(other.hex);
                }
            }
        } else if ent.card.radar_range > 0 || ent.card.poison_stacks > 0 || ent.card.root_stacks > 0 {
            let rng = ent
                .card
                .radar_range
                .max(ent.card.ranged_distance)
                .max(2);
            let vis = self.compute_visible_hexes(actor);
            let det = self.compute_true_sight_hexes(actor);
            for (o_idx, other) in self.entities.iter().enumerate() {
                if other.team_id != actor
                    && other.team_id != 0
                    && other.unit.current_str_hp > 0
                    && ent.hex.unsigned_distance_to(other.hex) <= u32::from(rng)
                    && (ent.card.radar_range > 0 || self.is_entity_visible_to(o_idx, actor, &vis, &det))
                {
                    targets.insert(other.hex);
                }
            }
        } else {
            targets.insert(ent.hex);
        }
        targets
    }

    fn spend_munition_for_entity(&mut self, ent_idx: usize, cost: u8) -> bool {
        if cost == 0 {
            return true;
        }
        let team = self.entities[ent_idx].team_id;
        let hex = self.entities[ent_idx].hex;
        let is_water_fish = self.entities[ent_idx].card.faction == OrganismFaction::Fish
            && self.scenario.map.tiles.get(&hex).map_or(false, |t| matches!(t.morphology, Morphology::DeepRiver | Morphology::OpenOcean | Morphology::ShallowFord | Morphology::ArmoredDam));
        let pelagic_supply = is_water_fish && self.entities.iter().any(|b| {
            b.team_id == team && b.unit.current_str_hp > 0 && b.card.combat_class.is_building() && b.hex.unsigned_distance_to(hex) <= 6
        });
        let in_supply = self.team_supply_hexes(team).contains(&hex) || pelagic_supply;
        if in_supply {
            let home = self.economy.home_base_munitions.entry(team).or_insert(0);
            if *home >= u16::from(cost) {
                *home -= u16::from(cost);
                return true;
            }
        }
        // Check nearby friendly Remote Cargo Drop Cache (within 2 hexes) before personal ammo!
        if let Some(cache_idx) = self.entities.iter().position(|e| {
            e.team_id == team
                && e.card.card_key == "Cargo Drop"
                && e.unit.current_str_hp > 0
                && e.unit.carried_munitions >= cost
                && e.hex.unsigned_distance_to(hex) <= 2
        }) {
            self.entities[cache_idx].unit.carried_munitions -= cost;
            if self.entities[cache_idx].unit.carried_munitions == 0 {
                self.entities[cache_idx].unit.current_str_hp = 0;
                self.combat_log.push(format!("📦 P{} Remote Cargo Drop cache depleted and expended!", team));
            }
            return true;
        }
        if self.entities[ent_idx].unit.carried_munitions >= cost {
            self.entities[ent_idx].unit.carried_munitions -= cost;
            return true;
        }
        false
    }

    pub fn handle_hex_click(
        &mut self,
        clicked_hex: Hex,
        vfx: &mut VfxManager,
        audio: &mut SpatialAudioEngine,
    ) {
        if self.hotseat_handoff_pending {
            return;
        }
        if !self.can_local_player_act() {
            self.selected_hex = Some(clicked_hex);
            return;
        }
        let actor = self.active_team;
        let actions_left = self.economy.actions_remaining.get(&actor).copied().unwrap_or(0);

        match self.mode {
            CommandMode::DeployHandCard(hand_idx) => {
                if actions_left == 0 {
                    self.combat_log.push("No Actions remaining this turn—press End Turn!".into());
                    self.mode = CommandMode::Inspect;
                    return;
                }
                let card_key_opt = if actor == 2 {
                    self.ai_hand.get(hand_idx).cloned()
                } else {
                    self.hand.get(hand_idx).cloned()
                };
                let Some(card_key) = card_key_opt else {
                    self.mode = CommandMode::Inspect;
                    return;
                };
                if self.valid_deploy_hexes_for_team(actor, &card_key).contains(&clicked_hex) {
                    let Some(card) = self.card_def(&card_key).cloned() else {
                        return;
                    };
                    let p_deck = self.decks[(actor - 1).min(1) as usize].clone();
                    let mut deploy_cost = if card.card_key == "Cargo Drop"
                        || (p_deck.is_pure_biotype() && p_deck.primary_faction == OrganismFaction::Plant && card.combat_class == CombatClass::Trap)
                    {
                        0
                    } else {
                        u16::from(card.encumbrance)
                    };
                    if card.faction == OrganismFaction::Insect && deploy_cost > 1 {
                        let adj_insect = (p_deck.is_pure_biotype() && p_deck.primary_faction == OrganismFaction::Insect)
                            || self.entities.iter().any(|e| {
                                e.team_id == actor
                                    && e.unit.current_str_hp > 0
                                    && e.card.faction == OrganismFaction::Insect
                                    && e.hex.unsigned_distance_to(clicked_hex) == 1
                            });
                        if adj_insect {
                            deploy_cost -= 1;
                        }
                    }
                    let home_mun = self.economy.home_base_munitions.entry(actor).or_insert(0);
                    if *home_mun < deploy_cost {
                        self.combat_log.push(format!(
                            "Need {} Home Munitions to deploy {} (have {}).",
                            deploy_cost, card.unit_name, *home_mun
                        ));
                        return;
                    }
                    *home_mun -= deploy_cost;
                    *self.economy.actions_remaining.entry(actor).or_insert(1) -= 1;
                    if actor == 2 {
                        self.ai_hand.remove(hand_idx);
                    } else {
                        self.hand.remove(hand_idx);
                        self.stats_units_deployed += 1;
                    }

                    let anchor = hex_world_anchor(&self.scenario.map, clicked_hex, 0.2);
                    if card.combat_class == CombatClass::TacticalDeployment || (card.combat_class == CombatClass::Artifact && card.card_key != "Cargo Drop") {
                        self.resolve_tactical_deployment(actor, &card, clicked_hex, vfx, audio);
                    } else if card.combat_class == CombatClass::Artifact && card.card_key == "Cargo Drop" {
                        let uid = self.next_uid;
                        self.next_uid += 1;
                        let mut cache_u = make_board_unit(&card);
                        cache_u.carried_munitions = 5;
                        self.entities.push(LiveEntity {
                            uid,
                            team_id: actor,
                            hex: clicked_hex,
                            vox_slug: card_to_vox_slug(&card).to_string(),
                            yaw_radians: 0.0,
                            card: card.clone(),
                            unit: cache_u,
                        });
                        vfx.spawn_orbital_deploy(anchor, [0.95, 0.68, 0.12]);
                        vfx.spawn_popup(anchor, "📦 REMOTE CARGO DROP", "5 Munitions Cache (Single-Use)", [250, 196, 52]);
                        audio.play_spatial(SfxCue::OrbitalDeploy, anchor);
                        self.combat_log.push(format!("P{} dropped single-use Remote Cargo Cache (5 Mun) at ({}, {})!", actor, clicked_hex.x, clicked_hex.y));
                    } else {
                        let uid = self.next_uid;
                        self.next_uid += 1;
                        let slug = card_to_vox_slug(&card).to_string();
                        self.entities.push(LiveEntity {
                            uid,
                            team_id: actor,
                            hex: clicked_hex,
                            vox_slug: slug,
                            yaw_radians: if actor == 1 { -0.85 } else { 2.30 },
                            card: card.clone(),
                            unit: make_board_unit_for_deck(&card, &p_deck),
                        });
                        if card.faction == OrganismFaction::Fish {
                            let breach_cap = if p_deck.is_pure_biotype() && p_deck.primary_faction == OrganismFaction::Fish { 2 } else { 1 };
                            let mut breached = 0;
                            let mut nbrs = clicked_hex.all_neighbors().to_vec();
                            nbrs.sort_by_key(|nh| u8::from(self.entity_at(*nh).is_some()));
                            for flood_h in nbrs {
                                if breached >= breach_cap { break; }
                                if let Some(t) = self.scenario.map.tiles.get_mut(&flood_h) {
                                    if matches!(t.morphology, Morphology::Flat | Morphology::RoughCover | Morphology::RadiationFallout) {
                                        t.morphology = Morphology::ShallowFord;
                                        t.elevation = 0;
                                        self.breached_canal_hexes.insert(flood_h);
                                        breached += 1;
                                    }
                                }
                            }
                        }
                        self.refresh_confusion_states();
                        self.update_explored_fog();
                        let beam_col = if actor == 1 { [0.12, 0.85, 0.96] } else { [0.96, 0.30, 0.22] };
                        vfx.spawn_orbital_deploy(anchor, beam_col);
                        vfx.spawn_popup(
                            anchor,
                            format!("⬇ P{} DEPLOYED {}", actor, card.unit_name),
                            format!("-{} Munitions", deploy_cost),
                            if actor == 1 { [82, 228, 250] } else { [252, 118, 92] },
                        );
                        audio.play_spatial(SfxCue::OrbitalDeploy, anchor);
                        self.combat_log.push(format!(
                            "P{} deployed {} to ({}, {}) for {} Munitions.",
                            actor, card.unit_name, clicked_hex.x, clicked_hex.y, deploy_cost
                        ));
                    }
                    self.selected_hex = Some(clicked_hex);
                    self.mode = CommandMode::Inspect;
                    self.emit_lockstep_cmd(LockstepCommand::DeployCard {
                        team_id: actor,
                        hand_idx,
                        hex: clicked_hex,
                    });
                    return;
                }
                self.mode = CommandMode::Inspect;
            }
            CommandMode::MoveSelectedUnit => {
                if let Some(sel) = self.selected_hex {
                    if let Some(ent_idx) = self.entity_at(sel) {
                        if actions_left > 0
                            && self.valid_move_hexes_for_team(ent_idx, actor).contains(&clicked_hex)
                        {
                            let from_w = hex_world_anchor(&self.scenario.map, sel, 0.2);
                            let to_w = hex_world_anchor(&self.scenario.map, clicked_hex, 0.2);
                            let uid = self.entities[ent_idx].uid;
                            let name = self.entities[ent_idx].card.unit_name.clone();
                            self.entities[ent_idx].yaw_radians =
                                (to_w.x - from_w.x).atan2(to_w.z - from_w.z);
                            self.entities[ent_idx].hex = clicked_hex;
                            self.entities[ent_idx].unit.target_locked = false;
                            *self.economy.actions_remaining.entry(actor).or_insert(1) -= 1;
                            self.refresh_confusion_states();
                            self.update_explored_fog();
                            vfx.trigger_unit_lunge(uid, to_w, from_w, 0.45);
                            audio.play_spatial(SfxCue::UiCardTurn, to_w);
                            self.combat_log.push(format!(
                                "P{} moved {} -> ({}, {}).",
                                actor, name, clicked_hex.x, clicked_hex.y
                            ));
                            self.selected_hex = Some(clicked_hex);
                            self.mode = CommandMode::Inspect;
                            self.emit_lockstep_cmd(LockstepCommand::MoveUnit {
                                team_id: actor,
                                from_hex: sel,
                                to_hex: clicked_hex,
                            });
                            return;
                        }
                    }
                }
                self.mode = CommandMode::Inspect;
            }
            CommandMode::AttackWithSelectedUnit => {
                if let Some(sel) = self.selected_hex {
                    if let (Some(att_idx), Some(def_idx)) =
                        (self.entity_at(sel), self.entity_at(clicked_hex))
                    {
                        if actions_left > 0 && self.valid_attack_hexes(att_idx).contains(&clicked_hex) {
                            self.execute_combat_pair(att_idx, def_idx, vfx, audio);
                            self.mode = CommandMode::Inspect;
                            self.emit_lockstep_cmd(LockstepCommand::AttackUnit {
                                team_id: actor,
                                attacker_hex: sel,
                                defender_hex: clicked_hex,
                            });
                            return;
                        }
                    }
                }
                self.mode = CommandMode::Inspect;
            }
            CommandMode::AbilityWithSelectedUnit => {
                if let Some(sel) = self.selected_hex {
                    if let Some(ent_idx) = self.entity_at(sel) {
                        if actions_left > 0
                            && self.valid_ability_hexes(ent_idx).contains(&clicked_hex)
                        {
                            self.execute_unit_ability(ent_idx, clicked_hex, vfx, audio);
                            self.mode = CommandMode::Inspect;
                            self.emit_lockstep_cmd(LockstepCommand::CastAbility {
                                team_id: actor,
                                caster_hex: sel,
                                target_hex: clicked_hex,
                            });
                            return;
                        }
                    }
                }
                self.mode = CommandMode::Inspect;
            }
            CommandMode::Inspect => {}
        }

        self.selected_hex = Some(clicked_hex);
    }

    /// Triggers the active Faction Command Card ability for the given team (costs Actions & Home Munitions).
    pub fn execute_faction_command_ability(
        &mut self,
        team_id: u8,
        emit_net: bool,
        vfx: &mut VfxManager,
        audio: &mut SpatialAudioEngine,
    ) -> bool {
        let deck = self.decks[(team_id - 1).min(1) as usize].clone();
        let fac = deck.faction_card();
        if self.economy.faction_ability_used.get(&team_id).copied().unwrap_or(false) {
            self.combat_log.push("Faction Command Ability already used this turn!".into());
            return false;
        }
        let act_rem = self.economy.actions_remaining.get(&team_id).copied().unwrap_or(0);
        let mun_rem = self.economy.home_base_munitions.get(&team_id).copied().unwrap_or(0);
        let mun_cost = u16::from(fac.active_munition_cost);
        if act_rem < fac.active_action_cost || mun_rem < mun_cost {
            self.combat_log.push(format!("Insufficient Actions ({}/{}) or Munitions ({}/{}) for Faction Ability!", act_rem, fac.active_action_cost, mun_rem, fac.active_munition_cost));
            return false;
        }

        self.economy.faction_ability_used.insert(team_id, true);
        *self.economy.actions_remaining.entry(team_id).or_insert(1) -= fac.active_action_cost;
        *self.economy.home_base_munitions.entry(team_id).or_insert(0) -= mun_cost;

        let hq_hex = self.entities.iter().find(|e| e.team_id == team_id && e.card.combat_class == CombatClass::Installation).map(|e| e.hex).unwrap_or(Hex::new(0, 0));
        let anchor_w = hex_world_anchor(&self.scenario.map, hq_hex, 0.4);

        match deck.faction {
            OriginAllegiance::Australia => {
                let mut repaired = false;
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.unit.current_str_hp > 0 && ent.unit.current_str_hp < ent.unit.max_str_hp {
                        ent.unit.current_str_hp = (ent.unit.current_str_hp + 2).min(ent.unit.max_str_hp);
                        let w_pos = hex_world_anchor(&self.scenario.map, ent.hex, 0.4);
                        vfx.spawn_bio_ability_ring(w_pos, [0.15, 0.85, 0.95]);
                        vfx.spawn_popup(w_pos, "[AU] NANITE SURGE", format!("{}: +2 STR", ent.card.unit_name), [112, 232, 248]);
                        audio.play_spatial(SfxCue::BioHealChime, w_pos);
                        self.combat_log.push(format!("[AU] Australia Nanite Surge repaired +2 STR to {}!", ent.card.unit_name));
                        repaired = true;
                        break;
                    }
                }
                if !repaired {
                    self.combat_log.push("[AU] Australia Nanite Surge activated: field units are already optimal!".into());
                }
            }
            OriginAllegiance::Africa => {
                let enemy_team = if team_id == 1 { 2 } else { 1 };
                let mut target_found = false;
                if let Some(pivot_hex) = self.entities.iter().find(|e| e.team_id == enemy_team && e.unit.current_str_hp > 0 && !e.card.combat_class.is_building()).map(|e| e.hex) {
                    let mut hit_cnt = 0;
                    for ent in self.entities.iter_mut() {
                        if ent.team_id == enemy_team && ent.unit.current_str_hp > 0 && ent.hex.unsigned_distance_to(pivot_hex) <= 1 {
                            ent.unit.target_locked = true;
                            ent.unit.blinded_this_turn = true;
                            hit_cnt += 1;
                        }
                    }
                    let w_pos = hex_world_anchor(&self.scenario.map, pivot_hex, 0.4);
                    vfx.spawn_bio_ability_ring(w_pos, [1.0, 0.85, 0.15]);
                    vfx.spawn_popup(w_pos, "[AF] SOLAR FLARE BURST", format!("Blinded & Locked {} foes", hit_cnt), [255, 210, 60]);
                    audio.play_spatial(SfxCue::LaserPulse, w_pos);
                    self.combat_log.push(format!("[AF] Africa Solar Flare Burst blinded & locked {} enemies in Radius 1!", hit_cnt));
                    target_found = true;
                }
                if !target_found {
                    self.combat_log.push("[AF] Africa Solar Flare Burst activated, no targets in range.".into());
                }
            }
            OriginAllegiance::Asia => {
                let enemy_team = if team_id == 1 { 2 } else { 1 };
                let mut count = 0;
                for ent in self.entities.iter_mut() {
                    if ent.team_id == enemy_team && ent.unit.current_str_hp > 0 && !ent.card.combat_class.is_building() {
                        if ent.card.stealth || ent.unit.burrowed || count < 2 {
                            ent.unit.revealed_until_turn_start = true;
                            ent.unit.burrowed = false;
                            ent.unit.target_locked = true;
                            count += 1;
                        }
                    }
                }
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.unit.current_str_hp > 0 && !ent.card.combat_class.is_building() && ent.card.card_key != "Cargo Drop" {
                        ent.unit.fortified_stacks = (ent.unit.fortified_stacks + 1).min(3);
                        ent.unit.current_str_hp = (ent.unit.current_str_hp + 1).min(ent.unit.max_str_hp);
                        ent.unit.carried_munitions = (ent.unit.carried_munitions + 1).min(ent.card.encumbrance.max(1));
                        ent.unit.temp_move_bonus = ent.unit.temp_move_bonus.saturating_add(1);
                        break;
                    }
                }
                vfx.spawn_bio_ability_ring(anchor_w, [0.35, 0.75, 1.0]);
                vfx.spawn_popup(anchor_w, "[AS] ORBITAL DRONE SWEEP", format!("Locked {} + Drone Shield/Ammo", count), [120, 220, 255]);
                audio.play_spatial(SfxCue::VenomSnare, anchor_w);
                self.combat_log.push(format!("[AS] Asia Orbital Drone Sweep locked {} foes and shielded/resupplied lead unit!", count));
            }
            OriginAllegiance::Europe => {
                let mut buffed = 0;
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.unit.current_str_hp > 0 && !ent.card.combat_class.is_building() {
                        ent.unit.fortified_stacks = (ent.unit.fortified_stacks + 1).min(3);
                        let w_pos = hex_world_anchor(&self.scenario.map, ent.hex, 0.4);
                        vfx.spawn_bio_ability_ring(w_pos, [0.95, 0.75, 0.25]);
                        vfx.spawn_popup(w_pos, "[EU] BASTION PROTOCOL", "+1 Fortified Shield", [248, 200, 60]);
                        buffed += 1;
                        if buffed >= 1 { break; }
                    }
                }
                audio.play_spatial(SfxCue::BioHealChime, anchor_w);
                self.combat_log.push(format!("[EU] Europe Bastion Protocol granted +1 Fortified shield to {} frontliners!", buffed));
            }
            OriginAllegiance::NorthAmerica => {
                let enemy_team = if team_id == 1 { 2 } else { 1 };
                let mut affected = 0;
                for ent in self.entities.iter_mut() {
                    if ent.team_id == enemy_team && ent.unit.current_str_hp > 0 && !ent.card.combat_class.is_building() && affected < 2 {
                        ent.unit.fortified_stacks = 0;
                        ent.unit.target_locked = true;
                        affected += 1;
                    }
                }
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.unit.current_str_hp > 0 && !ent.card.combat_class.is_building() && ent.card.card_key != "Cargo Drop" {
                        ent.unit.fortified_stacks = (ent.unit.fortified_stacks + 1).min(3);
                        ent.unit.current_str_hp = (ent.unit.current_str_hp + 1).min(ent.unit.max_str_hp);
                        break;
                    }
                }
                vfx.spawn_bio_ability_ring(anchor_w, [0.25, 0.55, 1.0]);
                vfx.spawn_popup(anchor_w, "[US] EMP & AEGIS PULSE", "Stripped Shields + Aegis Repair", [80, 180, 255]);
                audio.play_spatial(SfxCue::LaserPulse, anchor_w);
                self.combat_log.push(format!("[US] North America EMP & Aegis Pulse disrupted {} foes and fortified lead unit!", affected));
            }
            OriginAllegiance::SouthAmerica => {
                let mut purged = 0;
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.unit.current_str_hp > 0 {
                        if ent.unit.poisoned_stacks > 0 || ent.unit.rooted_turns > 0 {
                            ent.unit.poisoned_stacks = 0;
                            ent.unit.rooted_turns = 0;
                            purged += 1;
                        }
                        ent.unit.current_str_hp = (ent.unit.current_str_hp + 1).min(ent.unit.max_str_hp);
                        if ent.card.faction != OrganismFaction::Human {
                            ent.unit.temp_move_bonus = ent.unit.temp_move_bonus.saturating_add(1);
                        }
                    }
                }
                vfx.spawn_bio_ability_ring(anchor_w, [0.15, 0.95, 0.45]);
                vfx.spawn_popup(anchor_w, "[BR] PRIMAL SURGE", format!("Purged {} & Healed +1 STR", purged), [78, 248, 158]);
                audio.play_spatial(SfxCue::BioHealChime, anchor_w);
                self.combat_log.push(format!("[BR] South America Primal Surge cleansed toxins from {} units and healed all allies!", purged));
            }
            OriginAllegiance::Neutral => {
                *self.economy.home_base_munitions.entry(team_id).or_insert(0) += 2;
                self.combat_log.push("Frontier Scavengers gathered +2 extra Munitions!".into());
            }
        }
        if emit_net {
            self.emit_lockstep_cmd(LockstepCommand::FactionCommand { team_id });
        }
        true
    }

    /// Triggers the primary Bio-Type Primal Surge ability for the given team.
    pub fn execute_biotype_primal_surge(
        &mut self,
        team_id: u8,
        emit_net: bool,
        vfx: &mut VfxManager,
        audio: &mut SpatialAudioEngine,
    ) -> bool {
        let deck = self.decks[(team_id - 1).min(1) as usize].clone();
        let bio = deck.primary_biotype_card();
        if self.economy.primal_surge_used.get(&team_id).copied().unwrap_or(false) {
            self.combat_log.push("Bio-Type Primal Surge already used this turn!".into());
            return false;
        }
        let act_rem = self.economy.actions_remaining.get(&team_id).copied().unwrap_or(0);
        let mun_rem = self.economy.home_base_munitions.get(&team_id).copied().unwrap_or(0);
        let mun_cost = u16::from(bio.primal_surge_cost.1);
        if act_rem < bio.primal_surge_cost.0 || mun_rem < mun_cost {
            self.combat_log.push("Insufficient Actions or Munitions for Primal Surge!".into());
            return false;
        }

        self.economy.primal_surge_used.insert(team_id, true);
        *self.economy.actions_remaining.entry(team_id).or_insert(1) -= bio.primal_surge_cost.0;
        *self.economy.home_base_munitions.entry(team_id).or_insert(0) -= mun_cost;

        let enemy_team = if team_id == 1 { 2 } else { 1 };
        let hq_hex = self.entities.iter().find(|e| e.team_id == team_id && e.card.combat_class == CombatClass::Installation).map(|e| e.hex).unwrap_or(Hex::new(0, 0));
        let anchor_w = hex_world_anchor(&self.scenario.map, hq_hex, 0.4);

        match deck.primary_faction {
            OrganismFaction::Mammal => {
                let mut charged = false;
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.card.faction == OrganismFaction::Mammal && ent.unit.current_str_hp > 0 && !ent.card.combat_class.is_building() {
                        ent.unit.temp_move_bonus = ent.unit.temp_move_bonus.saturating_add(1);
                        let w_pos = hex_world_anchor(&self.scenario.map, ent.hex, 0.4);
                        vfx.spawn_popup(w_pos, "🐾 STAMPEDE RUSH", "+1 Charge Move & Impact", [240, 160, 60]);
                        audio.play_spatial(SfxCue::MeleeClash, w_pos);
                        self.combat_log.push(format!("🐾 Mammal Stampede Rush: {} gained +1 Charge MOVE!", ent.card.unit_name));
                        charged = true;
                        break;
                    }
                }
                if !charged {
                    self.combat_log.push("🐾 Mammal Stampede Rush activated!".into());
                }
            }
            OrganismFaction::Reptile => {
                let mut count = 0;
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.card.faction == OrganismFaction::Reptile && ent.unit.current_str_hp > 0 {
                        ent.unit.fortified_stacks = (ent.unit.fortified_stacks + 2).min(3);
                        let w_pos = hex_world_anchor(&self.scenario.map, ent.hex, 0.4);
                        vfx.spawn_popup(w_pos, "🦎 HARDENED CARAPACE", "+2 Shield Blocks", [120, 240, 80]);
                        count += 1;
                    }
                }
                audio.play_spatial(SfxCue::BioHealChime, anchor_w);
                self.combat_log.push(format!("🦎 Reptile Hardened Carapace fortified {} cold-blooded units with +2 Shields!", count));
            }
            OrganismFaction::Plant => {
                let plant_hexes: Vec<Hex> = self.entities.iter().filter(|e| e.team_id == team_id && e.card.faction == OrganismFaction::Plant && e.unit.current_str_hp > 0).map(|e| e.hex).collect();
                let mut rooted_cnt = 0;
                for ent in self.entities.iter_mut() {
                    if ent.team_id == enemy_team && ent.unit.current_str_hp > 0 && !ent.card.can_fly {
                        if plant_hexes.iter().any(|ph| ph.unsigned_distance_to(ent.hex) <= 1) {
                            ent.unit.rooted_turns = 2;
                                                        let w_pos = hex_world_anchor(&self.scenario.map, ent.hex, 0.4);
                            vfx.spawn_popup(w_pos, "🌿 SPORE CLOUD", "Rooted (MOVE 0)", [80, 220, 120]);
                            rooted_cnt += 1;
                        }
                    }
                }
                audio.play_spatial(SfxCue::VenomSnare, anchor_w);
                self.combat_log.push(format!("🌿 Plant Spore Cloud Burst rooted {} adjacent enemy units!", rooted_cnt));
            }
            OrganismFaction::Bird => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == enemy_team {
                        ent.unit.target_locked = true;
                        ent.unit.revealed_until_turn_start = true;
                    }
                }
                vfx.spawn_popup(anchor_w, "🦅 AERIAL RECON", "True Sight & Target Lock", [120, 200, 255]);
                audio.play_spatial(SfxCue::VenomSnare, anchor_w);
                self.combat_log.push("🦅 Bird High-Altitude Recon granted True Sight and Target Locks against all spotted enemies!".into());
            }
            OrganismFaction::Arachnid => {
                let mut snared = false;
                for ent in self.entities.iter_mut() {
                    if ent.team_id == enemy_team && ent.unit.current_str_hp > 0 {
                        ent.unit.rooted_turns = 2;
                        ent.unit.poisoned_stacks = (ent.unit.poisoned_stacks + 2).min(3);
                        let w_pos = hex_world_anchor(&self.scenario.map, ent.hex, 0.4);
                        vfx.spawn_popup(w_pos, "🦂 SILK SNARE", "Rooted & +2 Poison", [180, 100, 220]);
                        audio.play_spatial(SfxCue::VenomSnare, w_pos);
                        self.combat_log.push(format!("🦂 Arachnid Silk Snare trapped and envenomed {}!", ent.card.unit_name));
                        snared = true;
                        break;
                    }
                }
                if !snared {
                    self.combat_log.push("🦂 Arachnid Silk Snare activated, no targets in range.".into());
                }
            }
            OrganismFaction::Insect => {
                let mut count = 0;
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.card.faction == OrganismFaction::Insect && ent.unit.current_str_hp > 0 {
                        ent.unit.carried_munitions = (ent.unit.carried_munitions + 1).min(ent.card.encumbrance);
                        count += 1;
                    }
                }
                *self.economy.actions_remaining.entry(team_id).or_insert(0) += 1;
                vfx.spawn_popup(anchor_w, "⚡ SWARM FRENZY", "+1 Action & Free Ammo", [220, 180, 50]);
                audio.play_spatial(SfxCue::UiCardTurn, anchor_w);
                self.combat_log.push(format!("⚡ Insect Swarm Frenzy stimulated {} insects with +1 Action and emergency ammo!", count));
            }
            OrganismFaction::Fish => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && (ent.card.faction == OrganismFaction::Fish || ent.card.amphibious_tier > 0) {
                        ent.unit.temp_move_bonus = ent.unit.temp_move_bonus.saturating_add(2);
                    }
                }
                vfx.spawn_popup(anchor_w, "🐟 HYDRO-JET SURGE", "+2 Swim Movement", [60, 210, 230]);
                audio.play_spatial(SfxCue::BioHealChime, anchor_w);
                self.combat_log.push("🐟 Fish Hydro-Jet Surge granted +2 Movement across all aquatic forces!".into());
            }
            OrganismFaction::Cnidarian => {
                let cnid_hexes: Vec<Hex> = self.entities.iter().filter(|e| e.team_id == team_id && e.card.faction == OrganismFaction::Cnidarian && e.unit.current_str_hp > 0).map(|e| e.hex).collect();
                let mut shocked = 0;
                for ent in self.entities.iter_mut() {
                    if ent.team_id == enemy_team && ent.unit.current_str_hp > 0 {
                        if cnid_hexes.iter().any(|ch| ch.unsigned_distance_to(ent.hex) <= 1) {
                            ent.unit.fortified_stacks = 0;
                            ent.unit.stunned_this_turn = true;
                            let w_pos = hex_world_anchor(&self.scenario.map, ent.hex, 0.4);
                            vfx.spawn_popup(w_pos, "⚡ BIO-ELECTRIC SHOCK", "Stunned & Shields Stripped", [220, 90, 180]);
                            shocked += 1;
                        }
                    }
                }
                audio.play_spatial(SfxCue::LaserPulse, anchor_w);
                self.combat_log.push(format!("⚡ Cnidarian Bio-Electric Discharge stunned {} adjacent enemies!", shocked));
            }
            OrganismFaction::Amphibian => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.card.faction == OrganismFaction::Amphibian && ent.unit.current_str_hp > 0 {
                        ent.unit.current_str_hp = (ent.unit.current_str_hp + 1).min(ent.unit.max_str_hp);
                        ent.unit.temp_stealth = true;
                    }
                }
                vfx.spawn_popup(anchor_w, "🐸 SECRETORY SLIME", "Stealth Cloaked & +1 STR", [100, 220, 140]);
                audio.play_spatial(SfxCue::BioHealChime, anchor_w);
                self.combat_log.push("🐸 Amphibian Secretory Slime cloaked all amphibians in stealth and restored +1 STR!".into());
            }
            _ => {}
        }
        if emit_net {
            self.emit_lockstep_cmd(LockstepCommand::PrimalSurge { team_id });
        }
        true
    }

    /// Resolves a single-use `CombatClass::TacticalDeployment` spell card deployed onto `target_hex`.
    /// Because Tactica cards use `GraveyardFacility::Consumable`, they are permanently exiled
    /// after casting and NEVER enter Hospital or Factory recovery queues!
    pub fn resolve_tactical_deployment(
        &mut self,
        team_id: u8,
        card: &UnitCardDef,
        target_hex: Hex,
        vfx: &mut VfxManager,
        audio: &mut SpatialAudioEngine,
    ) {
        let enemy_team = if team_id == 1 { 2 } else { 1 };
        let center_w = hex_world_anchor(&self.scenario.map, target_hex, 0.45);
        let key = card.card_key.as_str();

        match key {
            "Orbital Kinetic Strike" => {
                if let Some(idx) = self.entity_at(target_hex) {
                    if self.entities[idx].team_id == enemy_team && self.entities[idx].card.combat_class != CombatClass::Installation {
                        self.entities[idx].unit.current_str_hp = self.entities[idx].unit.current_str_hp.saturating_sub(3);
                        self.combat_log.push(format!("☄ P{} Orbital Kinetic Strike hit {} for 3 armor-piercing STR!", team_id, self.entities[idx].card.unit_name));
                    }
                }
                vfx.spawn_orbital_deploy(center_w, [1.0, 0.35, 0.15]);
                vfx.spawn_popup(center_w, "☄ ORBITAL KINETIC STRIKE", "-3 Piercing STR (Single-Use)", [255, 110, 70]);
                audio.play_spatial(SfxCue::VoxelExplosion, center_w);
            }
            "Tactical Relay Overclock" => {
                *self.economy.actions_remaining.entry(team_id).or_insert(0) += 3; // Net +2 Actions after 1 Action deployed cost!
                vfx.spawn_bio_ability_ring(center_w, [0.2, 0.9, 1.0]);
                vfx.spawn_popup(center_w, "⚡ RELAY OVERCLOCK", "+2 Net Bonus Actions!", [90, 235, 255]);
                audio.play_spatial(SfxCue::BioHealChime, center_w);
                self.combat_log.push(format!("⚡ P{} deployed Tactical Relay Overclock (+2 net Actions this turn)!", team_id));
            }
            "Ion Curtain Shield Dome" => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.unit.current_str_hp > 0 && ent.hex.unsigned_distance_to(target_hex) <= 1 {
                        ent.unit.fortified_stacks = (ent.unit.fortified_stacks + 2).min(3);
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [0.25, 0.78, 1.0]);
                vfx.spawn_popup(center_w, "🛡 ION CURTAIN DOME", "+2 Shields (Radius 1)", [110, 220, 255]);
                audio.play_spatial(SfxCue::BioHealChime, center_w);
                self.combat_log.push(format!("🛡 P{} deployed Ion Curtain Shield Dome at ({}, {})!", team_id, target_hex.x, target_hex.y));
            }
            "Nano-Triage Field Burst" => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.unit.current_str_hp > 0 && ent.hex.unsigned_distance_to(target_hex) <= 1 {
                        ent.unit.current_str_hp = (ent.unit.current_str_hp + 3).min(ent.unit.max_str_hp);
                        ent.unit.poisoned_stacks = 0;
                        ent.unit.rooted_turns = 0;
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [0.15, 0.96, 0.48]);
                vfx.spawn_popup(center_w, "💚 NANO-TRIAGE BURST", "+3 STR & Cleanse", [80, 250, 155]);
                audio.play_spatial(SfxCue::BioHealChime, center_w);
                self.combat_log.push(format!("💚 P{} deployed Nano-Triage Field Burst!", team_id));
            }
            "EMP Blackout Pulse" => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == enemy_team && ent.unit.current_str_hp > 0 && ent.hex.unsigned_distance_to(target_hex) <= 1 {
                        ent.unit.fortified_stacks = 0;
                        ent.unit.stunned_this_turn = true;
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [0.35, 0.55, 1.0]);
                vfx.spawn_popup(center_w, "⚡ EMP BLACKOUT", "Shields Stripped & Stunned", [110, 175, 255]);
                audio.play_spatial(SfxCue::LaserPulse, center_w);
                self.combat_log.push(format!("⚡ P{} deployed EMP Blackout Pulse!", team_id));
            }
            "Satellite Recon Sweep" => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == enemy_team && ent.unit.current_str_hp > 0 && ent.hex.unsigned_distance_to(target_hex) <= 2 {
                        ent.unit.revealed_until_turn_start = true;
                        ent.unit.burrowed = false;
                        ent.unit.target_locked = true;
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [0.2, 0.85, 1.0]);
                vfx.spawn_popup(center_w, "🛰 SATELLITE SWEEP", "Uncloaked & Target Locked", [100, 225, 255]);
                audio.play_spatial(SfxCue::VenomSnare, center_w);
                self.combat_log.push(format!("🛰 P{} deployed Satellite Recon Sweep!", team_id));
            }
            "Apex Bloodlust Roar" => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.card.faction == OrganismFaction::Mammal && ent.unit.current_str_hp > 0 {
                        ent.unit.temp_move_bonus = ent.unit.temp_move_bonus.saturating_add(2);
                        ent.unit.current_armor = ent.unit.current_armor.saturating_add(1);
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [0.95, 0.52, 0.16]);
                vfx.spawn_popup(center_w, "🐾 APEX BLOODLUST", "Mammals +2 MOV / +1 ARM", [250, 155, 65]);
                audio.play_spatial(SfxCue::MeleeClash, center_w);
                self.combat_log.push(format!("🐾 P{} deployed Apex Bloodlust Roar!", team_id));
            }
            "Stampede Shockwave" => {
                if let Some(idx) = self.entity_at(target_hex) {
                    if self.entities[idx].team_id == enemy_team && !self.entities[idx].card.can_fly {
                        let raw = 2u8;
                        let abs = raw.min(self.entities[idx].unit.current_armor);
                        self.entities[idx].unit.current_armor -= abs;
                        self.entities[idx].unit.current_str_hp = self.entities[idx].unit.current_str_hp.saturating_sub(raw - abs);
                        self.entities[idx].unit.stunned_this_turn = true;
                    }
                }
                vfx.spawn_popup(center_w, "🐾 STAMPEDE SHOCKWAVE", "2 DMG & Stun", [245, 165, 75]);
                audio.play_spatial(SfxCue::MeleeClash, center_w);
                self.combat_log.push(format!("🐾 P{} deployed Stampede Shockwave!", team_id));
            }
            "Napalm Basking Surge" => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == enemy_team && ent.unit.current_str_hp > 0 && ent.hex.unsigned_distance_to(target_hex) <= 1 && ent.card.combat_class != CombatClass::Installation {
                        let abs = 2u8.min(ent.unit.current_armor);
                        ent.unit.current_armor -= abs;
                        ent.unit.current_str_hp = ent.unit.current_str_hp.saturating_sub(2 - abs);
                        if !ent.card.poison_immune {
                            ent.unit.poisoned_stacks = (ent.unit.poisoned_stacks + 1).min(3);
                        }
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [1.0, 0.45, 0.1]);
                vfx.spawn_popup(center_w, "🔥 NAPALM SURGE", "2 Fire DMG + Burn", [255, 130, 50]);
                audio.play_spatial(SfxCue::VoxelExplosion, center_w);
                self.combat_log.push(format!("🔥 P{} deployed Napalm Basking Surge!", team_id));
            }
            "Spore Monsoon" => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == enemy_team && ent.unit.current_str_hp > 0 && !ent.card.can_fly && ent.hex.unsigned_distance_to(target_hex) <= 1 {
                        ent.unit.rooted_turns = 2;
                        if !ent.card.poison_immune {
                            ent.unit.poisoned_stacks = (ent.unit.poisoned_stacks + 1).min(3);
                        }
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [0.3, 0.92, 0.35]);
                vfx.spawn_popup(center_w, "🌿 SPORE MONSOON", "Radius-1 Root & Poison", [110, 240, 120]);
                audio.play_spatial(SfxCue::VenomSnare, center_w);
                self.combat_log.push(format!("🌿 P{} deployed Spore Monsoon!", team_id));
            }
            "Bramble Barricade" => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.card.faction == OrganismFaction::Plant && ent.unit.current_str_hp > 0 && ent.hex.unsigned_distance_to(target_hex) <= 2 {
                        ent.unit.current_str_hp = (ent.unit.current_str_hp + 2).min(ent.unit.max_str_hp);
                        ent.unit.fortified_stacks = (ent.unit.fortified_stacks + 1).min(3);
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [0.2, 0.85, 0.4]);
                vfx.spawn_popup(center_w, "🌵 BRAMBLE BARRICADE", "Plants +2 STR / +1 Shield", [95, 235, 130]);
                audio.play_spatial(SfxCue::BioHealChime, center_w);
                self.combat_log.push(format!("🌵 P{} deployed Bramble Barricade!", team_id));
            }
            "Stratospheric Dive-Target" => {
                if let Some(idx) = self.entity_at(target_hex) {
                    if self.entities[idx].team_id == enemy_team && self.entities[idx].card.combat_class != CombatClass::Installation {
                        self.entities[idx].unit.current_str_hp = self.entities[idx].unit.current_str_hp.saturating_sub(2);
                        self.entities[idx].unit.target_locked = true;
                    }
                }
                vfx.spawn_popup(center_w, "🦅 DIVE-TARGET", "-2 Piercing STR & Lock", [115, 205, 255]);
                audio.play_spatial(SfxCue::BallisticVolley, center_w);
                self.combat_log.push(format!("🦅 P{} deployed Stratospheric Dive-Target!", team_id));
            }
            "Neuro-Venom Miasma" => {
                if let Some(idx) = self.entity_at(target_hex) {
                    if self.entities[idx].team_id == enemy_team {
                        self.entities[idx].unit.rooted_turns = 1;
                        if !self.entities[idx].card.poison_immune {
                            self.entities[idx].unit.poisoned_stacks = 3;
                        }
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [0.75, 0.25, 0.95]);
                vfx.spawn_popup(center_w, "🕷 NEURO-VENOM MIASMA", "3 Poison Stacks & Root", [205, 110, 250]);
                audio.play_spatial(SfxCue::VenomSnare, center_w);
                self.combat_log.push(format!("🕷 P{} deployed Neuro-Venom Miasma!", team_id));
            }
            "Pheromone Brood Call" => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.card.faction == OrganismFaction::Insect && ent.unit.current_str_hp > 0 {
                        ent.unit.carried_munitions = ent.card.encumbrance;
                    }
                }
                if team_id == 1 {
                    if let Some(drawn) = self.draw_pile.pop() { self.hand.push(drawn); }
                } else if let Some(drawn) = self.ai_draw_pile.pop() {
                    self.ai_hand.push(drawn);
                }
                vfx.spawn_bio_ability_ring(center_w, [0.95, 0.78, 0.18]);
                vfx.spawn_popup(center_w, "⚡ PHEROMONE CALL", "Insect Ammo Full + Draw 1", [245, 205, 70]);
                audio.play_spatial(SfxCue::UiCardTurn, center_w);
                self.combat_log.push(format!("⚡ P{} deployed Pheromone Brood Call!", team_id));
            }
            "Corrosive Acid Spray" => {
                if let Some(idx) = self.entity_at(target_hex) {
                    if self.entities[idx].team_id == enemy_team && self.entities[idx].card.combat_class != CombatClass::Installation {
                        self.entities[idx].unit.current_armor = 0;
                        self.entities[idx].unit.current_str_hp = self.entities[idx].unit.current_str_hp.saturating_sub(2);
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [0.7, 0.98, 0.15]);
                vfx.spawn_popup(center_w, "☠ CORROSIVE ACID", "Armor Melted + 2 DMG", [185, 255, 65]);
                audio.play_spatial(SfxCue::VenomSnare, center_w);
                self.combat_log.push(format!("☠ P{} deployed Corrosive Acid Spray!", team_id));
            }
            "Abyssal Riptide" => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && (ent.card.faction == OrganismFaction::Fish || ent.card.amphibious_tier > 0) && ent.unit.current_str_hp > 0 {
                        ent.unit.temp_move_bonus = ent.unit.temp_move_bonus.saturating_add(2);
                        ent.unit.fortified_stacks = (ent.unit.fortified_stacks + 1).min(3);
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [0.1, 0.82, 0.92]);
                vfx.spawn_popup(center_w, "🌊 ABYSSAL RIPTIDE", "+2 MOV & +1 Shield", [65, 220, 245]);
                audio.play_spatial(SfxCue::BioHealChime, center_w);
                self.combat_log.push(format!("🌊 P{} deployed Abyssal Riptide!", team_id));
            }
            "Neural Short-Circuit" => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == enemy_team && ent.unit.current_str_hp > 0 && ent.hex.unsigned_distance_to(target_hex) <= 1 {
                        ent.unit.fortified_stacks = 0;
                        ent.unit.stunned_this_turn = true;
                        if !ent.card.poison_immune {
                            ent.unit.poisoned_stacks = (ent.unit.poisoned_stacks + 2).min(3);
                        }
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [0.92, 0.25, 0.85]);
                vfx.spawn_popup(center_w, "⚡ NEURAL SHORT-CIRCUIT", "Stun + 2 Poison", [240, 95, 220]);
                audio.play_spatial(SfxCue::LaserPulse, center_w);
                self.combat_log.push(format!("⚡ P{} deployed Neural Short-Circuit!", team_id));
            }
            "Phantom Mist Veil" => {
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.unit.current_str_hp > 0 && ent.hex.unsigned_distance_to(target_hex) <= 2 {
                        ent.unit.temp_stealth = true;
                        ent.unit.current_str_hp = (ent.unit.current_str_hp + 2).min(ent.unit.max_str_hp);
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [0.2, 0.9, 0.72]);
                vfx.spawn_popup(center_w, "🐸 PHANTOM MIST VEIL", "Stealth Cloak & +2 STR", [90, 240, 195]);
                audio.play_spatial(SfxCue::BioHealChime, center_w);
                self.combat_log.push(format!("🐸 P{} deployed Phantom Mist Veil!", team_id));
            }
            "Hydro-Breach Canal" => {
                let mut flooded = 0;
                for h in std::iter::once(target_hex).chain(target_hex.all_neighbors().into_iter()) {
                    if flooded >= 2 { break; }
                    if let Some(t) = self.scenario.map.tiles.get_mut(&h) {
                        if matches!(t.morphology, Morphology::Flat | Morphology::RoughCover | Morphology::RadiationFallout) {
                            t.morphology = if flooded == 0 { Morphology::DeepRiver } else { Morphology::ShallowFord };
                            t.elevation = 0;
                            self.breached_canal_hexes.insert(h);
                            flooded += 1;
                            if let Some(ei) = self.entity_at(h) {
                                if self.entities[ei].team_id == enemy_team
                                    && !self.entities[ei].card.can_fly
                                    && self.entities[ei].card.amphibious_tier == 0
                                    && self.entities[ei].card.combat_class != CombatClass::Installation
                                {
                                    self.entities[ei].unit.current_str_hp = self.entities[ei].unit.current_str_hp.saturating_sub(2);
                                }
                            }
                        }
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [0.1, 0.85, 0.95]);
                vfx.spawn_popup(center_w, "🌊 HYDRO-BREACH CANAL", "2 Hexes Flooded to Waterway!", [70, 225, 250]);
                audio.play_spatial(SfxCue::OrbitalDeploy, center_w);
                self.combat_log.push(format!("🌊 P{} deployed Hydro-Breach Canal, carving {} navigable waterway hexes!", team_id, flooded));
            }
            "Command Bonus" => {
                *self.economy.actions_remaining.entry(team_id).or_insert(0) += 2;
                vfx.spawn_popup(center_w, "📡 COMMAND BONUS", "+1 Net Action Surge", [110, 230, 255]);
                self.combat_log.push(format!("📡 P{} expended Command Bonus (+1 net Action)!", team_id));
            }
            _ if card.combat_class == CombatClass::Artifact => {
                // Single-Use Genetic Splice Vat Consumable: +1 Shield, +2 STR Heal & Cleanse in Radius 2!
                for ent in self.entities.iter_mut() {
                    if ent.team_id == team_id && ent.unit.current_str_hp > 0 && ent.hex.unsigned_distance_to(target_hex) <= 2 {
                        ent.unit.fortified_stacks = (ent.unit.fortified_stacks + 1).min(3);
                        ent.unit.current_str_hp = (ent.unit.current_str_hp + 2).min(ent.unit.max_str_hp);
                        ent.unit.poisoned_stacks = 0;
                        ent.unit.rooted_turns = 0;
                    }
                }
                vfx.spawn_bio_ability_ring(center_w, [0.25, 0.95, 0.65]);
                vfx.spawn_popup(center_w, "✦ GENETIC SPLICE SURGE", "+1 Shield & +2 STR (Single-Use)", [95, 245, 175]);
                audio.play_spatial(SfxCue::BioHealChime, center_w);
                self.combat_log.push(format!("✦ P{} expended {} (+1 Shield & +2 STR in Radius 2)!", team_id, card.unit_name));
            }
            _ => {
                self.combat_log.push(format!("✨ P{} deployed {}!", team_id, card.unit_name));
            }
        }
        self.reap_destroyed_units();
        self.evaluate_hq_knockout();
    }

    pub fn execute_combat_pair(
        &mut self,
        att_idx: usize,
        def_idx: usize,
        vfx: &mut VfxManager,
        audio: &mut SpatialAudioEngine,
    ) {
        let att_team = self.entities[att_idx].team_id;
        let is_laser = self.entities[att_idx].unit.is_laser;
        let mun_cost = if is_laser { 0 } else { 1 };
        if !self.spend_munition_for_entity(att_idx, mun_cost) {
            self.combat_log.push(format!(
                "{} is out of Munitions and outside Home Supply Range!",
                self.entities[att_idx].card.unit_name
            ));
            return;
        }
        *self.economy.actions_remaining.entry(att_team).or_insert(1) -= 1;
        if is_laser {
            self.entities[att_idx].unit.laser_fired_this_turn = true;
        }
        // Firing reveals a Stealth / Burrowed ambusher until the start of its next turn!
        self.entities[att_idx].unit.revealed_until_turn_start = true;

        let ctx = self.build_combat_context(att_idx, def_idx);
        let is_melee = ctx.is_melee;
        let att_hex = self.entities[att_idx].hex;
        let def_hex = self.entities[def_idx].hex;
        let dist = ctx.hex_distance;

        let att_deck = self.decks[(att_team - 1).min(1) as usize].clone();
        let def_team = self.entities[def_idx].team_id;
        let def_deck = self.decks[(def_team - 1).min(1) as usize].clone();

        // Passive Doctrine: Europe Aegis Bastion (+1 Armor vs ranged attacks exceeding 2 hexes)
        if def_deck.faction == OriginAllegiance::Europe && dist > 2 {
            let def_c = self.entities[def_idx].card.combat_class;
            if def_c == CombatClass::Heavy || def_c == CombatClass::Commander {
                self.entities[def_idx].unit.current_armor = self.entities[def_idx].unit.current_armor.saturating_add(1);
            }
        }

        // Pure Mammal Apex Pack: +1 to Hit rolls when adjacent to at least 1 Mammal ally
        let pure_mammal_apex = att_deck.is_pure_biotype()
            && att_deck.primary_faction == OrganismFaction::Mammal
            && ctx.attacker_pack_tactics_bonus;

        let hit_d6 = self.roll_d6();
        let dmg_d6 = self.roll_d6();
        let had_fortify = self.entities[def_idx].unit.fortified_stacks > 0;
        let delta_str_preview = (i16::from(self.entities[att_idx].unit.current_str_hp)
            - i16::from(self.entities[def_idx].unit.current_str_hp))
        .clamp(-2, 3);

        let (mut att_u, mut def_u) = (
            self.entities[att_idx].unit.clone(),
            self.entities[def_idx].unit.clone(),
        );
        let effective_hit_d6 = if pure_mammal_apex {
            if hit_d6 == 1 {
                1
            } else {
                hit_d6.saturating_add(1).min(6)
            }
        } else {
            hit_d6
        };
        let outcome = resolve_attack(&mut att_u, &mut def_u, ctx, effective_hit_d6, dmg_d6);

        if outcome.hit_succeeded {
            // Passive Doctrine: North America Hardened Core (Laser deals +1 damage on natural 6 hit roll)
            if att_deck.faction == OriginAllegiance::NorthAmerica && is_laser && hit_d6 == 6 && def_u.current_str_hp > 0 {
                def_u.current_str_hp = def_u.current_str_hp.saturating_sub(1);
                self.combat_log.push("[US] North America Hardened Core: Laser Critical dealt +1 bonus damage!".into());
            }
            // Passive Doctrine: Pure Bird Stratospheric Apex (+1 aerial dive/strafe damage)
            if att_deck.is_pure_biotype() && att_deck.primary_faction == OrganismFaction::Bird && self.entities[att_idx].card.can_fly && dist == 1 && def_u.current_str_hp > 0 {
                def_u.current_str_hp = def_u.current_str_hp.saturating_sub(1);
                self.combat_log.push("🦅 Pure Bird Stratospheric Apex: dealt +1 bonus aerial strike damage!".into());
            }
        }

        if outcome.hit_succeeded && self.entities[att_idx].card.root_stacks > 0 {
            let p_imm = self.entities[def_idx].card.poison_immune;
            execute_web_snare(self.entities[att_idx].card.faction, &mut def_u, p_imm);
        }
        if outcome.hit_succeeded && self.entities[att_idx].card.poison_stacks > 0 {
            let p_imm = self.entities[def_idx].card.poison_immune;
            execute_venom_spit(self.entities[att_idx].card.faction, &mut def_u, p_imm);
        }

        let mut retal_loss = 0u8;
        if outcome.triggers_melee_retaliation {
            let r_d6 = self.roll_d6();
            let raw_r = r_d6.min(def_u.current_str_hp);
            let abs_r = raw_r.min(att_u.current_armor);
            att_u.current_armor -= abs_r;
            retal_loss = (raw_r - abs_r).min(att_u.current_str_hp);
            att_u.current_str_hp -= retal_loss;
        }

        if outcome.defender_destroyed && att_team == 1 {
            self.stats_enemies_destroyed += 1;
        }

        self.entities[att_idx].unit = att_u;
        self.entities[def_idx].unit = def_u;

        let from_pos = hex_world_anchor(
            &self.scenario.map,
            att_hex,
            if self.entities[att_idx].card.can_fly { 3.6 } else { 2.7 },
        );
        let to_pos = hex_world_anchor(
            &self.scenario.map,
            def_hex,
            if self.entities[def_idx].card.can_fly { 3.5 } else { 2.6 },
        );
        self.entities[att_idx].yaw_radians = (to_pos.x - from_pos.x).atan2(to_pos.z - from_pos.z);

        let att_uid = self.entities[att_idx].uid;
        // Spawn volumetric 3D tumbling dice in the voxel viewport!
        vfx.spawn_tumbling_combat_dice(
            from_pos,
            to_pos,
            hit_d6,
            dmg_d6,
            outcome.hit_succeeded,
        );
        if is_laser {
            vfx.spawn_laser_attack(
                att_uid,
                from_pos,
                to_pos,
                outcome.hit_succeeded,
                outcome.defender_destroyed,
            );
            audio.play_spatial(SfxCue::LaserPulse, from_pos);
        } else {
            vfx.spawn_ballistic_or_melee(
                att_uid,
                from_pos,
                to_pos,
                is_melee,
                outcome.hit_succeeded,
                outcome.defender_destroyed,
            );
            audio.play_spatial(
                if is_melee {
                    SfxCue::MeleeClash
                } else {
                    SfxCue::BallisticVolley
                },
                from_pos,
            );
        }
        if outcome.defender_destroyed {
            audio.play_spatial(SfxCue::VoxelExplosion, to_pos);
        }

        if had_fortify && outcome.hit_threshold == 0 {
            vfx.spawn_popup(
                to_pos,
                "🛡 FORTIFY BLOCKED!",
                "Absorbed entire strike (-1 Shield Stack)",
                [245, 198, 62],
            );
        } else if outcome.hit_threshold == 7 {
            vfx.spawn_popup(
                to_pos,
                "🕳 BURROWED IMMUNE!",
                "Ranged attack missed underground target",
                [172, 192, 206],
            );
        } else if outcome.hit_succeeded {
            let head = if is_melee {
                format!("⚔ MELEE STRIKE! Dmg D6 [{}]", dmg_d6)
            } else {
                format!("🎲 HIT! D6 [{}] vs {}+  |  Dmg D6 [{}]", hit_d6, outcome.hit_threshold, dmg_d6)
            };
            let sub = format!(
                "ΔSTR {:+} → -{} 🛡 -{} ❤ STR{}{}",
                delta_str_preview,
                outcome.armor_absorbed,
                outcome.strength_damage_dealt,
                if outcome.defender_destroyed { " 💥 KO!" } else { "" },
                if retal_loss > 0 {
                    format!(" (Retal -{}❤)", retal_loss)
                } else {
                    String::new()
                }
            );
            let col = if outcome.defender_destroyed {
                [255, 92, 74]
            } else if att_team == 1 {
                [92, 246, 168]
            } else {
                [255, 156, 68]
            };
            vfx.spawn_popup(to_pos, head, sub, col);
        } else {
            vfx.spawn_popup(
                to_pos,
                format!("💨 MISS! D6 [{}] < {}+", hit_d6, outcome.hit_threshold),
                if retal_loss > 0 {
                    format!("Melee Retaliation: -{} ❤ STR", retal_loss)
                } else {
                    "No damage dealt".to_string()
                },
                [176, 196, 212],
            );
        }

        let att_name = self.entities[att_idx].card.unit_name.clone();
        let def_name = self.entities[def_idx].card.unit_name.clone();
        if outcome.hit_succeeded {
            self.combat_log.push(format!(
                "{} hit {} (D6={} vs {}+, DmgD6={}) -> -{} Armor, -{} STR{}{}",
                att_name,
                def_name,
                hit_d6,
                outcome.hit_threshold,
                dmg_d6,
                outcome.armor_absorbed,
                outcome.strength_damage_dealt,
                if outcome.defender_destroyed { " [DESTROYED]" } else { "" },
                if retal_loss > 0 {
                    format!(" (Retaliation -{} STR)", retal_loss)
                } else {
                    String::new()
                }
            ));
        } else {
            self.combat_log.push(format!(
                "{} missed {} (D6={} vs {}+).",
                att_name, def_name, hit_d6, outcome.hit_threshold
            ));
        }

        self.reap_destroyed_units();
        self.refresh_confusion_states();
        self.update_explored_fog();
        self.reap_destroyed_units();
        self.evaluate_hq_knockout();
    }

    fn execute_unit_ability(
        &mut self,
        ent_idx: usize,
        target_hex: Hex,
        vfx: &mut VfxManager,
        audio: &mut SpatialAudioEngine,
    ) {
        let team = self.entities[ent_idx].team_id;
        if !self.spend_munition_for_entity(ent_idx, 1) {
            if team == self.viewing_team() {
                self.combat_log.push("Insufficient Munitions for ability!".into());
            }
            return;
        }
        *self.economy.actions_remaining.entry(team).or_insert(1) -= 1;

        let caster_card = self.entities[ent_idx].card.clone();
        let center_w = hex_world_anchor(&self.scenario.map, target_hex, 0.35);

        if caster_card.heal_repair_range > 0 {
            if let Some(t_idx) = self.entity_at(target_hex) {
                execute_field_heal(&mut self.entities[t_idx].unit);
                vfx.spawn_bio_ability_ring(center_w, [0.12, 0.95, 0.42]);
                vfx.spawn_popup(
                    center_w,
                    "💚 FIELD BIO-MEND",
                    format!(
                        "{}: {}/{} ❤ STR",
                        self.entities[t_idx].card.unit_name,
                        self.entities[t_idx].unit.current_str_hp,
                        self.entities[t_idx].unit.max_str_hp
                    ),
                    [78, 248, 158],
                );
                audio.play_spatial(SfxCue::BioHealChime, center_w);
                self.combat_log.push(format!(
                    "{} healed/detoxified {} (now {}/{} STR).",
                    caster_card.unit_name,
                    self.entities[t_idx].card.unit_name,
                    self.entities[t_idx].unit.current_str_hp,
                    self.entities[t_idx].unit.max_str_hp
                ));
            }
        } else if caster_card.radar_range > 0 && target_hex != self.entities[ent_idx].hex {
            if let Some(t_idx) = self.entity_at(target_hex) {
                // Active Radar Ping locks AND breaks Stealth!
                self.entities[t_idx].unit.target_locked = true;
                self.entities[t_idx].unit.revealed_until_turn_start = true;
                vfx.spawn_bio_ability_ring(center_w, [0.08, 0.78, 1.0]);
                vfx.spawn_popup(center_w, "📡 RADAR TARGET LOCK", "Revealed & -1 Hit Roll", [76, 224, 255]);
                audio.play_spatial(SfxCue::VenomSnare, center_w);
                self.combat_log.push(format!(
                    "{} locked & revealed {} via Radar Ping!",
                    caster_card.unit_name, self.entities[t_idx].card.unit_name
                ));
            }
        } else if (caster_card.root_stacks > 0 || caster_card.poison_stacks > 0)
            && target_hex != self.entities[ent_idx].hex
        {
            if let Some(t_idx) = self.entity_at(target_hex) {
                let p_imm = self.entities[t_idx].card.poison_immune;
                if caster_card.root_stacks > 0 {
                    execute_web_snare(caster_card.faction, &mut self.entities[t_idx].unit, p_imm);
                }
                if caster_card.poison_stacks > 0 {
                    execute_venom_spit(caster_card.faction, &mut self.entities[t_idx].unit, p_imm);
                }
                vfx.spawn_bio_ability_ring(center_w, [0.62, 0.96, 0.14]);
                vfx.spawn_popup(center_w, "☠ VENOM / WEB SNARE", "Rooted & Envenomed", [168, 252, 68]);
                audio.play_spatial(SfxCue::VenomSnare, center_w);
                self.combat_log.push(format!(
                    "{} snared/envenomed {}!",
                    caster_card.unit_name, self.entities[t_idx].card.unit_name
                ));
            }
        } else {
            self.entities[ent_idx].unit.fortified_stacks =
                (self.entities[ent_idx].unit.fortified_stacks + 1).min(3);
            vfx.spawn_bio_ability_ring(center_w, [0.95, 0.72, 0.16]);
            vfx.spawn_popup(
                center_w,
                "🛡 FORTIFIED",
                format!("+1 Shield Block (×{})", self.entities[ent_idx].unit.fortified_stacks),
                [248, 202, 64],
            );
            audio.play_spatial(SfxCue::BioHealChime, center_w);
            self.combat_log.push(format!(
                "{} fortified (+1 block stack, now {}).",
                caster_card.unit_name, self.entities[ent_idx].unit.fortified_stacks
            ));
        }
    }

    fn reap_destroyed_units(&mut self) {
        let mut survivors = Vec::with_capacity(self.entities.len());
        for ent in self.entities.drain(..) {
            if ent.unit.current_str_hp > 0 {
                survivors.push(ent);
            } else if matches!(
                ent.card.graveyard_queue,
                GraveyardFacility::HospitalBio | GraveyardFacility::FactoryMech
            ) {
                let deck_idx = (ent.team_id.saturating_sub(1)).min(1) as usize;
                let base_copies: u8 = self.decks[deck_idx]
                    .entries
                    .iter()
                    .filter(|de| de.card_key == ent.card.card_key)
                    .map(|de| de.quantity)
                    .sum::<u8>()
                    .max(1);
                let brood_mult: u8 = if self.decks[deck_idx].is_pure_biotype()
                    && self.decks[deck_idx].primary_faction == OrganismFaction::Insect
                    && ent.card.faction == OrganismFaction::Insect
                    && matches!(ent.card.combat_class, CombatClass::Light | CombatClass::Medium)
                {
                    2
                } else {
                    1
                };
                let max_revives: u8 = base_copies.saturating_mul(brood_mult);
                let rev_key = format!("{}:{}", ent.team_id, ent.card.card_key);
                let used = self.economy.revives_used.get(&rev_key).copied().unwrap_or(0);
                if used < max_revives {
                    *self.economy.revives_used.entry(rev_key).or_insert(0) += 1;
                    let ticket = GraveyardTicket {
                        card_key: ent.card.card_key.clone(),
                        facility: ent.card.graveyard_queue,
                        recovered_str: 0,
                        target_str: ent.card.strength_hp,
                    };
                    if ent.team_id == 1 {
                        self.graveyard.push(ticket);
                    } else if ent.team_id == 2 {
                        self.ai_graveyard.push(ticket);
                    }
                } else {
                    self.combat_log.push(format!(
                        "⚰ P{} {} template exhausted ({}/{} revives used) — permanently lost!",
                        ent.team_id, ent.card.unit_name, used, max_revives
                    ));
                }
            }
        }
        self.entities = survivors;
    }

    fn evaluate_hq_knockout(&mut self) {
        if self.match_outcome.is_some() {
            return;
        }
        let p1_hq_alive = self.entities.iter().any(|e| {
            e.team_id == 1
                && e.unit.current_str_hp > 0
                && e.card.combat_class == CombatClass::Installation
        });
        let p2_hq_alive = self.entities.iter().any(|e| {
            e.team_id == 2
                && e.unit.current_str_hp > 0
                && e.card.combat_class == CombatClass::Installation
        });

        if !p2_hq_alive {
            self.match_outcome = Some(MatchOutcome {
                winner_team: 1,
                headline: if self.hotseat_mode {
                    "PLAYER 1 VICTORY ON THE WILDFRONT!".into()
                } else {
                    "VICTORY ON THE WILDFRONT!".into()
                },
                reason: "Team 1 razed Team 2's Command Center and severed their continental bunker uplink.".into(),
                turns_taken: self.turn_number,
                units_deployed: self.stats_units_deployed,
                enemies_destroyed: self.stats_enemies_destroyed,
            });
        } else if !p1_hq_alive {
            self.match_outcome = Some(MatchOutcome {
                winner_team: 2,
                headline: if self.hotseat_mode {
                    "PLAYER 2 VICTORY ON THE WILDFRONT!".into()
                } else {
                    "COMMAND CENTER LOST — DEFEAT".into()
                },
                reason: "Team 2 razed Team 1's Command Center, collapsing their Supply and Control network.".into(),
                turns_taken: self.turn_number,
                units_deployed: self.stats_units_deployed,
                enemies_destroyed: self.stats_enemies_destroyed,
            });
        }
    }

    fn evaluate_turn_objectives(&mut self) {
        self.reap_destroyed_units();
        self.evaluate_hq_knockout();
        if self.match_outcome.is_some() {
            return;
        }

        // Score Scenario Objectives at End of Full Round (`active_team == 2`), giving P1 & P2 equal turns!
        if self.active_team == 1 {
            return;
        }

        // Turn 30+ Orbital Fallout Storm & Mutual Roster Exhaustion Sudden Death:
        // Prevents late-game building-only stalemates and caps ultra-long attrition wars cleanly!
        if self.active_team == 2 && self.turn_number >= 30 {
            let decay: u8 = if self.turn_number >= 36 { 2 } else { 1 };
            for ent in self.entities.iter_mut() {
                if ent.card.combat_class == CombatClass::Installation {
                    ent.unit.current_str_hp = ent.unit.current_str_hp.saturating_sub(decay);
                }
            }
            self.combat_log.push(format!("☢ Orbital Fallout Storm (Turn {}): -{} hull STR to all Command Centers!", self.turn_number, decay));
            let p1_alive = self.entities.iter().any(|e| e.team_id == 1 && e.card.combat_class == CombatClass::Installation && e.unit.current_str_hp > 0);
            let p2_alive = self.entities.iter().any(|e| e.team_id == 2 && e.card.combat_class == CombatClass::Installation && e.unit.current_str_hp > 0);
            if !p1_alive || !p2_alive {
                let winner = if p1_alive && !p2_alive {
                    1
                } else if p2_alive && !p1_alive {
                    2
                } else {
                    let s1 = self.entities.iter().filter(|e| e.team_id == 1 && e.unit.current_str_hp > 0).count();
                    let s2 = self.entities.iter().filter(|e| e.team_id == 2 && e.unit.current_str_hp > 0).count();
                    if s1 > s2 { 1 } else { 2 }
                };
                self.match_outcome = Some(MatchOutcome {
                    winner_team: winner,
                    headline: format!("ORBITAL FALLOUT SURVIVOR — P{} WINS!", winner),
                    reason: "Orbital Fallout Storm collapsed the rival Command Center.".into(),
                    turns_taken: self.turn_number,
                    units_deployed: self.stats_units_deployed,
                    enemies_destroyed: self.stats_enemies_destroyed,
                });
                return;
            }
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
                let vault = Hex::ZERO;
                let p1_near = self
                    .entities
                    .iter()
                    .any(|e| e.team_id == 1 && e.unit.current_str_hp > 0 && e.hex.unsigned_distance_to(vault) <= 1);
                let p2_near = self
                    .entities
                    .iter()
                    .any(|e| e.team_id == 2 && e.unit.current_str_hp > 0 && e.hex.unsigned_distance_to(vault) <= 1);
                if p1_near && !p2_near {
                    self.p1_obj_progress += 1;
                } else if p2_near && !p1_near {
                    self.p2_obj_progress += 1;
                }
                if self.p1_obj_progress >= u16::from(target_turns) {
                    self.match_outcome = Some(MatchOutcome {
                        winner_team: 1,
                        headline: "PLAYER 1 SECURED THE CALDERA VAULT!".into(),
                        reason: format!("Team 1 held the central Bunker Vault uncontested for {} turns.", target_turns),
                        turns_taken: self.turn_number,
                        units_deployed: self.stats_units_deployed,
                        enemies_destroyed: self.stats_enemies_destroyed,
                    });
                } else if self.p2_obj_progress >= u16::from(target_turns) {
                    self.match_outcome = Some(MatchOutcome {
                        winner_team: 2,
                        headline: if self.hotseat_mode { "PLAYER 2 SECURED THE CALDERA VAULT!".into() } else { "CALDERA VAULT LOST — DEFEAT".into() },
                        reason: format!("Team 2 held the central Bunker Vault for {} turns.", target_turns),
                        turns_taken: self.turn_number,
                        units_deployed: self.stats_units_deployed,
                        enemies_destroyed: self.stats_enemies_destroyed,
                    });
                }
            }
            VictoryCondition::SalvageMunitionsOrRaze { target_munitions } => {
                for &oh in &obj_hexes {
                    if let Some(idx) = self.entity_at(oh) {
                        let t = self.entities[idx].team_id;
                        if t == 1 {
                            self.p1_obj_progress += 2;
                            *self.economy.home_base_munitions.entry(1).or_insert(0) += 1;
                        } else if t == 2 {
                            self.p2_obj_progress += 2;
                            *self.economy.home_base_munitions.entry(2).or_insert(0) += 1;
                        }
                    }
                }
                if self.p1_obj_progress >= target_munitions {
                    self.match_outcome = Some(MatchOutcome {
                        winner_team: 1,
                        headline: "BIO-RIVER CONVOY SALVAGED — P1 WINS!".into(),
                        reason: format!("Team 1 harvested {}/{} Salvaged Munitions.", self.p1_obj_progress, target_munitions),
                        turns_taken: self.turn_number,
                        units_deployed: self.stats_units_deployed,
                        enemies_destroyed: self.stats_enemies_destroyed,
                    });
                } else if self.p2_obj_progress >= target_munitions {
                    self.match_outcome = Some(MatchOutcome {
                        winner_team: 2,
                        headline: if self.hotseat_mode { "BIO-RIVER CONVOY SALVAGED — P2 WINS!".into() } else { "CONVOY CACHES PLUNDERED — DEFEAT".into() },
                        reason: format!("Team 2 harvested {}/{} Salvaged Munitions.", self.p2_obj_progress, target_munitions),
                        turns_taken: self.turn_number,
                        units_deployed: self.stats_units_deployed,
                        enemies_destroyed: self.stats_enemies_destroyed,
                    });
                }
            }
            VictoryCondition::ControlIslandRelays {
                required_relays,
                target_turns,
            } => {
                let mut p1_relays = 0u8;
                let mut p2_relays = 0u8;
                for &oh in &obj_hexes {
                    let w1: u32 = self
                        .entities
                        .iter()
                        .filter(|e| e.team_id == 1 && e.unit.current_str_hp > 0 && e.card.card_key != "Cargo Drop")
                        .map(|e| {
                            let d = e.hex.unsigned_distance_to(oh);
                            if d == 0 { 2 } else if d == 1 { 1 } else { 0 }
                        })
                        .sum();
                    let w2: u32 = self
                        .entities
                        .iter()
                        .filter(|e| e.team_id == 2 && e.unit.current_str_hp > 0 && e.card.card_key != "Cargo Drop")
                        .map(|e| {
                            let d = e.hex.unsigned_distance_to(oh);
                            if d == 0 { 2 } else if d == 1 { 1 } else { 0 }
                        })
                        .sum();
                    if w1 > w2 {
                        p1_relays += 1;
                    } else if w2 > w1 {
                        p2_relays += 1;
                    }
                }
                if p1_relays >= required_relays {
                    self.p1_obj_progress += 1;
                }
                if p2_relays >= required_relays {
                    self.p2_obj_progress += 1;
                }
                if self.p1_obj_progress >= u16::from(target_turns) {
                    self.match_outcome = Some(MatchOutcome {
                        winner_team: 1,
                        headline: "ATOLL RELAY NETWORK DOMINATED — P1 WINS!".into(),
                        reason: format!("Team 1 controlled {}+ island relays for {} turns.", required_relays, target_turns),
                        turns_taken: self.turn_number,
                        units_deployed: self.stats_units_deployed,
                        enemies_destroyed: self.stats_enemies_destroyed,
                    });
                } else if self.p2_obj_progress >= u16::from(target_turns) {
                    self.match_outcome = Some(MatchOutcome {
                        winner_team: 2,
                        headline: if self.hotseat_mode { "ATOLL RELAY NETWORK DOMINATED — P2 WINS!".into() } else { "ATOLL RELAYS CAPTURED — DEFEAT".into() },
                        reason: format!("Team 2 controlled {}+ island relays for {} turns.", required_relays, target_turns),
                        turns_taken: self.turn_number,
                        units_deployed: self.stats_units_deployed,
                        enemies_destroyed: self.stats_enemies_destroyed,
                    });
                }
            }
            VictoryCondition::HoldCentralControlSpire { target_turns } => {
                let spire = Hex::ZERO;
                let p1_adj = self
                    .entities
                    .iter()
                    .any(|e| e.team_id == 1 && e.unit.current_str_hp > 0 && e.hex.unsigned_distance_to(spire) <= 1);
                let p2_adj = self
                    .entities
                    .iter()
                    .any(|e| e.team_id == 2 && e.unit.current_str_hp > 0 && e.hex.unsigned_distance_to(spire) <= 1);
                if p1_adj && !p2_adj {
                    self.p1_obj_progress += 1;
                } else if p2_adj && !p1_adj {
                    self.p2_obj_progress += 1;
                }
                if self.p1_obj_progress >= u16::from(target_turns) {
                    self.match_outcome = Some(MatchOutcome {
                        winner_team: 1,
                        headline: "CONTROL SPIRE ASCENDANCY — P1 WINS!".into(),
                        reason: format!("Team 1 held the Central Psychic Control Spire for {} turns.", target_turns),
                        turns_taken: self.turn_number,
                        units_deployed: self.stats_units_deployed,
                        enemies_destroyed: self.stats_enemies_destroyed,
                    });
                } else if self.p2_obj_progress >= u16::from(target_turns) {
                    self.match_outcome = Some(MatchOutcome {
                        winner_team: 2,
                        headline: if self.hotseat_mode { "CONTROL SPIRE ASCENDANCY — P2 WINS!".into() } else { "CONTROL SPIRE OVERRUN — DEFEAT".into() },
                        reason: format!("Team 2 held the Central Control Spire for {} turns.", target_turns),
                        turns_taken: self.turn_number,
                        units_deployed: self.stats_units_deployed,
                        enemies_destroyed: self.stats_enemies_destroyed,
                    });
                }
            }
        }
    }

    fn start_team_turn(&mut self, team_id: u8) {
        let mut pairs: Vec<(PlayerOwner, &mut BoardUnit)> = self
            .entities
            .iter_mut()
            .map(|e| (PlayerOwner(e.team_id), &mut e.unit))
            .collect();
        begin_player_turn(team_id, &mut self.economy, &mut pairs);

        // Passive Doctrine: Africa Solar Dynamo (+1 bonus Munition every even turn)
        let team_deck = self.decks[(team_id - 1).min(1) as usize].clone();
        if team_deck.faction == OriginAllegiance::Africa && (self.turn_number % 3 == 0) {
            *self.economy.home_base_munitions.entry(team_id).or_insert(0) += 1;
            self.combat_log.push(format!("🌍 Africa Solar Dynamo: Team {} Command Center generated +1 bonus Munition!", team_id));
        }

        // Passive Doctrine: South America Biophilic Synthesis (Units on Jungle/River heal +1 STR at turn start)
        if team_deck.faction == OriginAllegiance::SouthAmerica {
            for ent in self.entities.iter_mut() {
                if ent.team_id == team_id && ent.unit.current_str_hp > 0 && ent.card.faction != OrganismFaction::Human {
                    if let Some(tile) = self.scenario.map.tiles.get(&ent.hex) {
                        if matches!(tile.morphology, Morphology::RoughCover | Morphology::ShallowFord | Morphology::ArmoredDam) {
                            if ent.unit.current_str_hp < ent.unit.max_str_hp {
                                ent.unit.current_str_hp = (ent.unit.current_str_hp + 1).min(ent.unit.max_str_hp);
                            }
                        }
                    }
                }
            }
        }

        // Pelagic Home-Water Gill Recovery: Fish submerged in DeepRiver/OpenOcean restore +1 STR at turn start
        for ent in self.entities.iter_mut() {
            if ent.team_id == team_id && ent.card.faction == OrganismFaction::Fish && ent.unit.current_str_hp > 0 && ent.unit.current_str_hp < ent.unit.max_str_hp {
                if self.scenario.map.tiles.get(&ent.hex).map_or(false, |t| t.morphology.is_water()) {
                    ent.unit.current_str_hp = (ent.unit.current_str_hp + 1).min(ent.unit.max_str_hp);
                }
            }
        }

        // Passive Doctrine: Pure Amphibian Metamorphic Osmosis (Regenerate +1 STR every turn)
        if team_deck.is_pure_biotype() && team_deck.primary_faction == OrganismFaction::Amphibian {
            for ent in self.entities.iter_mut() {
                if ent.team_id == team_id && ent.card.faction == OrganismFaction::Amphibian && ent.unit.current_str_hp > 0 {
                    if ent.unit.current_str_hp < ent.unit.max_str_hp {
                        ent.unit.current_str_hp = (ent.unit.current_str_hp + 1).min(ent.unit.max_str_hp);
                        if ent.card.faction != OrganismFaction::Human {
                            ent.unit.temp_move_bonus = ent.unit.temp_move_bonus.saturating_add(1);
                        }
                    }
                }
            }
        }

        self.reap_destroyed_units();
        self.refresh_confusion_states();
        self.update_explored_fog();
        self.reap_destroyed_units();
        self.evaluate_hq_knockout();

        let supply = self.team_supply_hexes(team_id);
        let has_hosp = self
            .entities
            .iter()
            .any(|e| e.team_id == team_id && e.card.card_key == "Hospital" && supply.contains(&e.hex));
        let has_fact = self
            .entities
            .iter()
            .any(|e| e.team_id == team_id && e.card.card_key == "Factory" && supply.contains(&e.hex));

        if team_id == 1 {
            if self.hand.len() < 7 {
                if let Some(drawn) = self.draw_pile.pop() {
                    self.hand.push(drawn);
                }
            }
            let mut rem = Vec::new();
            let mut active_bays = 0u8;
            for mut t in self.graveyard.drain(..) {
                let can_bay = (t.facility == GraveyardFacility::HospitalBio && has_hosp)
                    || (t.facility == GraveyardFacility::FactoryMech && has_fact);
                let cur_m = self.economy.home_base_munitions.get(&1).copied().unwrap_or(0);
                if can_bay && active_bays < 2 && cur_m >= 1 {
                    *self.economy.home_base_munitions.entry(1).or_insert(0) -= 1;
                    active_bays += 1;
                    if t.tick_recovery(has_hosp, has_fact) {
                        self.combat_log.push(format!(
                            "P1 {} restored from Graveyard back to hand (-1 Munition)!",
                            t.card_key
                        ));
                        self.hand.push(t.card_key);
                    } else {
                        rem.push(t);
                    }
                } else {
                    rem.push(t);
                }
            }
            self.graveyard = rem;
        } else if team_id == 2 {
            if self.ai_hand.len() < 7 {
                if let Some(drawn) = self.ai_draw_pile.pop() {
                    self.ai_hand.push(drawn);
                }
            }
            let mut rem = Vec::new();
            let mut active_bays = 0u8;
            for mut t in self.ai_graveyard.drain(..) {
                let can_bay = (t.facility == GraveyardFacility::HospitalBio && has_hosp)
                    || (t.facility == GraveyardFacility::FactoryMech && has_fact);
                let cur_m = self.economy.home_base_munitions.get(&2).copied().unwrap_or(0);
                if can_bay && active_bays < 2 && cur_m >= 1 {
                    *self.economy.home_base_munitions.entry(2).or_insert(0) -= 1;
                    active_bays += 1;
                    if t.tick_recovery(has_hosp, has_fact) {
                        self.combat_log.push(format!(
                            "P2 {} restored from Graveyard back to hand (-1 Munition)!",
                            t.card_key
                        ));
                        self.ai_hand.push(t.card_key);
                    } else {
                        rem.push(t);
                    }
                } else {
                    rem.push(t);
                }
            }
            self.ai_graveyard = rem;
        }
    }

    /// Unified End-Turn dispatcher supporting both 1v1 Local Hotseat and Solo vs AI.
    pub fn end_active_turn(
        &mut self,
        vfx: &mut VfxManager,
        audio: &mut SpatialAudioEngine,
    ) {
        if self.match_outcome.is_some() {
            return;
        }
        if self.hotseat_handoff_pending {
            self.hotseat_handoff_pending = false;
            audio.play_ui(SfxCue::UiCardTurn);
            return;
        }

        if let Some(net_team) = self.online_local_team {
            if self.active_team != net_team {
                return;
            }
            self.advance_two_player_turn(audio);
            self.emit_lockstep_cmd(LockstepCommand::EndTurn { team_id: net_team });
            return;
        }
        if self.hotseat_mode {
            audio.play_ui(SfxCue::UiCardTurn);
            self.mode = CommandMode::Inspect;
            self.selected_hex = None;
            if self.active_team == 1 {
                self.active_team = 2;
                self.start_team_turn(2);
                if self.match_outcome.is_none() && self.fog_enabled {
                    self.hotseat_handoff_pending = true;
                }
            } else {
                self.evaluate_turn_objectives();
                if self.match_outcome.is_none() {
                    self.turn_number += 1;
                    self.active_team = 1;
                    self.start_team_turn(1);
                    self.push_timeline_keyframe();
                    if self.fog_enabled {
                        self.hotseat_handoff_pending = true;
                    }
                }
            }
        } else {
            self.end_player_turn_and_run_ai(vfx, audio);
        }
    }

    pub fn advance_two_player_turn(&mut self, audio: &mut SpatialAudioEngine) {
        audio.play_ui(SfxCue::UiCardTurn);
        self.mode = CommandMode::Inspect;
        self.selected_hex = None;
        if self.active_team == 1 {
            self.active_team = 2;
            self.start_team_turn(2);
        } else {
            self.evaluate_turn_objectives();
            if self.match_outcome.is_none() {
                self.turn_number += 1;
                self.active_team = 1;
                self.start_team_turn(1);
                self.push_timeline_keyframe();
            }
        }
    }

    pub fn rest_unit_at_hex(
        &mut self,
        unit_hex: Hex,
        emit_net: bool,
        vfx: &mut VfxManager,
        audio: &mut SpatialAudioEngine,
    ) {
        let Some(ent_idx) = self.entity_at(unit_hex) else {
            return;
        };
        let team = self.entities[ent_idx].team_id;
        if self.economy.actions_remaining.get(&team).copied().unwrap_or(0) == 0 {
            return;
        }
        if self.entities[ent_idx].unit.current_str_hp >= self.entities[ent_idx].unit.max_str_hp {
            return;
        }
        *self.economy.actions_remaining.entry(team).or_insert(1) -= 1;
        let r_deck = &self.decks[(team - 1).min(1) as usize];
        let heal_amt = if r_deck.is_pure_biotype() && r_deck.primary_faction == OrganismFaction::Reptile && self.entities[ent_idx].card.faction == OrganismFaction::Reptile { 2 } else { 1 };
        self.entities[ent_idx].unit.current_str_hp = (self.entities[ent_idx].unit.current_str_hp + heal_amt).min(self.entities[ent_idx].unit.max_str_hp);
        let wpos = hex_world_anchor(&self.scenario.map, unit_hex, 0.3);
        vfx.spawn_bio_ability_ring(wpos, [0.2, 0.95, 0.5]);
        vfx.spawn_popup(wpos, "💤 FIELD REST", "+1 ❤ STR", [88, 242, 152]);
        audio.play_spatial(SfxCue::BioHealChime, wpos);
        self.combat_log.push(format!(
            "P{} {} rested (+1 STR).",
            team, self.entities[ent_idx].card.unit_name
        ));
        if emit_net {
            self.emit_lockstep_cmd(LockstepCommand::RestUnit { team_id: team, unit_hex });
        }
    }

    fn emit_lockstep_cmd(&mut self, cmd: LockstepCommand) {
        if self.online_local_team.is_some() {
            let post_state_hash = self.compute_state_hash();
            self.outbound_net_cmds.push(StampedLockstepCommand {
                cmd,
                post_state_hash,
            });
        }
    }

    /// Computes a deterministic 64-bit FNV-1a state digest across turn, economy, cards, entities, and map tiles.
    pub fn compute_state_hash(&self) -> u64 {
        #[inline]
        fn fnv_u64(mut h: u64, val: u64) -> u64 {
            for b in val.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01B3);
            }
            h
        }
        #[inline]
        fn fnv_str(mut h: u64, s: &str) -> u64 {
            for b in s.as_bytes() {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x0100_0000_01B3);
            }
            h ^= 0xFF;
            h.wrapping_mul(0x0100_0000_01B3)
        }

        let mut h: u64 = 0xCBF2_9CE4_8422_2325;
        h = fnv_u64(h, u64::from(self.turn_number));
        h = fnv_u64(h, u64::from(self.active_team));
        h = fnv_u64(h, u64::from(self.p1_obj_progress));
        h = fnv_u64(h, u64::from(self.p2_obj_progress));
        h = fnv_u64(h, self.next_uid);
        for tid in [1u8, 2u8] {
            h = fnv_u64(h, u64::from(self.economy.home_base_munitions.get(&tid).copied().unwrap_or(0)));
            h = fnv_u64(h, u64::from(self.economy.actions_remaining.get(&tid).copied().unwrap_or(0)));
            h = fnv_u64(h, u64::from(self.economy.faction_ability_used.get(&tid).copied().unwrap_or(false)));
            h = fnv_u64(h, u64::from(self.economy.primal_surge_used.get(&tid).copied().unwrap_or(false)));
        }
        for ck in &self.hand {
            h = fnv_str(h, ck);
        }
        h = fnv_u64(h, self.draw_pile.len() as u64);
        for ck in &self.ai_hand {
            h = fnv_str(h, ck);
        }
        h = fnv_u64(h, self.ai_draw_pile.len() as u64);
        for gy in [&self.graveyard, &self.ai_graveyard] {
            h = fnv_u64(h, gy.len() as u64);
            for t in gy {
                h = fnv_str(h, &t.card_key);
                h = fnv_u64(h, u64::from(t.recovered_str));
                h = fnv_u64(h, u64::from(t.target_str));
            }
        }

        let mut sorted_ents: Vec<&LiveEntity> = self
            .entities
            .iter()
            .filter(|e| e.unit.current_str_hp > 0)
            .collect();
        sorted_ents.sort_by_key(|e| e.uid);
        for e in sorted_ents {
            h = fnv_u64(h, e.uid);
            h = fnv_u64(h, u64::from(e.team_id));
            h = fnv_u64(h, (e.hex.x as i64) as u64);
            h = fnv_u64(h, (e.hex.y as i64) as u64);
            h = fnv_u64(h, u64::from(e.unit.current_str_hp));
            h = fnv_u64(h, u64::from(e.unit.current_armor));
            h = fnv_u64(h, u64::from(e.unit.evade));
            h = fnv_u64(h, u64::from(e.unit.fortified_stacks));
            h = fnv_u64(h, u64::from(e.unit.carried_munitions));
            h = fnv_u64(h, u64::from(e.unit.poisoned_stacks));
            h = fnv_u64(h, u64::from(e.unit.rooted_turns));
            h = fnv_u64(h, u64::from(e.unit.temp_move_bonus));
            h = fnv_u64(h, u64::from(e.unit.target_locked));
            h = fnv_u64(h, u64::from(e.unit.stunned_this_turn));
            h = fnv_u64(h, u64::from(e.unit.temp_stealth));
            h = fnv_u64(h, u64::from(e.unit.burrowed));
        }

        let mut tiles: Vec<_> = self.scenario.map.tiles.iter().collect();
        tiles.sort_by_key(|(hx, _)| (hx.x, hx.y));
        for (hx, t) in tiles {
            h = fnv_u64(h, (((hx.x as i64) << 16) ^ (hx.y as i64)) as u64);
            h = fnv_u64(h, t.morphology as u64);
            h = fnv_u64(h, u64::from(t.elevation));
        }
        h
    }

    /// Replays a remote peer's `LockstepCommand` deterministically and verifies its `post_state_hash`.
    pub fn apply_remote_lockstep_command(
        &mut self,
        cmd: LockstepCommand,
        expected_post_hash: u64,
        vfx: &mut VfxManager,
        audio: &mut SpatialAudioEngine,
    ) -> Result<u64, String> {
        let saved_net = self.online_local_team.take();
        let prev_hotseat = self.hotseat_mode;
        self.hotseat_mode = true; // Allow either team's move/deploy/attack validation to execute cleanly
        match cmd {
            LockstepCommand::DeployCard { team_id, hand_idx, hex } => {
                self.active_team = team_id;
                self.mode = CommandMode::DeployHandCard(hand_idx);
                self.handle_hex_click(hex, vfx, audio);
            }
            LockstepCommand::MoveUnit { team_id, from_hex, to_hex } => {
                self.active_team = team_id;
                self.selected_hex = Some(from_hex);
                self.mode = CommandMode::MoveSelectedUnit;
                self.handle_hex_click(to_hex, vfx, audio);
            }
            LockstepCommand::AttackUnit { team_id, attacker_hex, defender_hex } => {
                self.active_team = team_id;
                if let (Some(att_i), Some(def_i)) = (self.entity_at(attacker_hex), self.entity_at(defender_hex)) {
                    self.execute_combat_pair(att_i, def_i, vfx, audio);
                }
            }
            LockstepCommand::CastAbility { team_id, caster_hex, target_hex } => {
                self.active_team = team_id;
                if let Some(ent_i) = self.entity_at(caster_hex) {
                    self.execute_unit_ability(ent_i, target_hex, vfx, audio);
                }
            }
            LockstepCommand::RestUnit { team_id, unit_hex } => {
                self.active_team = team_id;
                self.rest_unit_at_hex(unit_hex, false, vfx, audio);
            }
            LockstepCommand::FactionCommand { team_id } => {
                self.active_team = team_id;
                self.execute_faction_command_ability(team_id, false, vfx, audio);
            }
            LockstepCommand::PrimalSurge { team_id } => {
                self.active_team = team_id;
                self.execute_biotype_primal_surge(team_id, false, vfx, audio);
            }
            LockstepCommand::EndTurn { team_id } => {
                self.active_team = team_id;
                self.advance_two_player_turn(audio);
            }
        }
        self.hotseat_mode = prev_hotseat;
        self.online_local_team = saved_net;
        self.outbound_net_cmds.clear();

        let actual_hash = self.compute_state_hash();
        if actual_hash != expected_post_hash {
            Err(format!(
                "TCP Lockstep desync detected: expected 0x{expected_post_hash:016X}, local 0x{actual_hash:016X}"
            ))
        } else {
            Ok(actual_hash)
        }
    }

    fn try_ai_deploy_from_hand(&mut self, vfx: &mut VfxManager, audio: &mut SpatialAudioEngine) -> bool {
        let ai_mun = self.economy.home_base_munitions.get(&2).copied().unwrap_or(0);
        if ai_mun == 0 || self.ai_hand.is_empty() {
            return false;
        }
        let ai_board_count = self
            .entities
            .iter()
            .filter(|e| e.team_id == 2 && e.unit.current_str_hp > 0)
            .count();
        let board_cap = match self.ai_difficulty {
            wf_sim::AiDifficulty::Recruit => 7,
            wf_sim::AiDifficulty::Veteran => 10,
            wf_sim::AiDifficulty::Grandmaster2Ply => 14,
        };
        if ai_board_count >= board_cap {
            return false;
        }
        if self.ai_difficulty == wf_sim::AiDifficulty::Recruit && (self.dice_rng.next_u64() % 4 == 0) {
            return false;
        }

        let any_confused = self
            .entities
            .iter()
            .any(|e| e.team_id == 2 && e.unit.current_str_hp > 0 && e.unit.confused);
        let ai_nodes = self
            .entities
            .iter()
            .filter(|e| e.team_id == 2 && e.unit.current_str_hp > 0 && e.unit.emits_node)
            .count();
        let needs_hosp = self
            .ai_graveyard
            .iter()
            .any(|g| g.facility == GraveyardFacility::HospitalBio)
            && !self.entities.iter().any(|e| e.team_id == 2 && e.card.card_key == "Hospital");
        let needs_fact = self
            .ai_graveyard
            .iter()
            .any(|g| g.facility == GraveyardFacility::FactoryMech)
            && !self.entities.iter().any(|e| e.team_id == 2 && e.card.card_key == "Factory");

        let focal_hex = self
            .scenario
            .objective_hexes
            .first()
            .map(|&(q, r)| Hex::new(q, r))
            .unwrap_or(Hex::ZERO);

        let ai_hq_hex = self
            .entities
            .iter()
            .find(|e| e.team_id == 2 && e.card.combat_class == CombatClass::Installation)
            .map(|e| e.hex)
            .unwrap_or(Hex::ZERO);

        let mut best_choice: Option<(i32, usize, Hex, UnitCardDef, u16)> = None;
        for (h_idx, ck) in self.ai_hand.iter().enumerate() {
            let Some(card) = self.card_def(ck).cloned() else {
                continue;
            };
            let valid_hexes = self.valid_deploy_hexes_for_team(2, ck);
            if valid_hexes.is_empty() && !(card.combat_class == CombatClass::Artifact && ck == "Cargo Drop") {
                continue;
            }
            let cost = u16::from(card.encumbrance);
            if cost > ai_mun {
                continue;
            }

            let mut score = i32::from(card.deck_points);
            if self.ai_difficulty == wf_sim::AiDifficulty::Grandmaster2Ply {
                score += i32::from(card.combat_ehp_tenths / 5) + i32::from(card.ranged_distance) * 8;
            }
            if (any_confused || ai_nodes < 2) && card.emits_node {
                score += 65;
            }
            if needs_hosp && ck == "Hospital" {
                score += 55;
            }
            if needs_fact && ck == "Factory" {
                score += 55;
            }
            if card.combat_class == CombatClass::SupplyDepot && ai_board_count < 6 {
                score += 22;
            }

            let target_hex = if ck == "Hydro-Breach Canal" {
                let water_hexes: HashSet<Hex> = self
                    .scenario
                    .map
                    .tiles
                    .iter()
                    .filter(|(_, t)| t.morphology.is_water())
                    .map(|(&h, _)| h)
                    .collect();
                let Some(canal_hex) = self
                    .scenario
                    .map
                    .tiles
                    .iter()
                    .filter(|(&h, t)| {
                        matches!(t.morphology, Morphology::Flat | Morphology::RoughCover | Morphology::RadiationFallout)
                            && h != focal_hex
                            && h != ai_hq_hex
                            && h.all_neighbors().iter().any(|nb| water_hexes.contains(nb))
                    })
                    .map(|(&h, _)| h)
                    .min_by_key(|h| h.unsigned_distance_to(focal_hex))
                else {
                    continue;
                };
                canal_hex
            } else if card.combat_class == CombatClass::TacticalDeployment || (card.combat_class == CombatClass::Artifact && card.card_key != "Cargo Drop") {
                if card.heal_repair_range > 0 || card.shield > 0 || card.action_bonus > 0 {
                    self.entities.iter().find(|e| e.team_id == 2 && !e.card.combat_class.is_building()).map(|e| e.hex).unwrap_or(ai_hq_hex)
                } else if let Some(en) = self.entities.iter().find(|e| e.team_id == 1 && !e.card.combat_class.is_building() && e.unit.current_str_hp > 0) {
                    en.hex
                } else {
                    continue;
                }
            } else if card.combat_class.is_building() && card.combat_class != CombatClass::ControlNode {
                valid_hexes
                    .into_iter()
                    .min_by_key(|h| h.unsigned_distance_to(ai_hq_hex))
                    .unwrap_or(ai_hq_hex)
            } else {
                let refs: Vec<(PlayerOwner, HexPos, &BoardUnit)> = self
                    .entities
                    .iter()
                    .filter(|e| e.unit.current_str_hp > 0)
                    .map(|e| (PlayerOwner(e.team_id), HexPos(e.hex), &e.unit))
                    .collect();
                let leash = if card.is_autonomous { 0 } else { card.node_leash_range };
                valid_hexes
                    .into_iter()
                    .filter(|&h| card.emits_node || is_within_control_node(2, leash, h, &refs))
                    .min_by_key(|h| h.unsigned_distance_to(focal_hex))
                    .unwrap_or(ai_hq_hex)
            };

            if target_hex == ai_hq_hex && self.entity_at(target_hex).is_some() && ck != "Cargo Drop" {
                continue;
            }
            if best_choice.as_ref().map_or(true, |(bs, _, _, _, _)| score > *bs) {
                best_choice = Some((score, h_idx, target_hex, card, cost));
            }
        }

        let Some((_score, h_idx, deploy_hex, card, cost)) = best_choice else {
            return false;
        };

        *self.economy.home_base_munitions.entry(2).or_insert(0) -= cost;
        *self.economy.actions_remaining.entry(2).or_insert(1) -= 1;
        self.ai_hand.remove(h_idx);

        let anchor = hex_world_anchor(&self.scenario.map, deploy_hex, 0.2);
        let ai_deck_spawn = self.decks[1].clone();
        if card.combat_class == CombatClass::TacticalDeployment || (card.combat_class == CombatClass::Artifact && card.card_key != "Cargo Drop") {
            self.resolve_tactical_deployment(2, &card, deploy_hex, vfx, audio);
        } else if card.combat_class == CombatClass::Artifact && card.card_key == "Cargo Drop" {
            let uid = self.next_uid;
            self.next_uid += 1;
            let mut cache_u = make_board_unit(&card);
            cache_u.carried_munitions = 5;
            self.entities.push(LiveEntity {
                uid,
                team_id: 2,
                hex: deploy_hex,
                vox_slug: card_to_vox_slug(&card).to_string(),
                yaw_radians: 0.0,
                card: card.clone(),
                unit: cache_u,
            });
            self.combat_log.push(format!("[Enemy Turn] AI dropped single-use Remote Cargo Cache at ({}, {}).", deploy_hex.x, deploy_hex.y));
        } else {
            let uid = self.next_uid;
            self.next_uid += 1;
            let slug = card_to_vox_slug(&card).to_string();
            self.entities.push(LiveEntity {
                uid,
                team_id: 2,
                hex: deploy_hex,
                vox_slug: slug,
                yaw_radians: 2.30,
                card: card.clone(),
                unit: make_board_unit_for_deck(&card, &ai_deck_spawn),
            });
            if card.faction == OrganismFaction::Fish {
                let breach_cap = if ai_deck_spawn.is_pure_biotype() && ai_deck_spawn.primary_faction == OrganismFaction::Fish { 2 } else { 1 };
                let mut breached = 0;
                let mut nbrs = deploy_hex.all_neighbors().to_vec();
                nbrs.sort_by_key(|nh| u8::from(self.entity_at(*nh).is_some()));
                for flood_h in nbrs {
                    if breached >= breach_cap { break; }
                    if let Some(t) = self.scenario.map.tiles.get_mut(&flood_h) {
                        if matches!(t.morphology, Morphology::Flat | Morphology::RoughCover | Morphology::RadiationFallout) {
                            t.morphology = Morphology::ShallowFord;
                            t.elevation = 0;
                            self.breached_canal_hexes.insert(flood_h);
                            breached += 1;
                        }
                    }
                }
            }
            self.refresh_confusion_states();
            self.update_explored_fog();

            // Only show AI orbital drop VFX/popup if deployed inside Player 1's vision (or fog is off)!
            let p1_vis = self.compute_visible_hexes(1);
            if !self.fog_enabled || p1_vis.contains(&deploy_hex) {
                vfx.spawn_orbital_deploy(anchor, [0.96, 0.30, 0.22]);
                vfx.spawn_popup(
                    anchor,
                    format!("⬇ AI DEPLOY: {}", card.unit_name),
                    format!("({},{})", deploy_hex.x, deploy_hex.y),
                    [252, 118, 92],
                );
                audio.play_spatial(SfxCue::OrbitalDeploy, anchor);
                self.combat_log.push(format!(
                    "[Enemy Turn] AI deployed {} at ({}, {})!",
                    card.unit_name, deploy_hex.x, deploy_hex.y
                ));
            } else {
                self.combat_log.push("[Enemy Turn] Unidentified orbital drop signature in shrouded sector...".into());
            }
        }
        true
    }

    pub fn end_player_turn_and_run_ai(
        &mut self,
        vfx: &mut VfxManager,
        audio: &mut SpatialAudioEngine,
    ) {
        if self.match_outcome.is_some() {
            return;
        }
        audio.play_ui(SfxCue::UiCardTurn);
        self.mode = CommandMode::Inspect;
        self.start_team_turn(2);

        // AI Tactical Command Cards Evaluation (Faction Ability & Primal Surge)
        let ai_deck = self.decks[1].clone();
        let ai_fac = ai_deck.faction_card();
        let ai_bio = ai_deck.primary_biotype_card();

        // 1. Evaluate AI Faction Command Ability
        let ai_act = self.economy.actions_remaining.get(&2).copied().unwrap_or(0);
        let ai_mun = self.economy.home_base_munitions.get(&2).copied().unwrap_or(0);
        if ai_act >= ai_fac.active_action_cost && ai_mun >= u16::from(ai_fac.active_munition_cost) {
            let should_trigger = match ai_deck.faction {
                OriginAllegiance::Europe => self.entities.iter().any(|e| e.team_id == 2 && !e.card.combat_class.is_building() && e.unit.fortified_stacks == 0),
                OriginAllegiance::Australia => self.entities.iter().any(|e| e.team_id == 2 && e.unit.current_str_hp + 2 <= e.unit.max_str_hp),
                OriginAllegiance::Africa => self.entities.iter().any(|e| e.team_id == 1 && e.unit.current_str_hp > 0),
                OriginAllegiance::NorthAmerica => self.entities.iter().any(|e| e.team_id == 1 && e.unit.target_locked),
                OriginAllegiance::SouthAmerica => self.entities.iter().any(|e| e.team_id == 2 && (e.unit.poisoned_stacks > 0 || e.unit.rooted_turns > 0)) || self.entities.iter().filter(|e| e.team_id == 2 && e.card.faction != OrganismFaction::Human && e.unit.current_str_hp < e.unit.max_str_hp).count() >= 1,
                OriginAllegiance::Asia => self.entities.iter().any(|e| e.team_id == 1 && (e.card.stealth || e.unit.burrowed)),
                _ => false,
            };
            if should_trigger {
                self.execute_faction_command_ability(2, false, vfx, audio);
            }
        }

        // 2. Evaluate AI Bio-Type Primal Surge
        let ai_act = self.economy.actions_remaining.get(&2).copied().unwrap_or(0);
        let ai_mun = self.economy.home_base_munitions.get(&2).copied().unwrap_or(0);
        if ai_act >= ai_bio.primal_surge_cost.0 && ai_mun >= u16::from(ai_bio.primal_surge_cost.1) {
            let should_surge = match ai_deck.primary_faction {
                OrganismFaction::Plant => self.entities.iter().any(|e| e.team_id == 1 && self.entities.iter().any(|p| p.team_id == 2 && p.card.faction == OrganismFaction::Plant && p.hex.unsigned_distance_to(e.hex) <= 1)),
                OrganismFaction::Insect => self.entities.iter().filter(|e| e.team_id == 2 && e.card.faction == OrganismFaction::Insect).count() >= 2,
                OrganismFaction::Reptile => self.entities.iter().any(|e| e.team_id == 2 && e.card.faction == OrganismFaction::Reptile && e.unit.current_str_hp <= 2),
                OrganismFaction::Cnidarian => self.entities.iter().any(|e| e.team_id == 1 && self.entities.iter().any(|c| c.team_id == 2 && c.card.faction == OrganismFaction::Cnidarian && c.hex.unsigned_distance_to(e.hex) <= 1)),
                OrganismFaction::Mammal => self.entities.iter().any(|e| e.team_id == 2 && e.card.faction == OrganismFaction::Mammal),
                OrganismFaction::Bird => self.entities.iter().any(|e| e.team_id == 1),
                OrganismFaction::Arachnid => self.entities.iter().any(|e| e.team_id == 1),
                OrganismFaction::Fish => self.entities.iter().any(|e| e.team_id == 2 && e.card.faction == OrganismFaction::Fish),
                _ => false,
            };
            if should_surge {
                self.execute_biotype_primal_surge(2, false, vfx, audio);
            }
        }

        let mut safety_guard = 0u8;
        while self.economy.actions_remaining.get(&2).copied().unwrap_or(0) > 0
            && self.match_outcome.is_none()
            && safety_guard < 10
        {
            safety_guard += 1;

            let ai_units_count = self
                .entities
                .iter()
                .filter(|e| e.team_id == 2 && e.unit.current_str_hp > 0 && !e.card.combat_class.is_building())
                .count();
            let ai_mun = self.economy.home_base_munitions.get(&2).copied().unwrap_or(0);
            if (ai_units_count < 4 || ai_mun >= 5) && self.try_ai_deploy_from_hand(vfx, audio) {
                continue;
            }

            // AI Medic heal
            let mut heal_pair: Option<(usize, Hex)> = None;
            for (m_idx, m_ent) in self.entities.iter().enumerate() {
                if m_ent.team_id == 2
                    && m_ent.unit.current_str_hp > 0
                    && !m_ent.unit.confused
                    && m_ent.card.heal_repair_range > 0
                {
                    if let Some(wounded) = self.entities.iter().find(|ally| {
                        ally.team_id == 2
                            && ally.unit.current_str_hp > 0
                            && ally.unit.current_str_hp + 2 <= ally.unit.max_str_hp
                            && m_ent.hex.unsigned_distance_to(ally.hex)
                                <= u32::from(m_ent.card.heal_repair_range)
                    }) {
                        heal_pair = Some((m_idx, wounded.hex));
                        break;
                    }
                }
            }
            if let Some((m_idx, w_hex)) = heal_pair {
                let before_act = self.economy.actions_remaining.get(&2).copied().unwrap_or(0);
                self.execute_unit_ability(m_idx, w_hex, vfx, audio);
                if self.economy.actions_remaining.get(&2).copied().unwrap_or(0) < before_act {
                    continue;
                }
            }

            // Fair Fog-of-War & Stealth check for AI targeting!
            let ai_vis = self.compute_visible_hexes(2);
            let ai_det = self.compute_true_sight_hexes(2);

            let mut best_attack: Option<(i32, usize, usize)> = None;
            for (ai_idx, ai_ent) in self.entities.iter().enumerate() {
                if ai_ent.team_id != 2 || ai_ent.unit.confused || ai_ent.unit.current_str_hp == 0 {
                    continue;
                }
                let max_rng = ai_ent.card.ranged_distance.max(ai_ent.card.melee_range);
                if max_rng == 0 || (ai_ent.unit.is_laser && ai_ent.unit.laser_fired_this_turn) {
                    continue;
                }
                let has_ammo = ai_ent.unit.is_laser
                    || ai_ent.unit.carried_munitions > 0
                    || (self.team_supply_hexes(2).contains(&ai_ent.hex)
                        && self.economy.home_base_munitions.get(&2).copied().unwrap_or(0) > 0);
                if !has_ammo {
                    continue;
                }

                for (p_idx, p_ent) in self.entities.iter().enumerate() {
                    if p_ent.team_id == 1
                        && p_ent.unit.current_str_hp > 0
                        && self.is_entity_visible_to(p_idx, 2, &ai_vis, &ai_det)
                        && !(p_ent.card.can_fly && !ai_ent.card.can_fly && ai_ent.card.ranged_distance == 0)
                    {
                        let d = ai_ent.hex.unsigned_distance_to(p_ent.hex);
                        if d >= 1 && d <= u32::from(max_rng) {
                            let req = required_ranged_hit_roll(
                                &p_ent.unit,
                                CombatContext {
                                    hex_distance: d as u8,
                                    is_melee: d == 1 && ai_ent.card.ranged_distance == 0,
                                    attacker_can_fly: ai_ent.card.can_fly,
                                    attacker_stealth_ambush: ai_ent.card.stealth,
                                    attacker_adjacent_commander: false,
                                    attacker_pack_tactics_bonus: false,
                                    defender_in_rough_cover: false,
                                },
                            );
                            let mut atk_score = i32::from(p_ent.card.deck_points)
                                + i32::from(7u8.saturating_sub(req)) * 5;
                            if p_ent.unit.current_str_hp <= ai_ent.unit.current_str_hp {
                                atk_score += 35;
                            }
                            if p_ent.card.combat_class == CombatClass::Installation {
                                atk_score += if p_ent.unit.current_armor == 0 || p_ent.unit.current_str_hp < p_ent.unit.max_str_hp { 145 } else { 95 };
                            }
                            if best_attack.as_ref().map_or(true, |(bs, _, _)| atk_score > *bs) {
                                best_attack = Some((atk_score, ai_idx, p_idx));
                            }
                        }
                    }
                }
            }
            if let Some((_sc, att_i, def_i)) = best_attack {
                self.execute_combat_pair(att_i, def_i, vfx, audio);
                continue;
            }

            if self.try_ai_deploy_from_hand(vfx, audio) {
                continue;
            }

            // Advance toward Scenario Objectives & spotted Player 1 units (or P1 HQ area)
            let mut nav_goals: Vec<Hex> = self
                .scenario
                .objective_hexes
                .iter()
                .map(|&(q, r)| Hex::new(q, r))
                .filter(|&oh| {
                    let w2: u32 = self.entities.iter().filter(|e| e.team_id == 2 && e.unit.current_str_hp > 0).map(|e| match e.hex.unsigned_distance_to(oh) { 0 => 2, 1 => 1, _ => 0 }).sum();
                    let w1: u32 = self.entities.iter().filter(|e| e.team_id == 1 && e.unit.current_str_hp > 0).map(|e| match e.hex.unsigned_distance_to(oh) { 0 => 2, 1 => 1, _ => 0 }).sum();
                    w2 <= w1
                })
                .collect();
            nav_goals.extend(
                self.entities
                    .iter()
                    .enumerate()
                    .filter(|&(idx, e)| {
                        e.team_id == 1
                            && e.unit.current_str_hp > 0
                            && (e.card.combat_class == CombatClass::Installation
                                || self.is_entity_visible_to(idx, 2, &ai_vis, &ai_det))
                    })
                    .map(|(_, e)| e.hex),
            );
            if nav_goals.is_empty() {
                break;
            }

            let mut moved = false;
            let n_ents = self.entities.len();
            for ai_idx in 0..n_ents {
                let reachable = self.valid_move_hexes_for_team(ai_idx, 2);
                if reachable.is_empty() {
                    continue;
                }
                let curr = self.entities[ai_idx].hex;
                let curr_dist = nav_goals
                    .iter()
                    .map(|g| curr.unsigned_distance_to(*g))
                    .min()
                    .unwrap_or(99);
                if let Some(best_dst) = reachable.into_iter().min_by_key(|h| {
                    nav_goals
                        .iter()
                        .map(|g| h.unsigned_distance_to(*g))
                        .min()
                        .unwrap_or(99)
                }) {
                    let best_dist = nav_goals
                        .iter()
                        .map(|g| best_dst.unsigned_distance_to(*g))
                        .min()
                        .unwrap_or(99);
                    if best_dist < curr_dist || (self.entities[ai_idx].card.amphibious_tier == 2 && best_dist == curr_dist && best_dst != curr) {
                        let from_w = hex_world_anchor(&self.scenario.map, curr, 0.2);
                        let to_w = hex_world_anchor(&self.scenario.map, best_dst, 0.2);
                        let uid = self.entities[ai_idx].uid;
                        let u_name = self.entities[ai_idx].card.unit_name.clone();
                        self.entities[ai_idx].yaw_radians =
                            (to_w.x - from_w.x).atan2(to_w.z - from_w.z);
                        self.entities[ai_idx].hex = best_dst;
                        *self.economy.actions_remaining.entry(2).or_insert(1) -= 1;
                        self.refresh_confusion_states();
                        self.update_explored_fog();

                        let p1_vis = self.compute_visible_hexes(1);
                        let p1_det = self.compute_true_sight_hexes(1);
                        if self.is_entity_visible_to(ai_idx, 1, &p1_vis, &p1_det) {
                            vfx.trigger_unit_lunge(uid, to_w, from_w, 0.42);
                            self.combat_log.push(format!(
                                "[Enemy Turn] {} advanced to ({}, {}).",
                                u_name, best_dst.x, best_dst.y
                            ));
                        }
                        moved = true;
                        break;
                    }
                }
            }
            if !moved {
                break;
            }
        }

        self.evaluate_turn_objectives();
        if self.match_outcome.is_none() {
            self.turn_number += 1;
            self.active_team = 1;
            self.start_team_turn(1);
            self.push_timeline_keyframe();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_roster_and_decks() -> (RosterManifest, [DeckList; 2]) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let roster: RosterManifest = ron::from_str(
            &fs::read_to_string(root.join("assets/compiled/master_roster.ron")).unwrap(),
        )
        .unwrap();
        let d0: DeckList = ron::from_str(
            &fs::read_to_string(root.join("assets/compiled/starter_deck_plant_cnidarian.ron")).unwrap(),
        )
        .unwrap();
        let d1: DeckList = ron::from_str(
            &fs::read_to_string(root.join("assets/compiled/starter_deck_mammal_reptile.ron")).unwrap(),
        )
        .unwrap();
        (roster, [d0, d1])
    }

    #[test]
    fn fog_of_war_stealth_cloaks_until_radar_or_adjacent_detection() {
        let (roster, decks) = test_roster_and_decks();
        let mut session = ClientMatchState::new(roster, decks, ScenarioId::EmergenceSkirmish, 0x2072, 0);
        session.entities.clear();

        let lion = session.card_def("Fireclaw Lion").unwrap().clone();
        let stealth_stalker = session.card_def("Chameleon Stalker").unwrap().clone();
        let radar_tower = session.card_def("Radar Tower").unwrap().clone();

        session.entities.push(LiveEntity {
            uid: 1,
            team_id: 1,
            hex: Hex::new(0, 0),
            vox_slug: "mammal_fireclaw_lion".into(),
            yaw_radians: 0.0,
            unit: make_board_unit(&lion),
            card: lion,
        });
        session.entities.push(LiveEntity {
            uid: 2,
            team_id: 2,
            hex: Hex::new(2, 0), // Distance 2: inside normal sight, outside adjacent contact!
            vox_slug: "reptile_flamer_croc".into(),
            yaw_radians: 0.0,
            unit: make_board_unit(&stealth_stalker),
            card: stealth_stalker,
        });
        // Flatten intermediate & occupied tiles so MountainPeak never interferes
        for q in 0..=3 {
            if let Some(t) = session.scenario.map.tiles.get_mut(&Hex::new(q, 0)) {
                t.morphology = Morphology::Flat;
                t.elevation = 0;
            }
        }

        let vis = session.compute_visible_hexes(1);
        let det = session.compute_true_sight_hexes(1);
        assert!(vis.contains(&Hex::new(2, 0)));
        assert!(
            !session.is_entity_visible_to(1, 1, &vis, &det),
            "Stealthed Chameleon Stalker at dist 2 must remain cloaked without Radar!"
        );

        // Deploy friendly Radar Tower at (-1, 0) -> True-Sight reveals the stealthed enemy!
        session.entities.push(LiveEntity {
            uid: 3,
            team_id: 1,
            hex: Hex::new(-1, 0),
            vox_slug: "radar_laser_turret".into(),
            yaw_radians: 0.0,
            unit: make_board_unit(&radar_tower),
            card: radar_tower,
        });
        let vis2 = session.compute_visible_hexes(1);
        let det2 = session.compute_true_sight_hexes(1);
        assert!(
            session.is_entity_visible_to(1, 1, &vis2, &det2),
            "Radar Tower True-Sight must reveal stealthed enemy inside Radar Range!"
        );
    }

    #[test]
    fn mountain_peak_blocks_ground_los_but_flying_drone_sees_over() {
        let (roster, decks) = test_roster_and_decks();
        let mut session = ClientMatchState::new(roster, decks, ScenarioId::EmergenceSkirmish, 0x2072, 0);
        session.entities.clear();

        for q in 0..=2 {
            if let Some(t) = session.scenario.map.tiles.get_mut(&Hex::new(q, 0)) {
                t.morphology = if q == 1 { Morphology::MountainPeak } else { Morphology::Flat };
                t.elevation = if q == 1 { 3 } else { 0 };
            }
        }

        let lion = session.card_def("Fireclaw Lion").unwrap().clone();
        session.entities.push(LiveEntity {
            uid: 1,
            team_id: 1,
            hex: Hex::new(0, 0),
            vox_slug: "mammal_fireclaw_lion".into(),
            yaw_radians: 0.0,
            unit: make_board_unit(&lion),
            card: lion,
        });
        let ground_vis = session.compute_visible_hexes(1);
        assert!(
            !ground_vis.contains(&Hex::new(2, 0)),
            "MountainPeak at (1,0) must block ground sight to (2,0)"
        );

        let drone = session.card_def("Drone").unwrap().clone();
        session.entities.push(LiveEntity {
            uid: 2,
            team_id: 1,
            hex: Hex::new(0, 0),
            vox_slug: "drone".into(),
            yaw_radians: 0.0,
            unit: make_board_unit(&drone),
            card: drone,
        });
        let air_vis = session.compute_visible_hexes(1);
        assert!(
            air_vis.contains(&Hex::new(2, 0)),
            "Flying Drone must see cleanly over MountainPeak!"
        );
    }

    #[test]
    fn quicksave_and_quickload_roundtrip_preserves_exact_rng_and_board() {
        let (roster, decks) = test_roster_and_decks();
        let mut s1 = ClientMatchState::new(roster.clone(), decks.clone(), ScenarioId::BioRiverConvoy, 0xABCD_2072, 0);
        let mut vfx = VfxManager::default();
        let mut audio = SpatialAudioEngine::new();
        audio.muted = true;
        s1.end_player_turn_and_run_ai(&mut vfx, &mut audio);

        let tmp_save = std::env::temp_dir().join("wf_test_quicksave.ron");
        s1.save_quicksave(&tmp_save, 0xABCD_2072).expect("Quicksave failed");

        let mut s2 = ClientMatchState::new(roster, decks, ScenarioId::EmergenceSkirmish, 1, 1);
        let restored_seed = s2.load_quicksave(&tmp_save).expect("Quickload failed");
        let _ = fs::remove_file(&tmp_save);

        assert_eq!(restored_seed, 0xABCD_2072);
        assert_eq!(s1.turn_number, s2.turn_number);
        assert_eq!(s1.entities.len(), s2.entities.len());
        assert_eq!(s1.hand, s2.hand);
        assert_eq!(s1.ai_hand, s2.ai_hand);
        for _ in 0..8 {
            assert_eq!(s1.roll_d6(), s2.roll_d6(), "Restored SplitMix64 RNG diverged!");
        }
    }

    #[test]
    fn deterministic_lockstep_host_and_client_stay_identical() {
        let (roster, decks) = test_roster_and_decks();
        let seed = 0x5EED_7777;
        let mut host = ClientMatchState::new(roster.clone(), decks.clone(), ScenarioId::EmergenceSkirmish, seed, 0);
        let mut client = ClientMatchState::new(roster, decks, ScenarioId::EmergenceSkirmish, seed, 0);
        host.online_local_team = Some(1);
        client.online_local_team = Some(2);

        let mut vfx_h = VfxManager::default();
        let mut vfx_c = VfxManager::default();
        let mut aud_h = SpatialAudioEngine::new();
        let mut aud_c = SpatialAudioEngine::new();
        aud_h.muted = true;
        aud_c.muted = true;

        // Host ends Turn 1 -> transmits LockstepCommand::EndTurn { team_id: 1 } to Client
        host.end_active_turn(&mut vfx_h, &mut aud_h);
        let cmds_from_host: Vec<_> = host.outbound_net_cmds.drain(..).collect();
        assert_eq!(cmds_from_host.len(), 1);
        for stamped in cmds_from_host {
            client
                .apply_remote_lockstep_command(stamped.cmd, stamped.post_state_hash, &mut vfx_c, &mut aud_c)
                .expect("Host EndTurn lockstep hash must match client");
        }
        assert_eq!(host.active_team, 2);
        assert_eq!(client.active_team, 2);

        // Client (Team 2) executes FactionCommand, PrimalSurge, and EndTurn -> transmits back to Host
        assert!(client.execute_faction_command_ability(2, true, &mut vfx_c, &mut aud_c));
        assert!(client.execute_biotype_primal_surge(2, true, &mut vfx_c, &mut aud_c));
        client.end_active_turn(&mut vfx_c, &mut aud_c);
        let cmds_from_client: Vec<_> = client.outbound_net_cmds.drain(..).collect();
        assert_eq!(cmds_from_client.len(), 3);
        assert!(matches!(cmds_from_client[0].cmd, LockstepCommand::FactionCommand { team_id: 2 }));
        assert!(matches!(cmds_from_client[1].cmd, LockstepCommand::PrimalSurge { team_id: 2 }));
        assert!(matches!(cmds_from_client[2].cmd, LockstepCommand::EndTurn { team_id: 2 }));

        for stamped in cmds_from_client {
            host.apply_remote_lockstep_command(stamped.cmd, stamped.post_state_hash, &mut vfx_h, &mut aud_h)
                .expect("Client command lockstep hash must match host");
        }
        assert_eq!(host.compute_state_hash(), client.compute_state_hash());
        assert_eq!(host.turn_number, 2);
        assert_eq!(client.turn_number, 2);
        assert_eq!(host.hand, client.hand);
        assert_eq!(host.ai_hand, client.ai_hand);
        for _ in 0..6 {
            assert_eq!(host.roll_d6(), client.roll_d6());
        }

        // Tampered post_state_hash must return an explicit desync Err
        let desync = host.apply_remote_lockstep_command(
            LockstepCommand::FactionCommand { team_id: 1 },
            0xDEAD_0000_BEEF_9999,
            &mut vfx_h,
            &mut aud_h,
        );
        assert!(desync.is_err(), "Mismatched post_state_hash must be flagged");
    }

    #[test]
    fn hover_combat_odds_preview_reflects_live_board_modifiers() {
        let (roster, mut decks) = test_roster_and_decks();
        decks[0].primary_faction = OrganismFaction::Mammal;
        decks[0].secondary_faction = OrganismFaction::Mammal;
        decks[0].is_pure_biotype = true;
        decks[1].faction = OriginAllegiance::Europe;

        let mut session = ClientMatchState::new(roster, decks, ScenarioId::EmergenceSkirmish, 0x2072, 0);
        session.entities.clear();

        let lion = session.card_def("Fireclaw Lion").unwrap().clone();
        let commander = session.card_def("Commander (Australia)").unwrap().clone();
        let ally_mammal = session.card_def("Fireclaw Lion").unwrap().clone();
        let defender = session.card_def("Battlephant").unwrap().clone();

        for h in [Hex::new(0, 0), Hex::new(-1, 0), Hex::new(0, -1), Hex::new(3, 0)] {
            if let Some(t) = session.scenario.map.tiles.get_mut(&h) {
                t.morphology = if h == Hex::new(3, 0) {
                    Morphology::RoughCover
                } else {
                    Morphology::Flat
                };
                t.elevation = 0;
            }
        }

        session.entities.push(LiveEntity {
            uid: 10,
            team_id: 1,
            hex: Hex::new(0, 0),
            vox_slug: "mammal_fireclaw_lion".into(),
            yaw_radians: 0.0,
            unit: make_board_unit(&lion),
            card: lion,
        });
        session.entities.push(LiveEntity {
            uid: 11,
            team_id: 1,
            hex: Hex::new(-1, 0),
            vox_slug: "human_commander".into(),
            yaw_radians: 0.0,
            unit: make_board_unit(&commander),
            card: commander,
        });
        session.entities.push(LiveEntity {
            uid: 12,
            team_id: 1,
            hex: Hex::new(0, -1),
            vox_slug: "mammal_fireclaw_lion".into(),
            yaw_radians: 0.0,
            unit: make_board_unit(&ally_mammal),
            card: ally_mammal,
        });
        let mut def_u = make_board_unit(&defender);
        def_u.fortified_stacks = 0;
        session.entities.push(LiveEntity {
            uid: 20,
            team_id: 2,
            hex: Hex::new(3, 0),
            vox_slug: "mammal_battlephant".into(),
            yaw_radians: 0.0,
            unit: def_u,
            card: defender,
        });

        let preview = session.preview_combat_odds(0, 3);
        assert_eq!(preview.dist, 3);
        assert!(preview.ctx.attacker_adjacent_commander);
        assert!(preview.ctx.attacker_pack_tactics_bonus);
        assert!(preview.ctx.defender_in_rough_cover);
        assert!(preview.pure_mammal_apex);
        assert!(preview.aegis_bastion);
        assert_eq!(preview.effective_armor, session.entities[3].unit.current_armor + 1);
        assert_eq!(preview.required_d6, Some(2));
        assert!(preview.hit_summary.contains("2+"));

        // Verify Shield block and Burrowed ranged immunity states
        session.entities[3].unit.fortified_stacks = 1;
        let blocked = session.preview_combat_odds(0, 3);
        assert_eq!(blocked.required_d6, None);
        assert!(blocked.hit_summary.contains("BLOCKED"));

        session.entities[3].unit.fortified_stacks = 0;
        session.entities[3].unit.burrowed = true;
        let burrowed = session.preview_combat_odds(0, 3);
        assert_eq!(burrowed.required_d6, None);
        assert!(burrowed.hit_summary.contains("IMMUNE"));
    }

    #[test]
    fn radar_equipped_ground_unit_penetrates_mountain_los_and_stealth() {
        let (roster, decks) = test_roster_and_decks();
        let mut session = ClientMatchState::new(roster, decks, ScenarioId::EmergenceSkirmish, 0x2072, 0);
        session.entities.clear();

        for q in 0..=2 {
            if let Some(t) = session.scenario.map.tiles.get_mut(&Hex::new(q, 0)) {
                t.morphology = if q == 1 { Morphology::MountainPeak } else { Morphology::Flat };
                t.elevation = if q == 1 { 3 } else { 0 };
            }
        }

        let koala = session.card_def("Combat Koala").unwrap().clone();
        assert!(!koala.can_fly && koala.radar_range >= 2);
        let stealth_stalker = session.card_def("Chameleon Stalker").unwrap().clone();

        session.entities.push(LiveEntity {
            uid: 1,
            team_id: 1,
            hex: Hex::new(0, 0),
            vox_slug: "combat_koala_squad".into(),
            yaw_radians: 0.0,
            unit: make_board_unit(&koala),
            card: koala,
        });
        session.entities.push(LiveEntity {
            uid: 2,
            team_id: 2,
            hex: Hex::new(2, 0),
            vox_slug: "reptile_chameleon_stalker".into(),
            yaw_radians: 0.0,
            unit: make_board_unit(&stealth_stalker),
            card: stealth_stalker,
        });

        let vis = session.compute_visible_hexes(1);
        let det = session.compute_true_sight_hexes(1);
        assert!(
            vis.contains(&Hex::new(2, 0)),
            "Ground unit with radar_range >= 2 must detect hex (2,0) beyond MountainPeak"
        );
        assert!(
            session.is_entity_visible_to(1, 1, &vis, &det),
            "Ground unit with radar_range >= 2 must reveal stealthed enemy at dist 2"
        );
    }

    #[test]
    fn load_scenario_resets_next_uid_and_economy_for_lockstep_parity() {
        let (roster, decks) = test_roster_and_decks();
        let mut host = ClientMatchState::new(
            roster.clone(),
            decks.clone(),
            ScenarioId::EmergenceSkirmish,
            0x2072_0814,
            0,
        );
        let mut client = ClientMatchState::new(
            roster,
            decks,
            ScenarioId::EmergenceSkirmish,
            0x2072_0814,
            0,
        );

        let mut vfx = VfxManager::default();
        let mut aud = SpatialAudioEngine::new();

        // Simulate pre-match activity on host: scenario switch + local FactionCommand/PrimalSurge
        host.load_scenario(ScenarioId::BunkerBreakout, 0x9999);
        assert!(host.execute_faction_command_ability(1, false, &mut vfx, &mut aud));
        assert!(host.execute_biotype_primal_surge(1, false, &mut vfx, &mut aud));
        assert!(host.next_uid > 3);

        // Both peers now load the same match scenario (as in MatchInit / ClientJoinDeck)
        host.online_local_team = Some(1);
        client.online_local_team = Some(2);
        host.load_scenario(ScenarioId::BioRiverConvoy, 0x2072_0814);
        client.load_scenario(ScenarioId::BioRiverConvoy, 0x2072_0814);

        assert_eq!(host.next_uid, client.next_uid);
        assert_eq!(
            host.compute_state_hash(),
            client.compute_state_hash(),
            "Host and Client hashes must match after load_scenario regardless of prior activity"
        );

        host.outbound_net_cmds.clear();
        assert!(host.execute_faction_command_ability(1, true, &mut vfx, &mut aud));
        let stamped = host.outbound_net_cmds.pop().expect("Command must be stamped");
        client
            .apply_remote_lockstep_command(stamped.cmd, stamped.post_state_hash, &mut vfx, &mut aud)
            .expect("Lockstep command after scenario reload must pass hash check");
    }

    #[test]
    fn preview_combat_odds_matches_execute_combat_pair_on_burrowed_fortified_and_apex_natural_one() {
        let (roster, mut decks) = test_roster_and_decks();
        decks[0].primary_faction = OrganismFaction::Mammal;
        decks[0].secondary_faction = OrganismFaction::Mammal;
        decks[0].is_pure_biotype = true;

        let mut session = ClientMatchState::new(roster, decks, ScenarioId::EmergenceSkirmish, 0x2072, 0);
        session.entities.clear();
        for h in [Hex::new(0, 0), Hex::new(-1, 0), Hex::new(0, -1), Hex::new(2, 0)] {
            if let Some(t) = session.scenario.map.tiles.get_mut(&h) {
                t.morphology = Morphology::Flat;
                t.elevation = 0;
            }
        }

        let lion = session.card_def("Fireclaw Lion").unwrap().clone();
        let commander = session.card_def("Commander (Australia)").unwrap().clone();
        let defender = session.card_def("Battlephant").unwrap().clone();

        session.entities.push(LiveEntity {
            uid: 10,
            team_id: 1,
            hex: Hex::new(0, 0),
            vox_slug: "mammal_fireclaw_lion".into(),
            yaw_radians: 0.0,
            unit: make_board_unit(&lion),
            card: lion.clone(),
        });
        session.entities.push(LiveEntity {
            uid: 11,
            team_id: 1,
            hex: Hex::new(-1, 0),
            vox_slug: "human_commander".into(),
            yaw_radians: 0.0,
            unit: make_board_unit(&commander),
            card: commander,
        });
        session.entities.push(LiveEntity {
            uid: 12,
            team_id: 1,
            hex: Hex::new(0, -1),
            vox_slug: "mammal_fireclaw_lion".into(),
            yaw_radians: 0.0,
            unit: make_board_unit(&lion),
            card: lion,
        });

        let mut def_u = make_board_unit(&defender);
        def_u.fortified_stacks = 1;
        def_u.burrowed = true;
        session.entities.push(LiveEntity {
            uid: 20,
            team_id: 2,
            hex: Hex::new(2, 0),
            vox_slug: "mammal_battlephant".into(),
            yaw_radians: 0.0,
            unit: def_u,
            card: defender,
        });

        let mut vfx = VfxManager::default();
        let mut aud = SpatialAudioEngine::new();

        // 1) Burrowed + Fortified at dist=2 (!is_melee): preview must say IMMUNE (Burrowed) and shield stays intact
        let preview_bf = session.preview_combat_odds(0, 3);
        assert_eq!(preview_bf.required_d6, None);
        assert!(
            preview_bf.hit_summary.contains("IMMUNE (Burrowed)"),
            "Expected IMMUNE (Burrowed) before shield check, got: {}",
            preview_bf.hit_summary
        );
        let hp_before = session.entities[3].unit.current_str_hp;
        session.economy.actions_remaining.insert(1, 5);
        session.execute_combat_pair(0, 3, &mut vfx, &mut aud);
        assert_eq!(session.entities[3].unit.current_str_hp, hp_before);
        assert_eq!(
            session.entities[3].unit.fortified_stacks, 1,
            "Burrowed ranged immunity must resolve before fortified_stacks is consumed"
        );

        // 2) Pure Mammal Apex natural-1 always misses even when base_req == 2
        session.entities[3].unit.burrowed = false;
        session.entities[3].unit.fortified_stacks = 0;
        let preview_apex = session.preview_combat_odds(0, 3);
        assert_eq!(preview_apex.required_d6, Some(2));
        assert!(preview_apex.pure_mammal_apex);

        // Find deterministic seed whose first roll_d6() yields natural 1
        let nat1_seed = (0u64..1000)
            .find(|&s| ((SplitMix64::new(s).next_u64() % 6) + 1) == 1)
            .expect("Must find seed yielding natural 1 on first D6 roll");
        session.dice_rng = SplitMix64::new(nat1_seed);
        let armor_before = session.entities[3].unit.current_armor;
        let hp_before_apex = session.entities[3].unit.current_str_hp;
        session.execute_combat_pair(0, 3, &mut vfx, &mut aud);
        assert_eq!(
            session.entities[3].unit.current_armor, armor_before,
            "Natural 1 must miss under Pure Mammal Apex"
        );
        assert_eq!(
            session.entities[3].unit.current_str_hp, hp_before_apex,
            "Natural 1 must miss under Pure Mammal Apex"
        );
    }
}
