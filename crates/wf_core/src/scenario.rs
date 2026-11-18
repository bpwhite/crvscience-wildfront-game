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

//! Procedural game scenario generator combining `mapgen` environments with
//! victory conditions, objective beacons, and balanced starting deployments.

use hexx::Hex;
use serde::{Deserialize, Serialize};
use crate::hex_map::{BattleHexMap, Biome, Morphology};
use crate::mapgen::{
    default_base_anchors, ensure_base_connectivity, generate_procedural_map, MapGenConfig,
    SymmetryMode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ScenarioId {
    EmergenceSkirmish = 1,
    BunkerBreakout = 2,
    BioRiverConvoy = 3,
    PelagicAtollAssault = 4,
    KingOfTheControlSpire = 5,
}

impl ScenarioId {
    pub const ALL: [Self; 5] = [
        Self::EmergenceSkirmish,
        Self::BunkerBreakout,
        Self::BioRiverConvoy,
        Self::PelagicAtollAssault,
        Self::KingOfTheControlSpire,
    ];

    pub fn from_index(idx: u8) -> Self {
        match idx {
            1 => Self::EmergenceSkirmish,
            2 => Self::BunkerBreakout,
            3 => Self::BioRiverConvoy,
            4 => Self::PelagicAtollAssault,
            _ => Self::KingOfTheControlSpire,
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::EmergenceSkirmish => "1. Emergence Skirmish (Savannah Dustbowl)",
            Self::BunkerBreakout => "2. Bunker Breakout (Ruined Bunker Caldera)",
            Self::BioRiverConvoy => "3. Bio-River Convoy (Overgrown Bio-Jungle)",
            Self::PelagicAtollAssault => "4. Pelagic Atoll Assault (Sunken Oceanic Atoll)",
            Self::KingOfTheControlSpire => "5. King of the Control Spire (Irradiated Badlands)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VictoryCondition {
    DestroyEnemyCommandCenter,
    HoldVaultTurns { target_turns: u8 },
    SalvageMunitionsOrRaze { target_munitions: u16 },
    ControlIslandRelays { required_relays: u8, target_turns: u8 },
    HoldCentralControlSpire { target_turns: u8 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioPlacement {
    pub team_id: u8,
    pub card_key: String,
    pub vox_slug: String,
    pub q: i32,
    pub r: i32,
    pub yaw_radians: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedScenario {
    pub scenario_id: ScenarioId,
    pub title: String,
    pub seed: u64,
    pub symmetry: SymmetryMode,
    pub victory: VictoryCondition,
    pub objective_hexes: Vec<(i32, i32)>,
    pub placements: Vec<ScenarioPlacement>,
    pub map: BattleHexMap,
}

/// Procedurally generates one of the 5 *Wildfront Battlegrounds* scenarios from any 64-bit `seed`.
pub fn generate_scenario(scenario_id: ScenarioId, seed: u64) -> GeneratedScenario {
    let (cfg, victory) = match scenario_id {
        ScenarioId::EmergenceSkirmish => (
            MapGenConfig::for_biome(seed, Biome::SavannahDustbowl),
            VictoryCondition::DestroyEnemyCommandCenter,
        ),
        ScenarioId::BunkerBreakout => (
            MapGenConfig::for_biome(seed, Biome::RuinedBunkerCaldera),
            VictoryCondition::HoldVaultTurns { target_turns: 12 },
        ),
        ScenarioId::BioRiverConvoy => (
            MapGenConfig::for_biome(seed, Biome::OvergrownBioJungle),
            VictoryCondition::SalvageMunitionsOrRaze { target_munitions: 18 },
        ),
        ScenarioId::PelagicAtollAssault => (
            MapGenConfig::for_biome(seed, Biome::SunkenAtoll),
            VictoryCondition::ControlIslandRelays {
                required_relays: 3,
                target_turns: 8,
            },
        ),
        ScenarioId::KingOfTheControlSpire => {
            let mut c = MapGenConfig::for_biome(seed, Biome::IrradiatedBadlands);
            c.radius = 6;
            (c, VictoryCondition::HoldCentralControlSpire { target_turns: 7 })
        }
    };

    let mut map = generate_procedural_map(&cfg);
    let (p1_hq, p2_hq) = default_base_anchors(cfg.radius, cfg.symmetry);
    let mut objective_hexes = Vec::new();

    // Stamp scenario-specific objective tiles onto the procedural map
    match scenario_id {
        ScenarioId::EmergenceSkirmish => {
            if let Some(t) = map.tiles.get_mut(&Hex::ZERO) {
                t.morphology = Morphology::ShallowFord;
                t.elevation = 0;
                t.objective_tag = 1;
                objective_hexes.push((0, 0));
            }
        }
        ScenarioId::BunkerBreakout => {
            if let Some(t) = map.tiles.get_mut(&Hex::ZERO) {
                t.elevation = 1;
                t.morphology = Morphology::BunkerRuins;
                t.objective_tag = 2;
                objective_hexes.push((0, 0));
            }
        }
        ScenarioId::BioRiverConvoy => {
            for oh in [Hex::new(0, -3), Hex::ZERO, Hex::new(0, 3)] {
                if let Some(t) = map.tiles.get_mut(&oh) {
                    t.elevation = 0;
                    t.morphology = Morphology::BunkerRuins;
                    t.objective_tag = 1;
                    objective_hexes.push((oh.x, oh.y));
                }
            }
        }
        ScenarioId::PelagicAtollAssault => {
            for oh in [Hex::new(-3, 0), Hex::new(3, 0), Hex::new(0, -3), Hex::new(0, 3), Hex::ZERO] {
                if let Some(t) = map.tiles.get_mut(&oh) {
                    t.elevation = 1;
                    t.morphology = Morphology::BunkerRuins;
                    t.objective_tag = 2;
                    objective_hexes.push((oh.x, oh.y));
                }
            }
        }
        ScenarioId::KingOfTheControlSpire => {
            for h in Hex::ZERO.range(1) {
                if let Some(t) = map.tiles.get_mut(&h) {
                    t.elevation = 1;
                    t.morphology = if h == Hex::ZERO {
                        Morphology::BunkerRuins
                    } else {
                        Morphology::Flat
                    };
                    if h == Hex::ZERO {
                        t.objective_tag = 2;
                    }
                }
            }
            objective_hexes.push((0, 0));
        }
    }

    map.recompute_cliff_masks();
    ensure_base_connectivity(&mut map, p1_hq, p2_hq, false);
    for &(ox, oy) in &objective_hexes {
        ensure_base_connectivity(&mut map, p1_hq, Hex::new(ox, oy), false);
    }

    // Pick valid passable neighbor slots around P1 and P2 Command Centers for starting units
    let p1_nbs: Vec<Hex> = p1_hq
        .all_neighbors()
        .into_iter()
        .filter(|h| map.tiles.contains_key(h))
        .collect();
    let p2_nbs: Vec<Hex> = p2_hq
        .all_neighbors()
        .into_iter()
        .filter(|h| map.tiles.contains_key(h))
        .collect();

    let mut placements = vec![
        ScenarioPlacement {
            team_id: 1,
            card_key: "Command Center (Australia)".into(),
            vox_slug: "command_center".into(),
            q: p1_hq.x,
            r: p1_hq.y,
            yaw_radians: 0.45,
        },
        ScenarioPlacement {
            team_id: 2,
            card_key: "Command Center (Africa)".into(),
            vox_slug: "command_center".into(),
            q: p2_hq.x,
            r: p2_hq.y,
            yaw_radians: -2.65,
        },
    ];

    // Scenario-themed starting showcase across all 25 custom 3D `.vox` rigs
    let (p1_roster, p2_roster): (&[(&str, &str)], &[(&str, &str)]) = match scenario_id {
        // Standard Constructed Match: ONLY Command Centers start on the board (2 base Actions on Turn 1;
        // Control Nodes, Depots, Commanders & Squads are deployed from each player's 5-card hand!).
        ScenarioId::EmergenceSkirmish => (&[], &[]),
        ScenarioId::BunkerBreakout => (
            &[
                ("Hospital", "hospital"),
                ("Factory", "factory"),
                ("Laser Turret", "radar_laser_turret"),
                ("Combat Medic", "human_medic"),
            ],
            &[
                ("Control Node", "control_node"),
                ("Deathstalker Scorpion", "arachnid_scorpion"),
                ("Bombardier Siege Beetle", "insect_siege_beetle"),
                ("Mantis Blade Stalker", "insect_mantis"),
            ],
        ),
        ScenarioId::BioRiverConvoy => (
            &[
                ("Supply Depot", "supply_depot"),
                ("Convoy", "supply_truck"),
                ("Spore-Pod Mortar", "plant_spore_mortar"),
                ("Venus Snapper", "plant_trap"),
            ],
            &[
                ("Control Node", "control_node"),
                ("Shellguard Tortoise", "reptile_shellguard"),
                ("Cane Toad Mortar", "amphibian_venom_frog"),
                ("Ragin' Caiman", "reptile_flamer_croc"),
            ],
        ),
        ScenarioId::PelagicAtollAssault => (
            &[
                ("Control Node", "control_node"),
                ("Jelly Zeppelin", "jelly_zeppelin"),
                ("Optic-Laser Shark", "fish_laser_shark"),
                ("Skyhunter Falcon", "bird_skyhunter"),
            ],
            &[
                ("Control Node", "control_node"),
                ("Mainframe Man-o'-War", "cnidarian_manowar"),
                ("Carrier Pigeon Dreadnought", "bird_skyhunter"),
                ("Armada Manta Ray", "fish_laser_shark"),
            ],
        ),
        ScenarioId::KingOfTheControlSpire => (
            &[
                ("Radar Tower", "radar_laser_turret"),
                ("Commander (North America)", "human_commander"),
                ("Drone", "drone"),
                ("Ngangkari", "human_medic"),
            ],
            &[
                ("Control Node", "control_node"),
                ("Battleclaw Komodo", "reptile_flamer_croc"),
                ("Skyhunter Falcon", "bird_skyhunter"),
                ("Deathstalker Scorpion", "arachnid_scorpion"),
            ],
        ),
    };

    for (slot_i, &(card_key, slug)) in p1_roster.iter().enumerate() {
        if let Some(&nb) = p1_nbs.get(slot_i) {
            placements.push(ScenarioPlacement {
                team_id: 1,
                card_key: card_key.into(),
                vox_slug: slug.into(),
                q: nb.x,
                r: nb.y,
                yaw_radians: -0.75,
            });
        }
    }
    for (slot_i, &(card_key, slug)) in p2_roster.iter().enumerate() {
        if let Some(&eb) = p2_nbs.get(slot_i) {
            placements.push(ScenarioPlacement {
                team_id: 2,
                card_key: card_key.into(),
                vox_slug: slug.into(),
                q: eb.x,
                r: eb.y,
                yaw_radians: 2.35,
            });
        }
    }

    if scenario_id == ScenarioId::KingOfTheControlSpire {
        placements.push(ScenarioPlacement {
            team_id: 0,
            card_key: "Control Node".into(),
            vox_slug: "control_node".into(),
            q: 0,
            r: 0,
            yaw_radians: 0.0,
        });
    }

    GeneratedScenario {
        scenario_id,
        title: scenario_id.title().to_string(),
        seed,
        symmetry: cfg.symmetry,
        victory,
        objective_hexes,
        placements,
        map,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapgen::has_traversal_path;

    #[test]
    fn bio_river_convoy_and_all_scenarios_reach_all_objectives_and_sync_cliffs() {
        let mut seeds = vec![0x2072_0814u64];
        for i in 1u64..20 {
            seeds.push(0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i));
        }

        for scenario_id in ScenarioId::ALL {
            for &seed in &seeds {
                let scen = generate_scenario(scenario_id, seed);
                let p1_hq = scen
                    .placements
                    .iter()
                    .find(|p| p.team_id == 1 && p.card_key.contains("Command Center"))
                    .map(|p| Hex::new(p.q, p.r))
                    .expect("Team 1 Command Center must exist");
                let p2_hq = scen
                    .placements
                    .iter()
                    .find(|p| p.team_id == 2 && p.card_key.contains("Command Center"))
                    .map(|p| Hex::new(p.q, p.r))
                    .expect("Team 2 Command Center must exist");

                assert!(
                    has_traversal_path(&scen.map, p1_hq, p2_hq, 0),
                    "{scenario_id:?} seed {seed:#x}: P1 HQ {p1_hq:?} cannot reach P2 HQ {p2_hq:?} on ground"
                );
                for &(ox, oy) in &scen.objective_hexes {
                    let oh = Hex::new(ox, oy);
                    assert!(
                        has_traversal_path(&scen.map, p1_hq, oh, 0),
                        "{scenario_id:?} seed {seed:#x}: objective {oh:?} unreachable on ground from {p1_hq:?}"
                    );
                }

                // Verify cliff_edges mask matches current tile elevations after objective stamping
                let mut recomputed = scen.map.clone();
                recomputed.recompute_cliff_masks();
                for (h, tile) in &scen.map.tiles {
                    assert_eq!(
                        tile.cliff_edges,
                        recomputed.tiles[h].cliff_edges,
                        "{scenario_id:?} seed {seed:#x}: stale cliff_edges at {h:?}"
                    );
                }
            }
        }
    }
}
