# Wildfront Battlegrounds (`v3.4.1`) — Code Review & Local Build Guide

## 1. Local Toolchain & System Prerequisites

- **Rust Toolchain**: Stable Rust **`1.87+`** (`rustup update stable` — required by `wgpu 30.0`).
- **Graphics API**: Vulkan (`Windows` / `Linux`), DirectX 12 (`Windows`), or Metal (`macOS`).
- **Linux System Headers** (Ubuntu / Debian / Pop!_OS only — Windows & macOS require zero system packages):
  ```bash
  sudo apt-get update && sudo apt-get install -y pkg-config libasound2-dev libudev-dev     libx11-dev libxcursor-dev libxi-dev libxrandr-dev libxkbcommon-dev libwayland-dev libvulkan-dev
  ```

## 2. One-Command Build, Test & Launch

- **Linux / macOS**:
  ```bash
  ./scripts/verify_and_run.sh
  ```
- **Windows (PowerShell)**:
  ```powershell
  .\scripts\verify_and_run.ps1
  ```

Or step-by-step via Cargo:

```bash
# 1. Run all unit & integration tests across all 4 crates
cargo test --workspace -- --nocapture

# 2. Lint workspace with Clippy
cargo clippy --workspace --all-targets

# 3. Run a headless 102-match symmetric tournament benchmark (no GPU/window needed)
cargo run -p wf_sim --release -- tournament --ai veteran --export-ron reports/tournament_102.ron

# 4. Batch-export all 170 watertight 2-manifold 3D-printable .STL models (45mm hexes + V-cradles)
cargo run -p wf_assets --release -- --export-stl

# 5. Launch the native wgpu + egui 3D Voxel Client
cargo run -p wf_client --release
```

## 3. Multi-Crate Architecture Map for Code Review

| Crate / Path | Responsibility | Key Files to Review |
| :--- | :--- | :--- |
| **`crates/wf_core`** | Deterministic game rules, 152-card schema, SQLite persistence, D6 combat, BFS supply/node logistics, procedural hex mapgen, and Sparse 64-Tree packing | `schema.rs`, `combat.rs`, `logistics.rs`, `mapgen.rs`, `scenario.rs`, `voxel.rs`, `sqlite_db.rs` |
| **`crates/wf_assets`** | Offline/CLI asset pipeline: `calamine` Excel-to-RON compiler, `dot_vox` Sparse 64-Tree baker, and watertight 2-manifold `.STL` V-cradle carver | `xlsx_compiler.rs`, `vox_baker.rs`, `stl_exporter.rs`, `tests/asset_pipeline_test.rs` |
| **`crates/wf_sim`** | Zero-GPU headless match engine, 64-bit FNV-1a state hashing, `.ron` action replay verifier, 2-ply minimax AI planner, and 102-match tournament CLI | `sim_engine.rs`, `ai_planner.rs`, `tournament.rs`, `tests/sim_and_ai_test.rs` |
| **`crates/wf_client`** | Native `winit 0.30` + `wgpu 30.0` + `egui 0.37` + `kira 0.12` desktop client, TCP 1v1 lockstep netcode, and interactive in-game studios (`F1`–`F4`, `F8`, `B`, `R`, `L`) | `shaders/raymarch_64tree.wgsl`, `voxel_pipeline.rs`, `game_session.rs`, `map_editor_ui.rs`, `replay_ui.rs`, `stl_export_ui.rs`, `net_lockstep.rs` |

## 4. Key Engineering Invariants Verified by `cargo test --workspace`

1. **100% 1-to-1 Asset Parity**: All `152` cards in `data/Wildfront_PC_Master_Stats_and_DeckBuilder.xlsx`, `assets/compiled/master_roster.ron`, and `assets/compiled/wildfront_roster.db` map to `152` distinct `.vox` and `_vox.ron` models (`+ 12` modular `45.0 mm` terrain hexes `+ 6` scatter props = `170` total models).
2. **Watertight 2-Manifold `.STL` Geometry**: Half-edge topology audit (`boundary_edges == 0`, `non_manifold_edges == 0`) and minimum `>= 2.00 mm` (`2.80 mm` default) center septum wall on dual micro-dice V-cradles.
3. **Anti-Stalemate & Initiative Rules**: Turn-1 strict `2` Base Actions for both players (`P1`: `5` Munitions / `5` cards; `P2`: `8` Munitions / `6` cards), non-refreshing `2-ARM` static building plating, `1 Munition/tick` Hospital/Factory bay upkeep + clone cap, and Round-30+ Orbital Fallout Storm.
4. **WGSL Uniform Barrier Discipline**: `raymarch_64tree.wgsl` executes cooperative `8x8` workgroup ray-cone tile culling (`wg_tile_insts`) and `workgroupBarrier()` strictly in uniform control flow prior to out-of-bounds pixel early-exit.
