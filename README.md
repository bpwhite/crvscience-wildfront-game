# Wildfront Battlegrounds — Complete Native Rust Voxel, Hex, Deck-Builder & Spatial Audio Engine (v5)

> *"The year is 2072. Only six continents remain. The earth has been wracked by global warming, drought, and radiation following years of nuclear war..."*

## Complete v5 Feature Set
1. **Interactive `egui 0.37` 250-Point Two-Faction Deck Builder (`crates/wf_client/src/deck_builder_ui.rs`)**:
   - Press **`B`** or click **`🃏 Deck Builder (B)`** in the Top Command Bar to open the full-screen Deck Builder.
   - Choose any 2 non-Human Bio-Factions (`Mammal`, `Reptile`, `Plant`, `Bird`, `Arachnid`, `Insect`, `Fish`, `Cnidarian`, `Amphibian`) alongside always-unlocked `Human` Core Infrastructure.
   - Search & filter all **133 balanced cards**, inspect Bio (`Hospital`) & Mech (`Factory`) Graveyard synergy counts, load 4 built-in tournament presets (`Plant+Cnidarian`, `Mammal+Reptile`, `Fish+Amphibian`, `Bird+Arachnid`), save/load `.ron` custom decks (`assets/compiled/custom_user_deck.ron`), and click **`⚔ Launch Match With This Deck`**!
2. **`kira 0.12.6` 3D Spatial Combat Audio (`crates/wf_client/src/audio.rs` + `assets/sfx/*.wav`)**:
   - Includes 8 synthesized 44.1 kHz 16-bit PCM `.wav` sound effects (`laser_pulse.wav`, `ballistic_volley.wav`, `melee_clash.wav`, `voxel_explosion.wav`, `orbital_deploy.wav`, `bio_heal_chime.wav`, `venom_snare.wav`, `ui_card_turn.wav`).
   - Projects world-space hex combat & deployment events into camera-view space via `SpatialTrackBuilder` for stereo panning & distance rolloff, with a Top Bar mute toggle & dB volume slider (and graceful headless fallback).
3. **3D Stepped-Hex Ray-Picking, Playable Turn/AI Loop & Volumetric GPU Voxel/VFX Raymarching (`raymarch_64tree.wgsl`)**:
   - 8 Sparse 64-Tree `.vox` rigs, 6 procedural 2072 biomes, 5 seeded scenarios (`1`–`5`, `N`), laser beams, tracers, shockwaves, and 3D bouncing micro-voxel shrapnel debris.

## Run Locally
```bash
cargo run -p wf_client --release
```


# Wildfront Battlegrounds — 25 Custom `.vox` & Sparse 64-Tree Rig Catalog

Total sculpted solid voxels: **336,891** across **22,619** sparse `4×4×4` GPU leaf bricks.

| # | Slug | Title | Faction / Role | Grid (`X×Y×Z`) | Solid Voxels | `4×4×4` Bricks |
|---:|:-----|:------|:---------------|:--------------:|-------------:|---------------:|
| 1 | `command_center` | Command Center HQ | Human Installation | `88×88×64` | 12,727 | 1,036 |
| 2 | `control_node` | Psychic Control Node | Human Node | `64×64×88` | 9,832 | 774 |
| 3 | `supply_depot` | Supply Depot | Human Logistics | `64×64×44` | 10,307 | 665 |
| 4 | `hospital` | Bio-Vat Hospital | Human Graveyard | `76×76×68` | 12,494 | 969 |
| 5 | `factory` | Mech Repair Factory | Human Graveyard | `80×80×74` | 24,657 | 1,606 |
| 6 | `radar_laser_turret` | Radar / Laser Turret | Human Defense | `64×64×76` | 9,463 | 731 |
| 7 | `drone` | Recon Node Drone | Human Air | `56×56×36` | 2,772 | 286 |
| 8 | `human_commander` | Continental Commander | Human Commander | `72×72×68` | 9,026 | 642 |
| 9 | `human_medic` | Field Medic Squad | Human Medic | `76×76×58` | 10,342 | 542 |
| 10 | `supply_truck` | 6x6 Supply Convoy | Human Support | `84×84×54` | 20,543 | 1,145 |
| 11 | `combat_koala_squad` | Combat Koala Squad | Mammal Infantry | `96×96×56` | 16,274 | 899 |
| 12 | `mecharoo` | Mecharoo Walker | Mammal Mech Heavy | `96×96×116` | 22,171 | 1,559 |
| 13 | `mammal_battlephant` | Cyber Battlephant | Mammal War-Beast | `92×92×86` | 22,167 | 1,434 |
| 14 | `reptile_flamer_croc` | Flamer Croc / Komodo | Reptile Assault | `92×92×56` | 17,154 | 1,011 |
| 15 | `reptile_shellguard` | Shellguard Tortoise | Reptile Bastion | `84×84×66` | 15,946 | 1,074 |
| 16 | `plant_trap` | Venus Snapper Trap | Plant Trap | `52×52×40` | 5,638 | 411 |
| 17 | `plant_spore_mortar` | Spore-Pod Treant | Plant Artillery | `80×80×78` | 14,811 | 1,054 |
| 18 | `bird_skyhunter` | Skyhunter Falcon | Bird Air Strike | `88×88×68` | 8,456 | 626 |
| 19 | `arachnid_scorpion` | Deathstalker Scorpion | Arachnid Heavy | `88×88×72` | 15,635 | 974 |
| 20 | `insect_mantis` | Mantis Blade Stalker | Insect Striker | `84×84×70` | 15,295 | 986 |
| 21 | `insect_siege_beetle` | Bombardier Beetle | Insect Siege | `88×88×64` | 16,921 | 1,053 |
| 22 | `fish_laser_shark` | Optic-Laser Shark | Fish Pelagic | `92×92×64` | 12,471 | 775 |
| 23 | `jelly_zeppelin` | Jelly Zeppelin | Cnidarian Airship | `56×56×64` | 6,016 | 553 |
| 24 | `cnidarian_manowar` | Mainframe Man-o'-War | Cnidarian Synapse | `76×76×74` | 12,312 | 790 |
| 25 | `amphibian_venom_frog` | Venom-Mortar Frog | Amphibian Artillery | `80×80×62` | 13,461 | 1,024 |

## Playtest Controls & Hotkeys (v0.7.0)

- **Left Click**: Select hex / unit, deploy selected hand card, move, attack, or cast ability
- **Right Drag / Left Drag**: Orbit 3D camera around the hex battlefield
- **Mouse Wheel**: Zoom smoothly from strategic theater LOD down to individual voxels
- **`WASD` / Arrow Keys**: Pan camera target across the map
- **`Space`**: End Player Turn & execute full Team 2 AI turn (deck draw, orbital deployment, healing, attacks & movement)
- **`B`**: Open / close the 250-Point Two-Faction Deck Builder
- **`Tab`**: Toggle GPU Supply (Amber) & Psychic Control Node (Cyan) hex aura overlays
- **`H`**: Toggle floating 3D world-space Unit HP / Armor / Munition / Status bars
- **`P`**: Toggle 3D Voxel Terrain Props (mountain crags, bunker ruins, fallout crystals, ford bridges, cover & dams)
- **`F`**: Toggle Fog-of-War visibility masking (Sight, MountainPeak LoS, `Radar Range` True-Sight & `Stealth` ambush)
- **`M`**: Switch live between Solo `vs AI` and `1v1 Local Hotseat` mode (with Turn-Handoff Privacy Curtain)
- **`F5` / `F9`**: Quicksave / Quickload deterministic match state (`saves/quicksave.ron`)
- **`1`–`5` / `N`**: Switch among the 5 procedural scenarios or roll a new map seed

## v1.0.0 Complete Edition: Main Menu, Field Manual Codex (`F1`) & 1v1 TCP Lockstep

- **2072 Main Menu (`Esc` / `🏠 Menu`)**: Live orbiting 3D Sparse 64-Tree hex background, 2072 Emergence lore, Scenario/Seed previewer, Solo vs AI, 1v1 Local Hotseat, and 1v1 TCP Lockstep Host/Join.
- **Field Manual & Hotkey Codex (`F1` / `📖 Codex`)**: 4-tab in-game manual covering Core Logistics, D6 Combat & Stealth/Radar math, all 10 Faction Doctrines, and multiplayer setup.
- **1v1 TCP Lockstep Multiplayer**: Launch two instances on one PC (`127.0.0.1:20720`) or across LAN (`0.0.0.0:20720`) with deterministic `SplitMix64` RNG lockstep and private per-window Fog-of-War.

## v1.3.0 Automated Test Suite & Cross-Platform Release CI

- **Run All 19 Unit & Integration Tests**: `cargo test --workspace`
  - Covers 133-card roster & 25-rig slug mapping, 250-pt two-faction deck validation, Supply Chain BFS & severance, Psychic Control Node leash, D6 combat & Strength-Invariance damage, Hospital/Factory Graveyard queues, Excel & `.vox` RIFF asset compilation, Fog-of-War (`Radar Range` vs `Stealth` & `MountainPeak` LoS), `.ron` Quicksave/Quickload, deterministic Host/Client lockstep sync, and live `127.0.0.1` TCP socket framing.
- **One-Command Local Test + Build + Launch**: `./scripts/verify_and_run.sh` (Linux/macOS) or `.\scripts\verify_and_run.ps1` (Windows PowerShell).
- **GitHub Actions Cross-Platform Matrix (`.github/workflows/ci_and_release.yml`)**: Automatically tests and packages portable `Windows`, `macOS`, and `Linux` release archives on every push/PR and publishes GitHub Releases on `v*` tags.


## v1.3.0 — High-Density Plinth-Free Micro-Voxel Overhaul & Compact Hex Scale
- **31 Plinth-Free `104³–126³` Micro-Voxel Rigs**: **724,125** solid surface voxels packed into **58,007** Sparse 64-Tree `4×4×4` leaf bricks with 5-stop curvature/crevice shading ramps.
- **Compact Tactical In-Hex Proportions (`scene.rs`)**: Mobile units occupy **31%–53%** of a `4.16m` hex width at `1.38–1.75 cm/voxel` resolution with zero flat dirt coasters.
- **Real-Time 4-Tap GPU Voxel Crevice Ambient Occlusion (`raymarch_64tree.wgsl`)**: 440-step DDA traversal + tangent-plane crevice AO.


## v1.3.0 — Towering Staunton Chess-Piece Proportions & 72%–79% Hex Bases
- **31 Towering `136×136×220` to `148×148×248` Chess-Piece Rigs**: **1,977,131** solid surface voxels across **158,485** Sparse 64-Tree `4×4×4` leaf bricks.
- **72%–79% Hex-Width Turned Bases & `1.62×–1.90×` Vertical Silhouettes**: Every unit commands 72%–79% of its hex tile on a beveled Staunton pedestal while rising 5.1m–5.8m vertically.


## v3.4.1 Engine Expansion Highlights

- **`crates/wf_sim` Headless Engine, 2-Ply Alpha-Beta AI & Tournament CLI**:
  - `cargo run -p wf_sim --release -- match --p1-primary Mammal --p2-primary Fish --scenario 4 --ai grandmaster --save-replay replays/sample.ron`
  - `cargo run -p wf_sim --release -- verify-replay replays/sample.ron`
  - `cargo run -p wf_sim --release -- tournament --ai veteran --export-ron reports/tournament_102.ron`
- **GPU Compute Raymarcher (`raymarch_64tree.wgsl`)**:
  - Cooperative `8x8` workgroup ray-cone tile instance culling (`wg_tile_insts`), 8-tap voxel crevice ambient occlusion, dual-wave animated water normals + sun specular glint, and 4 dynamic atmosphere modes (`L`: Clear Daylight, Dusk Tactical, Orbital Fallout Storm, Night Bio-Luminescence).
- **In-Game Studios (`wf_client`)**:
  - **`F3` Hex Map & Scenario Studio**: Live 3D viewport terrain painting with 180-degree Rotational and Axial Mirror symmetry brushes plus `.ron` scenario export/import.
  - **`F4` Replay Scrubber & Battle Telemetry Theater**: Turn-by-turn keyframe scrubbing, tactical branching ("What-If" play from any turn), live EHP curves, and `.ron` timeline export/import.
