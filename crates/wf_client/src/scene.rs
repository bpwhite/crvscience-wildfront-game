//! Builds GPU storage tables (`GpuHexOverlay` + `GpuUnitInstance`) from the live
//! `ClientMatchState`, packing hover/selection/move/attack reticles, towering Staunton
//! chess-proportioned voxel rigs (`72%–79%` hex-width base, `1.6x–1.9x` height), AND 3D Terrain Props.

use std::collections::{BTreeMap, HashSet};
use std::f32::consts::PI;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use hexx::Hex;
use wf_core::voxel::CompiledModelHandle;
use wf_core::{BattleHexMap, Morphology};
use crate::game_session::{ClientMatchState, CommandMode};
use crate::vfx::VfxManager;

pub const HEX_WORLD_RADIUS: f32 = 2.4;
pub const MAX_GPU_HEX_RADIUS: i32 = 9;
pub const GPU_HEX_STRIDE: usize = (MAX_GPU_HEX_RADIUS * 2 + 1) as usize; // 19x19 = 361

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct GpuHexOverlay {
    pub q: i32,
    pub r: i32,
    pub elevation_tier: i32,
    pub packed_flags: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct GpuUnitInstance {
    pub world_to_voxel: [[f32; 4]; 4],
    pub voxel_to_world_scale: f32,
    pub root_node_index: u32,
    pub team_id: u32,
    pub _pad0: u32,
    pub world_aabb_min: [f32; 4],
    pub world_aabb_max: [f32; 4],
    pub size_voxels: [f32; 4],
}

pub fn axial_to_world_xz(hex: Hex) -> (f32, f32) {
    let q = hex.x as f32;
    let r = hex.y as f32;
    let wx = HEX_WORLD_RADIUS * 3.0_f32.sqrt() * (q + r * 0.5);
    let wz = HEX_WORLD_RADIUS * 1.5 * r;
    (wx, wz)
}

pub fn tile_surface_height(morph: Morphology, elev: u8) -> (i32, f32) {
    match morph {
        Morphology::DeepRiver | Morphology::OpenOcean => (-1, -0.24),
        Morphology::RavineChasm => (-1, -0.46),
        Morphology::ShallowFord => (0, -0.05),
        _ => (i32::from(elev), f32::from(elev) * 0.55),
    }
}

/// Maps each `136..148`-wide Staunton chess-piece rig so its beveled octagonal foot spans
/// **72%–79% of the `4.16m` hex width** (`3.00m–3.27m`), nesting right inside the hex's
/// tactical status ring while rising `5.1m–5.8m` vertically (`1.62x–1.90x` height/base).
pub fn unit_chess_voxel_scale(vox_slug: &str) -> f32 {
    match vox_slug {
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
        _ => 0.0234,
    }
}

fn compute_ford_bridge_yaw(map: &BattleHexMap, h: Hex) -> f32 {
    let (hx, hz) = axial_to_world_xz(h);
    for nb in h.all_neighbors() {
        if let Some(nt) = map.tiles.get(&nb) {
            if !nt.morphology.is_water() && nt.morphology != Morphology::RavineChasm {
                let (nx, nz) = axial_to_world_xz(nb);
                return (nx - hx).atan2(nz - hz);
            }
        }
    }
    0.0
}

fn make_gpu_voxel_instance(
    handle: &CompiledModelHandle,
    world_pos: Vec3,
    yaw_radians: f32,
    v_scale: f32,
    team_id: u32,
    hp_tag: u32,
) -> (GpuUnitInstance, Vec3, Vec3) {
    let sv = Vec3::new(
        handle.size_voxels[0] as f32,
        handle.size_voxels[1] as f32,
        handle.size_voxels[2] as f32,
    );
    let center_offset = Mat4::from_translation(Vec3::new(-sv.x * 0.5, 0.0, -sv.z * 0.5));
    let voxel_to_world = Mat4::from_translation(world_pos)
        * Mat4::from_rotation_y(yaw_radians)
        * Mat4::from_scale(Vec3::splat(v_scale))
        * center_offset;
    let world_to_voxel = voxel_to_world.inverse();

    let mut bmin = Vec3::splat(f32::MAX);
    let mut bmax = Vec3::splat(f32::MIN);
    for cx in [0.0, sv.x] {
        for cy in [0.0, sv.y] {
            for cz in [0.0, sv.z] {
                let wp = voxel_to_world.transform_point3(Vec3::new(cx, cy, cz));
                bmin = bmin.min(wp);
                bmax = bmax.max(wp);
            }
        }
    }
    let ray_pad = Vec3::splat(0.05);
    let aabb_min = bmin - ray_pad;
    let aabb_max = bmax + ray_pad;

    let inst = GpuUnitInstance {
        world_to_voxel: world_to_voxel.to_cols_array_2d(),
        voxel_to_world_scale: v_scale,
        root_node_index: handle.root_node_index,
        team_id,
        _pad0: hp_tag,
        world_aabb_min: [aabb_min.x, aabb_min.y, aabb_min.z, 0.0],
        world_aabb_max: [aabb_max.x, aabb_max.y, aabb_max.z, 0.0],
        size_voxels: [sv.x, sv.y, sv.z, 0.0],
    };
    (inst, aabb_min, aabb_max)
}

pub fn build_live_gpu_scene(
    session: &ClientMatchState,
    vfx: &VfxManager,
    handles: &BTreeMap<String, CompiledModelHandle>,
    show_terrain_props: bool,
) -> (
    Vec<GpuHexOverlay>,
    Vec<GpuUnitInstance>,
    Vec<(usize, Hex, Vec3, Vec3)>,
) {
    let mut hex_table = vec![
        GpuHexOverlay {
            q: 0,
            r: 0,
            elevation_tier: -2,
            packed_flags: 0,
        };
        GPU_HEX_STRIDE * GPU_HEX_STRIDE
    ];

    let viewer = session.viewing_team();
    let supply_hexes = session.team_supply_hexes(viewer);
    let node_hexes = session.team_node_hexes(viewer);
    let vis_hexes = session.compute_visible_hexes(viewer);
    let true_sight = session.compute_true_sight_hexes(viewer);
    let explored = &session.explored_hexes[usize::from(viewer.saturating_sub(1)).min(1)];

    let (green_highlights, red_highlights): (HashSet<Hex>, HashSet<Hex>) = match session.mode {
        CommandMode::DeployHandCard(h_idx) => {
            let set = session
                .active_hand()
                .get(h_idx)
                .map(|k| session.valid_deploy_hexes(k))
                .unwrap_or_default();
            (set, HashSet::new())
        }
        CommandMode::MoveSelectedUnit => {
            let set = session
                .selected_hex
                .and_then(|sh| session.entity_at(sh))
                .map(|idx| session.valid_move_hexes(idx))
                .unwrap_or_default();
            (set, HashSet::new())
        }
        CommandMode::AttackWithSelectedUnit => {
            let set = session
                .selected_hex
                .and_then(|sh| session.entity_at(sh))
                .map(|idx| session.valid_attack_hexes(idx))
                .unwrap_or_default();
            (HashSet::new(), set)
        }
        CommandMode::AbilityWithSelectedUnit => {
            let set = session
                .selected_hex
                .and_then(|sh| session.entity_at(sh))
                .map(|idx| session.valid_ability_hexes(idx))
                .unwrap_or_default();
            (HashSet::new(), set)
        }
        CommandMode::Inspect => (HashSet::new(), HashSet::new()),
    };

    for r in -MAX_GPU_HEX_RADIUS..=MAX_GPU_HEX_RADIUS {
        for q in -MAX_GPU_HEX_RADIUS..=MAX_GPU_HEX_RADIUS {
            let idx = ((q + MAX_GPU_HEX_RADIUS) as usize)
                + ((r + MAX_GPU_HEX_RADIUS) as usize) * GPU_HEX_STRIDE;
            let h = Hex::new(q, r);
            let Some(tile) = session.scenario.map.tiles.get(&h) else {
                hex_table[idx] = GpuHexOverlay {
                    q,
                    r,
                    elevation_tier: -2,
                    packed_flags: 0,
                };
                continue;
            };

            let (tier, _) = tile_surface_height(tile.morphology, tile.elevation);
            let mut flags = 0u32;
            if supply_hexes.contains(&h) {
                flags |= 1;
            }
            if node_hexes.contains(&h) {
                flags |= 2;
            }
            if tile.morphology == Morphology::RadiationFallout {
                flags |= 4;
            }
            if tile.objective_tag > 0 {
                flags |= 8;
            }
            if session.hovered_hex == Some(h) {
                flags |= 16;
            }
            if session.selected_hex == Some(h) {
                flags |= 32;
            }
            if green_highlights.contains(&h) {
                flags |= 64;
            }
            if red_highlights.contains(&h) {
                flags |= 128;
            }
            let fog_state: u32 = if !session.fog_enabled {
                0
            } else if session.hotseat_handoff_pending {
                2
            } else if vis_hexes.contains(&h) {
                0
            } else if explored.contains(&h) {
                1
            } else {
                2
            };
            if fog_state == 2 {
                flags &= !(1 | 2 | 4);
            }
            flags |= ((tile.biome as u32) & 0xFF) << 8;
            flags |= ((tile.morphology as u32) & 0xFF) << 16;
            flags |= (fog_state & 3) << 24;

            hex_table[idx] = GpuHexOverlay {
                q,
                r,
                elevation_tier: tier,
                packed_flags: flags,
            };
        }
    }

    let mut instances = Vec::with_capacity(192);
    let mut unit_occupied_hexes = HashSet::new();
    let mut pick_boxes = Vec::with_capacity(session.entities.len());

    // 1. Towering Staunton Chess-Proportioned Units & Installations (72%-79% hex base!)
    for (slot_idx, ent) in session.entities.iter().enumerate() {
        if !session.is_entity_visible_to(slot_idx, viewer, &vis_hexes, &true_sight) {
            continue;
        }
        unit_occupied_hexes.insert(ent.hex);
        let Some(handle) = handles.get(&ent.vox_slug) else {
            continue;
        };
        let tile_morph = session
            .scenario
            .map
            .tiles
            .get(&ent.hex)
            .map(|t| (t.morphology, tile_surface_height(t.morphology, t.elevation).1))
            .unwrap_or((Morphology::Flat, 0.0));
        let mut base_y = tile_morph.1;
        if show_terrain_props && tile_morph.0 == Morphology::ShallowFord {
            base_y += 0.41;
        }

        // All 25 chess-piece rigs now include a turned 72%-79% Staunton base collar on z=0,
        // with airborne aircraft/aerostats soaring high at the top of their 226..248-voxel column!
        let v_scale = unit_chess_voxel_scale(ent.vox_slug.as_str());
        let hover = if ent.card.can_fly {
            (vfx.clock_seconds * 2.4 + (ent.uid as f32) * 1.3).sin().abs() * 0.05
        } else {
            0.0
        };

        let (wx, wz) = axial_to_world_xz(ent.hex);
        let impulse = vfx.unit_impulse_offset(ent.uid);
        let world_pos = Vec3::new(wx + impulse.x, base_y + hover, wz + impulse.z);

        let (inst, pick_min, pick_max) = make_gpu_voxel_instance(
            handle,
            world_pos,
            ent.yaw_radians,
            v_scale,
            u32::from(ent.team_id),
            u32::from(ent.unit.current_str_hp),
        );
        pick_boxes.push((slot_idx, ent.hex, pick_min, pick_max));
        instances.push(inst);
    }

    // 2. Procedural 3D Voxel Terrain Props (`team_id = 0`)
    if show_terrain_props {
        let mut sorted_tiles: Vec<(Hex, Morphology, u8)> = session
            .scenario
            .map
            .tiles
            .iter()
            .map(|(&h, t)| (h, t.morphology, t.elevation))
            .collect();
        sorted_tiles.sort_by_key(|(h, m, _)| {
            let prio = match m {
                Morphology::MountainPeak => 0,
                Morphology::ShallowFord => 1,
                Morphology::ArmoredDam => 2,
                Morphology::BunkerRuins => 3,
                Morphology::RadiationFallout => 4,
                Morphology::RoughCover => 5,
                _ => 9,
            };
            (prio, h.y, h.x)
        });

        for (h, morph, elev) in sorted_tiles {
            if instances.len() >= 252 {
                break;
            }
            if session.fog_enabled && (session.hotseat_handoff_pending || (!vis_hexes.contains(&h) && !explored.contains(&h))) {
                continue;
            }
            // Keep ShallowFord bridges & ArmoredDam causeways under units, while hiding surface clutter
            // on occupied tiles so 75%-base Staunton pedestals never intersect rocks/walls!
            let is_breached_canal = session.breached_canal_hexes.contains(&h);
            if unit_occupied_hexes.contains(&h) && !is_breached_canal && !matches!(morph, Morphology::ShallowFord | Morphology::ArmoredDam) {
                continue;
            }

            let prop_spec: Option<(&str, f32, f32)> = if is_breached_canal {
                Some(("hex_04_hydro_breach_overlay", 0.0288675, -0.18))
            } else {
                match morph {
                Morphology::MountainPeak => Some(("prop_mountain_crag", 0.0255, -0.04)),
                Morphology::BunkerRuins => Some(("prop_bunker_ruins", 0.0315, -0.02)),
                Morphology::RadiationFallout => Some(("prop_fallout_crystals", 0.0315, -0.02)),
                Morphology::ShallowFord => Some(("prop_ford_bridge", 0.0315, -0.06)),
                Morphology::RoughCover => Some(("prop_rough_cover", 0.0315, -0.02)),
                Morphology::ArmoredDam => Some(("prop_armored_dam", 0.0315, -0.02)),
                _ => None,
                }
            };
            let Some((slug, p_scale, y_nudge)) = prop_spec else {
                continue;
            };
            let Some(handle) = handles.get(slug) else {
                continue;
            };

            let (_, surf_y) = tile_surface_height(morph, elev);
            let (wx, wz) = axial_to_world_xz(h);
            let yaw = if morph == Morphology::ShallowFord {
                compute_ford_bridge_yaw(&session.scenario.map, h)
            } else {
                let rot_step = (h.x * 7 + h.y * 13).rem_euclid(6) as f32;
                rot_step * (PI / 3.0)
            };

            let (inst, _, _) = make_gpu_voxel_instance(
                handle,
                Vec3::new(wx, surf_y + y_nudge, wz),
                yaw,
                p_scale,
                0,
                0,
            );
            instances.push(inst);
        }
    }

    (hex_table, instances, pick_boxes)
}
