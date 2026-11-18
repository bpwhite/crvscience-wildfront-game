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

//! Watertight 2-Manifold Binary `.STL` 3D-Print Exporter (`wf_assets::stl_exporter`).
//!
//! Converts MagicaVoxel `.vox` unit, installation, and terrain prop rigs directly into
//! physical millimeter `.STL` meshes matching the `45.0mm` Tabletop Edition 1:1 scale,
//! sculpting support-free **45-degree Recessed Micro-Dice V-Cradles** into the rear (`+Y`)
//! Staunton pedestal skirt with a **guaranteed print-thick solid center wall** between dice
//! (`DEFAULT_CENTER_WALL_MM = 2.80mm` = 7 perimeters on a `0.40mm` FDM nozzle; hard-clamped
//! `>= MIN_PRINTABLE_CENTER_WALL_MM = 2.00mm`).

use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use wf_core::{DeckList, RosterManifest, UnitCardDef};
use crate::vox_baker::bake_vox_file;

/// PC hex flat-to-flat width (`2.4 * sqrt(3) = 4.1569219` world meters).
pub const PC_HEX_FLAT_WIDTH_M: f32 = 4.156_922;

/// Default physical flat-to-flat tabletop hex width (`45.0mm`).
pub const DEFAULT_HEX_FLAT_MM: f32 = 45.0;

/// Hard minimum allowed center wall thickness between adjacent dice V-cradles (`2.00mm`,
/// guaranteeing at least 5 solid perimeters on a standard `0.40mm` FDM nozzle).
pub const MIN_PRINTABLE_CENTER_WALL_MM: f32 = 2.00;

/// Default center divider wall thickness between Left (`STR`) and Right (`MUN`) dice (`2.80mm`,
/// yielding 7 solid perimeters on a `0.40mm` nozzle and zero fragile thin-wall artifacts).
pub const DEFAULT_CENTER_WALL_MM: f32 = 2.80;

/// Default outer side-wall thickness flanking the outermost cradle edges (`2.20mm`).
pub const DEFAULT_OUTER_WALL_MM: f32 = 2.20;

/// Default per-side pocket clearance (`0.40mm` per side -> `7.80mm` pocket for `7.0mm` D6,
/// `10.40mm` pocket for `9.6mm` kite-span mini-D10).
pub const DEFAULT_DIE_CLEARANCE_MM: f32 = 0.40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CradleMode {
    /// Automatically chooses pocket count & die size (`7mm D6` vs `10mm D10`) from card `STR` & `ENC`.
    AutoFromCardStats,
    /// Forces twin `7mm` D6 cradles (`Left = STR`, `Right = MUN`) on all non-prop models.
    ForceDualD6,
    /// Forces a single centered `10mm` mini-D10 cradle on all non-prop models.
    ForceSingleD10,
    /// Exports pure Staunton chess piece with zero dice pockets carved.
    SculptOnlyNoCradles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolvedCradleLayout {
    None,
    SingleCenteredD6 { str_cap: u8 },
    SingleCenteredD10 { str_cap: u8 },
    DualD6 { str_cap: u8, mun_cap: u8 },
    AsymmetricD6AndD10 { str_cap: u8, mun_cap: u8 },
}

impl ResolvedCradleLayout {
    pub fn summary_label(&self, actual_wall_mm: f32) -> String {
        match *self {
            Self::None => "Sculpt Only (No Dice Cradles)".to_string(),
            Self::SingleCenteredD6 { str_cap } => {
                format!("Single 45° V-Cradle: 7mm D6 (STR {str_cap}, ENC 0)")
            }
            Self::SingleCenteredD10 { str_cap } => {
                format!("Single 45° V-Cradle: 10mm D10 (STR {str_cap}, ENC 0)")
            }
            Self::DualD6 { str_cap, mun_cap } => format!(
                "Dual 45° V-Cradles: 7mm D6 (STR {str_cap}) + 7mm D6 (MUN {mun_cap}) | {actual_wall_mm:.2}mm Center Wall"
            ),
            Self::AsymmetricD6AndD10 { str_cap, mun_cap } => format!(
                "Dual 45° V-Cradles: 7mm D6 (STR {str_cap}) + 10mm D10 (MUN {mun_cap}) | {actual_wall_mm:.2}mm Center Wall"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TabletopPrintConfig {
    pub target_hex_flat_mm: f32,
    pub center_wall_mm: f32,
    pub outer_wall_mm: f32,
    pub die_clearance_mm: f32,
    pub cradle_mode: CradleMode,
    pub include_front_chevron: bool,
    pub include_magnet_pocket: bool,
}

impl Default for TabletopPrintConfig {
    fn default() -> Self {
        Self {
            target_hex_flat_mm: DEFAULT_HEX_FLAT_MM,
            center_wall_mm: DEFAULT_CENTER_WALL_MM,
            outer_wall_mm: DEFAULT_OUTER_WALL_MM,
            die_clearance_mm: DEFAULT_DIE_CLEARANCE_MM,
            cradle_mode: CradleMode::AutoFromCardStats,
            include_front_chevron: true,
            include_magnet_pocket: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StlExportReport {
    pub model_label: String,
    pub vox_slug: String,
    pub output_path: PathBuf,
    pub triangle_count: usize,
    pub layout: ResolvedCradleLayout,
    pub resolved_cradle_desc: String,
    pub min_center_wall_mm: f32,
    pub footprint_mm: (f32, f32, f32),
}

/// Canonical world scale (`meters / voxel`) matching `wf_client::scene::unit_chess_voxel_scale`.
pub fn canonical_voxel_scale(slug: &str) -> f32 {
    match slug {
        "command_center" | "bird_dreadnought" | "mammal_apex_orca" => 0.0228,
        "mammal_ground_sloth" | "mammal_warstag_elk" => 0.0230,
        "mammal_shadowclaw_panda" | "fish_rapier_swordfish" | "shield_generator_dome" | "radar_dish_tower" => 0.0232,
        "mammal_battle_llama" | "mammal_blockade_beaver" | "mammal_leechwing_bat" | "reptile_chameleon_stalker" | "insect_inferno_ants" | "fish_voltaic_eel" | "cnidarian_hydra_polyp" => 0.0234,
        "mammal_saboteur_skunk" | "plant_sundew_nightshade" => 0.0238,
        "mammal_rocket_rhino" | "mammal_warback_gorilla" | "mammal_frostclaw_bear" => 0.0230,
        "mammal_warneck_giraffe" | "arachnid_tarantula" | "fish_manta_ray" => 0.0232,
        "mammal_fireclaw_lion" | "mammal_timberwolf_pack" | "reptile_cobra_titanoboa" | "insect_bomber_hornet" | "amphibian_axolotl_mender" => 0.0234,
        "plant_pitcher_vat" | "bio_splice_vat" => 0.0238,
        "factory" | "mammal_battlephant" | "reptile_shellguard" | "insect_siege_beetle" => 0.0230,
        "hospital" | "supply_truck" | "plant_spore_mortar" | "bird_skyhunter" | "arachnid_scorpion" => 0.0232,
        "supply_depot" | "combat_koala_squad" | "mecharoo" | "reptile_flamer_croc" | "fish_laser_shark" | "amphibian_venom_frog" => 0.0234,
        "human_commander" | "human_medic" | "insect_mantis" | "jelly_zeppelin" | "cnidarian_manowar" => 0.0236,
        "control_node" | "radar_laser_turret" => 0.0238,
        "drone" | "plant_trap" => 0.0240,
        "amphibian_bullfrog_grapple_scout" | "amphibian_cane_toad_mortar" | "amphibian_hellbender_salamander" | "arachnid_brown_warweaver" | "arachnid_funnel_web_trapper" | "arachnid_stalker_wolf_spider" | "art_avian_aerie_splice" | "art_cargo_drop" | "art_chitin_carapace_splice" | "art_cnidocyte_synapse_vat" | "art_command_bonus" | "art_cryo_hibernation_vat" | "bird_bombardier_ravens" | "bird_crestbird_flier" | "bird_dive_strike_peregrine" | "cnidarian_box_jelly_lurker" | "cnidarian_siphonophore_frigate" | "command_center_africa" | "command_center_asia" | "command_center_europe" | "command_center_north_america" | "command_center_south_america" | "commander_africa" | "commander_asia" | "commander_europe" | "commander_north_america" | "commander_south_america" | "fish_recon_trout" | "fish_tidehopper_mudskipper" | "human_falconer" | "human_ichuri" | "human_inyanga" | "human_kampo" | "human_ngangkari" | "human_plague_doctor" | "insect_cicada_signal_relay" | "insect_royal_jelly_mender" | "mammal_armored_supply_buffalo" | "mammal_auxiliary_llama" | "mammal_battletusker" | "mammal_cargo_sloth" | "mammal_combat_wombat" | "mammal_earthclaw_badger" | "mammal_flamecoats" | "mammal_ironclaw_lion" | "mammal_jump_rat" | "mammal_jungle_stalker_panther" | "mammal_mercy_dog" | "mammal_oaken_sentinel" | "mammal_pack_cackle_hyena" | "mammal_pocket_gopher" | "mammal_recon_rover" | "mammal_roostriker" | "mammal_saberclaw_warcat" | "mammal_saboteur" | "mammal_sentinel_elk" | "mammal_shadow_rat" | "mammal_shadowtail_scout" | "mammal_sneak_paw_polecat" | "mammal_support_giraffe" | "mammal_support_panda" | "mammal_support_roo" | "mammal_thylacine" | "mammal_treeclaw_sloth" | "mammal_tusker" | "mammal_warclaw_battleshrew" | "mammal_yangtze_scoutfin" | "plant_bittersweet_nightshade" | "plant_deadly_elephant_ear" | "plant_deadly_nightshade" | "plant_devil_s_thornapple" | "plant_mire_pitfall_pitcher" | "plant_toxiwood_chi_ute" | "reptile_battleclaw_komodo" | "reptile_cobra_striker" | "reptile_infiltrator_vine_python" | "reptile_ragin_caiman" | "reptile_recon_gila" | "td_abyssal_riptide" | "td_apex_bloodlust_roar" | "td_bramble_barricade" | "td_corrosive_acid_spray" | "td_emp_blackout_pulse" | "td_hydro_breach_canal" | "td_ion_curtain_shield_dome" | "td_nano_triage_field_burst" | "td_napalm_basking_surge" | "td_neural_short_circuit" | "td_neuro_venom_miasma" | "td_orbital_kinetic_strike" | "td_phantom_mist_veil" | "td_pheromone_brood_call" | "td_satellite_recon_sweep" | "td_spore_monsoon" | "td_stampede_shockwave" | "td_stratospheric_dive_target" | "td_tactical_relay_overclock" => 0.0352,
        "prop_mountain_crag" => 0.0255,
        s if s.starts_with("hex_") => 0.0288675,
        s if s.starts_with("prop_") => 0.0315,
        _ => 0.0234,
    }
}

/// Resolves the exact V-cradle pocket arrangement for a card or raw `.vox` slug.
pub fn resolve_cradle_layout(
    card: Option<&UnitCardDef>,
    vox_slug: &str,
    mode: CradleMode,
) -> ResolvedCradleLayout {
    if vox_slug.starts_with("prop_") || vox_slug.starts_with("hex_") || vox_slug.starts_with("td_") || (vox_slug.starts_with("art_") && vox_slug != "art_cargo_drop") || mode == CradleMode::SculptOnlyNoCradles {
        return ResolvedCradleLayout::None;
    }
    match mode {
        CradleMode::SculptOnlyNoCradles => ResolvedCradleLayout::None,
        CradleMode::ForceDualD6 => ResolvedCradleLayout::DualD6 {
            str_cap: card.map_or(6, |c| c.strength_hp),
            mun_cap: card.map_or(6, |c| c.encumbrance.max(1)),
        },
        CradleMode::ForceSingleD10 => ResolvedCradleLayout::SingleCenteredD10 {
            str_cap: card.map_or(10, |c| c.strength_hp),
        },
        CradleMode::AutoFromCardStats => {
            if let Some(c) = card {
                let s = c.strength_hp;
                let e = c.encumbrance;
                if e == 0 {
                    if s > 6 {
                        ResolvedCradleLayout::SingleCenteredD10 { str_cap: s }
                    } else {
                        ResolvedCradleLayout::SingleCenteredD6 { str_cap: s }
                    }
                } else if e > 6 {
                    ResolvedCradleLayout::AsymmetricD6AndD10 { str_cap: s, mun_cap: e }
                } else if s > 6 {
                    ResolvedCradleLayout::SingleCenteredD10 { str_cap: s }
                } else {
                    ResolvedCradleLayout::DualD6 { str_cap: s, mun_cap: e }
                }
            } else {
                match vox_slug {
                    "command_center" => ResolvedCradleLayout::SingleCenteredD10 { str_cap: 10 },
                    "control_node" => ResolvedCradleLayout::SingleCenteredD10 { str_cap: 7 },
                    "hospital" | "factory" | "radar_laser_turret" | "plant_trap" => {
                        ResolvedCradleLayout::SingleCenteredD6 { str_cap: 6 }
                    }
                    "supply_depot" => ResolvedCradleLayout::AsymmetricD6AndD10 { str_cap: 6, mun_cap: 10 },
                    "supply_truck" => ResolvedCradleLayout::AsymmetricD6AndD10 { str_cap: 5, mun_cap: 7 },
                    _ => ResolvedCradleLayout::DualD6 { str_cap: 6, mun_cap: 5 },
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct PocketVoxelSpan {
    ix0: usize,
    ix1: usize, // exclusive: pocket clears [ix0 .. ix1]
    cx_mm: f32,
    top_clear_z_mm: f32,
}

pub fn sanitize_filename_slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_under = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_under = false;
        } else if !prev_under && !out.is_empty() {
            out.push('_');
            prev_under = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "wildfront_unit".to_string()
    } else {
        out
    }
}

fn fill_interior_cavities(occ: &mut [bool], sx: usize, sy: usize, sz: usize) {
    let stride_y = sz;
    let stride_x = sy * sz;
    let mut ext_air = vec![false; sx * sy * sz];
    let mut queue = VecDeque::new();

    for x in 0..sx {
        for y in 0..sy {
            for z in 0..sz {
                if x == 0 || x + 1 == sx || y == 0 || y + 1 == sy || z == 0 || z + 1 == sz {
                    let idx = x * stride_x + y * stride_y + z;
                    if !occ[idx] {
                        ext_air[idx] = true;
                        queue.push_back((x, y, z));
                    }
                }
            }
        }
    }
    while let Some((x, y, z)) = queue.pop_front() {
        let nbrs = [
            (x.wrapping_sub(1), y, z),
            (x + 1, y, z),
            (x, y.wrapping_sub(1), z),
            (x, y + 1, z),
            (x, y, z.wrapping_sub(1)),
            (x, y, z + 1),
        ];
        for (nx, ny, nz) in nbrs {
            if nx < sx && ny < sy && nz < sz {
                let nidx = nx * stride_x + ny * stride_y + nz;
                if !occ[nidx] && !ext_air[nidx] {
                    ext_air[nidx] = true;
                    queue.push_back((nx, ny, nz));
                }
            }
        }
    }
    for i in 0..occ.len() {
        if !ext_air[i] {
            occ[i] = true;
        }
    }
}

fn keep_only_z0_grounded_component(occ: &mut [bool], sx: usize, sy: usize, sz: usize) {
    let stride_y = sz;
    let stride_x = sy * sz;
    let mut grounded = vec![false; sx * sy * sz];
    let mut queue = VecDeque::new();

    for x in 0..sx {
        for y in 0..sy {
            let idx = x * stride_x + y * stride_y;
            if occ[idx] {
                grounded[idx] = true;
                queue.push_back((x, y, 0usize));
            }
        }
    }
    while let Some((x, y, z)) = queue.pop_front() {
        let nbrs = [
            (x.wrapping_sub(1), y, z),
            (x + 1, y, z),
            (x, y.wrapping_sub(1), z),
            (x, y + 1, z),
            (x, y, z.wrapping_sub(1)),
            (x, y, z + 1),
        ];
        for (nx, ny, nz) in nbrs {
            if nx < sx && ny < sy && nz < sz {
                let nidx = nx * stride_x + ny * stride_y + nz;
                if occ[nidx] && !grounded[nidx] {
                    grounded[nidx] = true;
                    queue.push_back((nx, ny, nz));
                }
            }
        }
    }
    for i in 0..occ.len() {
        if !grounded[i] {
            occ[i] = false;
        }
    }
}

/// Eliminates all 2x2 diagonal non-manifold edge saddles across XY, XZ, and YZ slices,
/// then eliminates 2x2x2 corner-only vertex pinches so the resulting voxel boundary is a
/// strict watertight 2-manifold in every slicer.
fn repair_2manifold_saddles(occ: &mut [bool], sx: usize, sy: usize, sz: usize) {
    let stride_y = sz;
    let stride_x = sy * sz;
    let idx = |x: usize, y: usize, z: usize| -> usize { x * stride_x + y * stride_y + z };

    for _pass in 0..6 {
        let mut changed = 0usize;

        // 1. XY slices
        for z in 0..sz {
            for x in 0..sx.saturating_sub(1) {
                for y in 0..sy.saturating_sub(1) {
                    let a00 = occ[idx(x, y, z)];
                    let a10 = occ[idx(x + 1, y, z)];
                    let a01 = occ[idx(x, y + 1, z)];
                    let a11 = occ[idx(x + 1, y + 1, z)];
                    if a00 && a11 && !a10 && !a01 {
                        occ[idx(x + 1, y, z)] = true;
                        occ[idx(x, y + 1, z)] = true;
                        changed += 1;
                    } else if a10 && a01 && !a00 && !a11 {
                        occ[idx(x, y, z)] = true;
                        occ[idx(x + 1, y + 1, z)] = true;
                        changed += 1;
                    }
                }
            }
        }

        // 2. XZ slices
        for y in 0..sy {
            for x in 0..sx.saturating_sub(1) {
                for z in 0..sz.saturating_sub(1) {
                    let b00 = occ[idx(x, y, z)];
                    let b10 = occ[idx(x + 1, y, z)];
                    let b01 = occ[idx(x, y, z + 1)];
                    let b11 = occ[idx(x + 1, y, z + 1)];
                    if b00 && b11 && !b10 && !b01 {
                        occ[idx(x + 1, y, z)] = true;
                        occ[idx(x, y, z + 1)] = true;
                        changed += 1;
                    } else if b10 && b01 && !b00 && !b11 {
                        occ[idx(x, y, z)] = true;
                        occ[idx(x + 1, y, z + 1)] = true;
                        changed += 1;
                    }
                }
            }
        }

        // 3. YZ slices
        for x in 0..sx {
            for y in 0..sy.saturating_sub(1) {
                for z in 0..sz.saturating_sub(1) {
                    let c00 = occ[idx(x, y, z)];
                    let c10 = occ[idx(x, y + 1, z)];
                    let c01 = occ[idx(x, y, z + 1)];
                    let c11 = occ[idx(x, y + 1, z + 1)];
                    if c00 && c11 && !c10 && !c01 {
                        occ[idx(x, y + 1, z)] = true;
                        occ[idx(x, y, z + 1)] = true;
                        changed += 1;
                    } else if c10 && c01 && !c00 && !c11 {
                        occ[idx(x, y, z)] = true;
                        occ[idx(x, y + 1, z + 1)] = true;
                        changed += 1;
                    }
                }
            }
        }

        if changed == 0 {
            break;
        }
    }
}

/// Core parametric `.vox` -> `.STL` compiler with positive Rear Cradle Bolster,
/// guaranteed print-thick Center Septum Wall (`>= MIN_PRINTABLE_CENTER_WALL_MM`),
/// 45° Support-Free V-Troughs, Thumb-Scoop Lip Notches, Front Chevron, and Magnet Socket.
pub fn export_vox_with_config(
    vox_path: &Path,
    out_stl: &Path,
    card: Option<&UnitCardDef>,
    cfg: &TabletopPrintConfig,
) -> Result<StlExportReport> {
    let baked = bake_vox_file(vox_path)?;
    let (raw_sx, raw_sy, raw_sz) = (
        baked.dimensions.0 as usize,
        baked.dimensions.1 as usize,
        baked.dimensions.2 as usize,
    );
    if raw_sx == 0 || raw_sy == 0 || raw_sz == 0 {
        return Err(anyhow!("Empty dimensions in {}", vox_path.display()));
    }

    let hex_mm = cfg.target_hex_flat_mm.clamp(25.0, 90.0);
    let mm_per_vox = canonical_voxel_scale(&baked.model_name) * (hex_mm / PC_HEX_FLAT_WIDTH_M);
    let is_prop = baked.model_name.starts_with("prop_");
    let layout = resolve_cradle_layout(card, &baked.model_name, cfg.cradle_mode);

    // Pad XY by 6.0mm on each side so bolsters and front chevrons never touch boundary air
    let pad_xy = ((6.0 / mm_per_vox).ceil() as usize).max(16);
    let sx = raw_sx + pad_xy * 2;
    let sy = raw_sy + pad_xy * 2;
    let sz = raw_sz + 4;
    let stride_y = sz;
    let stride_x = sy * sz;
    let mut occ = vec![false; sx * sy * sz];

    for sb in &baked.bricks {
        let (bx, by, bz) = (
            usize::from(sb.brick_pos.0),
            usize::from(sb.brick_pos.1),
            usize::from(sb.brick_pos.2),
        );
        for lz in 0..4usize {
            for ly in 0..4usize {
                for lx in 0..4usize {
                    if sb.voxels_z_yx[lz][lx + ly * 4] != 0 {
                        let (vx, vy, vz) = (bx * 4 + lx, by * 4 + ly, bz * 4 + lz);
                        if vx < raw_sx && vy < raw_sy && vz < raw_sz {
                            let gx = vx + pad_xy;
                            let gy = vy + pad_xy;
                            occ[gx * stride_x + gy * stride_y + vz] = true;
                        }
                    }
                }
            }
        }
    }

    // Fill hollow GPU raymarching core into a solid printable volume first
    fill_interior_cavities(&mut occ, sx, sy, sz);

    let center_ix = sx / 2;
    let center_iy = sy / 2;
    let cx_mm = (center_ix as f32) * mm_per_vox;
    let cy_mm = (center_iy as f32) * mm_per_vox;

    // Measure actual base foot radius over z <= 2.5mm
    let z_foot_lim = ((2.5 / mm_per_vox).ceil() as usize).min(sz);
    let mut r_base_mm = 16.0_f32;
    for x in 0..sx {
        let dx = (x as f32 + 0.5) * mm_per_vox - cx_mm;
        for y in 0..sy {
            let dy = (y as f32 + 0.5) * mm_per_vox - cy_mm;
            for z in 0..z_foot_lim {
                if occ[x * stride_x + y * stride_y + z] {
                    r_base_mm = r_base_mm.max((dx * dx + dy * dy).sqrt());
                }
            }
        }
    }

    // 1. Optional Front `-Y` Directional Tactical Facing Chevron
    if !is_prop && cfg.include_front_chevron {
        let z_chev_max = ((2.6 / mm_per_vox).ceil() as usize).min(sz);
        for z in 0..z_chev_max {
            let z_mm = (z as f32 + 0.5) * mm_per_vox;
            let half_w_mm = (2.8 - z_mm * 0.65).max(0.6);
            let tip_y_mm = -r_base_mm - 1.35 + z_mm * 0.40;
            let ix_min = (((cx_mm - half_w_mm) / mm_per_vox).floor() as isize).max(1) as usize;
            let ix_max = (((cx_mm + half_w_mm) / mm_per_vox).ceil() as usize).min(sx - 2);
            for x in ix_min..=ix_max {
                let dx_abs = ((x as f32 + 0.5) * mm_per_vox - cx_mm).abs();
                let y_front_mm = tip_y_mm + dx_abs * 0.72;
                let y_back_mm = -r_base_mm + 2.2;
                let iy_min = (((cy_mm + y_front_mm) / mm_per_vox).floor() as isize).max(1) as usize;
                let iy_max = (((cy_mm + y_back_mm) / mm_per_vox).ceil() as usize).min(sy - 2);
                for y in iy_min..=iy_max {
                    occ[x * stride_x + y * stride_y + z] = true;
                }
            }
        }
    }

    // 2. Parametric Support-Free 45° Rear (`+Y`) Micro-Dice V-Cradles + Guaranteed Thick Center Wall
    let mut measured_wall_mm = 0.0_f32;
    if layout != ResolvedCradleLayout::None {
        let clr = cfg.die_clearance_mm.clamp(0.20, 0.85);
        let w_d6_mm = 7.0 + 2.0 * clr;
        let w_d10_mm = 9.6 + 2.0 * clr;
        let req_wall_mm = cfg.center_wall_mm.max(MIN_PRINTABLE_CENTER_WALL_MM);
        let req_outer_mm = cfg.outer_wall_mm.max(1.80);

        // Snap wall & pocket spans UPWARD to exact integer voxel counts so rounding never thins a wall!
        let wall_vox = (req_wall_mm / mm_per_vox).ceil() as usize;
        let outer_vox = (req_outer_mm / mm_per_vox).ceil() as usize;
        let d6_vox = (w_d6_mm / mm_per_vox).ceil() as usize;
        let d10_vox = (w_d10_mm / mm_per_vox).ceil() as usize;

        let (pockets, bolster_ix0, bolster_ix1): (Vec<PocketVoxelSpan>, usize, usize) = match layout {
            ResolvedCradleLayout::None => (Vec::new(), center_ix, center_ix),
            ResolvedCradleLayout::SingleCenteredD6 { .. } => {
                let ix0 = center_ix - d6_vox / 2;
                let ix1 = ix0 + d6_vox;
                let cx_p = ((ix0 + ix1) as f32 * 0.5) * mm_per_vox;
                (
                    vec![PocketVoxelSpan { ix0, ix1, cx_mm: cx_p, top_clear_z_mm: 12.8 }],
                    ix0 - outer_vox,
                    ix1 + outer_vox,
                )
            }
            ResolvedCradleLayout::SingleCenteredD10 { .. } => {
                let ix0 = center_ix - d10_vox / 2;
                let ix1 = ix0 + d10_vox;
                let cx_p = ((ix0 + ix1) as f32 * 0.5) * mm_per_vox;
                (
                    vec![PocketVoxelSpan { ix0, ix1, cx_mm: cx_p, top_clear_z_mm: 13.8 }],
                    ix0 - outer_vox,
                    ix1 + outer_vox,
                )
            }
            ResolvedCradleLayout::DualD6 { .. } => {
                let total_inner = d6_vox + wall_vox + d6_vox;
                let left_ix0 = center_ix - total_inner / 2;
                let left_ix1 = left_ix0 + d6_vox;
                let right_ix0 = left_ix1 + wall_vox;
                let right_ix1 = right_ix0 + d6_vox;
                (
                    vec![
                        PocketVoxelSpan {
                            ix0: left_ix0,
                            ix1: left_ix1,
                            cx_mm: ((left_ix0 + left_ix1) as f32 * 0.5) * mm_per_vox,
                            top_clear_z_mm: 12.8,
                        },
                        PocketVoxelSpan {
                            ix0: right_ix0,
                            ix1: right_ix1,
                            cx_mm: ((right_ix0 + right_ix1) as f32 * 0.5) * mm_per_vox,
                            top_clear_z_mm: 12.8,
                        },
                    ],
                    left_ix0 - outer_vox,
                    right_ix1 + outer_vox,
                )
            }
            ResolvedCradleLayout::AsymmetricD6AndD10 { .. } => {
                let total_inner = d6_vox + wall_vox + d10_vox;
                let left_ix0 = center_ix - total_inner / 2;
                let left_ix1 = left_ix0 + d6_vox;
                let right_ix0 = left_ix1 + wall_vox;
                let right_ix1 = right_ix0 + d10_vox;
                (
                    vec![
                        PocketVoxelSpan {
                            ix0: left_ix0,
                            ix1: left_ix1,
                            cx_mm: ((left_ix0 + left_ix1) as f32 * 0.5) * mm_per_vox,
                            top_clear_z_mm: 12.8,
                        },
                        PocketVoxelSpan {
                            ix0: right_ix0,
                            ix1: right_ix1,
                            cx_mm: ((right_ix0 + right_ix1) as f32 * 0.5) * mm_per_vox,
                            top_clear_z_mm: 13.8,
                        },
                    ],
                    left_ix0 - outer_vox,
                    right_ix1 + outer_vox,
                )
            }
        };

        let z_v_mm = 1.95_f32; // >= 1.95mm solid floor under V-vertex
        let y_v_mm = (r_base_mm - 6.40).max(9.80);
        let y_outer_mm = y_v_mm + 3.35; // 3.35mm +45° front retaining lip
        let y_bolster_start_mm = 3.40_f32; // safely behind centered r=3.1mm magnet pocket!
        let z_bolster_top_mm = 8.40_f32;

        let bolster_cx_mm = ((bolster_ix0 + bolster_ix1) as f32 * 0.5) * mm_per_vox;
        let bolster_half_w_mm = ((bolster_ix1 - bolster_ix0) as f32 * 0.5) * mm_per_vox;

        // Step A: Positively sculpt solid Staunton Rear Cradle Housing Bolster (guaranteeing
        // full-height unbroken center divider rib and outer side walls anchored to z = 0!)
        let iy_start = (((cy_mm + y_bolster_start_mm) / mm_per_vox).floor() as usize).min(sy - 2);
        let iy_end = (((cy_mm + y_outer_mm + 0.5) / mm_per_vox).ceil() as usize).min(sy - 2);
        let iz_end = (((z_bolster_top_mm + 0.5) / mm_per_vox).ceil() as usize).min(sz - 2);

        for x in bolster_ix0..bolster_ix1.min(sx - 1) {
            let x_pos_mm = (x as f32 + 0.5) * mm_per_vox;
            let dx_bolster = (x_pos_mm - bolster_cx_mm).abs();
            // Gentle 45° outer-corner bevel ONLY inside outer side walls (never narrowing pockets)
            let corner_trim = (dx_bolster - (bolster_half_w_mm - 1.35)).max(0.0) * 0.75;
            let y_max_local = y_outer_mm - corner_trim;

            for y in iy_start..=iy_end {
                let dy_mm = (y as f32 + 0.5) * mm_per_vox - cy_mm;
                if dy_mm > y_max_local {
                    continue;
                }
                // Smoothly slope the center/outer walls parallel to (+1.75mm proud of) the +45° lip on the rear apron
                let z_cap_apron = if dy_mm > y_v_mm - 1.50 {
                    (z_v_mm + (dy_mm - y_v_mm).abs() + 2.15).min(z_bolster_top_mm)
                } else {
                    z_bolster_top_mm
                };
                let shoulder_trim = (dx_bolster - (bolster_half_w_mm - 1.10)).max(0.0) * 0.85;
                let z_max_local = (z_cap_apron - shoulder_trim).max(2.60);

                for z in 0..=iz_end {
                    let z_mm = (z as f32 + 0.5) * mm_per_vox;
                    if z_mm <= z_max_local {
                        occ[x * stride_x + y * stride_y + z] = true;
                    }
                }
            }
        }

        // Step B: Carve each 45°-tilted 90° V-Cradle channel + Thumb-Scoop Notch
        // strictly within `[p.ix0 .. p.ix1]`, leaving `[p0.ix1 .. p1.ix0]` 100% untouched solid!
        for p in &pockets {
            for x in p.ix0..p.ix1.min(sx - 1) {
                let dx_p_mm = (x as f32 + 0.5) * mm_per_vox - p.cx_mm;
                let iy_carve_min = (((cy_mm + y_v_mm - 5.30) / mm_per_vox).floor() as usize).min(sy - 2);
                let iy_carve_max = (((cy_mm + y_outer_mm + 1.5) / mm_per_vox).ceil() as usize).min(sy - 2);
                let iz_carve_max = ((p.top_clear_z_mm / mm_per_vox).ceil() as usize).min(sz - 2);

                for y in iy_carve_min..=iy_carve_max {
                    let dy_mm = (y as f32 + 0.5) * mm_per_vox - cy_mm;
                    let dy_v = dy_mm - y_v_mm;

                    let z_floor = if dy_v >= 0.0 {
                        let mut zf = z_v_mm + dy_v;
                        // Curved fingertip scoop notch in the outer lip (never below z_v_mm)
                        if dy_v > 1.75 && dx_p_mm.abs() < 1.65 {
                            let u = dx_p_mm / 1.65;
                            zf = (zf - 1.30 * (1.0 - u * u)).max(z_v_mm + 0.35);
                        }
                        zf
                    } else {
                        z_v_mm + (-dy_v)
                    };

                    for z in 1..=iz_carve_max {
                        let z_mm = (z as f32 + 0.5) * mm_per_vox;
                        // Above z = 7.25mm, follow the 45° upper-back clearance envelope of the tilted die
                        let y_back_env = if z_mm > 7.25 {
                            y_v_mm - (p.top_clear_z_mm - z_mm).max(0.60)
                        } else {
                            y_v_mm - 5.30
                        };
                        if dy_mm >= y_back_env && z_mm >= z_floor && z_mm <= p.top_clear_z_mm {
                            occ[x * stride_x + y * stride_y + z] = false;
                        }
                    }
                }
            }
        }

        // Step C: Measure actual solid wall thickness (in mm) between Left and Right cradles
        if pockets.len() == 2 {
            let wall_ix0 = pockets[0].ix1;
            let wall_ix1 = pockets[1].ix0;
            let iy_probe = (((cy_mm + y_v_mm) / mm_per_vox).round() as usize).min(sy - 1);
            let iz_probe = (((z_v_mm + 1.20) / mm_per_vox).round() as usize).min(sz - 1);
            let mut solid_cols = 0usize;
            for x in wall_ix0..wall_ix1 {
                if occ[x * stride_x + iy_probe * stride_y + iz_probe] {
                    solid_cols += 1;
                }
            }
            measured_wall_mm = (solid_cols as f32) * mm_per_vox;
        }
    }

    // 3. Optional Centered `6.2mm x 2.2mm` Underside Neodymium Magnet Recess
    if !is_prop && cfg.include_magnet_pocket {
        let r_mag_mm = 3.10_f32;
        let h_mag_vox = ((2.20 / mm_per_vox).round() as usize).clamp(2, sz.saturating_sub(4));
        for x in 0..sx {
            let dx = (x as f32 + 0.5) * mm_per_vox - cx_mm;
            if dx.abs() > r_mag_mm + mm_per_vox {
                continue;
            }
            for y in 0..sy {
                let dy = (y as f32 + 0.5) * mm_per_vox - cy_mm;
                if dx * dx + dy * dy <= r_mag_mm * r_mag_mm {
                    for z in 0..h_mag_vox {
                        occ[x * stride_x + y * stride_y + z] = false;
                    }
                }
            }
        }
    }

    // 4. Prune any stray crumbs, repair all 2x2 diagonal non-manifold saddles, and re-verify grounding
    keep_only_z0_grounded_component(&mut occ, sx, sy, sz);
    repair_2manifold_saddles(&mut occ, sx, sy, sz);
    keep_only_z0_grounded_component(&mut occ, sx, sy, sz);

    // 5. Extract T-junction-free coplanar-merged watertight 2-manifold boundary triangles & bounding box
    let (tris, bmin, bmax) =
        build_coplanar_voxel_stl_triangles(&occ, sx, sy, sz, mm_per_vox, cx_mm, cy_mm);

    // 6. Fast single-allocation binary `.STL` write
    let label = card.map_or_else(|| baked.model_name.clone(), |c| c.card_key.clone());
    write_binary_stl_triangles(out_stl, &label, measured_wall_mm, &tris)?;

    let footprint = if tris.is_empty() {
        (0.0, 0.0, 0.0)
    } else {
        (bmax[0] - bmin[0], bmax[1] - bmin[1], bmax[2] - bmin[2])
    };
    let desc = layout.summary_label(measured_wall_mm);

    Ok(StlExportReport {
        model_label: label,
        vox_slug: baked.model_name,
        output_path: out_stl.to_path_buf(),
        triangle_count: tris.len(),
        layout,
        resolved_cradle_desc: desc,
        min_center_wall_mm: measured_wall_mm,
        footprint_mm: footprint,
    })
}

/// Writes an array of `(normal, [v0, v1, v2])` triangles to a binary `.STL` file.
pub fn write_binary_stl_triangles(
    out_stl: &Path,
    label: &str,
    measured_wall_mm: f32,
    tris: &[([f32; 3], [[f32; 3]; 3])],
) -> Result<()> {
    if let Some(parent) = out_stl.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut raw_buf = Vec::with_capacity(84 + tris.len() * 50);
    let mut hdr = [0u8; 80];
    let title_bytes = format!(
        "Wildfront STL: {} | Wall {:.2}mm",
        label, measured_wall_mm
    );
    let copy_len = title_bytes.len().min(79);
    hdr[..copy_len].copy_from_slice(&title_bytes.as_bytes()[..copy_len]);
    raw_buf.extend_from_slice(&hdr);
    raw_buf.extend_from_slice(&(tris.len() as u32).to_le_bytes());

    for (n, verts) in tris {
        for c in n {
            raw_buf.extend_from_slice(&c.to_le_bytes());
        }
        for v in verts {
            for c in v {
                raw_buf.extend_from_slice(&c.to_le_bytes());
            }
        }
        raw_buf.extend_from_slice(&0u16.to_le_bytes());
    }

    let mut f = BufWriter::new(File::create(out_stl)?);
    f.write_all(&raw_buf)?;
    f.flush()?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FaceDir6 {
    PosZ,
    NegZ,
    PosX,
    NegX,
    PosY,
    NegY,
}

impl FaceDir6 {
    const ALL: [Self; 6] = [
        Self::PosZ,
        Self::NegZ,
        Self::PosX,
        Self::NegX,
        Self::PosY,
        Self::NegY,
    ];

    fn normal(self) -> [f32; 3] {
        match self {
            Self::PosZ => [0.0, 0.0, 1.0],
            Self::NegZ => [0.0, 0.0, -1.0],
            Self::PosX => [1.0, 0.0, 0.0],
            Self::NegX => [-1.0, 0.0, 0.0],
            Self::PosY => [0.0, 1.0, 0.0],
            Self::NegY => [0.0, -1.0, 0.0],
        }
    }

    fn flip_winding(self) -> bool {
        matches!(self, Self::NegZ | Self::NegX | Self::PosY)
    }

    fn plane_w(self, w: usize) -> usize {
        match self {
            Self::PosZ | Self::PosX | Self::PosY => w + 1,
            Self::NegZ | Self::NegX | Self::NegY => w,
        }
    }

    fn to_grid_corner(self, plane_w: usize, u: usize, v: usize) -> (usize, usize, usize) {
        match self {
            Self::PosZ | Self::NegZ => (u, v, plane_w),
            Self::PosX | Self::NegX => (plane_w, u, v),
            Self::PosY | Self::NegY => (u, plane_w, v),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct GreedyRect {
    dir: FaceDir6,
    plane_w: usize,
    u0: usize,
    u1: usize,
    v0: usize,
    v1: usize,
}

/// Builds T-junction-free greedy coplanar-merged boundary triangles for a 2-manifold voxel occupancy grid.
///
/// Pass 1 greedily merges maximal coplanar rectangles per axis slice across all 6 face directions.
/// Pass 2 marks every rectangle corner on the `(sx+1)*(sy+1)*(sz+1)` vertex lattice and subdivides
/// touching edges at shared corner vertices before triangulating, preserving strict 2-manifold half-edges,
/// exact signed volume, bounding boxes, and outward face normals.
pub fn build_coplanar_voxel_stl_triangles(
    occ: &[bool],
    sx: usize,
    sy: usize,
    sz: usize,
    mm_per_vox: f32,
    cx_mm: f32,
    cy_mm: f32,
) -> (Vec<([f32; 3], [[f32; 3]; 3])>, [f32; 3], [f32; 3]) {
    if sx == 0 || sy == 0 || sz == 0 || occ.len() != sx * sy * sz {
        return (Vec::new(), [0.0; 3], [0.0; 3]);
    }
    let stride_y = sz;
    let stride_x = sy * sz;

    let is_solid = |x: isize, y: isize, z: isize| -> bool {
        if x < 0 || y < 0 || z < 0 || (x as usize) >= sx || (y as usize) >= sy || (z as usize) >= sz {
            false
        } else {
            occ[(x as usize) * stride_x + (y as usize) * stride_y + (z as usize)]
        }
    };

    let mut bmin = [f32::MAX; 3];
    let mut bmax = [f32::MIN; 3];
    let mut any_solid = false;
    for x in 0..sx {
        for y in 0..sy {
            for z in 0..sz {
                if occ[x * stride_x + y * stride_y + z] {
                    any_solid = true;
                    let x0 = x as f32 * mm_per_vox - cx_mm;
                    let x1 = (x + 1) as f32 * mm_per_vox - cx_mm;
                    let y0 = y as f32 * mm_per_vox - cy_mm;
                    let y1 = (y + 1) as f32 * mm_per_vox - cy_mm;
                    let z0 = z as f32 * mm_per_vox;
                    let z1 = (z + 1) as f32 * mm_per_vox;
                    bmin[0] = bmin[0].min(x0);
                    bmax[0] = bmax[0].max(x1);
                    bmin[1] = bmin[1].min(y0);
                    bmax[1] = bmax[1].max(y1);
                    bmin[2] = bmin[2].min(z0);
                    bmax[2] = bmax[2].max(z1);
                }
            }
        }
    }
    if !any_solid {
        return (Vec::new(), [0.0; 3], [0.0; 3]);
    }

    // Pass 1: Greedy maximal coplanar rectangles per slice across all 6 directions
    let mut rects: Vec<GreedyRect> = Vec::with_capacity(sx * sy + sy * sz + sx * sz);
    for dir in FaceDir6::ALL {
        let (w_max, u_max, v_max) = match dir {
            FaceDir6::PosZ | FaceDir6::NegZ => (sz, sx, sy),
            FaceDir6::PosX | FaceDir6::NegX => (sx, sy, sz),
            FaceDir6::PosY | FaceDir6::NegY => (sy, sx, sz),
        };
        let mut mask = vec![false; u_max * v_max];
        for w in 0..w_max {
            mask.fill(false);
            let mut has_exposed = false;
            for v in 0..v_max {
                for u in 0..u_max {
                    let (cur_solid, nbr_solid) = match dir {
                        FaceDir6::PosZ => (
                            is_solid(u as isize, v as isize, w as isize),
                            is_solid(u as isize, v as isize, w as isize + 1),
                        ),
                        FaceDir6::NegZ => (
                            is_solid(u as isize, v as isize, w as isize),
                            is_solid(u as isize, v as isize, w as isize - 1),
                        ),
                        FaceDir6::PosX => (
                            is_solid(w as isize, u as isize, v as isize),
                            is_solid(w as isize + 1, u as isize, v as isize),
                        ),
                        FaceDir6::NegX => (
                            is_solid(w as isize, u as isize, v as isize),
                            is_solid(w as isize - 1, u as isize, v as isize),
                        ),
                        FaceDir6::PosY => (
                            is_solid(u as isize, w as isize, v as isize),
                            is_solid(u as isize, w as isize + 1, v as isize),
                        ),
                        FaceDir6::NegY => (
                            is_solid(u as isize, w as isize, v as isize),
                            is_solid(u as isize, w as isize - 1, v as isize),
                        ),
                    };
                    if cur_solid && !nbr_solid {
                        mask[u + v * u_max] = true;
                        has_exposed = true;
                    }
                }
            }
            if !has_exposed {
                continue;
            }
            let plane_w = dir.plane_w(w);
            for v in 0..v_max {
                let mut u = 0;
                while u < u_max {
                    if !mask[u + v * u_max] {
                        u += 1;
                        continue;
                    }
                    let mut w_len = 1usize;
                    while u + w_len < u_max && mask[(u + w_len) + v * u_max] {
                        w_len += 1;
                    }
                    let mut h_len = 1usize;
                    'expand_v: while v + h_len < v_max {
                        let row_off = (v + h_len) * u_max;
                        for du in 0..w_len {
                            if !mask[u + du + row_off] {
                                break 'expand_v;
                            }
                        }
                        h_len += 1;
                    }
                    for dv in 0..h_len {
                        let row_off = (v + dv) * u_max;
                        for du in 0..w_len {
                            mask[u + du + row_off] = false;
                        }
                    }
                    rects.push(GreedyRect {
                        dir,
                        plane_w,
                        u0: u,
                        u1: u + w_len,
                        v0: v,
                        v1: v + h_len,
                    });
                    u += w_len;
                }
            }
        }
    }

    // Pass 2: Mark all rectangle corner vertices on the (sx+1)*(sy+1)*(sz+1) grid
    let vy_stride = sz + 1;
    let vx_stride = (sy + 1) * vy_stride;
    let mut is_corner = vec![false; (sx + 1) * vx_stride];
    let mark_corner = |corners: &mut [bool], (gx, gy, gz): (usize, usize, usize)| {
        corners[gx * vx_stride + gy * vy_stride + gz] = true;
    };
    let check_corner = |corners: &[bool], (gx, gy, gz): (usize, usize, usize)| -> bool {
        corners[gx * vx_stride + gy * vy_stride + gz]
    };

    for r in &rects {
        mark_corner(&mut is_corner, r.dir.to_grid_corner(r.plane_w, r.u0, r.v0));
        mark_corner(&mut is_corner, r.dir.to_grid_corner(r.plane_w, r.u1, r.v0));
        mark_corner(&mut is_corner, r.dir.to_grid_corner(r.plane_w, r.u1, r.v1));
        mark_corner(&mut is_corner, r.dir.to_grid_corner(r.plane_w, r.u0, r.v1));
    }

    let to_world = |dir: FaceDir6, plane_w: usize, u: usize, v: usize| -> [f32; 3] {
        let (gx, gy, gz) = dir.to_grid_corner(plane_w, u, v);
        [
            gx as f32 * mm_per_vox - cx_mm,
            gy as f32 * mm_per_vox - cy_mm,
            gz as f32 * mm_per_vox,
        ]
    };

    let mut tris: Vec<([f32; 3], [[f32; 3]; 3])> = Vec::with_capacity(rects.len() * 3);
    let mut bottom: Vec<(usize, usize)> = Vec::with_capacity(32);
    let mut right: Vec<(usize, usize)> = Vec::with_capacity(32);
    let mut top: Vec<(usize, usize)> = Vec::with_capacity(32);
    let mut left: Vec<(usize, usize)> = Vec::with_capacity(32);

    let emit_tri = |tris: &mut Vec<([f32; 3], [[f32; 3]; 3])>,
                    dir: FaceDir6,
                    plane_w: usize,
                    a: (usize, usize),
                    b: (usize, usize),
                    c: (usize, usize)| {
        let n = dir.normal();
        let pa = to_world(dir, plane_w, a.0, a.1);
        let pb = to_world(dir, plane_w, b.0, b.1);
        let pc = to_world(dir, plane_w, c.0, c.1);
        if dir.flip_winding() {
            tris.push((n, [pa, pc, pb]));
        } else {
            tris.push((n, [pa, pb, pc]));
        }
    };

    for r in &rects {
        bottom.clear();
        right.clear();
        top.clear();
        left.clear();

        for u in r.u0..=r.u1 {
            if check_corner(&is_corner, r.dir.to_grid_corner(r.plane_w, u, r.v0)) {
                bottom.push((u, r.v0));
            }
            if check_corner(&is_corner, r.dir.to_grid_corner(r.plane_w, u, r.v1)) {
                top.push((u, r.v1));
            }
        }
        for v in r.v0..=r.v1 {
            if check_corner(&is_corner, r.dir.to_grid_corner(r.plane_w, r.u0, v)) {
                left.push((r.u0, v));
            }
            if check_corner(&is_corner, r.dir.to_grid_corner(r.plane_w, r.u1, v)) {
                right.push((r.u1, v));
            }
        }

        let p = bottom.len() - 1;
        let q = top.len() - 1;
        let l_cnt = left.len() - 1;
        let s = right.len() - 1;

        if p == 1 && q == 1 && l_cnt == 1 && s == 1 {
            emit_tri(&mut tris, r.dir, r.plane_w, (r.u0, r.v0), (r.u1, r.v0), (r.u1, r.v1));
            emit_tri(&mut tris, r.dir, r.plane_w, (r.u0, r.v0), (r.u1, r.v1), (r.u0, r.v1));
        } else {
            // 4-step T-junction-free corner-adjacent fan triangulation
            // 1. Fan left[0..=l_cnt] from bottom[1]
            for i in 0..l_cnt {
                emit_tri(&mut tris, r.dir, r.plane_w, bottom[1], left[i + 1], left[i]);
            }
            // 2. Fan bottom[1..=p] from top[0]
            for i in 1..p {
                emit_tri(&mut tris, r.dir, r.plane_w, top[0], bottom[i], bottom[i + 1]);
            }
            // 3. Fan right[0..=s] from top[q-1]
            for i in 0..s {
                emit_tri(&mut tris, r.dir, r.plane_w, top[q - 1], right[i], right[i + 1]);
            }
            // 4. Fan top[0..=q-1] from right[0]
            for j in 0..q.saturating_sub(1) {
                emit_tri(&mut tris, r.dir, r.plane_w, right[0], top[j + 1], top[j]);
            }
        }
    }

    (tris, bmin, bmax)
}

/// Exports a specific `UnitCardDef` (using its exact `STR` & `ENC` stats to size the 45° V-cradles).
pub fn export_card_to_stl(
    card: &UnitCardDef,
    vox_dir: &Path,
    out_stl: &Path,
    cfg: &TabletopPrintConfig,
) -> Result<StlExportReport> {
    let slug = card.default_vox_slug();
    let vox_path = vox_dir.join(format!("{slug}.vox"));
    export_vox_with_config(&vox_path, out_stl, Some(card), cfg)
}

/// Exports all unique cards in a `DeckList` into `out_dir`, prefixing each `.STL` with copy count
/// (`1x_`, `2x_`, `3x_`) and writing a `PRINT_MANIFEST.txt` checklist with exact dice requirements.
pub fn export_deck_to_stl_folder(
    deck: &DeckList,
    roster: &RosterManifest,
    vox_dir: &Path,
    out_dir: &Path,
    cfg: &TabletopPrintConfig,
) -> Result<Vec<StlExportReport>> {
    let cancel = AtomicBool::new(false);
    let (reps, _) = export_deck_to_stl_folder_cancellable(
        deck,
        roster,
        vox_dir,
        out_dir,
        cfg,
        &cancel,
        &mut |_, _, _| {},
    )?;
    Ok(reps)
}

/// Cancellable, progress-reporting variant of `export_deck_to_stl_folder` for background UI threads.
pub fn export_deck_to_stl_folder_cancellable(
    deck: &DeckList,
    roster: &RosterManifest,
    vox_dir: &Path,
    out_dir: &Path,
    cfg: &TabletopPrintConfig,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(usize, usize, &str),
) -> Result<(Vec<StlExportReport>, bool)> {
    fs::create_dir_all(out_dir)?;
    let lookup = roster.index_by_key();
    let entries_to_export: Vec<_> = deck
        .entries
        .iter()
        .filter(|e| e.quantity > 0)
        .filter_map(|e| lookup.get(e.card_key.as_str()).map(|&card| (e.quantity, card)))
        .collect();
    let total = entries_to_export.len();
    let mut reports = Vec::with_capacity(total);
    let mut total_d6_str = 0u32;
    let mut total_d6_mun = 0u32;
    let mut total_d10_str = 0u32;
    let mut total_d10_mun = 0u32;

    for (idx, (qty, card)) in entries_to_export.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Ok((reports, true));
        }
        on_progress(idx, total, &card.card_key);
        let safe = sanitize_filename_slug(&card.card_key);
        let fname = format!(
            "{}x_{}__STR{}_ENC{}.stl",
            qty, safe, card.strength_hp, card.encumbrance
        );
        let rep = export_card_to_stl(card, vox_dir, &out_dir.join(fname), cfg)?;
        let q = u32::from(qty);
        match rep.layout {
            ResolvedCradleLayout::None => {}
            ResolvedCradleLayout::SingleCenteredD6 { .. } => total_d6_str += q,
            ResolvedCradleLayout::SingleCenteredD10 { .. } => total_d10_str += q,
            ResolvedCradleLayout::DualD6 { .. } => {
                total_d6_str += q;
                total_d6_mun += q;
            }
            ResolvedCradleLayout::AsymmetricD6AndD10 { .. } => {
                total_d6_str += q;
                total_d10_mun += q;
            }
        }
        reports.push(rep);
        on_progress(idx + 1, total, &card.card_key);
    }

    let mut manifest = String::new();
    manifest.push_str(&format!(
        "WILDRONT BATTLEGROUNDS — 3D PRINT DECK MANIFEST\nDeck: {}\nHex Scale: {:.1}mm flat-to-flat | Min Center Wall Setting: {:.2}mm\n\n",
        deck.deck_name,
        cfg.target_hex_flat_mm,
        cfg.center_wall_mm.max(MIN_PRINTABLE_CENTER_WALL_MM),
    ));
    manifest.push_str(&format!(
        "DICE HARDWARE NEEDED FOR FULL DEPLOYMENT:\n  - Crimson 7mm Micro-D6 (STR): {}\n  - Crimson 10mm Mini-D10 (HQ/Node STR): {}\n  - Amber 7mm Micro-D6 (Squad MUN): {}\n  - Amber 10mm Mini-D10 (Hauler/Depot MUN): {}\n\n",
        total_d6_str, total_d10_str, total_d6_mun, total_d10_mun
    ));
    manifest.push_str("EXPORTED .STL FILES:\n");
    for r in &reports {
        manifest.push_str(&format!(
            "  - {}  [{:.1} x {:.1} x {:.1} mm | {} tris | {}]\n",
            r.output_path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
            r.footprint_mm.0,
            r.footprint_mm.1,
            r.footprint_mm.2,
            r.triangle_count,
            r.resolved_cradle_desc
        ));
    }
    fs::write(out_dir.join("PRINT_MANIFEST.txt"), manifest)?;
    Ok((reports, false))
}

/// Backwards-compatible single `.vox` export using default `TabletopPrintConfig` with `target_hex_flat_mm`.
pub fn export_vox_to_stl(vox_path: &Path, out_stl: &Path, target_hex_flat_mm: f32) -> Result<usize> {
    let cfg = TabletopPrintConfig {
        target_hex_flat_mm,
        ..TabletopPrintConfig::default()
    };
    let rep = export_vox_with_config(vox_path, out_stl, None, &cfg)?;
    Ok(rep.triangle_count)
}

/// Batch-exports all `.vox` files in `vox_dir` into `out_stl_dir/<slug>.stl`.
pub fn export_vox_directory_to_stl(
    vox_dir: &Path,
    out_stl_dir: &Path,
    target_hex_flat_mm: f32,
) -> Result<usize> {
    let cfg = TabletopPrintConfig {
        target_hex_flat_mm,
        ..TabletopPrintConfig::default()
    };
    export_vox_directory_with_config(vox_dir, out_stl_dir, &cfg)
}

/// Batch-exports all `.vox` files in `vox_dir` using custom `TabletopPrintConfig`.
pub fn export_vox_directory_with_config(
    vox_dir: &Path,
    out_stl_dir: &Path,
    cfg: &TabletopPrintConfig,
) -> Result<usize> {
    let cancel = AtomicBool::new(false);
    let (count, _) = export_vox_directory_cancellable(
        vox_dir,
        out_stl_dir,
        cfg,
        &cancel,
        &mut |_, _, _| {},
    )?;
    Ok(count)
}

/// Cancellable, progress-reporting variant of `export_vox_directory_with_config`.
pub fn export_vox_directory_cancellable(
    vox_dir: &Path,
    out_stl_dir: &Path,
    cfg: &TabletopPrintConfig,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(usize, usize, &str),
) -> Result<(usize, bool)> {
    fs::create_dir_all(out_stl_dir)?;
    let mut entries: Vec<_> = fs::read_dir(vox_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("vox"))
        .collect();
    entries.sort();

    let total = entries.len();
    let mut count = 0usize;
    for (idx, path) in entries.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Ok((count, true));
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("model").to_string();
        on_progress(idx, total, &stem);
        export_vox_with_config(&path, &out_stl_dir.join(format!("{stem}.stl")), None, cfg)?;
        count += 1;
        on_progress(idx + 1, total, &stem);
    }
    Ok((count, false))
}

/// Exports all `hex_*.vox` modular tabletop terrain hexes (including `hex_04_hydro_breach_overlay`)
/// scaled to exact `config.target_hex_flat_mm` (`45.0mm` default).
pub fn export_terrain_hexes_with_config(
    vox_dir: &Path,
    out_stl_dir: &Path,
    config: &TabletopPrintConfig,
) -> Result<usize> {
    let cancel = AtomicBool::new(false);
    let (count, _) = export_terrain_hexes_cancellable(
        vox_dir,
        out_stl_dir,
        config,
        &cancel,
        &mut |_, _, _| {},
    )?;
    Ok(count)
}

/// Cancellable, progress-reporting variant of `export_terrain_hexes_with_config`.
pub fn export_terrain_hexes_cancellable(
    vox_dir: &Path,
    out_stl_dir: &Path,
    config: &TabletopPrintConfig,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(usize, usize, &str),
) -> Result<(usize, bool)> {
    fs::create_dir_all(out_stl_dir)?;
    let mut entries: Vec<_> = fs::read_dir(vox_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|s| s.to_str()) == Some("vox")
                && p.file_stem().and_then(|s| s.to_str()).map_or(false, |st| st.starts_with("hex_"))
        })
        .collect();
    entries.sort();

    let mut tile_cfg = *config;
    tile_cfg.cradle_mode = CradleMode::SculptOnlyNoCradles;
    let total = entries.len();
    let mut count = 0usize;
    for (idx, path) in entries.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Ok((count, true));
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("hex_tile").to_string();
        on_progress(idx, total, &stem);
        let out_p = out_stl_dir.join(format!("{stem}_{:.0}mm.stl", tile_cfg.target_hex_flat_mm));
        export_vox_with_config(&path, &out_p, None, &tile_cfg)?;
        count += 1;
        on_progress(idx + 1, total, &stem);
    }
    Ok((count, false))
}
