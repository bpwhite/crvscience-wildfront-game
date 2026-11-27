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

#![forbid(unsafe_code)]

//! Core deterministic rules, balanced unit/ability schemas, Sparse 64-Tree voxel hierarchy,
//! procedural hexagonal environments, scenarios, logistics graphs, and combat math
//! for *Wildfront Battlegrounds*.

pub mod abilities;
pub mod combat;
pub mod hex_map;
pub mod logistics;
pub mod mapgen;
pub mod scenario;
pub mod schema;
pub mod sqlite_db;
pub mod voxel;

pub use abilities::{
    AbilityCatalog, AbilityId, AbilitySpec, FactionPassiveSpec, TriggerMode,
};
pub use hex_map::{BattleHexMap, Biome, HexTile, Morphology};
pub use mapgen::{generate_procedural_map, MapGenConfig, SymmetryMode};
pub use scenario::{
    generate_scenario, GeneratedScenario, ScenarioId, ScenarioPlacement, VictoryCondition,
};
pub use schema::{
    BioTypeCardDef, CombatClass, DeckEntry, DeckList, DeckValidationReport,
    FactionCardDef, GraveyardFacility,
    OrganismFaction, OriginAllegiance, RosterManifest, UnitCardDef, ValidationError,
    WILDFRONT_THEME_2072,
};
pub use voxel::{
    compile_multi_asset_64tree, BakedVoxelAsset, GpuLeafBrick, GpuNode64,
    MultiAssetGpuBuffers, SparseBrick4,
};
