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

//! Immediate-mode `egui 0.37` Tactical HUD, Unit/Hex Inspector, Graveyard Queue,
//! Quicksave/Quickload Controls, Scenario Victory/Defeat Modal, and 5-Card Hand UI.

use egui::{Align2, Color32, Frame, Margin, Order, RichText, ScrollArea, Stroke, Ui, Window};
use wf_core::combat::{required_ranged_hit_roll, CombatContext};
use wf_core::{OrganismFaction, ScenarioId};
use crate::audio::{SfxCue, SpatialAudioEngine};
use crate::game_session::{ClientMatchState, CommandMode};
use crate::vfx::VfxManager;

pub struct HudActions {
    pub switch_scenario: Option<ScenarioId>,
    pub rematch_current: bool,
    pub roll_new_seed: bool,
    pub toggle_overlays: bool,
    pub toggle_unit_badges: bool,
    pub toggle_terrain_props: bool,
    pub toggle_deck_builder: bool,
    pub toggle_stl_studio: bool,
    pub toggle_dice_roller: bool,
    pub toggle_db_manager: bool,
    pub toggle_map_editor: bool,
    pub toggle_replay_theater: bool,
    pub cycle_atmosphere: bool,
    pub toggle_main_menu: bool,
    pub toggle_codex: bool,
    pub quicksave: bool,
    pub quickload: bool,
    pub switch_deck: Option<usize>,
    pub end_turn: bool,
}

impl Default for HudActions {
    fn default() -> Self {
        Self {
            switch_scenario: None,
            rematch_current: false,
            roll_new_seed: false,
            toggle_overlays: false,
            toggle_unit_badges: false,
            toggle_terrain_props: false,
            toggle_deck_builder: false,
            toggle_stl_studio: false,
            toggle_dice_roller: false,
            toggle_db_manager: false,
            toggle_map_editor: false,
            toggle_replay_theater: false,
            cycle_atmosphere: false,
            toggle_main_menu: false,
            toggle_codex: false,
            quicksave: false,
            quickload: false,
            switch_deck: None,
            end_turn: false,
        }
    }
}

fn faction_badge_color(fac: OrganismFaction) -> Color32 {
    match fac {
        OrganismFaction::Human => Color32::from_rgb(56, 132, 184),
        OrganismFaction::Mammal => Color32::from_rgb(186, 118, 52),
        OrganismFaction::Reptile => Color32::from_rgb(76, 154, 72),
        OrganismFaction::Plant => Color32::from_rgb(44, 168, 102),
        OrganismFaction::Bird => Color32::from_rgb(92, 162, 224),
        OrganismFaction::Arachnid => Color32::from_rgb(148, 78, 168),
        OrganismFaction::Insect => Color32::from_rgb(198, 148, 42),
        OrganismFaction::Fish => Color32::from_rgb(38, 158, 178),
        OrganismFaction::Cnidarian => Color32::from_rgb(196, 74, 154),
        OrganismFaction::Amphibian => Color32::from_rgb(84, 182, 132),
    }
}

pub fn draw_tactical_hud(
    root_ui: &mut Ui,
    session: &mut ClientMatchState,
    vfx: &mut VfxManager,
    audio: &mut SpatialAudioEngine,
    show_overlays: bool,
    show_unit_badges: bool,
    show_terrain_props: bool,
) -> HudActions {
    let mut actions = HudActions::default();

    // 1. TOP STRATEGIC BAR
    egui::Panel::top("wf_top_command_bar")
        .frame(
            Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(15, 26, 32, 238))
                .inner_margin(Margin::symmetric(14, 8))
                .stroke(Stroke::new(1.0, Color32::from_rgb(45, 78, 92))),
        )
        .show(root_ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("🏠 Menu").on_hover_text("Open 2072 Main Menu & TCP Multiplayer (Esc)").clicked() {
                    actions.toggle_main_menu = true;
                }
                if ui.button("📖 Codex (F1)").on_hover_text("Open Rules, Faction Doctrines & Hotkey Codex (F1)").clicked() {
                    actions.toggle_codex = true;
                }
                if ui.button("🖨 STL (F8)").on_hover_text("Open 3D Print Tabletop .STL Studio with Recessed Micro-Dice V-Cradles (F8)").clicked() {
                    actions.toggle_stl_studio = true;
                }
                if ui.button("🎲 Dice (R)").on_hover_text("Open Interactive Simulated Combat Dice Roller & Round Logistics Hub (R)").clicked() {
                    actions.toggle_dice_roller = true;
                }
                if ui.button("🗄 DB (F2)").on_hover_text("Open 152-Card & 170-Model Database Manager (F2)").clicked() {
                    actions.toggle_db_manager = true;
                }
                if ui.button("🗺 Map (F3)").on_hover_text("Open Live 3D Hex Map & Scenario Editor Studio (F3)").clicked() {
                    actions.toggle_map_editor = true;
                }
                if ui.button("🎬 Replay (F4)").on_hover_text("Open Turn-by-Turn Replay Scrubber, Tactical Undo & EHP Telemetry (F4)").clicked() {
                    actions.toggle_replay_theater = true;
                }
                if ui.button("🌤 Sky (L)").on_hover_text("Cycle GPU Atmosphere: Clear Day / Dusk / Orbital Storm / Night Bio-Glow (L)").clicked() {
                    actions.cycle_atmosphere = true;
                }
                ui.separator();
                let viewer = session.viewing_team();
                let team_lbl = if let Some(nt) = session.online_local_team {
                    if session.active_team == nt {
                        format!("🌐 TCP P{} (YOUR TURN {})", nt, session.turn_number)
                    } else {
                        format!("⏳ TCP P{} (WAITING P{} T{})", nt, session.active_team, session.turn_number)
                    }
                } else if session.hotseat_mode {
                    format!("P{} TURN {}", viewer, session.turn_number)
                } else {
                    format!("TURN {}", session.turn_number)
                };
                let team_col = if viewer == 2 {
                    Color32::from_rgb(255, 138, 118)
                } else {
                    Color32::from_rgb(112, 238, 255)
                };
                ui.label(RichText::new(team_lbl).strong().size(15.5).color(team_col));
                let act_rem = session.economy.actions_remaining.get(&viewer).copied().unwrap_or(0);
                let home_mun = session.economy.home_base_munitions.get(&viewer).copied().unwrap_or(0);

                ui.label(
                    RichText::new(format!("⚡ Actions: {}", act_rem))
                        .strong()
                        .color(Color32::from_rgb(82, 224, 245)),
                );
                ui.separator();
                ui.label(
                    RichText::new(format!("📦 Munitions: {}", home_mun))
                        .strong()
                        .color(Color32::from_rgb(246, 178, 54)),
                );
                ui.separator();
                ui.label(
                    RichText::new(session.objective_summary_banner())
                        .small()
                        .strong()
                        .color(Color32::from_rgb(244, 214, 98)),
                );
                ui.separator();

                if ui
                    .button(if show_overlays { "🌐 Auras: ON" } else { "🌐 Auras: OFF" })
                    .clicked()
                {
                    actions.toggle_overlays = true;
                }
                if ui
                    .button(if show_unit_badges { "🏷 Bars: ON" } else { "🏷 Bars: OFF" })
                    .on_hover_text("Toggle floating 3D unit HP/Armor/Munition badges (H)")
                    .clicked()
                {
                    actions.toggle_unit_badges = true;
                }
                if ui
                    .button(if show_terrain_props { "🌲 Props: ON" } else { "🌲 Props: OFF" })
                    .on_hover_text("Toggle 3D voxel crags, ruins, fallout crystals & ford bridges (P)")
                    .clicked()
                {
                    actions.toggle_terrain_props = true;
                }
                if ui
                    .button(if session.fog_enabled { "🌫 Fog: ON" } else { "🌫 Fog: OFF" })
                    .on_hover_text("Toggle Radar Range & Stealth Fog-of-War masking (F)")
                    .clicked()
                {
                    session.fog_enabled = !session.fog_enabled;
                }
                if ui
                    .button(if session.hotseat_mode { "👥 1v1 Hotseat" } else { "🤖 vs AI" })
                    .on_hover_text("Switch between Solo vs AI and 2-Player Local Hotseat (M)")
                    .clicked()
                {
                    session.hotseat_mode = !session.hotseat_mode;
                    session.hotseat_handoff_pending = false;
                    session.mode = CommandMode::Inspect;
                }

                egui::ComboBox::from_id_salt("scenario_combo")
                    .selected_text(&session.scenario.title)
                    .show_ui(ui, |ui| {
                        for sid in ScenarioId::ALL {
                            if ui
                                .selectable_label(session.scenario.scenario_id == sid, sid.title())
                                .clicked()
                            {
                                actions.switch_scenario = Some(sid);
                            }
                        }
                    });

                if ui.button("🎲 Seed (N)").clicked() {
                    actions.roll_new_seed = true;
                }
                if ui
                    .button("💾 Save (F5)")
                    .on_hover_text("Quicksave full match & RNG state to saves/quicksave.ron")
                    .clicked()
                {
                    actions.quicksave = true;
                }
                if ui
                    .button("📂 Load (F9)")
                    .on_hover_text("Quickload match from saves/quicksave.ron")
                    .clicked()
                {
                    actions.quickload = true;
                }

                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("🃏 Deck Builder (B)")
                                .strong()
                                .color(Color32::from_rgb(220, 248, 255)),
                        )
                        .fill(Color32::from_rgb(32, 96, 126)),
                    )
                    .clicked()
                {
                    audio.play_ui(SfxCue::UiCardTurn);
                    actions.toggle_deck_builder = true;
                }

                let hw_online = audio.is_hardware_online();
                let audio_lbl = if audio.muted || !hw_online { "🔇" } else { "🔊" };
                let audio_tip = if hw_online {
                    "Toggle 3D Spatial Audio"
                } else {
                    "No audio hardware output detected (silent fallback)"
                };
                if ui.button(audio_lbl).on_hover_text(audio_tip).clicked() {
                    audio.muted = !audio.muted;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let end_enabled = session.can_local_player_act();
                    if ui
                        .add_enabled(
                            end_enabled,
                            egui::Button::new(
                                RichText::new("⏭ End Turn (Space)").strong().color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(196, 92, 28)),
                        )
                        .clicked()
                    {
                        actions.end_turn = true;
                    }
                    if let Some((toast_msg, _)) = &session.status_toast {
                        ui.label(
                            RichText::new(toast_msg)
                                .small()
                                .strong()
                                .color(Color32::from_rgb(112, 248, 178)),
                        );
                    }
                });
            });
        });

    // 2. BOTTOM 5-CARD HAND & DECK DEPLOYMENT TRAY WITH 3 COMMAND CARDS ACTION BAR
    egui::Panel::bottom("wf_card_hand_tray")
        .frame(
            Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(14, 22, 28, 240))
                .inner_margin(Margin::symmetric(14, 10))
                .stroke(Stroke::new(1.0, Color32::from_rgb(45, 78, 92))),
        )
        .show(root_ui, |ui| {
            // COMMAND CARDS ACTION TRAY
            let viewing_team = session.viewing_team();
            let team_deck = session.decks[(viewing_team - 1).min(1) as usize].clone();
            let fac_card = team_deck.faction_card();
            let bio_card = team_deck.primary_biotype_card();
            let is_pure = team_deck.is_pure_biotype();

            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("3 COMMAND CARDS:").strong().color(Color32::from_rgb(112, 232, 248)));

                let fac_act_ready = session.can_local_player_act()
                    && !session.economy.faction_ability_used.get(&viewing_team).copied().unwrap_or(false)
                    && session.economy.actions_remaining.get(&viewing_team).copied().unwrap_or(0) >= fac_card.active_action_cost
                    && session.economy.home_base_munitions.get(&viewing_team).copied().unwrap_or(0) >= u16::from(fac_card.active_munition_cost);

                let fac_btn = egui::Button::new(
                    RichText::new(format!("🇦🇺 {} ({} Act, {} Mun)", fac_card.active_ability_name, fac_card.active_action_cost, fac_card.active_munition_cost))
                        .strong()
                        .color(Color32::WHITE),
                )
                .fill(if fac_act_ready { Color32::from_rgb(38, 128, 168) } else { Color32::from_rgb(55, 68, 76) });

                if ui.add_enabled(fac_act_ready, fac_btn)
                    .on_hover_text(format!("{}
Doctrine: {}
↳ {}", fac_card.title, fac_card.command_doctrine, fac_card.active_effect_summary))
                    .clicked()
                {
                    session.execute_faction_command_ability(viewing_team, vfx, audio);
                }

                let bio_act_ready = session.can_local_player_act()
                    && !session.economy.primal_surge_used.get(&viewing_team).copied().unwrap_or(false)
                    && session.economy.actions_remaining.get(&viewing_team).copied().unwrap_or(0) >= bio_card.primal_surge_cost.0
                    && session.economy.home_base_munitions.get(&viewing_team).copied().unwrap_or(0) >= u16::from(bio_card.primal_surge_cost.1);

                let bio_btn = egui::Button::new(
                    RichText::new(format!("🧬 {} ({} Act, {} Mun)", bio_card.primal_surge_name, bio_card.primal_surge_cost.0, bio_card.primal_surge_cost.1))
                        .strong()
                        .color(Color32::WHITE),
                )
                .fill(if bio_act_ready { Color32::from_rgb(38, 158, 96) } else { Color32::from_rgb(55, 68, 76) });

                if ui.add_enabled(bio_act_ready, bio_btn)
                    .on_hover_text(format!("{}
Primal Surge: {}", bio_card.title, bio_card.primal_surge_effect))
                    .clicked()
                {
                    session.execute_biotype_primal_surge(viewing_team, vfx, audio);
                }

                if is_pure {
                    ui.label(
                        RichText::new(format!("★ PURE APEX: {}", bio_card.pure_doctrine_name))
                            .strong()
                            .color(Color32::from_rgb(250, 212, 60)),
                    )
                    .on_hover_text(&bio_card.pure_doctrine_effect);
                } else if let Some(bio2) = team_deck.secondary_biotype_card() {
                    ui.label(
                        RichText::new(format!("🧬 HYBRID: {} ({})", bio2.title, bio2.hybrid_doctrine_name))
                            .small()
                            .color(Color32::from_rgb(240, 180, 120)),
                    )
                    .on_hover_text(&bio2.hybrid_doctrine_effect);
                }
            });
            ui.separator();

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "P{} TACTICAL HAND ({})  |  Draw: {}  |  Graveyard: {}  ||  Opponent Hand: {} (Deck: {})",
                        session.viewing_team(),
                        session.active_hand().len(),
                        session.active_draw_pile().len(),
                        session.active_graveyard().len(),
                        session.opponent_hand_len(),
                        session.opponent_draw_len()
                    ))
                    .strong()
                    .color(Color32::from_rgb(188, 212, 222)),
                );
                if matches!(session.mode, CommandMode::DeployHandCard(_)) {
                    ui.separator();
                    ui.label(
                        RichText::new("Click any green highlighted Supply/Mycelial hex on the board to deploy")
                            .color(Color32::from_rgb(112, 242, 164)),
                    );
                    if ui.small_button("Cancel Deploy").clicked() {
                        session.mode = CommandMode::Inspect;
                    }
                }
            });
            ui.add_space(4.0);

            ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal(|ui| {
                    let hand_snapshot = if session.hotseat_handoff_pending { Vec::new() } else { session.active_hand().clone() };
                    for (idx, card_key) in hand_snapshot.iter().enumerate() {
                        let Some(card) = session.card_def(card_key).cloned() else {
                            continue;
                        };
                        let is_selected = session.mode == CommandMode::DeployHandCard(idx);
                        let badge = faction_badge_color(card.faction);
                        let border_col = if is_selected {
                            Color32::from_rgb(112, 250, 170)
                        } else {
                            badge
                        };

                        Frame::NONE
                            .fill(Color32::from_rgb(24, 36, 44))
                            .stroke(Stroke::new(if is_selected { 2.5 } else { 1.2 }, border_col))
                            .corner_radius(6)
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(214.0);
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&card.unit_name)
                                                    .strong()
                                                    .color(Color32::WHITE),
                                            )
                                            .truncate(),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(format!("{}p", card.deck_points))
                                                        .small()
                                                        .color(Color32::from_rgb(200, 215, 225)),
                                                );
                                            },
                                        );
                                    });
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!(
                                                "{:?} • {:?} • {}",
                                                card.faction,
                                                card.combat_class,
                                                card.default_vox_slug()
                                            ))
                                            .small()
                                            .color(badge),
                                        )
                                        .truncate(),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!(
                                                "STR {} | ARM {} | MOV {} | RNG {}",
                                                card.strength_hp,
                                                card.armor,
                                                card.movement,
                                                card.ranged_distance
                                            ))
                                            .small()
                                            .color(Color32::from_rgb(212, 224, 232)),
                                        )
                                        .truncate(),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&card.signature_ability)
                                                .small()
                                                .italics()
                                                .color(Color32::from_rgb(152, 198, 214)),
                                        )
                                        .truncate(),
                                    );
                                    ui.add_space(2.0);
                                    let btn_txt = format!("Deploy ({} Mun)", card.encumbrance);
                                    if ui
                                        .add_enabled(session.can_local_player_act(), egui::Button::new(btn_txt))
                                        .clicked()
                                    {
                                        session.mode = if is_selected {
                                            CommandMode::Inspect
                                        } else {
                                            CommandMode::DeployHandCard(idx)
                                        };
                                    }
                                });
                            });
                    }
                });
            });
        });

    // 3. LEFT HEX & UNIT INSPECTOR + ACTION COMMANDS + COMBAT LOG
    egui::Panel::left("wf_tactical_inspector")
        .frame(
            Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(16, 26, 33, 228))
                .inner_margin(Margin::same(12))
                .stroke(Stroke::new(1.0, Color32::from_rgb(42, 72, 86))),
        )
        .show(root_ui, |ui| {
            ui.set_min_width(285.0);
            ui.set_max_width(310.0);
            ui.label(
                RichText::new("HEX & UNIT INSPECTOR")
                    .strong()
                    .color(Color32::from_rgb(122, 216, 238)),
            );
            ui.separator();

            if let Some(hov) = session.hovered_hex {
                if let Some(tile) = session.scenario.map.tiles.get(&hov) {
                    ui.label(
                        RichText::new(format!(
                            "Cursor Hex ({}, {}) — {}",
                            hov.x,
                            hov.y,
                            tile.biome.display_name()
                        ))
                        .strong()
                        .color(Color32::WHITE),
                    );
                    ui.label(
                        RichText::new(format!(
                            "Terrain: {:?} | Elev Tier {}",
                            tile.morphology, tile.elevation
                        ))
                        .small()
                        .color(Color32::from_rgb(180, 198, 208)),
                    );
                }

                if let (Some(sel), Some(hov_idx)) = (session.selected_hex, session.visible_entity_at_for_viewer(hov)) {
                    if let Some(sel_idx) = session.visible_entity_at_for_viewer(sel) {
                        let att = &session.entities[sel_idx];
                        let def = &session.entities[hov_idx];
                        let viewer_t = session.viewing_team();
                        if att.team_id == viewer_t && def.team_id != viewer_t && def.team_id != 0 {
                            let dist = sel.unsigned_distance_to(hov) as u8;
                            let ctx_preview = CombatContext {
                                hex_distance: dist,
                                is_melee: dist == 1 && att.card.ranged_distance == 0,
                                attacker_can_fly: att.card.can_fly,
                                attacker_stealth_ambush: att.card.stealth,
                                attacker_adjacent_commander: false,
                                attacker_pack_tactics_bonus: false,
                                defender_in_rough_cover: false,
                            };
                            let req = required_ranged_hit_roll(&def.unit, ctx_preview);
                            let d_str = (i16::from(att.unit.current_str_hp)
                                - i16::from(def.unit.current_str_hp))
                            .clamp(-2, 3);
                            ui.label(
                                RichText::new(format!(
                                    "🎯 Odds @ {} hex: Hit {}+ on D6 | ΔSTR {:+} vs Armor {}",
                                    dist, req, d_str, def.unit.current_armor
                                ))
                                .small()
                                .strong()
                                .color(Color32::from_rgb(255, 168, 92)),
                            );
                        }
                    }
                }
            } else {
                ui.label(
                    RichText::new("Move cursor over the 3D hex board to inspect tiles & odds.")
                        .small()
                        .color(Color32::GRAY),
                );
            }

            ui.add_space(8.0);
            ui.separator();

            if let Some(sel_hex) = session.selected_hex {
                if let Some(ent_idx) = session.visible_entity_at_for_viewer(sel_hex) {
                    let supply_cov = session.team_supply_hexes(session.entities[ent_idx].team_id);
                    let ent = session.entities[ent_idx].clone();
                    let in_supply = supply_cov.contains(&ent.hex);

                    ui.label(
                        RichText::new(format!(
                            "{} [Team {}]",
                            ent.card.unit_name, ent.team_id
                        ))
                        .strong()
                        .size(15.0)
                        .color(Color32::WHITE),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{:?} • {:?} • {}.vox",
                            ent.card.faction, ent.card.combat_class, ent.vox_slug
                        ))
                        .small()
                        .color(faction_badge_color(ent.card.faction)),
                    );
                    ui.label(format!(
                        "STR (HP & Dmg Cap): {} / {}",
                        ent.unit.current_str_hp, ent.unit.max_str_hp
                    ));
                    ui.label(format!(
                        "Armor: {} / {}  |  Evade: {}  |  Fortify: {}",
                        ent.unit.current_armor, ent.unit.base_armor, ent.unit.evade, ent.unit.fortified_stacks
                    ));
                    ui.label(format!(
                        "Local Ammo: {} / {} {}",
                        ent.unit.carried_munitions,
                        ent.unit.encumbrance_cap,
                        if in_supply { "(Supply Linked)" } else { "" }
                    ));
                    if ent.unit.confused {
                        ui.label(
                            RichText::new("⚠ CONFUSED: Out of Control Node Range!")
                                .color(Color32::from_rgb(255, 110, 110)),
                        );
                    } else if ent.team_id == session.viewing_team() && session.can_local_player_act() {
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            if ent.card.movement > 0 && ui.button("🥾 Move").clicked() {
                                session.mode = CommandMode::MoveSelectedUnit;
                            }
                            if (ent.card.ranged_distance > 0 || ent.card.melee_range > 0)
                                && ui.button("⚔ Attack").clicked()
                            {
                                session.mode = CommandMode::AttackWithSelectedUnit;
                            }
                            if ui.button("🎲 Dice Sim").on_hover_text("Simulate and inspect combat roll math for selected attacker").clicked() {
                                actions.toggle_dice_roller = true;
                            }
                            if ui.button("✨ Ability / Fortify").clicked() {
                                session.mode = CommandMode::AbilityWithSelectedUnit;
                            }
                        });
                        if session.mode != CommandMode::Inspect && ui.small_button("Cancel Command").clicked() {
                            session.mode = CommandMode::Inspect;
                        }
                        let vt = session.viewing_team();
                        if ent.unit.current_str_hp < ent.unit.max_str_hp
                            && session.economy.actions_remaining.get(&vt).copied().unwrap_or(0) > 0
                            && ui.small_button("💤 Rest (+1 STR)").clicked()
                        {
                            session.rest_unit_at_hex(ent.hex, true, vfx, audio);
                        }
                    }
                } else {
                    ui.label(format!("Selected Empty Hex ({}, {})", sel_hex.x, sel_hex.y));
                }
            }

            let active_gy = session.active_graveyard().clone();
            if !active_gy.is_empty() && !session.hotseat_handoff_pending {
                ui.add_space(8.0);
                ui.separator();
                ui.label(RichText::new(format!("GRAVEYARD QUEUES (P{})", session.viewing_team())).strong());
                for t in &active_gy {
                    ui.label(
                        RichText::new(format!(
                            "• {} ({:?}): {}/{} STR",
                            t.card_key, t.facility, t.recovered_str, t.target_str
                        ))
                        .small(),
                    );
                }
            }

            ui.add_space(8.0);
            ui.separator();
            ui.label(
                RichText::new("COMBAT & LOGISTICS LOG")
                    .strong()
                    .color(Color32::from_rgb(188, 212, 222)),
            );
            ScrollArea::vertical().max_height(210.0).stick_to_bottom(true).show(ui, |ui| {
                for entry in &session.combat_log {
                    ui.label(RichText::new(entry).small().color(Color32::from_rgb(206, 220, 228)));
                }
            });
        });

    // 4. SCENARIO VICTORY / DEFEAT SUMMARY MODAL
    if let Some(outcome) = session.match_outcome.clone() {
        if !session.outcome_modal_dismissed {
            let player_won = outcome.winner_team == 1 || session.hotseat_mode;
            let banner_col = if player_won {
                Color32::from_rgb(76, 242, 162)
            } else {
                Color32::from_rgb(252, 98, 92)
            };
            Window::new("⚔ MATCH RESULT")
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .collapsible(false)
                .resizable(false)
                .order(Order::Foreground)
                .frame(
                    Frame::NONE
                        .fill(Color32::from_rgba_unmultiplied(12, 20, 28, 248))
                        .stroke(Stroke::new(2.5, banner_col))
                        .corner_radius(10)
                        .inner_margin(Margin::same(20)),
                )
                .show(root_ui.ctx(), |ui| {
                    ui.set_min_width(430.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(if player_won { "🏆" } else { "💀" }).size(34.0),
                        );
                        ui.label(
                            RichText::new(&outcome.headline)
                                .size(21.0)
                                .strong()
                                .color(banner_col),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(&outcome.reason)
                                .size(13.5)
                                .color(Color32::from_rgb(224, 236, 244)),
                        );
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!(
                                "Scenario: {}   |   Turns Elapsed: {}",
                                session.scenario.title, outcome.turns_taken
                            ))
                            .strong()
                            .color(Color32::from_rgb(180, 212, 228)),
                        );
                        ui.label(
                            RichText::new(format!(
                                "Cards Deployed: {}   |   Enemy Units Destroyed: {}",
                                outcome.units_deployed, outcome.enemies_destroyed
                            ))
                            .color(Color32::from_rgb(198, 218, 228)),
                        );
                        ui.add_space(14.0);

                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("🔄 Rematch Same Map").strong().color(Color32::WHITE),
                                    )
                                    .fill(Color32::from_rgb(32, 108, 138)),
                                )
                                .clicked()
                            {
                                actions.rematch_current = true;
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("🎲 New Map Seed").strong().color(Color32::WHITE),
                                    )
                                    .fill(Color32::from_rgb(44, 128, 86)),
                                )
                                .clicked()
                            {
                                actions.roll_new_seed = true;
                            }
                            let next_idx = (session.scenario.scenario_id as u8 % 5) + 1;
                            let next_sid = ScenarioId::from_index(next_idx);
                            if ui.button("⏭ Next Scenario").clicked() {
                                actions.switch_scenario = Some(next_sid);
                            }
                            if ui.button("👁 Inspect Board").clicked() {
                                session.outcome_modal_dismissed = true;
                            }
                        });
                    });
                });
        }
    }


    // 5. 1v1 HOTSEAT FOG-OF-WAR PRIVACY CURTAIN
    if session.hotseat_mode && session.hotseat_handoff_pending && session.match_outcome.is_none() {
        let incoming = session.active_team;
        let accent = if incoming == 1 {
            Color32::from_rgb(82, 228, 250)
        } else {
            Color32::from_rgb(252, 118, 92)
        };
        Window::new("🔄 HOTSEAT TURN HANDOFF")
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .order(Order::Foreground)
            .frame(
                Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(10, 16, 24, 250))
                    .stroke(Stroke::new(2.5, accent))
                    .corner_radius(10)
                    .inner_margin(Margin::same(24)),
            )
            .show(root_ui.ctx(), |ui| {
                ui.set_min_width(400.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(format!("PASS CONTROLS TO PLAYER {}", incoming))
                            .size(20.0)
                            .strong()
                            .color(accent),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "Fog-of-War & Tactical Hand are cloaked during handoff so neither commander sees stealthed deployments.",
                        )
                        .color(Color32::from_rgb(206, 222, 232)),
                    );
                    ui.add_space(14.0);
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!(
                                    "👁 Reveal Player {} View (Space)",
                                    incoming
                                ))
                                .size(15.0)
                                .strong()
                                .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(32, 112, 142)),
                        )
                        .clicked()
                    {
                        session.hotseat_handoff_pending = false;
                        audio.play_ui(SfxCue::UiCardTurn);
                    }
                });
            });
    }

    actions
}
