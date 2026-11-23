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

//! Dual-graph Breadth-First Search (BFS) solvers for:
//! 1. Contiguous Home Base Supply Chain & `Supply Range` munitions projection
//! 2. Psychic Control Node network (`Commanders`, `Drones`, static `Control Nodes`, Cnidarian Heavy relays)

use std::collections::{HashMap, HashSet, VecDeque};
use bevy_ecs::prelude::*;
use hexx::Hex;
use serde::{Deserialize, Serialize};
use crate::schema::{CombatClass, GraveyardFacility, OrganismFaction};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Component, Serialize, Deserialize)]
pub struct PlayerOwner(pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Component, Serialize, Deserialize)]
pub struct HexPos(pub Hex);

#[derive(Debug, Clone, Component, Serialize, Deserialize)]
pub struct BoardUnit {
    pub card_key: String,
    pub faction: OrganismFaction,
    pub combat_class: CombatClass,
    pub graveyard_queue: GraveyardFacility,
    pub current_str_hp: u8,
    pub max_str_hp: u8,
    pub current_armor: u8,
    pub base_armor: u8,
    pub evade: u8,
    pub encumbrance_cap: u8,
    pub carried_munitions: u8,
    pub node_leash_range: u8,
    pub emits_node: bool,
    pub node_emit_radius: u8,
    pub supply_range: u8,
    pub is_laser: bool,
    pub laser_fired_this_turn: bool,
    pub confused: bool,
    pub poisoned_stacks: u8,
    pub fortified_stacks: u8,
    pub rooted_turns: u8,
    pub stunned_this_turn: bool,
    pub burrowed: bool,
    pub target_locked: bool,
    pub revealed_until_turn_start: bool,
    #[serde(default)]
    pub temp_move_bonus: u8,
    #[serde(default)]
    pub temp_stealth: bool,
    #[serde(default)]
    pub blinded_this_turn: bool,
}

#[derive(Debug, Clone, Default, Resource, Serialize, Deserialize)]
pub struct PlayerEconomy {
    pub home_base_munitions: HashMap<u8, u16>,
    pub actions_remaining: HashMap<u8, u8>,
    #[serde(default)]
    pub faction_ability_used: HashMap<u8, bool>,
    #[serde(default)]
    pub primal_surge_used: HashMap<u8, bool>,
    #[serde(default)]
    pub revives_used: HashMap<String, u8>,
}

/// Computes start-of-turn munitions (+5 to Home Base), Poison tick, Armor refresh,
/// and Action allotment (`2 + static_control_nodes`, capped at +3).
pub fn begin_player_turn(
    player_id: u8,
    economy: &mut PlayerEconomy,
    units: &mut [(PlayerOwner, &mut BoardUnit)],
) {
    *economy.home_base_munitions.entry(player_id).or_insert(0) += 5;

    let mut static_control_nodes: u8 = 0;
    for (owner, unit) in units.iter_mut() {
        if owner.0 == player_id && unit.current_str_hp > 0 {
            if !unit.combat_class.is_building() {
                unit.current_armor = unit.base_armor;
            }
            unit.laser_fired_this_turn = false;
            unit.stunned_this_turn = false;
            unit.revealed_until_turn_start = false;
            unit.temp_move_bonus = 0;
            unit.temp_stealth = false;
            unit.blinded_this_turn = false;
            if unit.rooted_turns > 0 {
                unit.rooted_turns -= 1;
            }
            if unit.poisoned_stacks > 0 {
                let p_dmg = if unit.poisoned_stacks >= 2 { 2 } else { 1 };
                unit.current_str_hp = unit.current_str_hp.saturating_sub(p_dmg);
                unit.poisoned_stacks -= 1;
            }
            if unit.current_str_hp > 0 && unit.combat_class == CombatClass::ControlNode {
                static_control_nodes += 1;
            }
        }
    }
    let actions = 2 + static_control_nodes.min(3);
    economy.actions_remaining.insert(player_id, actions);
    economy.faction_ability_used.insert(player_id, false);
    economy.primal_surge_used.insert(player_id, false);
}

pub fn compute_home_supply_coverage(
    player_id: u8,
    board_entities: &[(PlayerOwner, HexPos, &BoardUnit)],
) -> HashSet<Hex> {
    let mut building_hexes: HashMap<Hex, &BoardUnit> = HashMap::new();
    let mut base_roots: Vec<Hex> = Vec::new();

    for (owner, pos, unit) in board_entities {
        if owner.0 == player_id && unit.current_str_hp > 0 && unit.combat_class.is_building() {
            building_hexes.insert(pos.0, unit);
            if unit.combat_class == CombatClass::Installation {
                base_roots.push(pos.0);
            }
        }
    }

    let mut linked_buildings: HashSet<Hex> = HashSet::new();
    let mut queue: VecDeque<Hex> = VecDeque::new();
    for root in base_roots {
        linked_buildings.insert(root);
        queue.push_back(root);
    }

    while let Some(curr) = queue.pop_front() {
        for neighbor in curr.all_neighbors() {
            if building_hexes.contains_key(&neighbor) && linked_buildings.insert(neighbor) {
                queue.push_back(neighbor);
            }
        }
    }

    let mut supplied_hexes: HashSet<Hex> = HashSet::new();
    for b_hex in &linked_buildings {
        let b_unit = building_hexes[b_hex];
        let radius = u32::from(b_unit.supply_range.max(1));
        for h in b_hex.range(radius) {
            supplied_hexes.insert(h);
        }
    }
    supplied_hexes
}

pub fn is_within_control_node(
    player_id: u8,
    unit_leash: u8,
    target_hex: Hex,
    board_entities: &[(PlayerOwner, HexPos, &BoardUnit)],
) -> bool {
    if unit_leash == 0 {
        return true;
    }
    for (owner, pos, emitter) in board_entities {
        if owner.0 == player_id && emitter.current_str_hp > 0 && emitter.emits_node {
            let dist = pos.0.unsigned_distance_to(target_hex);
            let allowed = u32::from(unit_leash.max(emitter.node_emit_radius));
            if dist <= allowed {
                return true;
            }
        }
    }
    false
}
