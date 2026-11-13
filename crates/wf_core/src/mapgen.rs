//! Deterministic seeded procedural hexagonal map generator for *Wildfront Battlegrounds*.
//!
//! Pipeline stages:
//!   1. `SplitMix64` PRNG + 2D Axial Hex Multi-Octave Value Noise
//!   2. Symmetry transforms (`Rotational180`, `AxialMirror`, `AsymmetricSiege`)
//!   3. Stepped elevation quantization (`0..=3`) + automatic ramp carving
//!   4. Meandering DeepRiver / RavineChasm routing + ShallowFord crossings
//!   5. 2072 RadiationFallout, RoughCover, and BunkerRuins placement
//!   6. Breadth-First connectivity guarantee between base anchors

use std::collections::{HashMap, HashSet, VecDeque};
use hexx::Hex;
use serde::{Deserialize, Serialize};
use crate::hex_map::{BattleHexMap, Biome, BiomeSnapshot, HexTile, Morphology};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymmetryMode {
    Rotational180,
    AxialMirror,
    AsymmetricSiege,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapGenConfig {
    pub seed: u64,
    pub radius: u8,
    pub biome: Biome,
    pub symmetry: SymmetryMode,
    pub water_ratio: f32,
    pub highland_ratio: f32,
    pub mountain_ratio: f32,
    pub hazard_ratio: f32,
}

impl MapGenConfig {
    pub fn for_biome(seed: u64, biome: Biome) -> Self {
        match biome {
            Biome::IrradiatedBadlands => Self {
                seed,
                radius: 6,
                biome,
                symmetry: SymmetryMode::Rotational180,
                water_ratio: 0.08,
                highland_ratio: 0.28,
                mountain_ratio: 0.14,
                hazard_ratio: 0.18,
            },
            Biome::SavannahDustbowl => Self {
                seed,
                radius: 6,
                biome,
                symmetry: SymmetryMode::Rotational180,
                water_ratio: 0.14,
                highland_ratio: 0.22,
                mountain_ratio: 0.10,
                hazard_ratio: 0.10,
            },
            Biome::AshwoodBasin => Self {
                seed,
                radius: 7,
                biome,
                symmetry: SymmetryMode::Rotational180,
                water_ratio: 0.18,
                highland_ratio: 0.24,
                mountain_ratio: 0.12,
                hazard_ratio: 0.12,
            },
            Biome::OvergrownBioJungle => Self {
                seed,
                radius: 7,
                biome,
                symmetry: SymmetryMode::AxialMirror,
                water_ratio: 0.26,
                highland_ratio: 0.20,
                mountain_ratio: 0.10,
                hazard_ratio: 0.14,
            },
            Biome::SunkenAtoll => Self {
                seed,
                radius: 8,
                biome,
                symmetry: SymmetryMode::Rotational180,
                water_ratio: 0.50,
                highland_ratio: 0.18,
                mountain_ratio: 0.06,
                hazard_ratio: 0.08,
            },
            Biome::RuinedBunkerCaldera => Self {
                seed,
                radius: 7,
                biome,
                symmetry: SymmetryMode::AsymmetricSiege,
                water_ratio: 0.10,
                highland_ratio: 0.32,
                mountain_ratio: 0.16,
                hazard_ratio: 0.20,
            },
        }
    }
}

/// Fast, deterministic `SplitMix64` mixer (zero external RNG crate discrepancy across platforms).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed ^ 0x9E37_79B9_7F4A_7C15 }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn next_f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as u32) as f32 / ((1u32 << 24) as f32)
    }
}

fn hash_2d(ix: i32, iy: i32, salt: u64) -> f32 {
    let ux = (ix as i64 as u64).wrapping_mul(0x517C_C1B7_2722_0A95);
    let uy = (iy as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut rng = SplitMix64::new(ux ^ uy.rotate_left(29) ^ salt);
    rng.next_f32()
}

fn smooth_value_noise(x: f32, y: f32, salt: u64) -> f32 {
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let fx = x - x.floor();
    let fy = y - y.floor();
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uy = fy * fy * (3.0 - 2.0 * fy);

    let v00 = hash_2d(x0, y0, salt);
    let v10 = hash_2d(x0 + 1, y0, salt);
    let v01 = hash_2d(x0, y0 + 1, salt);
    let v11 = hash_2d(x0 + 1, y0 + 1, salt);

    let a = v00 + (v10 - v00) * ux;
    let b = v01 + (v11 - v01) * ux;
    a + (b - a) * uy
}

fn fbm_hex(h: Hex, seed: u64, scale: f32) -> f32 {
    let wx = (h.x as f32 + h.y as f32 * 0.5) * 1.732_050_8 * scale;
    let wz = (h.y as f32) * 1.5 * scale;
    let o1 = smooth_value_noise(wx, wz, seed);
    let o2 = smooth_value_noise(wx * 2.07 + 13.1, wz * 2.07 - 7.4, seed.wrapping_add(101));
    let o3 = smooth_value_noise(wx * 4.13 - 5.2, wz * 4.13 + 19.8, seed.wrapping_add(202));
    o1 * 0.58 + o2 * 0.29 + o3 * 0.13
}

pub fn symmetric_partner(h: Hex, mode: SymmetryMode) -> Hex {
    match mode {
        SymmetryMode::Rotational180 => Hex::new(-h.x, -h.y),
        SymmetryMode::AxialMirror => Hex::new(-h.x - h.y, h.y),
        SymmetryMode::AsymmetricSiege => h,
    }
}

pub fn default_base_anchors(radius: u8, mode: SymmetryMode) -> (Hex, Hex) {
    let r = i32::from(radius.saturating_sub(2).max(2));
    match mode {
        SymmetryMode::Rotational180 => (Hex::new(-r, r / 2), Hex::new(r, -(r / 2))),
        SymmetryMode::AxialMirror => (Hex::new(-r, 0), Hex::new(r, 0)),
        SymmetryMode::AsymmetricSiege => (Hex::ZERO, Hex::new(r, -(r / 2))),
    }
}

/// Generates a complete deterministic `BattleHexMap` with guaranteed base-to-base playability.
pub fn generate_procedural_map(cfg: &MapGenConfig) -> BattleHexMap {
    let r_i32 = i32::from(cfg.radius.clamp(4, 9));
    let (p1_base, p2_base) = default_base_anchors(cfg.radius, cfg.symmetry);
    let mut tiles: HashMap<Hex, HexTile> = HashMap::new();

    // Stage 1 & 2: Sample symmetrized multi-octave elevation & hazard noise
    for r in -r_i32..=r_i32 {
        for q in -r_i32..=r_i32 {
            let h = Hex::new(q, r);
            if h.unsigned_distance_to(Hex::ZERO) > u32::from(cfg.radius) {
                continue;
            }
            let sym_h = symmetric_partner(h, cfg.symmetry);
            let canonical = if (h.x, h.y) <= (sym_h.x, sym_h.y) { h } else { sym_h };

            let elev_n = fbm_hex(canonical, cfg.seed, 0.28);
            let haz_n = fbm_hex(canonical, cfg.seed.wrapping_add(0xA5A5_5A5A), 0.42);
            let cover_n = fbm_hex(canonical, cfg.seed.wrapping_add(0x33CC_CC33), 0.55);

            let dist_center = h.unsigned_distance_to(Hex::ZERO) as f32 / (cfg.radius as f32);

            let (mut morph, mut elev) = if cfg.biome == Biome::SunkenAtoll {
                // Archipelago ring around lagoon + outer ocean
                let ring_band = (dist_center - 0.56).abs();
                if ring_band > 0.28 && elev_n < 0.62 {
                    (Morphology::OpenOcean, 0)
                } else if elev_n < cfg.water_ratio * 0.72 {
                    (Morphology::DeepRiver, 0)
                } else if elev_n > 1.0 - cfg.mountain_ratio {
                    (Morphology::MountainPeak, 3)
                } else if elev_n > 1.0 - cfg.highland_ratio {
                    (Morphology::Flat, 1)
                } else {
                    (Morphology::Flat, 0)
                }
            } else if elev_n > 1.0 - cfg.mountain_ratio * 0.75 {
                if haz_n > 0.62 && cfg.biome == Biome::RuinedBunkerCaldera {
                    (Morphology::RavineChasm, 0)
                } else {
                    (Morphology::MountainPeak, 3)
                }
            } else if elev_n > 1.0 - (cfg.mountain_ratio + cfg.highland_ratio * 0.75) {
                (Morphology::Flat, 1)
            } else {
                (Morphology::Flat, 0)
            };

            // Scatter RadiationFallout, BunkerRuins & RoughCover on passable flat/highland tiles
            if morph == Morphology::Flat {
                if haz_n > 1.0 - cfg.hazard_ratio * 0.55 {
                    morph = Morphology::RadiationFallout;
                } else if haz_n < cfg.hazard_ratio * 0.28 {
                    morph = Morphology::BunkerRuins;
                } else if cover_n > 0.64 {
                    morph = Morphology::RoughCover;
                }
            }

            if cfg.symmetry == SymmetryMode::AsymmetricSiege && h.unsigned_distance_to(Hex::ZERO) <= 2 {
                elev = 1;
                if morph == Morphology::MountainPeak || morph == Morphology::RavineChasm {
                    morph = Morphology::Flat;
                }
            }

            tiles.insert(
                h,
                HexTile {
                    biome: cfg.biome,
                    morphology: morph,
                    elevation: elev,
                    cliff_edges: 0,
                    objective_tag: 0,
                },
            );
        }
    }

    // Stage 3: Carve meandering hydrological river channel across mid-board (with ShallowFords)
    if cfg.biome != Biome::SunkenAtoll && cfg.water_ratio >= 0.08 {
        for r in -r_i32..=r_i32 {
            let wobble_f = (smooth_value_noise(r as f32 * 0.45, 3.7, cfg.seed.wrapping_add(777)) - 0.5) * 2.4;
            let river_q = (wobble_f.round() as i32) - (r / 2);
            let h = Hex::new(river_q, r);
            let sym_h = symmetric_partner(h, cfg.symmetry);
            for target in [h, sym_h] {
                if let Some(t) = tiles.get_mut(&target) {
                    if target.unsigned_distance_to(p1_base) > 1
                        && target.unsigned_distance_to(p2_base) > 1
                    {
                        let is_ford = r == 0 || r.abs() == (r_i32 - 2).max(2);
                        t.elevation = 0;
                        t.morphology = if is_ford {
                            Morphology::ShallowFord
                        } else {
                            Morphology::DeepRiver
                        };
                    }
                }
            }
        }
    }

    // Stage 3b: Carve connected Base Estuary Tributaries to `dist == 2` from each HQ on ALL biomes
    // (including SunkenAtoll!) so Pure Aquatic (`tier == 2`) fleets always have navigable water in Supply!
    {
        for base in [p1_base, p2_base] {
            let mut cur = Hex::ZERO;
            let mut guard = 0;
            while cur.unsigned_distance_to(base) > 2 && guard < 16 {
                guard += 1;
                if let Some(next_h) = cur.all_neighbors().into_iter().min_by_key(|h| h.unsigned_distance_to(base)) {
                    cur = next_h;
                    if cur.unsigned_distance_to(base) >= 2 {
                        if let Some(t) = tiles.get_mut(&cur) {
                            t.elevation = 0;
                            t.morphology = if cur.unsigned_distance_to(base) == 2 {
                                Morphology::ShallowFord
                            } else {
                                Morphology::DeepRiver
                            };
                        }
                    }
                } else {
                    break;
                }
            }
        }
    }

    // Stage 4: Flatten & clear 1-hex starter base rings around P1 & P2 anchors
    for anchor in [p1_base, p2_base] {
        let base_elev = tiles.get(&anchor).map(|t| t.elevation.min(1)).unwrap_or(0);
        for h in anchor.range(1) {
            if let Some(t) = tiles.get_mut(&h) {
                t.elevation = base_elev;
                if !matches!(t.morphology, Morphology::Flat | Morphology::RoughCover) {
                    t.morphology = Morphology::Flat;
                }
            }
        }
    }

    // Stage 5: Compute directional cliff masks & guarantee terrestrial/shallow path connectivity
    recompute_cliff_masks(&mut tiles);
    let mut map = BattleHexMap {
        radius: cfg.radius,
        primary_biome: match cfg.biome {
            Biome::IrradiatedBadlands => BiomeSnapshot::Badlands,
            Biome::SavannahDustbowl => BiomeSnapshot::Savannah,
            Biome::AshwoodBasin => BiomeSnapshot::Ashwood,
            Biome::OvergrownBioJungle => BiomeSnapshot::BioJungle,
            Biome::SunkenAtoll => BiomeSnapshot::Atoll,
            Biome::RuinedBunkerCaldera => BiomeSnapshot::Caldera,
        },
        tiles,
    };

    if cfg.biome == Biome::SunkenAtoll {
        for (&h, t) in map.tiles.iter_mut() {
            let d0 = h.unsigned_distance_to(Hex::ZERO);
            if (1..=3).contains(&d0) && matches!(t.morphology, Morphology::OpenOcean) {
                t.morphology = Morphology::ShallowFord;
            }
        }
    }
    ensure_base_connectivity(&mut map, p1_base, p2_base, false);
    map
}

fn recompute_cliff_masks(tiles: &mut HashMap<Hex, HexTile>) {
    let snapshot: HashMap<Hex, u8> = tiles.iter().map(|(&h, t)| (h, t.elevation)).collect();
    for (&h, tile) in tiles.iter_mut() {
        let mut mask = 0u8;
        for (edge_idx, nb) in h.all_neighbors().into_iter().enumerate() {
            if let Some(&nb_elev) = snapshot.get(&nb) {
                if tile.elevation.abs_diff(nb_elev) >= 2 {
                    mask |= 1 << edge_idx;
                }
            }
        }
        tile.cliff_edges = mask;
    }
}

fn has_traversal_path(map: &BattleHexMap, start: Hex, goal: Hex, amphibious_tier: u8) -> bool {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    visited.insert(start);
    queue.push_back(start);

    while let Some(curr) = queue.pop_front() {
        if curr == goal {
            return true;
        }
        for nb in curr.all_neighbors() {
            if !visited.contains(&nb) && map.can_traverse(curr, nb, false, false, amphibious_tier) {
                visited.insert(nb);
                queue.push_back(nb);
            }
        }
    }
    false
}

/// Carves a direct ShallowFord / stepped-ramp corridor if random noise ever blocks ground access.
fn ensure_base_connectivity(map: &mut BattleHexMap, p1: Hex, p2: Hex, allow_semi_aquatic: bool) {
    let tier = if allow_semi_aquatic { 1 } else { 0 };
    if has_traversal_path(map, p1, p2, tier) {
        return;
    }
    let line = p1.line_to(p2);
    let mut prev_elev = map.tiles.get(&p1).map(|t| t.elevation).unwrap_or(0);
    for h in line {
        if let Some(t) = map.tiles.get_mut(&h) {
            if t.elevation.abs_diff(prev_elev) >= 2 {
                t.elevation = prev_elev;
            }
            prev_elev = t.elevation;
            match t.morphology {
                Morphology::MountainPeak | Morphology::RavineChasm => {
                    t.morphology = Morphology::Flat;
                }
                Morphology::DeepRiver | Morphology::OpenOcean if !allow_semi_aquatic => {
                    t.morphology = Morphology::ShallowFord;
                }
                _ => {}
            }
        }
    }
    recompute_cliff_masks(&mut map.tiles);
}
