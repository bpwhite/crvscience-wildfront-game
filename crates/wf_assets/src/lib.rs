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

//! Offline asset pipeline for *Wildfront Battlegrounds*:
//! - `xlsx_compiler`: Compiles `Wildfront_PC_Master_Stats_and_DeckBuilder.xlsx` into validated `.ron` files.
//! - `vox_baker`: Compiles hierarchical MagicaVoxel `.vox` rigs into Sparse 64-Tree GPU brickmaps.
//! - `stl_exporter`: Exports watertight 2-manifold `.STL` miniatures with 45-degree micro-dice V-cradles.

pub mod stl_exporter;
pub mod vox_baker;
pub mod xlsx_compiler;
