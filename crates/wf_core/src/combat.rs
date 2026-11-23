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

//! Combat resolution (Distance + Evade + Terrain Cover Hit Roll, Strength-Invariant Damage & Armor,
//! `Strength == Health`, Faction Pack Tactics, and `Hospital` / `Factory` Graveyard recovery).

use serde::{Deserialize, Serialize};
use crate::logistics::BoardUnit;
use crate::schema::GraveyardFacility;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatContext {
    pub hex_distance: u8,
    pub is_melee: bool,
    pub attacker_can_fly: bool,
    pub attacker_stealth_ambush: bool,
    pub attacker_adjacent_commander: bool,
    pub attacker_pack_tactics_bonus: bool,
    pub defender_in_rough_cover: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatOutcome {
    pub hit_threshold: u8,
    pub hit_succeeded: bool,
    pub armor_absorbed: u8,
    pub strength_damage_dealt: u8,
    pub defender_destroyed: bool,
    pub triggers_melee_retaliation: bool,
}

pub fn required_ranged_hit_roll(defender: &BoardUnit, ctx: CombatContext) -> u8 {
    let mut effective_evade = i16::from(defender.evade);
    if defender.rooted_turns > 0 {
        effective_evade = (effective_evade - 1).max(0);
    }
    if defender.blinded_this_turn {
        effective_evade = (effective_evade - 1).max(0);
    }
    if ctx.defender_in_rough_cover {
        effective_evade += 1;
    }

    let mut target: i16 = 1 + i16::from(ctx.hex_distance.max(1)) + effective_evade;
    if defender.target_locked {
        target -= 1;
    }
    if ctx.attacker_adjacent_commander {
        target -= 1;
    }
    if ctx.attacker_pack_tactics_bonus {
        target -= 1;
    }
    target.clamp(2, 6) as u8
}

pub fn resolve_attack(
    attacker: &mut BoardUnit,
    defender: &mut BoardUnit,
    ctx: CombatContext,
    hit_d6: u8,
    dmg_d6: u8,
) -> CombatOutcome {
    // Burrowed targets are immune to ranged fire from distance >= 2
    if defender.burrowed && !ctx.is_melee && ctx.hex_distance >= 2 {
        return CombatOutcome {
            hit_threshold: 7,
            hit_succeeded: false,
            armor_absorbed: 0,
            strength_damage_dealt: 0,
            defender_destroyed: false,
            triggers_melee_retaliation: false,
        };
    }

    if defender.fortified_stacks > 0 {
        defender.fortified_stacks -= 1;
        return CombatOutcome {
            hit_threshold: 0,
            hit_succeeded: false,
            armor_absorbed: 0,
            strength_damage_dealt: 0,
            defender_destroyed: false,
            triggers_melee_retaliation: false,
        };
    }

    let hit_d6 = hit_d6.clamp(1, 6);
    let hit_threshold = if ctx.is_melee {
        1
    } else {
        required_ranged_hit_roll(defender, ctx)
    };
    let hit_succeeded = if ctx.is_melee {
        true
    } else {
        hit_d6 == 6 || (hit_d6 > 1 && hit_d6 >= hit_threshold)
    };

    let triggers_retaliation = ctx.is_melee
        && !ctx.attacker_can_fly
        && !ctx.attacker_stealth_ambush
        && !defender.stunned_this_turn;

    if !hit_succeeded {
        return CombatOutcome {
            hit_threshold,
            hit_succeeded: false,
            armor_absorbed: 0,
            strength_damage_dealt: 0,
            defender_destroyed: false,
            triggers_melee_retaliation: triggers_retaliation,
        };
    }

    let capped_roll = i16::from(dmg_d6.min(attacker.current_str_hp));
    let mut delta_str = (i16::from(attacker.current_str_hp) - i16::from(defender.current_str_hp))
        .clamp(-2, 3);
    // Aerial Dive-Strike: Airborne flyers striking from altitude ignore negative ΔSTR frame penalties!
    if ctx.attacker_can_fly && delta_str < 0 {
        delta_str = 0;
    }
    let raw_strike = (capped_roll + delta_str).max(1) as u8;

    let armor_absorbed = raw_strike.min(defender.current_armor);
    defender.current_armor -= armor_absorbed;
    let net_damage = raw_strike - armor_absorbed;
    let actual_str_loss = net_damage.min(defender.current_str_hp);
    defender.current_str_hp -= actual_str_loss;

    CombatOutcome {
        hit_threshold,
        hit_succeeded: true,
        armor_absorbed,
        strength_damage_dealt: actual_str_loss,
        defender_destroyed: defender.current_str_hp == 0,
        triggers_melee_retaliation: triggers_retaliation && defender.current_str_hp > 0,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraveyardTicket {
    pub card_key: String,
    pub facility: GraveyardFacility,
    pub recovered_str: u8,
    pub target_str: u8,
}

impl GraveyardTicket {
    pub fn tick_recovery(&mut self, active_hospital: bool, active_factory: bool) -> bool {
        let can_progress = match self.facility {
            GraveyardFacility::HospitalBio => active_hospital,
            GraveyardFacility::FactoryMech => active_factory,
            _ => false,
        };
        if can_progress {
            self.recovered_str = (self.recovered_str + 2).min(self.target_str);
        }
        self.recovered_str >= self.target_str
    }
}
