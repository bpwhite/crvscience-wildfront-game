//! Offline asset pipeline for *Wildfront Battlegrounds*:
//! - `xlsx_compiler`: Compiles `Wildfront_PC_Master_Stats_and_DeckBuilder.xlsx` into validated `.ron` files.
//! - `vox_baker`: Compiles hierarchical MagicaVoxel `.vox` rigs into Sparse 64-Tree GPU brickmaps.
//! - `stl_exporter`: Exports watertight 2-manifold `.STL` miniatures with 45-degree micro-dice V-cradles.

pub mod stl_exporter;
pub mod vox_baker;
pub mod xlsx_compiler;
