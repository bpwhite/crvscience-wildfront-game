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
