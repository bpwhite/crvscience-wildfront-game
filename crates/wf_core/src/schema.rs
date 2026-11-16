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

//! Validated 133-card roster schema, calibrated 26-stat weighted point model,
//! and 250-point two-faction tournament deck legality verifier.

use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Canonical 2072 emergence narrative constant.
pub const WILDFRONT_THEME_2072: &str = "The year is 2072. Only six continents remain. \
The earth has been wracked by global warming, drought, and radiation following years of nuclear war. \
Humanity has retreated into underground bunkers along with a handful of surviving plants and animals. \
Lacking the resources to construct new military equipment, each surviving group of humans sought to \
bolster their forces through genetic experimentation. Armed with the most lethal lifeforms across \
the planet, humanity has begun to emerge from their underground dwellings. \
The war on the surface, the Wildfront, is where you stand today.";

/// Integer tenths (`weight * 10`) matching `Modular Stat Weights!$C$2:$C$27` in
/// `Wildfront_PC_Master_Stats_and_DeckBuilder.xlsx`:
/// `[STR:9, ENC:4, ARMR:15, SHD:15, MOV:9, EVD:12, MEL:9, RNG:14, LAS:20, LEASH:3,
///   EMIT:18, ERAD:8, ACT:40, SUP:11, RAD:6, HEAL:13, ROOT:14, STL:13, POIS:13, PIMM:7,
///   FLY:18, SAB:13, BUR:13, TNT:11, AMP:7, AUTO:13]`.
pub const STAT_WEIGHTS_TENTHS: [u16; 26] = [
    9, 4, 15, 15, 9, 12, 9, 14, 20, 3, 18, 8, 40, 11, 6, 13, 14, 13, 13, 7, 18, 13, 13, 11, 7, 13,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum OriginAllegiance {
    Australia,
    Africa,
    Asia,
    Europe,
    NorthAmerica,
    SouthAmerica,
    Neutral,
}

impl OriginAllegiance {
    pub fn from_label(raw: &str) -> Result<Self, ValidationError> {
        match raw.trim() {
            "Australia" => Ok(Self::Australia),
            "Africa" => Ok(Self::Africa),
            "Asia" => Ok(Self::Asia),
            "Europe" => Ok(Self::Europe),
            "North America" | "NorthAmerica" => Ok(Self::NorthAmerica),
            "South America" | "SouthAmerica" => Ok(Self::SouthAmerica),
            "Neutral" => Ok(Self::Neutral),
            other => Err(ValidationError::UnknownOrigin(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum OrganismFaction {
    Human,
    Mammal,
    Reptile,
    Plant,
    Bird,
    Arachnid,
    Insect,
    Fish,
    Cnidarian,
    Amphibian,
}

impl OrganismFaction {
    pub const ALL: [Self; 10] = [
        Self::Human,
        Self::Mammal,
        Self::Reptile,
        Self::Plant,
        Self::Bird,
        Self::Arachnid,
        Self::Insect,
        Self::Fish,
        Self::Cnidarian,
        Self::Amphibian,
    ];

    pub fn from_label(raw: &str) -> Result<Self, ValidationError> {
        match raw.trim() {
            "Human" => Ok(Self::Human),
            "Mammal" => Ok(Self::Mammal),
            "Reptile" => Ok(Self::Reptile),
            "Plant" => Ok(Self::Plant),
            "Bird" => Ok(Self::Bird),
            "Arachnid" => Ok(Self::Arachnid),
            "Insect" => Ok(Self::Insect),
            "Fish" => Ok(Self::Fish),
            "Cnidarian" => Ok(Self::Cnidarian),
            "Amphibian" => Ok(Self::Amphibian),
            other => Err(ValidationError::UnknownOrganism(other.to_string())),
        }
    }

    pub fn slug(&self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Mammal => "mammal",
            Self::Reptile => "reptile",
            Self::Plant => "plant",
            Self::Bird => "bird",
            Self::Arachnid => "arachnid",
            Self::Insect => "insect",
            Self::Fish => "fish",
            Self::Cnidarian => "cnidarian",
            Self::Amphibian => "amphibian",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CombatClass {
    Installation,
    SupportInstallation,
    ControlNode,
    SupplyDepot,
    Commander,
    Heavy,
    Medium,
    Support,
    Medic,
    Light,
    Trap,
    Artifact,
    #[serde(alias = "Tactica", alias = "Spell")]
    TacticalDeployment,
}

impl CombatClass {
    pub fn from_label(raw: &str) -> Result<Self, ValidationError> {
        match raw.trim() {
            "Installation" => Ok(Self::Installation),
            "Support Installation" | "SupportInstallation" => Ok(Self::SupportInstallation),
            "Control Node" | "ControlNode" => Ok(Self::ControlNode),
            "Supply Depot" | "SupplyDepot" => Ok(Self::SupplyDepot),
            "Commander" => Ok(Self::Commander),
            "Heavy" => Ok(Self::Heavy),
            "Medium" => Ok(Self::Medium),
            "Support" => Ok(Self::Support),
            "Medic" => Ok(Self::Medic),
            "Light" => Ok(Self::Light),
            "Trap" => Ok(Self::Trap),
            "Artifact" => Ok(Self::Artifact),
            "Tactical Deployment" | "TacticalDeployment" | "Tactica" | "Spell" => Ok(Self::TacticalDeployment),
            other => Err(ValidationError::UnknownCombatClass(other.to_string())),
        }
    }

    pub fn is_building(&self) -> bool {
        matches!(
            self,
            Self::Installation
                | Self::SupportInstallation
                | Self::ControlNode
                | Self::SupplyDepot
        )
    }

    pub fn default_squad_models(&self) -> u8 {
        match self {
            Self::Light => 4,
            Self::Medium | Self::Medic => 3,
            _ => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum GraveyardFacility {
    BaseCore,
    HospitalBio,
    FactoryMech,
    Consumable,
}

impl GraveyardFacility {
    pub fn from_label(raw: &str) -> Result<Self, ValidationError> {
        match raw.trim() {
            "Base Core" | "BaseCore" => Ok(Self::BaseCore),
            "Hospital (Bio)" | "HospitalBio" => Ok(Self::HospitalBio),
            "Factory (Mech)" | "FactoryMech" => Ok(Self::FactoryMech),
            "Consumable" => Ok(Self::Consumable),
            other => Err(ValidationError::UnknownGraveyardQueue(other.to_string())),
        }
    }
}

/// Balanced unit/installation/artifact card definition ingested from `Master Roster` (v3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitCardDef {
    pub card_key: String,
    pub unit_name: String,
    pub origin: OriginAllegiance,
    pub species: String,
    pub faction: OrganismFaction,
    pub combat_class: CombatClass,
    pub graveyard_queue: GraveyardFacility,
    pub signature_ability: String,
    pub strength_hp: u8,
    pub encumbrance: u8,
    pub armor: u8,
    pub shield: u8,
    pub movement: u8,
    pub evade: u8,
    pub melee_range: u8,
    pub ranged_distance: u8,
    pub is_laser: bool,
    pub node_leash_range: u8,
    pub emits_node: bool,
    pub node_emit_radius: u8,
    pub action_bonus: u8,
    pub supply_range: u8,
    pub radar_range: u8,
    pub heal_repair_range: u8,
    pub root_stacks: u8,
    pub stealth: bool,
    pub poison_stacks: u8,
    pub poison_immune: bool,
    pub can_fly: bool,
    pub saboteur: bool,
    pub can_burrow: bool,
    pub taunt_block: u8,
    /// `0` = Terrestrial, `1` = Semi-Aquatic (land + water), `2` = Pure Aquatic.
    pub amphibious_tier: u8,
    pub is_autonomous: bool,
    pub deck_points: u16,
    /// `10 * (STR + 2*ARMR + 2*Shield + 1.5*EVD)` for exact integer representation.
    pub combat_ehp_tenths: u16,
    pub squad_model_count: u8,
    pub provenance_note: String,
}

impl UnitCardDef {
    pub fn raw_stat_vector(&self) -> [u16; 26] {
        [
            u16::from(self.strength_hp),
            u16::from(self.encumbrance),
            u16::from(self.armor),
            u16::from(self.shield),
            u16::from(self.movement),
            u16::from(self.evade),
            u16::from(self.melee_range),
            u16::from(self.ranged_distance),
            u16::from(u8::from(self.is_laser)),
            u16::from(self.node_leash_range),
            u16::from(u8::from(self.emits_node)),
            u16::from(self.node_emit_radius),
            u16::from(self.action_bonus),
            u16::from(self.supply_range),
            u16::from(self.radar_range),
            u16::from(self.heal_repair_range),
            u16::from(self.root_stacks),
            u16::from(u8::from(self.stealth)),
            u16::from(self.poison_stacks),
            u16::from(u8::from(self.poison_immune)),
            u16::from(u8::from(self.can_fly)),
            u16::from(u8::from(self.saboteur)),
            u16::from(u8::from(self.can_burrow)),
            u16::from(self.taunt_block),
            u16::from(self.amphibious_tier),
            u16::from(u8::from(self.is_autonomous)),
        ]
    }

    /// Exact integer mirror of Excel `=ROUND(SUM(stat_i * weight_i), 0)`.
    pub fn computed_weighted_points(&self) -> u16 {
        let vals = self.raw_stat_vector();
        let tenths_sum: u32 = vals
            .iter()
            .zip(STAT_WEIGHTS_TENTHS.iter())
            .map(|(&v, &w)| u32::from(v) * u32::from(w))
            .sum();
        ((tenths_sum + 5) / 10) as u16
    }

    /// Maps every card in the 133-unit roster across all 10 Organism Factions and
    /// Human Support Installations to its dedicated Sparse 64-Tree `.vox` rig slug.
    pub fn default_vox_slug(&self) -> &'static str {
        match self.card_key.as_str() {
            "Abyssal Hydra Polyp" => "cnidarian_hydra_polyp",
            "Abyssal Riptide" => "td_abyssal_riptide",
            "Apex Bloodlust Roar" => "td_apex_bloodlust_roar",
            "Apex Orca" => "mammal_apex_orca",
            "Armada Manta Ray" => "fish_manta_ray",
            "Armored Supply Buffalo" => "mammal_armored_supply_buffalo",
            "Auxiliary Llama" => "mammal_auxiliary_llama",
            "Avian Aerie Splice" => "art_avian_aerie_splice",
            "Axolotl Bio-Mender" => "amphibian_axolotl_mender",
            "Battle Llama" => "mammal_battle_llama",
            "Battleclaw Komodo" => "reptile_battleclaw_komodo",
            "Battlephant" => "mammal_battlephant",
            "Battletusker" => "mammal_battletusker",
            "Bittersweet Nightshade" => "plant_bittersweet_nightshade",
            "Blockade Beaver" => "mammal_blockade_beaver",
            "Bombardier Ravens" => "bird_bombardier_ravens",
            "Bombardier Siege Beetle" => "insect_siege_beetle",
            "Bomber Hornet" => "insect_bomber_hornet",
            "Box-Jelly Lurker" => "cnidarian_box_jelly_lurker",
            "Bramble Barricade" => "td_bramble_barricade",
            "Brown Warweaver" => "arachnid_brown_warweaver",
            "Bullfrog Grapple Scout" => "amphibian_bullfrog_grapple_scout",
            "Cane Toad Mortar" => "amphibian_cane_toad_mortar",
            "Cargo Drop" => "art_cargo_drop",
            "Cargo Sloth" => "mammal_cargo_sloth",
            "Carrier Pigeon Dreadnought" => "bird_dreadnought",
            "Chameleon Stalker" => "reptile_chameleon_stalker",
            "Chitin Carapace Splice" => "art_chitin_carapace_splice",
            "Cicada Signal Relay" => "insect_cicada_signal_relay",
            "Cnidocyte Synapse Vat" => "art_cnidocyte_synapse_vat",
            "Cobra Striker" => "reptile_cobra_striker",
            "Combat Ground Sloth" => "mammal_ground_sloth",
            "Combat Koala" => "combat_koala_squad",
            "Combat Medic" => "human_medic",
            "Combat Wombat" => "mammal_combat_wombat",
            "Command Bonus" => "art_command_bonus",
            "Command Center (Africa)" => "command_center_africa",
            "Command Center (Asia)" => "command_center_asia",
            "Command Center (Australia)" => "command_center",
            "Command Center (Europe)" => "command_center_europe",
            "Command Center (North America)" => "command_center_north_america",
            "Command Center (South America)" => "command_center_south_america",
            "Commander (Africa)" => "commander_africa",
            "Commander (Asia)" => "commander_asia",
            "Commander (Australia)" => "human_commander",
            "Commander (Europe)" => "commander_europe",
            "Commander (North America)" => "commander_north_america",
            "Commander (South America)" => "commander_south_america",
            "Control Node" => "control_node",
            "Convoy" => "supply_truck",
            "Corrosive Acid Spray" => "td_corrosive_acid_spray",
            "Crestbird Flier" => "bird_crestbird_flier",
            "Cryo-Hibernation Vat" => "art_cryo_hibernation_vat",
            "Deadly Elephant Ear" => "plant_deadly_elephant_ear",
            "Deadly Nightshade" => "plant_deadly_nightshade",
            "Deathstalker Scorpion" => "arachnid_scorpion",
            "Devil's Thornapple" => "plant_devil_s_thornapple",
            "Dive-Strike Peregrine" => "bird_dive_strike_peregrine",
            "Drone" => "drone",
            "EMP Blackout Pulse" => "td_emp_blackout_pulse",
            "Earthclaw Badger" => "mammal_earthclaw_badger",
            "Factory" => "factory",
            "Falconer" => "human_falconer",
            "Fireclaw Lion" => "mammal_fireclaw_lion",
            "Flamecoats" => "mammal_flamecoats",
            "Flamer Croc" => "reptile_flamer_croc",
            "Frostclaw Bear" => "mammal_frostclaw_bear",
            "Funnel-Web Trapper" => "arachnid_funnel_web_trapper",
            "Hellbender Salamander" => "amphibian_hellbender_salamander",
            "Hospital" => "hospital",
            "Hover-Coil Titanoboa" => "reptile_cobra_titanoboa",
            "Hydro-Breach Canal" => "td_hydro_breach_canal",
            "Ichuri" => "human_ichuri",
            "Inferno Ants" => "insect_inferno_ants",
            "Infiltrator Vine Python" => "reptile_infiltrator_vine_python",
            "Inyanga" => "human_inyanga",
            "Ion Curtain Shield Dome" => "td_ion_curtain_shield_dome",
            "Ironclaw Lion" => "mammal_ironclaw_lion",
            "Jelly Zeppelin" => "jelly_zeppelin",
            "Jump Rat" => "mammal_jump_rat",
            "Jungle Stalker Panther" => "mammal_jungle_stalker_panther",
            "Kampo" => "human_kampo",
            "Laser Turret" => "radar_laser_turret",
            "Leechwing Bat" => "mammal_leechwing_bat",
            "Legspinner Tarantula" => "arachnid_tarantula",
            "Mainframe Man-o'-War" => "cnidarian_manowar",
            "Mantis Blade Stalker" => "insect_mantis",
            "Mecharoo (Kanga Mech)" => "mecharoo",
            "Mercy Dog" => "mammal_mercy_dog",
            "Mire Pitfall Pitcher" => "plant_mire_pitfall_pitcher",
            "Nano-Triage Field Burst" => "td_nano_triage_field_burst",
            "Napalm Basking Surge" => "td_napalm_basking_surge",
            "Neural Short-Circuit" => "td_neural_short_circuit",
            "Neuro-Venom Miasma" => "td_neuro_venom_miasma",
            "Ngangkari" => "human_ngangkari",
            "Oaken Sentinel" => "mammal_oaken_sentinel",
            "Optic-Laser Shark" => "fish_laser_shark",
            "Orbital Kinetic Strike" => "td_orbital_kinetic_strike",
            "Pack-Cackle Hyena" => "mammal_pack_cackle_hyena",
            "Pestilence Skunk" => "mammal_saboteur_skunk",
            "Phantom Mist Veil" => "td_phantom_mist_veil",
            "Pheromone Brood Call" => "td_pheromone_brood_call",
            "Plague Doctor" => "human_plague_doctor",
            "Pocket Gopher" => "mammal_pocket_gopher",
            "Radar Tower" => "radar_dish_tower",
            "Ragin' Caiman" => "reptile_ragin_caiman",
            "Rapier Swordfish" => "fish_rapier_swordfish",
            "Recon Gila" => "reptile_recon_gila",
            "Recon Rover" => "mammal_recon_rover",
            "Recon Trout" => "fish_recon_trout",
            "Rocket Rhino" => "mammal_rocket_rhino",
            "Roostriker" => "mammal_roostriker",
            "Royal Jelly Mender" => "insect_royal_jelly_mender",
            "Saberclaw Warcat" => "mammal_saberclaw_warcat",
            "Saboteur" => "mammal_saboteur",
            "Satellite Recon Sweep" => "td_satellite_recon_sweep",
            "Sentinel Elk" => "mammal_sentinel_elk",
            "Shadow Rat" => "mammal_shadow_rat",
            "Shadowclaw Panda" => "mammal_shadowclaw_panda",
            "Shadowtail Scout" => "mammal_shadowtail_scout",
            "Shellguard Tortoise" => "reptile_shellguard",
            "Shield Generator" => "shield_generator_dome",
            "Siphonophore Frigate" => "cnidarian_siphonophore_frigate",
            "Skyhunter Falcon" => "bird_skyhunter",
            "Sneak-Paw Polecat" => "mammal_sneak_paw_polecat",
            "Spore Monsoon" => "td_spore_monsoon",
            "Spore-Pod Mortar" => "plant_spore_mortar",
            "Stalker Wolf Spider" => "arachnid_stalker_wolf_spider",
            "Stampede Shockwave" => "td_stampede_shockwave",
            "Stealth Sundew" => "plant_sundew_nightshade",
            "Stratospheric Dive-Target" => "td_stratospheric_dive_target",
            "Supply Depot" => "supply_depot",
            "Support Giraffe" => "mammal_support_giraffe",
            "Support Panda" => "mammal_support_panda",
            "Support Roo" => "mammal_support_roo",
            "Tactical Relay Overclock" => "td_tactical_relay_overclock",
            "Thermogenic Scale Splice" => "bio_splice_vat",
            "Thylacine" => "mammal_thylacine",
            "Tidehopper Mudskipper" => "fish_tidehopper_mudskipper",
            "Timberwolves" => "mammal_timberwolf_pack",
            "Toxiwood Chi'ute" => "plant_toxiwood_chi_ute",
            "Treeclaw Sloth" => "mammal_treeclaw_sloth",
            "Tropical Acid Pitcher" => "plant_pitcher_vat",
            "Tusker" => "mammal_tusker",
            "Venom Dart" => "amphibian_venom_frog",
            "Venus Snapper" => "plant_trap",
            "Voltaic Eel Disruptor" => "fish_voltaic_eel",
            "Warback Gorilla" => "mammal_warback_gorilla",
            "Warclaw Battleshrew" => "mammal_warclaw_battleshrew",
            "Warneck Giraffe" => "mammal_warneck_giraffe",
            "Warstag" => "mammal_warstag_elk",
            "Yangtze Scoutfin" => "mammal_yangtze_scoutfin",
            _ => "combat_koala_squad",
        }
    }

    pub fn requires_control_node(&self) -> bool {
        !self.is_autonomous && self.node_leash_range > 0 && !self.emits_node
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.card_key.trim().is_empty() {
            return Err(ValidationError::EmptyField("card_key"));
        }
        if self.unit_name.trim().is_empty() {
            return Err(ValidationError::EmptyField("unit_name"));
        }
        if self.strength_hp == 0 {
            return Err(ValidationError::ZeroStrength {
                card_key: self.card_key.clone(),
            });
        }
        let expected = self.computed_weighted_points();
        if self.deck_points == 0 || self.deck_points.abs_diff(expected) > 5 {
            return Err(ValidationError::PointSumMismatch {
                card_key: self.card_key.clone(),
                spreadsheet_points: self.deck_points,
                computed_points: expected,
            });
        }
        if self.emits_node && self.node_emit_radius == 0 {
            return Err(ValidationError::InvalidNodeEmitter {
                card_key: self.card_key.clone(),
            });
        }
        if self.is_laser && self.ranged_distance == 0 {
            return Err(ValidationError::LaserWithoutRange {
                card_key: self.card_key.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterManifest {
    pub schema_version: u32,
    pub source_workbook: String,
    pub standard_deck_budget: u16,
    pub opening_hand_size: u8,
    pub base_actions_per_turn: u8,
    pub base_munitions_per_turn: u8,
    pub units: Vec<UnitCardDef>,
}

impl RosterManifest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        let mut seen_keys = BTreeSet::new();
        let mut faction_counts: BTreeMap<OrganismFaction, usize> = BTreeMap::new();
        let mut has_hospital = false;
        let mut has_factory = false;
        let mut has_drone = false;

        for u in &self.units {
            u.validate()?;
            if !seen_keys.insert(u.card_key.clone()) {
                return Err(ValidationError::DuplicateCardKey(u.card_key.clone()));
            }
            *faction_counts.entry(u.faction).or_insert(0) += 1;
            if u.card_key == "Hospital" {
                has_hospital = true;
            }
            if u.card_key == "Factory" && u.graveyard_queue == GraveyardFacility::FactoryMech {
                has_factory = true;
            }
            if u.card_key == "Drone" && u.emits_node && u.node_emit_radius == 2 {
                has_drone = true;
            }
        }

        if !has_hospital {
            return Err(ValidationError::MissingRequiredFacility("Hospital"));
        }
        if !has_factory {
            return Err(ValidationError::MissingRequiredFacility("Factory"));
        }
        if !has_drone {
            return Err(ValidationError::MissingRequiredFacility("Drone (Node Emit Radius 2)"));
        }
        for fac in OrganismFaction::ALL {
            let cnt = faction_counts.get(&fac).copied().unwrap_or(0);
            if cnt < 6 {
                return Err(ValidationError::InsufficientFactionDepth {
                    faction: format!("{:?}", fac),
                    count: cnt,
                });
            }
        }
        Ok(())
    }

    pub fn index_by_key(&self) -> BTreeMap<&str, &UnitCardDef> {
        self.units.iter().map(|u| (u.card_key.as_str(), u)).collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeckEntry {
    pub card_key: String,
    pub quantity: u8,
}

pub fn default_continent_origin() -> OriginAllegiance {
    OriginAllegiance::Australia
}

/// Canonical 1-of-6 Continental Faction Command Card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactionCardDef {
    pub origin: OriginAllegiance,
    pub title: String,
    pub bunker_stronghold: String,
    pub command_doctrine: String,
    pub passive_summary: String,
    pub active_ability_name: String,
    pub active_action_cost: u8,
    pub active_munition_cost: u8,
    pub active_effect_summary: String,
    pub narrative_fluff: String,
}

impl FactionCardDef {
    pub const ALL: [OriginAllegiance; 6] = [
        OriginAllegiance::Australia,
        OriginAllegiance::Africa,
        OriginAllegiance::Asia,
        OriginAllegiance::Europe,
        OriginAllegiance::NorthAmerica,
        OriginAllegiance::SouthAmerica,
    ];

    pub fn get(origin: OriginAllegiance) -> Self {
        match origin {
            OriginAllegiance::Australia => Self {
                origin,
                title: "Faction: Australia (Pacific Citadel)".into(),
                bunker_stronghold: "Underground Redoubt 07 — Great Dividing Range".into(),
                command_doctrine: "Nanite Grid & Autonomous Relay".into(),
                passive_summary: "Installations and Control Nodes gain +1 Emit Radius. Light units deployed adjacent to Nodes gain +1 MOVE on their first turn.".into(),
                active_ability_name: "Nanite Surge".into(),
                active_action_cost: 1,
                active_munition_cost: 2,
                active_effect_summary: "Repair +2 STR to an adjacent installation or friendly cybernetic/human rig.".into(),
                narrative_fluff: "Isolated by rising seas, the Pacific Citadel turned to nanite mesh networks and autonomous drone logic to patrol the scorched outback.".into(),
            },
            OriginAllegiance::Africa => Self {
                origin,
                title: "Faction: Africa (Pan-African Solar Fortress)".into(),
                bunker_stronghold: "Rift Valley Geothermal Bastion".into(),
                command_doctrine: "Solar Capacitors & Desert Fortification".into(),
                passive_summary: "Command Center generates +1 bonus Munition every even turn. Friendly units ignore movement penalties on Savannah / Caldera terrain.".into(),
                active_ability_name: "Solar Flare Burst".into(),
                active_action_cost: 1,
                active_munition_cost: 1,
                active_effect_summary: "Radius-1 solar flash blinds enemy cluster (-1 Evade & Target Locked) for 1 turn.".into(),
                narrative_fluff: "Harnessing equatorial solar radiation and deep rift energy, the Pan-African Redoubt excels in energy storage and endurance warfare.".into(),
            },
            OriginAllegiance::Asia => Self {
                origin,
                title: "Faction: Asia (Trans-Asian Cyber-Grid)".into(),
                bunker_stronghold: "Gobi Deep Sub-Territory Complex".into(),
                command_doctrine: "Command Uplink & Swarm Telemetry".into(),
                passive_summary: "Human and Drone units have zero Node leash requirements and gain +1 Range when targeting enemies detected by friendly Radar.".into(),
                active_ability_name: "Orbital Sensor Sweep".into(),
                active_action_cost: 1,
                active_munition_cost: 1,
                active_effect_summary: "Reveal all stealthed and burrowed units within 3 hexes until the end of the round.".into(),
                narrative_fluff: "Hardened against nuclear EMPs through buried fiber corridors, the Asian grid coordinates synchronized swarms across massive distances.".into(),
            },
            OriginAllegiance::Europe => Self {
                origin,
                title: "Faction: Europe (Alpine Bastion)".into(),
                bunker_stronghold: "Gotthard Granite Fortress Vault".into(),
                command_doctrine: "Fortress Wall & Heavy Ballistics".into(),
                passive_summary: "Heavy units and Commanders gain +1 Armor against ranged attacks exceeding 2 hexes. Installations enter play with +1 Fortified shield.".into(),
                active_ability_name: "Bastion Protocol".into(),
                active_action_cost: 1,
                active_munition_cost: 2,
                active_effect_summary: "Grant +1 Fortified shield (blocks 1 attack) to up to 2 adjacent friendly units.".into(),
                narrative_fluff: "Buried beneath Alpine granite, European survivors engineered indestructible blast bulkheads and heavy kinetic dampeners.".into(),
            },
            OriginAllegiance::NorthAmerica => Self {
                origin,
                title: "Faction: North America (Atlantic Shield Array)".into(),
                bunker_stronghold: "Cheyenne Mountain NORAD Redoubt".into(),
                command_doctrine: "EMP Shielding & Precision Laser Arrays".into(),
                passive_summary: "Units are immune to Voltaic Stun and electronic disruption. Laser weapons deal +1 damage on a natural 6 hit roll.".into(),
                active_ability_name: "EMP & Aegis Shockwave".into(),
                active_action_cost: 1,
                active_munition_cost: 1,
                active_effect_summary: "Disrupt enemy electronics within 2 hexes: strips Target Lock and halves enemy Radar range for 1 turn.".into(),
                narrative_fluff: "The subterranean NORAD command grid pioneered solid-state laser optics and magnetic shielding to survive orbital bombardments.".into(),
            },
            OriginAllegiance::SouthAmerica => Self {
                origin,
                title: "Faction: South America (Amazonian Biosphere)".into(),
                bunker_stronghold: "Guiana Shield Subterranean Canopy".into(),
                command_doctrine: "Bio-Capacitance & Accelerated Regeneration".into(),
                passive_summary: "Hospital recovery turns are reduced by 1. Biological units on Jungle, River, or Rough Cover heal +1 STR at turn start.".into(),
                active_ability_name: "Primal Surge".into(),
                active_action_cost: 1,
                active_munition_cost: 2,
                active_effect_summary: "Purge all Poison and Root stacks from friendly biological units within 2 hexes of the Commander.".into(),
                narrative_fluff: "Preserving equatorial genetic vaults deep in the Amazon basin, this faction excels in cellular mending and environmental adaptation.".into(),
            },
            OriginAllegiance::Neutral => Self {
                origin,
                title: "Faction: Frontier Mercenary / Independent".into(),
                bunker_stronghold: "Surface Nomads & Wasteland Outposts".into(),
                command_doctrine: "Scavenger Logistics".into(),
                passive_summary: "Depots store +2 extra Munitions. Units can scavenge 1 Munition from destroyed enemy wrecks.".into(),
                active_ability_name: "Emergency Rations".into(),
                active_action_cost: 1,
                active_munition_cost: 0,
                active_effect_summary: "Convert 1 Action into +1 carried Munition for a friendly unit.".into(),
                narrative_fluff: "Hardened scavengers living between the continental borders.".into(),
            },
        }
    }
}

/// Canonical 1-of-9 Organism Bio-Type Card with Hybrid and Pure Doctrines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BioTypeCardDef {
    pub bio_type: OrganismFaction,
    pub title: String,
    pub hybrid_doctrine_name: String,
    pub hybrid_doctrine_effect: String,
    pub pure_doctrine_name: String,
    pub pure_doctrine_effect: String,
    pub primal_surge_name: String,
    pub primal_surge_cost: (u8, u8),
    pub primal_surge_effect: String,
    pub affinity_biomes: Vec<String>,
}

impl BioTypeCardDef {
    pub const ALL: [OrganismFaction; 9] = [
        OrganismFaction::Mammal,
        OrganismFaction::Reptile,
        OrganismFaction::Plant,
        OrganismFaction::Bird,
        OrganismFaction::Arachnid,
        OrganismFaction::Insect,
        OrganismFaction::Fish,
        OrganismFaction::Cnidarian,
        OrganismFaction::Amphibian,
    ];

    pub fn get(bio_type: OrganismFaction) -> Self {
        match bio_type {
            OrganismFaction::Mammal => Self {
                bio_type,
                title: "Bio-Type: Mammal (Apex Endotherms)".into(),
                hybrid_doctrine_name: "Pack Tactics".into(),
                hybrid_doctrine_effect: "+1 to Hit rolls when adjacent to at least 1 friendly Mammal unit.".into(),
                pure_doctrine_name: "Apex Pack Dominion (Pure Bio-Type)".into(),
                pure_doctrine_effect: "+2 to Hit rolls when flanking; +1 MOVE on the first turn after deployment; all Mammals gain +1 max STR.".into(),
                primal_surge_name: "Stampede Rush".into(),
                primal_surge_cost: (1, 1),
                primal_surge_effect: "Free 1-hex charge into melee dealing +1 impact damage with zero defender retaliation.".into(),
                affinity_biomes: vec!["Savannah".into(), "Tundra".into(), "RoughCover".into()],
            },
            OrganismFaction::Reptile => Self {
                bio_type,
                title: "Bio-Type: Reptile (Armored Coldbloods)".into(),
                hybrid_doctrine_name: "Ectothermic Scales".into(),
                hybrid_doctrine_effect: "+1 Move on Badlands/Savannah/Caldera hexes and restores +1 Armor on Rest.".into(),
                pure_doctrine_name: "Armored Coldblood Dynasty (Pure Bio-Type)".into(),
                pure_doctrine_effect: "Heavy/Medium Reptiles enter with +1 Basking Scale Armor on deploy; Field Rest restores +2 STR; immune to fallout penalties.".into(),
                primal_surge_name: "Hardened Carapace".into(),
                primal_surge_cost: (1, 1),
                primal_surge_effect: "Absorbs the next 2 points of incoming combat damage until next turn.".into(),
                affinity_biomes: vec!["Badlands".into(), "Caldera".into(), "DeepRiver".into()],
            },
            OrganismFaction::Plant => Self {
                bio_type,
                title: "Bio-Type: Plant (Bio-Synthetics & Spores)".into(),
                hybrid_doctrine_name: "Mycelial Network".into(),
                hybrid_doctrine_effect: "Autonomous (no Node leash), PoisonImmune, and Traps deploy hidden within 2 hexes of any friendly Plant.".into(),
                pure_doctrine_name: "Ancient Overgrowth Canopy (Pure Bio-Type)".into(),
                pure_doctrine_effect: "All Plants gain +1 STR & enter with +1 Fortified Shield; first Plant Trap deployed each turn costs 0 Munitions; Plants share sightlines.".into(),
                primal_surge_name: "Spore Cloud Burst".into(),
                primal_surge_cost: (1, 1),
                primal_surge_effect: "Releases virulent spores, applying Rooted (MOVE 0) to all adjacent enemy ground units for 1 turn.".into(),
                affinity_biomes: vec!["Jungle".into(), "Forest".into(), "RoughCover".into()],
            },
            OrganismFaction::Bird => Self {
                bio_type,
                title: "Bio-Type: Bird (Aerial Dominance)".into(),
                hybrid_doctrine_name: "Aerial Dominance".into(),
                hybrid_doctrine_effect: "Flight bypasses cliffs, rivers & melee retaliation; +1 Move retreating toward Command Center.".into(),
                pure_doctrine_name: "Stratospheric Apex (Pure Bio-Type)".into(),
                pure_doctrine_effect: "All Birds gain +1 Range on ranged attacks and +1 Evade; diving attacks gain +1 D6 damage dice.".into(),
                primal_surge_name: "High-Altitude Recon".into(),
                primal_surge_cost: (1, 0),
                primal_surge_effect: "Grants True Sight and +1 Hit roll to all friendly ranged units against a target hex for 1 turn.".into(),
                affinity_biomes: vec!["Crag".into(), "Canyon".into(), "Plateau".into()],
            },
            OrganismFaction::Arachnid => Self {
                bio_type,
                title: "Bio-Type: Arachnid (Neurotoxic Predators)".into(),
                hybrid_doctrine_name: "Neurotoxic Webs".into(),
                hybrid_doctrine_effect: "Web/Root hits against 0-Armor targets also apply 1 Poisoned stack.".into(),
                pure_doctrine_name: "Chitinous Web Colony (Pure Bio-Type)".into(),
                pure_doctrine_effect: "Web/Root range +1; Poison deals 2 armor-piercing damage per turn instead of 1; webbed hexes cost enemies +2 MOVE.".into(),
                primal_surge_name: "Silk Snare Ambush".into(),
                primal_surge_cost: (1, 1),
                primal_surge_effect: "Instantly anchors an enemy within 3 hexes, reducing their MOVE to 0 and applying 1 Poison stack.".into(),
                affinity_biomes: vec!["Subterranean".into(), "RoughCover".into(), "Ruins".into()],
            },
            OrganismFaction::Insect => Self {
                bio_type,
                title: "Bio-Type: Insect (Eusocial Swarms)".into(),
                hybrid_doctrine_name: "Eusocial Swarm".into(),
                hybrid_doctrine_effect: "-1 Munition deploy cost (min 1) when deployed adjacent to another friendly Insect.".into(),
                pure_doctrine_name: "Hive Mind Resurgence (Pure Bio-Type)".into(),
                pure_doctrine_effect: "-2 Munitions deploy cost (min 0) for Light insects; defeated Light insects recycle into the deployment pool on 4+ D6.".into(),
                primal_surge_name: "Swarm Frenzy".into(),
                primal_surge_cost: (1, 1),
                primal_surge_effect: "Adjacent friendly insects gain +1 attack action this turn with zero munition cost.".into(),
                affinity_biomes: vec!["Badlands".into(), "Jungle".into(), "Wasteland".into()],
            },
            OrganismFaction::Fish => Self {
                bio_type,
                title: "Bio-Type: Fish (Pelagic Schooling)".into(),
                hybrid_doctrine_name: "Pelagic Schooling".into(),
                hybrid_doctrine_effect: "+1 Move on DeepRiver/OpenOcean hexes and 4+ D6 Water Shield blocks Poison.".into(),
                pure_doctrine_name: "Abyssal Leviathan School (Pure Bio-Type)".into(),
                pure_doctrine_effect: "Strictly Water-Bound Pelagic Mastery: Fish enter with +1 Water Shield, +1 Evade, +1 Range & +1 Swim MOVE; deploying a Fish breaches up to 2 adjacent Flat tiles into ShallowFord canals.".into(),
                primal_surge_name: "Hydro-Jet Surge".into(),
                primal_surge_cost: (1, 1),
                primal_surge_effect: "Instantly repositions a friendly unit 2 hexes along connected water or ford channels.".into(),
                affinity_biomes: vec!["DeepRiver".into(), "OpenOcean".into(), "FloodedBasin".into()],
            },
            OrganismFaction::Cnidarian => Self {
                bio_type,
                title: "Bio-Type: Cnidarian (Bio-Electric Siphonophores)".into(),
                hybrid_doctrine_name: "Cnidocyte Bio-Synapse".into(),
                hybrid_doctrine_effect: "Poison attacks apply 2 stacks instead of 1; Heavy Cnidarians project Node Radius 2.".into(),
                pure_doctrine_name: "Bioluminescent Overmind (Pure Bio-Type)".into(),
                pure_doctrine_effect: "All Cnidarians project Node Radius 1; electric shock attacks stun enemies; Poison ignores shields entirely.".into(),
                primal_surge_name: "Bio-Electric Discharge".into(),
                primal_surge_cost: (1, 1),
                primal_surge_effect: "Shocks all adjacent enemies, stripping 1 shield stack and disabling defender retaliation for 1 turn.".into(),
                affinity_biomes: vec!["DeepRiver".into(), "OpenOcean".into(), "CoastalReef".into()],
            },
            OrganismFaction::Amphibian => Self {
                bio_type,
                title: "Bio-Type: Amphibian (Torpor & Osmosis)".into(),
                hybrid_doctrine_name: "Torpor Regeneration".into(),
                hybrid_doctrine_effect: "Rest on Water/Jungle/RoughCover heals +1 STR and grants Stealth until next turn.".into(),
                pure_doctrine_name: "Metamorphic Osmosis (Pure Bio-Type)".into(),
                pure_doctrine_effect: "Amphibians automatically regenerate +1 STR every turn if not in combat; semi-aquatic everywhere; toxic skin damages melee attackers.".into(),
                primal_surge_name: "Secretory Slime Escape".into(),
                primal_surge_cost: (1, 1),
                primal_surge_effect: "Disengages from melee without triggering free strikes, leaping 2 hexes away to an unoccupied tile.".into(),
                affinity_biomes: vec!["Swamp".into(), "DeepRiver".into(), "Jungle".into()],
            },
            OrganismFaction::Human => Self {
                bio_type,
                title: "Bio-Type: Human Core Infrastructure".into(),
                hybrid_doctrine_name: "Autonomous Command".into(),
                hybrid_doctrine_effect: "Human installations and infantry form the backbone of all continental armies.".into(),
                pure_doctrine_name: "Human Core Mastery".into(),
                pure_doctrine_effect: "All human command centers project +1 Node range.".into(),
                primal_surge_name: "Emergency Scramble".into(),
                primal_surge_cost: (1, 0),
                primal_surge_effect: "Redeploy an infantry squad to home line.".into(),
                affinity_biomes: vec!["Bunker".into(), "Plains".into()],
            },
        }
    }
}

/// 250-Point Army Roster anchored by 3 Command Cards:
/// - 1 Faction Card (Continental Allegiance)
/// - 1-2 Bio-Type Cards (Organism Types, Pure or Hybrid)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeckList {
    pub deck_name: String,
    #[serde(default = "default_continent_origin")]
    pub faction: OriginAllegiance,
    pub primary_faction: OrganismFaction,
    pub secondary_faction: OrganismFaction,
    #[serde(default)]
    pub is_pure_biotype: bool,
    pub point_budget: u16,
    pub entries: Vec<DeckEntry>,
}

impl DeckList {
    pub fn is_pure_biotype(&self) -> bool {
        self.is_pure_biotype || self.primary_faction == self.secondary_faction
    }

    pub fn active_biotypes(&self) -> Vec<OrganismFaction> {
        if self.is_pure_biotype() {
            vec![self.primary_faction]
        } else {
            vec![self.primary_faction, self.secondary_faction]
        }
    }

    pub fn faction_card(&self) -> FactionCardDef {
        FactionCardDef::get(self.faction)
    }

    pub fn primary_biotype_card(&self) -> BioTypeCardDef {
        BioTypeCardDef::get(self.primary_faction)
    }

    pub fn secondary_biotype_card(&self) -> Option<BioTypeCardDef> {
        if self.is_pure_biotype() {
            None
        } else {
            Some(BioTypeCardDef::get(self.secondary_faction))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckValidationReport {
    pub is_legal: bool,
    pub total_cards: u16,
    pub points_used: u16,
    pub points_remaining: i32,
    pub max_actions_per_turn: u8,
    pub violations: Vec<String>,
}

impl DeckList {
    pub fn validate_against(&self, roster: &RosterManifest) -> DeckValidationReport {
        let lookup = roster.index_by_key();
        let mut violations = Vec::new();
        let mut points_used: u16 = 0;
        let mut total_cards: u16 = 0;
        let mut cmd_centers: u8 = 0;
        let mut supply_depots: u8 = 0;
        let mut commanders: u8 = 0;
        let mut control_nodes: u8 = 0;

        if self.primary_faction == OrganismFaction::Human
            || self.secondary_faction == OrganismFaction::Human
        {
            violations.push(
                "Primary and secondary slots are for non-Human bio-factions (Human core is always allowed)."
                    .to_string(),
            );
        }

        for entry in &self.entries {
            if entry.quantity == 0 {
                continue;
            }
            match lookup.get(entry.card_key.as_str()) {
                None => violations.push(format!("Unknown card_key '{}'", entry.card_key)),
                Some(card) => {
                    let qty_u16 = u16::from(entry.quantity);
                    total_cards += qty_u16;
                    points_used += card.deck_points * qty_u16;

                    match card.combat_class {
                        CombatClass::Installation => cmd_centers += entry.quantity,
                        CombatClass::SupplyDepot => supply_depots += entry.quantity,
                        CombatClass::Commander => commanders += entry.quantity,
                        CombatClass::ControlNode => control_nodes += entry.quantity,
                        CombatClass::TacticalDeployment => {
                            if entry.quantity > 2 {
                                violations.push(format!("Tactical Deployment '{}' exceeds max 2 copies per deck (has {})", card.card_key, entry.quantity));
                            }
                        }
                        _ => {}
                    }

                    let is_pure = self.is_pure_biotype();
                    let faction_ok = if is_pure {
                        card.faction == OrganismFaction::Human || card.faction == self.primary_faction
                    } else {
                        card.faction == OrganismFaction::Human
                            || card.faction == self.primary_faction
                            || card.faction == self.secondary_faction
                    };
                    if !faction_ok {
                        if is_pure {
                            violations.push(format!(
                                "Card '{}' belongs to {:?}, illegal in Pure {:?} Bio-Type deck (only Human + {:?} allowed)",
                                card.card_key, card.faction, self.primary_faction, self.primary_faction
                            ));
                        } else {
                            violations.push(format!(
                                "Card '{}' belongs to {:?}, outside allowed Human + ({:?}, {:?})",
                                card.card_key, card.faction, self.primary_faction, self.secondary_faction
                            ));
                        }
                    }
                }
            }
        }

        if points_used > self.point_budget {
            violations.push(format!(
                "Deck exceeds point budget: {} / {} pts",
                points_used, self.point_budget
            ));
        }
        if cmd_centers != 1 || supply_depots != 1 {
            violations.push(format!(
                "Deck must contain exactly 1 Command Center ({}) and 1 Supply Depot ({})",
                cmd_centers, supply_depots
            ));
        }
        if commanders > 1 {
            violations.push(format!("Deck exceeds max 1 Commander (found {})", commanders));
        }
        if control_nodes > 3 {
            violations.push(format!(
                "Deck exceeds max 3 static Control Nodes (found {})",
                control_nodes
            ));
        }

        DeckValidationReport {
            is_legal: violations.is_empty(),
            total_cards,
            points_used,
            points_remaining: i32::from(self.point_budget) - i32::from(points_used),
            max_actions_per_turn: roster.base_actions_per_turn + control_nodes.min(3),
            violations,
        }
    }
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("Required string field '{0}' is empty")]
    EmptyField(&'static str),
    #[error("Unknown Origin Allegiance '{0}'")]
    UnknownOrigin(String),
    #[error("Unknown Organism Faction '{0}'")]
    UnknownOrganism(String),
    #[error("Unknown Combat Class '{0}'")]
    UnknownCombatClass(String),
    #[error("Unknown Graveyard Queue '{0}'")]
    UnknownGraveyardQueue(String),
    #[error("Duplicate card_key '{0}' in Master Roster")]
    DuplicateCardKey(String),
    #[error("Card '{card_key}' has 0 Strength/HP")]
    ZeroStrength { card_key: String },
    #[error("Weighted point mismatch on '{card_key}': sheet={spreadsheet_points}, computed={computed_points}")]
    PointSumMismatch {
        card_key: String,
        spreadsheet_points: u16,
        computed_points: u16,
    },
    #[error("Node emitter '{card_key}' has node_emit_radius == 0")]
    InvalidNodeEmitter { card_key: String },
    #[error("Laser unit '{card_key}' has 0 ranged_distance")]
    LaserWithoutRange { card_key: String },
    #[error("Roster is missing required PC baseline entry: {0}")]
    MissingRequiredFacility(&'static str),
    #[error("Faction '{faction}' has only {count} cards (minimum 6 required for 2-faction balance)")]
    InsufficientFactionDepth { faction: String, count: usize },
}
