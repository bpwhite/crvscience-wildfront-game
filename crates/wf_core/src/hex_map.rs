//! Tactical axial hexagonal battlefield with 6 post-nuclear 2072 biomes,
//! stepped elevation plateaus, directional cliff edges, rivers, ravines,
//! radiation fallout zones, and pre-war bunker ruins.

use std::collections::HashMap;
use bevy_ecs::prelude::Resource;
use hexx::Hex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Biome {
    IrradiatedBadlands = 0,
    SavannahDustbowl = 1,
    AshwoodBasin = 2,
    OvergrownBioJungle = 3,
    SunkenAtoll = 4,
    RuinedBunkerCaldera = 5,
}

impl Biome {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::IrradiatedBadlands => "Irradiated Badlands",
            Self::SavannahDustbowl => "Savannah Dustbowl",
            Self::AshwoodBasin => "Ashwood Basin",
            Self::OvergrownBioJungle => "Overgrown Bio-Jungle",
            Self::SunkenAtoll => "Sunken Oceanic Atoll",
            Self::RuinedBunkerCaldera => "Ruined Bunker Caldera",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Morphology {
    Flat = 0,
    RoughCover = 1,
    MountainPeak = 2,
    RavineChasm = 3,
    DeepRiver = 4,
    OpenOcean = 5,
    RadiationFallout = 6,
    BunkerRuins = 7,
    ShallowFord = 8,
    ArmoredDam = 9,
}

impl Morphology {
    pub fn is_water(&self) -> bool {
        matches!(self, Self::DeepRiver | Self::OpenOcean)
    }

    pub fn grants_ranged_cover(&self) -> bool {
        matches!(self, Self::RoughCover | Self::BunkerRuins | Self::ArmoredDam)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HexTile {
    pub biome: Biome,
    pub morphology: Morphology,
    /// Stepped elevation plateau (`0..=3`). Adjacent `|Δ| >= 2` without a ramp forms a cliff.
    pub elevation: u8,
    /// Bitmask over the 6 `hexx` neighbor directions marking impassable ground cliffs.
    pub cliff_edges: u8,
    /// Scenario objective tag (`0` = none, `1` = Salvage Bunker, `2` = Control Spire / Relay).
    pub objective_tag: u8,
}

impl HexTile {
    pub fn has_cliff_edge(&self, edge_idx: usize) -> bool {
        edge_idx < 6 && (self.cliff_edges & (1 << edge_idx)) != 0
    }
}

#[derive(Debug, Clone, Default, Resource, Serialize, Deserialize)]
pub struct BattleHexMap {
    pub radius: u8,
    pub primary_biome: BiomeSnapshot,
    pub tiles: HashMap<Hex, HexTile>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BiomeSnapshot {
    #[default]
    Savannah,
    Badlands,
    Ashwood,
    BioJungle,
    Atoll,
    Caldera,
}

impl BattleHexMap {
    /// Checks whether a unit with given locomotion traits can step from `from` to adjacent `to`.
    /// `amphibious_tier`: `0` = Terrestrial, `1` = Semi-Aquatic, `2` = Pure Aquatic.
    pub fn can_traverse(
        &self,
        from: Hex,
        to: Hex,
        can_fly: bool,
        is_burrowed: bool,
        amphibious_tier: u8,
    ) -> bool {
        if from.unsigned_distance_to(to) != 1 {
            return false;
        }
        let (Some(src), Some(dst)) = (self.tiles.get(&from), self.tiles.get(&to)) else {
            return false;
        };

        // Flyers cross cliffs, ravines, and water freely; only towering MountainPeaks block flight.
        if can_fly {
            return dst.morphology != Morphology::MountainPeak;
        }

        // Pure Aquatic units (`amphibious_tier == 2`) stay in connected water or ShallowFord channels.
        if amphibious_tier == 2 {
            return matches!(
                dst.morphology,
                Morphology::DeepRiver | Morphology::OpenOcean | Morphology::ShallowFord | Morphology::ArmoredDam
            );
        }

        // Directional cliff barrier (`|Δelevation| >= 2` or explicit cliff edge) blocks ground movement.
        let neighbors = from.all_neighbors();
        if let Some(edge_idx) = neighbors.iter().position(|&n| n == to) {
            if src.has_cliff_edge(edge_idx) || src.elevation.abs_diff(dst.elevation) >= 2 {
                return false;
            }
        }

        match dst.morphology {
            Morphology::MountainPeak | Morphology::RavineChasm => false,
            Morphology::DeepRiver | Morphology::OpenOcean => {
                !is_burrowed && amphibious_tier >= 1
            }
            Morphology::Flat
            | Morphology::RoughCover
            | Morphology::RadiationFallout
            | Morphology::BunkerRuins
            | Morphology::ShallowFord
            | Morphology::ArmoredDam => true,
        }
    }
}
