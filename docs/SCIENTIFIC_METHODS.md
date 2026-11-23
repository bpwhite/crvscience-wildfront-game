# WildFront BattleGrounds — Scientific & Algorithmic Methods (`v3.4.1`)

**License:** `GPL-3.0-or-later` (`GNU General Public License v3.0 or later`) — CRV Science Network (`Bryan P. White`).  
**Companion Preprint (`APP-PREPRINT-12`):** [10.5281/zenodo.24222396](https://doi.org/10.5281/zenodo.24222396)

This document formalizes the rendering geometry, hexagonal topology, supply-network graph algorithms, probabilistic combat equations, procedural terrain synthesis, and deterministic search methods implemented in `wf_core`, `wf_assets`, `wf_sim`, and `wf_client`. All equations below use plain ASCII/Unicode mathematical notation for clean rendering across terminal, IDE, and web viewers.

---

## 1. Sparse 64-Tree GPU Voxel Raymarching & Surface Shading (`wf_core::voxel`, `raymarch_64tree.wgsl`)

### 1.1 Hierarchical 64-Tree Spatial Index (`[0..256]^3`)
Each volumetric `.vox` asset (compiled into `assets/compiled/vox_64tree.bin`) is mapped from Z-up MagicaVoxel coordinates `(vx, vy, vz)` into Y-up world voxel coordinates `(vx, vz, vy)` inside a 4-level branching hierarchy with branching factor `4^3 = 64` per node (`4^4 = 256` voxels per axis):

```
Root (64^3 voxels/child) -> Level 1 (16^3) -> Twig (4^3) -> Brick64 Leaf (1^3 per bit)
```

Every interior node stores a 64-bit occupancy mask split into `(mask_lo: u32, mask_hi: u32)`. Occupied child slots map densely to contiguous child indices via GPU hardware population count (`countOneBits` on masked prefix bits), avoiding pointer chasing for empty space while storing only non-empty `4×4×4` leaf bricks (`Brick64`) plus pre-filtered linear MIP colors for distant LOD sampling.

Across the full **170 compiled rigs** (`152` cards + `12` terrain tiles + `6` props), headless `t3` verification measures:
- **Dense bounding volume:** `423,958,096` voxels
- **Non-empty solid voxels:** `10,039,096` (`589,863` `Brick64` leaves, `32,717` interior nodes)
- **Compressed GPU SVO buffer:** `47,712,512` bytes (`8.89x` byte compression ratio vs. dense bounding grids)
- **CPU debug traversal throughput (`t3`):** `266,612.2` rays/s (`24,000` rays in `90.02 ms`, `48/48` exterior AABB misses verified)

### 1.2 Tile-Culled Amanatides–Woo DDA & World-Space Normal Reconstruction
On the GPU compute pass (`cs_main` in `crates/wf_client/src/shaders/raymarch_64tree.wgsl`), each `8×8` pixel workgroup builds a camera-frustum ray cone (`wg_tile_insts`) and tests instanced AABBs once per tile before per-pixel Amanatides–Woo 3D Digital Differential Analyzer (DDA) traversal (up to `680` steps per instance).

When a voxel face hits along axis normal `hit_n_local = normalize(-step_dir * last_mask)` in voxel-local coordinates, world-space normal reconstruction applies the inverse-transpose of the orthogonal `world_to_voxel` upper `3×3` matrix (`w2v_rot`, stored in `instances[i].world_to_voxel`):

```
hit_n_world = normalize(transpose(w2v_rot) * hit_n_local)
```

Because `w2v_rot` represents a pure rotation with uniform scale (normalized here via `normalize(...)`), `transpose(w2v_rot)` rotates the voxel face normal cleanly into world space so directional sun diffuse, hard shadows, half-vector Blinn-Phong specular (`n = 42.0`), and grazing Fresnel terms remain physically consistent under arbitrary yaw/tilt/bounce instance rotations.

### 1.3 Generalized 8-Tap X/Y/Z Tangent-Plane Crevice Ambient Occlusion
To preserve depth in vertical flanks as well as top/bottom faces, surface hits evaluate an 8-tap voxel neighborhood in the plane one voxel ahead of the hit (`front_v = map_pos + n_step`, where `n_step = vec3<i32>(round(hit_n_local))`):

- If `|n_step.x| > 0`: tangent basis `t1 = (0, 1, 0)`, `t2 = (0, 0, 1)` (`Y-Z` plane)
- If `|n_step.z| > 0`: tangent basis `t1 = (1, 0, 0)`, `t2 = (0, 1, 0)` (`X-Y` plane)
- If `|n_step.y| > 0`: tangent basis `t1 = (1, 0, 0)`, `t2 = (0, 0, 1)` (`X-Z` plane)

Summing cardinal (`front_v ± t1`, `front_v ± t2`, weight `1.0` each) and diagonal (`front_v ± t1 ± t2`, weight `0.55` each) neighbor occupancy produces:

```
vox_ao = clamp(1.0 - occ_n * 0.088, 0.46, 1.0)
```

Emissive voxels (`sample ∈ {11, 14, 15, 19, 20}` or `sample >= 240`) bypass AO attenuation (`emissive = 1.35`) to preserve bioluminescent and plasma accents.

### 1.4 Binary-Search Hex Terrain Refinement & Schlick Fresnel Rim Shading
In `trace_hex_terrain`, coarse raymarching steps `dt = 0.06` across `y ∈ [-0.7, 3.4]` m against `hex_surface_y(tile)`:
- `tier = -1`: `-0.24` m (river/ocean) or `-0.46` m (chasm, `morph_id == 3`)
- `morph_id == 8` (`ShallowFord`): `-0.05` m
- Otherwise: `f32(tile.elevation_tier) * 0.55` m

Once a coarse sample enters terrain (`p_coarse.y <= surface_y_coarse`), `trace_hex_terrain` runs **6 binary-search bisection steps** over `[lo, hi] = [t_prev, t]`, reducing intersection error from `dt = 0.06` m to `0.06 / 2^6 ≈ 0.00094` m (`< 1 mm`), eliminating stepped silhouette aliasing on hex rims and cliff lips before reconstructing bevel/cliff normals.

Finally, lit surfaces compute Schlick Fresnel grazing reflectance (`F0 = 0.04`, `V = -rd`):

```
ndotv           = clamp(dot(normal, -rd), 0.0, 1.0)
fresnel_schlick = 0.04 + 0.96 * (1.0 - ndotv)^5
rim             = (1.0 - ndotv)^3.5
```

Team-owned voxel instances (`hit_kind == 1`) blend sky-atmosphere bounce with subtle cyan (`Team 1`) or crimson (`Team 2`) silhouette rim cues modulated by `(0.14 * fresnel_schlick + 0.16 * rim)`.

---

## 2. Hexagonal Topology & Procedural Map Generation (`wf_core::hex_map`, `wf_core::mapgen`)

### 2.1 Flat-Top Axial Projection (`hexx 0.19`)
Battlefields reside on pointy/flat axial `Hex(q, r)` grids scaled by `HEX_WORLD_RADIUS = 2.4` m (`flat-to-flat width = 2.4 * sqrt(3) ≈ 4.156922` m):

```
wx(q, r) = R * sqrt(3) * (q + r / 2)
wz(q, r) = R * 1.5 * r
```

Symmetric axial hex distance:

```
d((q1, r1), (q2, r2)) = (|q1 - q2| + |q1 + r1 - q2 - r2| + |r1 - r2|) / 2
```

Cliff traversal is blocked whenever `|elev(a) - elev(b)| >= 2`, unless the moving unit has `can_fly = true`. Aquatic movement differentiates Pure Terrestrial (`amphibious_tier == 0`, blocked by deep water), Semi-Aquatic (`amphibious_tier == 1`), and Pure Aquatic (`amphibious_tier == 2`).

### 2.2 Clamped Radius, Symmetrized 3-Octave Axial FBM & Path Guarantees
`generate_procedural_map(cfg)` uses a stateless `SplitMix64` hash PRNG (`0x9E37_79B9_7F4A_7C15` Weyl step with `0xBF58_476D_1CE4_E5B9` / `0x94D0_49BB_1331_11EB` avalanche mixing) and samples a 3-octave axial Fractional Brownian Motion (FBM) field:

```
fbm_hex(h, seed, scale) = 0.58 * o1 + 0.29 * o2 + 0.13 * o3
```

Map radius is clamped up front to:

```
clamped_radius = clamp(cfg.radius, 4, 9)
```

Normalized radial distance uses `dist_center = d(h, (0, 0)) / float(clamped_radius)` with `clamped_radius >= 4`, preventing division by zero (`NaN`) even when callers construct `MapGenConfig { radius: 0, .. }`. Coordinates mirror via `SymmetryMode::Rotational180` (`(-q, -r)`), `SymmetryMode::AxialMirror` (`(-q - r, r)`), or `SymmetryMode::AsymmetricSiege`, followed by hydrological channel carving (`ShallowFord` crossings) and BFS passability repair between HQ anchors (`p1_base`, `p2_base`) and objective spires.

---

## 3. Supply Chains, Node Leashes & Resource Economy (`wf_core::logistics`)

### 3.1 Turn-1 Economy & Replenishment
- **Turn 1 Asymmetric Balance:** Player 1 starts with `5` Munitions and `5` cards; Player 2 starts with `8` Munitions and `6` cards; both begin with `2` Base Actions.
- **Start of Player Turn (`begin_player_turn`):**
  - `home_base_munitions += 5` (`begin_player_turn` base income; continental logistics add turn-cycle bonus Munitions)
  - `actions_remaining = 2 + min(static_control_node_count, 3)` (hard ceiling `5` Actions/turn)
  - Mobile units refresh `current_armor = base_armor`; static buildings (`Installation`, `SupportInstallation`, `ControlNode`, `SupplyDepot`) never auto-refresh structural Armor.
  - Round `>= 30` triggers **Orbital Fallout Storm**: `-1` HQ `STR`/round (escalating to `-2` at Round `>= 36`).

### 3.2 Contiguous Home Supply BFS & Psychic Control Node Leash
Let `G_struct = (V_team, E_adj)` be the subgraph of living friendly `CombatClass::is_building()` entities connected at hex distance `d <= 1`. Breadth-First Search (BFS) from each living `Installation` HQ identifies connected buildings `B_conn`. A tile `h` lies inside Home Supply (`🔗SUP`) iff there exists `b ∈ B_conn` with `d(h, b.hex) <= b.supply_range` (default `1` if `0`). Units inside Home Supply draw Munitions from `home_base_munitions`; unsupplied units spend `carried_munitions` (`Encumbrance`).

Similarly, non-autonomous engineered units (`!is_autonomous && node_leash_range > 0 && !emits_node`) evaluate `min_n d(unit.hex, n.hex) <= n.node_emit_radius` over living friendly emitters (static `ControlNode`, `Commander`, `Drone`, `Cnidarian` Synapse units). Out-of-leash units transition to `confused = true` until re-linked.

---

## 4. Probabilistic D6 Combat Mechanics & Exact Analytical Expected Value (`wf_core::combat`, `wf_sim::ai_planner`)

### 4.1 Hit Threshold & Immutable Base Evade
For ranged attacks (`!ctx.is_melee`), `required_ranged_hit_roll` computes dynamic effective evade without mutating `defender.evade`:

```
effective_evade = max(defender.evade - I[rooted_turns > 0] - I[blinded_this_turn], 0)
                  + I[defender_in_rough_cover]

hit_threshold = clamp(
    1 + max(hex_distance, 1) + effective_evade
      - I[target_locked] - I[attacker_adjacent_commander] - I[attacker_pack_tactics_bonus],
    2, 6
)
```

where `I[·] ∈ {0, 1}` is the indicator function. Note specifically that `blinded_this_turn` reduces effective evade cleanly inside `required_ranged_hit_roll` and expires back to `false` at `begin_player_turn` without ever incrementing `defender.evade` (preserving `0`-evade units at `evade == 0`).

Given clamped `hit_d6 = clamp(roll, 1, 6)`:
- `ctx.is_melee`: hits automatically (cannot target `can_fly = true` units from ground).
- Ranged: hits iff `hit_d6 == 6 || (hit_d6 > 1 && hit_d6 >= hit_threshold)`.

### 4.2 Strength-Invariant Damage & Armor Absorption
When an attack hits and is not intercepted by a `Fortified` stack (`fortified_stacks > 0` consumes 1 stack for `0` damage):

```
capped_roll = min(dmg_d6, attacker.current_str_hp)
delta_str   = clamp(attacker.current_str_hp - defender.current_str_hp, -2, +3)
raw_dmg     = max(1, capped_roll + (if attacker_can_fly { max(delta_str, 0) } else { delta_str }))

armor_abs   = min(raw_dmg, defender.current_armor)
str_loss    = min(raw_dmg - armor_abs, defender.current_str_hp)
```

### 4.3 Exact Analytical Expected Value (`wf_sim::ai_planner::compute_attack_ev`)
Veteran (`1-Ply EV Focus`) and Grandmaster AI evaluate attacks across the exact uniform finite outcome space rather than noisy single-sample Monte Carlo rolls:

- **Ranged / Non-Retaliated:** `36` equiprobable pairs `(hit_d6, dmg_d6) ∈ {1..6}^2`:

```
E[Y] = (1 / 36) * SUM_{h=1..6} SUM_{d=1..6} Y(h, d)
```

- **Melee With Surviving Counter-Retaliation (`triggers_melee_retaliation` and defender survives):** `216` equiprobable triples `(hit_d6, dmg_d6, ret_d6) ∈ {1..6}^3`, factoring defender counter-damage at its post-hit reduced `current_str_hp`:

```
E[Y] = (1 / 216) * SUM_{h=1..6} SUM_{d=1..6} SUM_{r=1..6} Y(h, d, r)
```

`compute_attack_ev` reports `hit_probability`, `expected_armor_absorbed`, `expected_str_damage`, `expected_total_damage`, `kill_probability`, `expected_retaliation_str_damage`, and exact expected board delta `expected_score_delta`, which `select_best_action` ranks deterministically in integer millipoints (`round(score * 1000.0)` as `i64`).

---

## 5. 26-Stat Deck Calibration, AI Search & Validation Benchmarks (`wf_core::schema`, `wf_sim`)

### 5.1 26-Stat Linear Utility Model & Combat EHP
Every unit card's `deck_points` is validated against the integer-tenths dot product matching `Wildfront_PC_Master_Stats_and_DeckBuilder.xlsx`:

```
STAT_WEIGHTS_TENTHS = [9, 4, 15, 15, 9, 12, 9, 14, 20, 3, 18, 8, 40, 11, 6, 13, 14, 13, 13, 7, 18, 13, 13, 11, 7, 13]
computed_weighted_points = floor((SUM_{i=0..25} stat_i * w_i + 5) / 10)
```

Effective Hit Points (stored in exact integer tenths as `combat_ehp_tenths`):

```
EHP = STR + 2 * ARMR + 2 * Shield + 1.5 * EVD
```

Across all `152` cards (`113` bio/combat units + `13` support installations + `26` tactical deployments & artifacts), Pearson correlation between published `deck_points` and `computed_weighted_points` is **`r = 0.998889`** (`mean_deck_points = 17.84`, `mean_weighted_utility_points = 17.80`).

### 5.2 Deterministic Replay Hashing & AI Difficulty Tiers
- **Replay Integrity (`wf_sim`):** Every turn step hashes sorted entity states, economy totals, winner flag, and RNG state with 64-bit **FNV-1a** (`offset = 0xCBF2_9CE4_8422_2325`, `prime = 0x0100_0000_01B3`).
- **AI Search Tiers:**
  - `Recruit (1-Ply Casual)`: heuristic selection biased toward noisy/sub-optimal plays.
  - `Veteran (1-Ply EV Focus)`: deterministic 1-ply action ranking over exact analytical EV outcomes.
  - `Grandmaster (2-Ply Lookahead)`: beam-pruned (top-6 moves) 2-ply minimax with opponent response simulation.

### 5.3 Headless Benchmark Summary (`validation/wildfront_validation_summary.json`)
Running `cargo test -p wf_sim --test headless_validation` produces deterministic empirical validation across three protocols:
- **T1 (`t1_hex_determinism_replay`):** `160` axial symmetry checks, `720` terrestrial water blocks vs `720` aquatic passes, and `3` full-match replay chains (`237` verified action steps across Scenarios `5`, `3`, and `2`, matching bit-exact final 64-bit FNV-1a state hashes `0x8e68eb4b68ff3955`, `0x2f5a2d53af14d3ff`, `0x4dd419d75ded337b`).
- **T2 (`t2_deck_audit_and_round_robin`):** Full `102`-match symmetric Pure Bio-Type (9 lineages × 16 games) + Continental (6 allegiances × 10 games) tournament under `Veteran (1-Ply EV Focus)` yielding **`83 / 102` decisive matches** (`55` P1 wins, `28` P2 wins, median `14.0` turns, wall time `38,293.80 ms`).
- **T3 (`t3_svo_64tree_raymarching`):** `170` compiled voxel rigs (`10,039,096` solid voxels in `47,712,512` bytes SVO memory, `8.89x` byte compression) achieving **`266,612.2` rays/s** on single-core debug CPU DDA traversal.
