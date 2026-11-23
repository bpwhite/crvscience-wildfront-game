# WildFront BattleGrounds — User Guide (`v3.4.1`)

**WildFront BattleGrounds** is a native Rust hexagonal ecological tactics wargame with GPU Sparse 64-Tree voxel raymarching (`wgpu`), spatialized 3D audio (`kira`), watertight tabletop `.STL` export, 250-point two-faction deck building, 1v1 TCP lockstep multiplayer, and a deterministic headless AI simulator (`wf_sim`).

*License: `GPL-3.0-or-later` — CRV Science Network*

---

## 1. Launching the Game & CLI Options

### Graphical Client (`wf_client`)
From the repository root:

```bash
cargo run -p wf_client --release
```

Or use the one-command verify + build + launch scripts:
- **Linux / macOS**: `./scripts/verify_and_run.sh`
- **Windows PowerShell**: `.\scripts\verify_and_run.ps1`

Supported `wf_client` command-line flags:

| Flag | Description |
|:-----|:------------|
| `-h`, `--help` | Print CLI usage summary and exit |
| `-V`, `--version` | Print `wf_client` version (`3.4.1`) and exit |
| `--screenshot <PATH>` | Render a headless `1600x1000` `wgpu` PNG and exit |
| `--screenshot-view <1\|2\|3>` | View preset for `--screenshot`: `1` = 3D Battlefield Arena, `2` = 250-Pt Deck Builder, `3` = 3D Hex Map Studio |

---

## 2. Controls & Hotkeys

### Camera & Battlefield Selection
- **Left Click**: Select hex/unit, deploy selected hand card, move, attack, or trigger active ability; when **3D Hex Map Studio (`F3`)** brush paint is enabled, Left Click paints the hovered hex.
- **Right Drag / Left Drag** (`>= 6 px`): Orbit the 3D camera around the hex battlefield.
- **Mouse Wheel**: Zoom smoothly (`3.2`–`68.0` world distance) from strategic theater overview down to close voxel detail.
- **`WASD` / Arrow Keys**: Pan camera target across the map (automatically clamped to the active board's spatial bounds).
- **`Home` / `C`**: Reset camera target, yaw, pitch, and zoom distance back to the default tactical overview.

### Turn Flow & Confirmation Guard
- **`Space`**:
  - In active play (when no modal/curtain is open): requests End Turn. If you still have `Actions > 0`, an **End Turn Confirmation** prompt prevents accidental turn skips (press `Enter` or click **Confirm End Turn** to proceed, or `Esc` / **Keep Playing** to cancel). When `Actions == 0`, `Space` ends the turn immediately.
  - In **1v1 Local Hotseat** mode when the privacy curtain is raised (and no other modal is open): reveals the incoming player's view.
  - In **Combat Dice Simulator (`R`)**: triggers a simulated 3D D6 roll.
- Gameplay hotkeys (`1`–`5`, `N`, `L`, `Tab`, `H`, `P`, `F`, `M`, `F5`, `F9`, `Space`) are automatically locked whenever any modal, studio window, or hotseat privacy curtain is open. Camera panning (`WASD`/Arrows/`Home`/`C`) stays active while **3D Hex Map Studio (`F3`)** is docked open so you can inspect and sculpt across the board, and locks under full-screen/dialog modals.

### Complete Hotkey Reference

| Key | Action |
|:----|:-------|
| `Esc` | Dismiss top active dialog / cancel active command mode / toggle **2072 Main Menu** |
| `F1` | Toggle **2072 Field Manual & Codex** (`Logistics`, `D6 Combat & Stealth`, `10 Faction Doctrines`, `Hotkeys & TCP 1v1`) |
| `B` | Toggle **250-Point Two-Faction Deck Builder** |
| `F2` | Toggle **Unit & Model SQLite Database Manager** (focuses currently selected unit when opened) |
| `F3` | Toggle **3D Hex Map & Scenario Studio** |
| `F4` | Toggle **Replay & Telemetry Theater** |
| `F8` | Toggle **3D Print `.STL` Studio** (focuses currently selected unit when opened) |
| `R` | Toggle **Combat Dice Simulator & Round Logistics Hub** |
| `L` | Cycle 4 GPU Atmosphere presets (`Clear Daylight`, `Dusk Tactical`, `Orbital Fallout Storm`, `Night Bio-Luminescence`) |
| `Tab` | Toggle GPU Supply (Amber) & Psychic Control Node (Cyan) hex aura overlays |
| `H` | Toggle floating 3D world-space Unit HP / Armor / Munition / Status plates |
| `P` | Toggle 3D Voxel Terrain Props (mountain crags, bunker ruins, fallout crystals, bridges, cover & dams) |
| `F` | Toggle Fog-of-War (`Sight`, `MountainPeak` line-of-sight, `Radar Range` true-sight, and `Stealth`) |
| `M` | Switch live between **Solo vs AI** and **1v1 Local Hotseat** mode (with Turn-Handoff privacy curtain) |
| `F5` / `F9` | Quicksave / Quickload deterministic match state (`saves/quicksave.ron`) |
| `1`–`5` | Switch among the 5 procedural scenarios (`Emergence Skirmish`, `Bunker Breakout`, `Bio-River Convoy`, `Pelagic Atoll Assault`, `King of the Control Spire`) |
| `N` | Roll a fresh 64-bit procedural seed on the current scenario |

---

## 3. Tactical HUD & Top Command Bar

The top bar adapts cleanly to 1280px+ viewports:
- **`🏠 Menu`**: Opens the 2072 Main Menu (`Esc`) for Solo AI, 1v1 Hotseat, 1v1 TCP Lockstep, scenario selection, and seed management.
- **`📖 Codex (F1)`**: Opens the searchable 4-tab Field Manual covering core rules, combat formulas, all 10 organism faction doctrines, and multiplayer setup.
- **`🛠 Studios ▾`**: Compact dropdown collecting all five creative & diagnostic studios:
  - **`🖨 3D Print .STL Studio (F8)`**
  - **`🗄 Unit & Model Database (F2)`**
  - **`🗺 3D Hex Map Studio (F3)`**
  - **`🎬 Replay & Telemetry (F4)`**
  - **`🎲 Combat Dice Simulator (R)`**
- **`🌤 Sky (L)`**: Cycles the 4 volumetric GPU lighting presets.
- **Turn & Economy Readout**: Hovering over **`⚡ Actions`** displays the exact rule (`2 Base + up to +3 from active static Control Nodes, max 5/turn`). Hovering over **`📦 Munitions`** explains Home Base replenishment (`+5/turn`, plus `+1` every 3rd turn for `Africa`/`Asia`), supply-chain sharing vs. carried `Encumbrance` ammo, and `0`-cost `⚡LSR` lasers.
- **Audio Controls & `⏭ End Turn (Space)`**: Mute toggle, dB gain slider for `kira` spatial SFX, and the End Turn button (with `actions_remaining > 0` safety confirmation).

---

## 4. Core Tactical Rules & Gameplay Loop

1. **Continental Allegiance & Decks (`B`)**:
   - Build a **250-point** deck combining always-unlocked **`Human` Core Infrastructure** with either **1 Pure Bio-Faction** (`Mammal`, `Reptile`, `Plant`, `Bird`, `Arachnid`, `Insect`, `Fish`, `Cnidarian`, `Amphibian`) or **2 Hybrid Bio-Factions**.
   - Select one of 6 Continental Allegiances (`Australia`, `Africa`, `Asia`, `Europe`, `North America`, `South America`), save/load `.ron` decks (`assets/compiled/custom_user_deck.ron`), inspect Bio (`Hospital`) vs. Mech (`Factory`) graveyard counts, or load built-in presets before clicking **`⚔ Launch Match With This Deck`**.
2. **Turn-1 Economy**:
   - **Player 1** begins with **5 Munitions**, **5 hand cards**, and **2 Base Actions**.
   - **Player 2** receives a first-turn asymmetry equalizer: **8 Munitions**, **6 hand cards**, and **2 Base Actions**.
3. **Home Supply & Psychic Control Nodes (`Tab`)**:
   - Your **Command Center** (`10 STR`, `2` non-refreshing structural Armor) generates **+5 Munitions/turn**. Any contiguous chain of friendly buildings touching your Command Center projects your **Home Supply** aura (Amber). Supplied units draw ranged/ability ammo from Home Base; unsupplied units spend carried `Encumbrance` ammo.
   - Engineered bio-units must stay within a friendly **Psychic Control Node** leash (Cyan — projected by static Control Nodes, Commanders, Recon Drones, and Cnidarian Synapse units), or they become **`⚠ NO NODE` (Confused)** until re-linked. Static Control Nodes also grant `+1 Action/turn` (max `5/turn`).
4. **Combat, Armor & Statuses**:
   - Melee attacks (`Range 1`) hit automatically (cannot target flyers from ground).
   - Ranged hit threshold on `1D6` scales with hex distance, defender effective evade, cover, target lock, adjacent Command aura, and Mammal Pack Tactics (clamped `2+`..`6+`; `6` always hits, `1` always misses). `Blind` and `Rooted` penalties reduce defender effective evade dynamically without altering printed base evade.
   - **Strength == Health (`❤ STR`)**: Taking damage lowers a unit's remaining `STR`, capping its maximum damage die roll accordingly. Armor absorbs incoming damage before `STR` (and refreshes on mobile units at turn start, never on static buildings).
5. **Victory & Sudden Death**:
   - Destroy the enemy **Command Center HQ** or hold the decisive victory condition (e.g., **Control Spire** dominance). Reaching **Round 30+** triggers **Orbital Fallout Storm** (`-1 HQ STR/round`, escalating to `-2` at **Round 36+**).

---

## 5. Creative, Diagnostic & 3D-Printing Studios

- **`.STL` Tabletop Print Studio (`F8`)**: Export watertight 2-manifold binary `.STL` miniatures scaled `1:1` for `45.0 mm` tabletop hexes with support-free `45°` recessed micro-dice V-cradles (`7 mm` D6 cradles, `10 mm` D10 cradles, or paired cradles, minimum `2.80 mm` center wall; hard safety floor `2.00 mm`). Supports single-unit export or full-roster batch export.
- **SQLite Database Manager (`F2`)**: Inspect and edit all `152` unit/deployment cards and `170` compiled voxel rigs backed by `assets/compiled/wildfront_roster.db`.
- **3D Hex Map & Scenario Studio (`F3`)**: Paint biome/morphology/elevation directly on the 3D hex board using single, `180°` Rotational, or Axial Mirror symmetry brushes, validate HQ BFS path connectivity, and export/import `.ron` scenarios.
- **Replay Scrubber & EHP Telemetry Theater (`F4`)**: Scrub turn-by-turn keyframes with live Effective HP (EHP) balance charts, branch into a live "What-If" playable match from any historical turn, or export/import `.ron` timelines.
- **Combat Dice Simulator (`R`)**: Inspect exact live hit thresholds, damage brackets, and round-level logistics between the currently selected attacker and target hex, or roll animated `1D6`/`2D6` previews (`Space`).

---

## 6. Multiplayer Modes

- **1v1 Local Hotseat (`M`)**: Pass-and-play on one machine. Between turns, when Fog-of-War is enabled, a **Turn-Handoff Privacy Curtain** conceals the board and hand until the incoming player presses `Space` or clicks Reveal.
- **1v1 TCP Lockstep (`Esc` Main Menu)**:
  1. Launch two instances (`cargo run -p wf_client --release`).
  2. **Host (Team 1)**: Choose Scenario & Deck in the Main Menu, then click **`📡 Host 1v1 TCP`** (binds `0.0.0.0:20720` by default).
  3. **Client (Team 2)**: Optionally customize Player 2's deck, enter `127.0.0.1:20720` (same PC) or the Host's LAN IP, and click **`🔌 Join 1v1 TCP`**.
  - Both peers synchronize the 64-bit scenario seed, custom decks, and `SplitMix64` RNG lockstep commands while keeping per-window Fog-of-War and hand privacy intact.

---

## 7. Headless Simulator (`wf_sim`) & Asset Compiler (`wf_assets`) CLI

### Headless Simulation, AI & Tournament CLI (`wf_sim`)

Simulate a single match and save a deterministic `.ron` replay:
```bash
cargo run -p wf_sim --release -- match \
  --p1-origin Australia --p1-primary Mammal --p1-secondary Mammal \
  --p2-origin Africa --p2-primary Fish --p2-secondary Fish \
  --scenario 4 --seed 544344084 --ai grandmaster \
  --save-replay replays/sample.ron
```

- `--ai` accepts `recruit` (or `easy`), `veteran` (or `normal` — exact 1-ply Expected Value focus), or `grandmaster` (default — 2-ply alpha-beta beam search).

Verify every FNV-1a turn state hash of a saved replay:
```bash
cargo run -p wf_sim --release -- verify-replay replays/sample.ron
```

Run the full 102-match symmetric Pure Bio-Type & Continental Round-Robin Tournament:
```bash
cargo run -p wf_sim --release -- tournament --ai veteran --max-rounds 45 --export-ron reports/tournament_102.ron
```

### Asset & `.STL` Batch Compiler CLI (`wf_assets`)

Recompile the Excel master stat sheet, `.vox` Sparse 64-Tree brickmaps, and optional printable `.STL` meshes:
```bash
cargo run -p wf_assets --release -- \
  --xlsx data/Wildfront_PC_Master_Stats_and_DeckBuilder.xlsx \
  --vox-dir assets/vox \
  --out-dir assets/compiled \
  --seed 20720814 \
  --export-stl --hex-mm 45.0 --center-wall-mm 2.80
```
