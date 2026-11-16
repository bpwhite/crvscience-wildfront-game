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

//! Balanced active & passive abilities, cooldown/munition rules, and
//! biome-aware Organism Faction synergies for *Wildfront Battlegrounds*.

use serde::{Deserialize, Serialize};
use crate::hex_map::{Biome, Morphology};
use crate::logistics::BoardUnit;
use crate::schema::OrganismFaction;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TriggerMode {
    Active,
    Passive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AbilityId {
    DeployUnitCard,
    AxialHexMove,
    RangedFire,
    LaserPulseFire,
    MeleeAssault,
    FortifyEntrench,
    RestAndWake,
    SupplyTransferOrDrop,
    FieldHealDetoxify,
    RadarTargetLock,
    VenomSpitCnidocytes,
    WebAndRootSnare,
    StealthInfiltrate,
    SubterraneanBurrow,
    SaboteurRaid,
    BuildArmoredDam,
    VoltaicArcStun,
    GeneticSpliceSurge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbilitySpec {
    pub id: AbilityId,
    pub display_name: String,
    pub trigger: TriggerMode,
    pub action_cost: u8,
    pub munition_cost: u8,
    pub max_range: u8,
    pub cooldown_turns: u8,
    pub effect_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactionPassiveSpec {
    pub faction: OrganismFaction,
    pub doctrine_name: String,
    pub bonus_effect: String,
    pub tactical_tradeoff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbilityCatalog {
    pub abilities: Vec<AbilitySpec>,
    pub faction_passives: Vec<FactionPassiveSpec>,
}

impl AbilityCatalog {
    pub fn canonical() -> Self {
        let abilities = vec![
            AbilitySpec {
                id: AbilityId::DeployUnitCard,
                display_name: "Deploy Unit Card".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 0,
                max_range: 2,
                cooldown_turns: 0,
                effect_summary: "Spawns voxel squad/rig within 2 hexes of backline; spends card ENC from Home Supply (-1 for adjacent Insect Swarm).".into(),
            },
            AbilitySpec {
                id: AbilityId::AxialHexMove,
                display_name: "Axial Hex Move".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 0,
                max_range: 4,
                cooldown_turns: 0,
                effect_summary: "Traverses axial hexes up to MOVE respecting cliffs, rivers, ravines & Node leash; clears Target Lock.".into(),
            },
            AbilitySpec {
                id: AbilityId::RangedFire,
                display_name: "Ranged 1+ Fire".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 1,
                max_range: 4,
                cooldown_turns: 0,
                effect_summary: "D6 Hit = 2+ at 1 hex (+1/hex + Evade; 6 auto-hits); Strength-Invariant damage minus Armor; zero retaliation.".into(),
            },
            AbilitySpec {
                id: AbilityId::LaserPulseFire,
                display_name: "Laser Pulse Fire".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 0,
                max_range: 3,
                cooldown_turns: 1,
                effect_summary: "0 Munitions energy beam, max 1 shot per turn cooldown (Laser Turret, Timberwolves, Optic-Laser Shark).".into(),
            },
            AbilitySpec {
                id: AbilityId::MeleeAssault,
                display_name: "Melee Assault / Melee 0".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 1,
                max_range: 2,
                cooldown_turns: 0,
                effect_summary: "Auto-hits ground targets; triggers defender Melee 0 retaliation unless attacker has Flight or Stealth Ambush.".into(),
            },
            AbilitySpec {
                id: AbilityId::FortifyEntrench,
                display_name: "Fortify & Entrench".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 1,
                max_range: 0,
                cooldown_turns: 0,
                effect_summary: "Adds +1 Fortified stack (max 3; blocks 1 attack/stack). Heavy units project Entrench blocking enemy pass-through.".into(),
            },
            AbilitySpec {
                id: AbilityId::RestAndWake,
                display_name: "Rest & Wake".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 0,
                max_range: 0,
                cooldown_turns: 1,
                effect_summary: "Unit rests 1 turn to heal +1 STR/HP or cure 1 Poisoned stack at next turn start.".into(),
            },
            AbilitySpec {
                id: AbilityId::SupplyTransferOrDrop,
                display_name: "Supply Transfer / Cargo Drop".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 0,
                max_range: 4,
                cooldown_turns: 0,
                effect_summary: "Transfers Munitions across Supply Range or delivers +5 Munitions via Cargo Drop artifact.".into(),
            },
            AbilitySpec {
                id: AbilityId::FieldHealDetoxify,
                display_name: "Field Heal & Detoxify".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 1,
                max_range: 3,
                cooldown_turns: 0,
                effect_summary: "Medics restore +1 STR/HP (up to max) and purge all Poisoned & Rooted stacks on friendly unit in range.".into(),
            },
            AbilitySpec {
                id: AbilityId::RadarTargetLock,
                display_name: "Radar Target Lock".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 1,
                max_range: 4,
                cooldown_turns: 0,
                effect_summary: "Applies Target Locked (+1 to friendly ranged hit rolls against target until it moves >=1 hex).".into(),
            },
            AbilitySpec {
                id: AbilityId::VenomSpitCnidocytes,
                display_name: "Venom Spit / Cnidocytes".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 1,
                max_range: 2,
                cooldown_turns: 0,
                effect_summary: "Applies 1 Poisoned stack (2 stacks for Cnidarians, max 3); deals 1 armor-piercing STR damage at turn start.".into(),
            },
            AbilitySpec {
                id: AbilityId::WebAndRootSnare,
                display_name: "Web & Root Snare".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 1,
                max_range: 2,
                cooldown_turns: 1,
                effect_summary: "Immobilizes target (rooted_turns = 2, MOVE = 0, -1 Evade); Arachnids also apply 1 Poison vs 0-Armor targets.".into(),
            },
            AbilitySpec {
                id: AbilityId::StealthInfiltrate,
                display_name: "Stealth Infiltrate".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 1,
                max_range: 3,
                cooldown_turns: 0,
                effect_summary: "Moves through enemy-occupied hexes to an empty destination; passive immunity to Target Lock.".into(),
            },
            AbilitySpec {
                id: AbilityId::SubterraneanBurrow,
                display_name: "Subterranean Burrow".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 1,
                max_range: 1,
                cooldown_turns: 0,
                effect_summary: "Toggles Burrowed: immune to ranged fire from dist >= 2, moves 1 hex/turn ignoring surface fallout.".into(),
            },
            AbilitySpec {
                id: AbilityId::SaboteurRaid,
                display_name: "Saboteur Raid".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 1,
                max_range: 1,
                cooldown_turns: 1,
                effect_summary: "Steals 1 carried Munition from enemy into own ENC (or spends 2 Munitions to apply Confused) + shifts 1 hex.".into(),
            },
            AbilitySpec {
                id: AbilityId::BuildArmoredDam,
                display_name: "Build Armored Dam / Ford".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 2,
                max_range: 1,
                cooldown_turns: 2,
                effect_summary: "Blockade Beaver converts an adjacent DeepRiver or Flat hex into a passable ArmoredDam choke.".into(),
            },
            AbilitySpec {
                id: AbilityId::VoltaicArcStun,
                display_name: "Voltaic Arc Stun".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 1,
                max_range: 2,
                cooldown_turns: 2,
                effect_summary: "Strips 1 Fortified/Armor point and stuns target (disables Melee 0 retaliation for 1 turn).".into(),
            },
            AbilitySpec {
                id: AbilityId::GeneticSpliceSurge,
                display_name: "Genetic Splice Surge".into(),
                trigger: TriggerMode::Active,
                action_cost: 1,
                munition_cost: 0,
                max_range: 2,
                cooldown_turns: 0,
                effect_summary: "Consumable Faction Artifact: grants +1 Fortified shield & purges Poison/Root for friendly faction units within 2 hexes.".into(),
            },
        ];

        let faction_passives = vec![
            FactionPassiveSpec {
                faction: OrganismFaction::Human,
                doctrine_name: "Command Uplink".into(),
                bonus_effect: "Autonomous (no Node leash); Human infantry gain +1 Range adjacent to Control Node or Radar Tower.".into(),
                tactical_tradeoff: "Unshielded biological Humans suffer 1 damage ending turn on Radiation Fallout hexes.".into(),
            },
            FactionPassiveSpec {
                faction: OrganismFaction::Mammal,
                doctrine_name: "Pack Tactics".into(),
                bonus_effect: "+1 to Hit rolls when adjacent to at least 1 friendly Mammal unit.".into(),
                tactical_tradeoff: "Requires 2–3 hex Control Node leash and lacks innate toxin immunity.".into(),
            },
            FactionPassiveSpec {
                faction: OrganismFaction::Reptile,
                doctrine_name: "Ectothermic Scales".into(),
                bonus_effect: "+1 Move on Badlands/Savannah/Caldera hexes and restores +1 Armor on Rest.".into(),
                tactical_tradeoff: "-1 Move (min 1) on DeepRiver/OpenOcean unless Semi-Aquatic.".into(),
            },
            FactionPassiveSpec {
                faction: OrganismFaction::Plant,
                doctrine_name: "Mycelial Network".into(),
                bonus_effect: "Autonomous, PoisonImmune, and Traps deploy hidden within 2 hexes of any friendly Plant.".into(),
                tactical_tradeoff: "Traps are stationary (MOVE 0) once rooted.".into(),
            },
            FactionPassiveSpec {
                faction: OrganismFaction::Bird,
                doctrine_name: "Aerial Dominance".into(),
                bonus_effect: "Flight bypasses cliffs, rivers & melee retaliation; +1 Move retreating toward Command Center.".into(),
                tactical_tradeoff: "Cannot use RoughCover or SubterraneanBurrow.".into(),
            },
            FactionPassiveSpec {
                faction: OrganismFaction::Arachnid,
                doctrine_name: "Neurotoxic Webs".into(),
                bonus_effect: "Web/Root hits against 0-Armor targets also apply 1 Poisoned stack.".into(),
                tactical_tradeoff: "Tighter 2-hex Node leash in enemy territory.".into(),
            },
            FactionPassiveSpec {
                faction: OrganismFaction::Insect,
                doctrine_name: "Eusocial Swarm".into(),
                bonus_effect: "-1 Munition deploy cost (min 1) when deployed adjacent to another friendly Insect.".into(),
                tactical_tradeoff: "-1 Evade when wounded below 50% max STR.".into(),
            },
            FactionPassiveSpec {
                faction: OrganismFaction::Fish,
                doctrine_name: "Pelagic Schooling".into(),
                bonus_effect: "+1 Move on DeepRiver/OpenOcean hexes and 4+ D6 Water Shield blocks Poison.".into(),
                tactical_tradeoff: "Pure Aquatic Fish (tier 2) require connected water channels.".into(),
            },
            FactionPassiveSpec {
                faction: OrganismFaction::Cnidarian,
                doctrine_name: "Cnidocyte Bio-Synapse".into(),
                bonus_effect: "Poison attacks apply 2 stacks instead of 1; Heavy Cnidarians project Node Radius 2.".into(),
                tactical_tradeoff: "Lower physical kinetic Armor against direct melee rushes.".into(),
            },
            FactionPassiveSpec {
                faction: OrganismFaction::Amphibian,
                doctrine_name: "Torpor Regeneration".into(),
                bonus_effect: "Rest on Water/Jungle/RoughCover heals +1 STR and grants Stealth until next turn.".into(),
                tactical_tradeoff: "-1 max ENC carried on Badlands/Caldera hexes (Dehydration).".into(),
            },
        ];

        Self {
            abilities,
            faction_passives,
        }
    }
}

/// Computes biome-aware movement budget for a unit given its Organism Faction & tile.
pub fn effective_movement_budget(
    base_move: u8,
    faction: OrganismFaction,
    biome: Biome,
    morphology: Morphology,
    rooted_turns: u8,
    burrowed: bool,
) -> u8 {
    if rooted_turns > 0 || base_move == 0 {
        return 0;
    }
    if burrowed {
        return 1;
    }
    let mut mv = i16::from(base_move);
    match faction {
        OrganismFaction::Reptile => {
            if matches!(
                biome,
                Biome::IrradiatedBadlands | Biome::SavannahDustbowl | Biome::RuinedBunkerCaldera
            ) {
                mv += 1;
            }
        }
        OrganismFaction::Fish => {
            if matches!(morphology, Morphology::DeepRiver | Morphology::OpenOcean) {
                mv += 1;
            }
        }
        _ => {}
    }
    mv.max(1) as u8
}

/// Executes Medic `FieldHealDetoxify` on a friendly target (`+1 STR` up to max and purges all `Poison` + `Root`).
pub fn execute_field_heal(target: &mut BoardUnit) {
    target.current_str_hp = (target.current_str_hp + 1).min(target.max_str_hp);
    target.poisoned_stacks = 0;
    target.rooted_turns = 0;
}

/// Executes `VenomSpitCnidocytes` (`+2` stacks for Cnidarian, `+1` otherwise, capped at 3).
pub fn execute_venom_spit(attacker_faction: OrganismFaction, defender: &mut BoardUnit, poison_immune: bool) -> u8 {
    if poison_immune {
        return 0;
    }
    let delta = if attacker_faction == OrganismFaction::Cnidarian { 2 } else { 1 };
    let before = defender.poisoned_stacks;
    defender.poisoned_stacks = (defender.poisoned_stacks + delta).min(3);
    defender.poisoned_stacks - before
}

/// Executes `WebAndRootSnare` (roots for 2 turns; Arachnid passive adds 1 Poison if unarmored).
pub fn execute_web_snare(attacker_faction: OrganismFaction, defender: &mut BoardUnit, poison_immune: bool) {
    defender.rooted_turns = 2;
    if attacker_faction == OrganismFaction::Arachnid && defender.current_armor == 0 && !poison_immune {
        defender.poisoned_stacks = (defender.poisoned_stacks + 1).min(3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{CombatClass, GraveyardFacility};

    fn sample_unit(current_str: u8, max_str: u8, poison: u8, rooted: u8) -> BoardUnit {
        BoardUnit {
            card_key: "Test Unit".into(),
            faction: OrganismFaction::Mammal,
            combat_class: CombatClass::Medium,
            graveyard_queue: GraveyardFacility::HospitalBio,
            current_str_hp: current_str,
            max_str_hp: max_str,
            current_armor: 1,
            base_armor: 1,
            evade: 1,
            encumbrance_cap: 3,
            carried_munitions: 3,
            node_leash_range: 4,
            emits_node: false,
            node_emit_radius: 0,
            supply_range: 0,
            is_laser: false,
            laser_fired_this_turn: false,
            confused: false,
            poisoned_stacks: poison,
            fortified_stacks: 0,
            rooted_turns: rooted,
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
    fn field_heal_purges_multi_stack_ailments_and_caps_str_at_max() {
        let mut wounded = sample_unit(3, 5, 3, 2);
        execute_field_heal(&mut wounded);
        assert_eq!(wounded.current_str_hp, 4);
        assert_eq!(wounded.poisoned_stacks, 0);
        assert_eq!(wounded.rooted_turns, 0);

        let mut full_hp = sample_unit(5, 5, 2, 1);
        execute_field_heal(&mut full_hp);
        assert_eq!(full_hp.current_str_hp, 5);
        assert_eq!(full_hp.poisoned_stacks, 0);
        assert_eq!(full_hp.rooted_turns, 0);
    }
}
