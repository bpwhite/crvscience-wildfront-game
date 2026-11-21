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

//! Cinematic 2072 Title Screen & Match Launcher rendered over the live orbiting
//! 3D GPU Sparse 64-Tree hexagonal voxel battlefield.

use egui::{Align2, Color32, Frame, Margin, Order, RichText, Stroke, Ui, Window};
use wf_core::{ScenarioId, WILDFRONT_THEME_2072};
use crate::net_lockstep::{NetLockstepManager, NetStatus};

#[derive(Default)]
pub struct MainMenuActions {
    pub start_solo_ai: bool,
    pub start_hotseat: bool,
    pub host_tcp: bool,
    pub join_tcp: bool,
    pub cancel_tcp: bool,
    pub open_deck_builder: bool,
    pub open_stl_studio: bool,
    pub open_dice_roller: bool,
    pub open_db_manager: bool,
    pub open_codex: bool,
    pub open_map_editor: bool,
    pub open_replay_theater: bool,
    pub open_about: bool,
    pub select_ai_difficulty: Option<wf_sim::AiDifficulty>,
    pub quickload: bool,
    pub select_scenario: Option<ScenarioId>,
    pub roll_seed: bool,
    pub quit_app: bool,
}

pub struct MainMenuScreen {
    pub is_open: bool,
    pub has_started_match: bool,
}

impl Default for MainMenuScreen {
    fn default() -> Self {
        Self {
            is_open: true,
            has_started_match: false,
        }
    }
}

impl MainMenuScreen {
    pub fn show(
        &mut self,
        root_ui: &mut Ui,
        current_scenario: ScenarioId,
        active_seed: u64,
        p1_deck_name: &str,
        p2_deck_name: &str,
        net: &mut NetLockstepManager,
        ai_difficulty: wf_sim::AiDifficulty,
    ) -> MainMenuActions {
        let mut act = MainMenuActions::default();
        if !self.is_open {
            return act;
        }

        Window::new("WILDFRONT_2072_MAIN_MENU")
            .title_bar(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .order(Order::Foreground)
            .frame(
                Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(9, 16, 22, 238))
                    .stroke(Stroke::new(2.2, Color32::from_rgb(68, 206, 232)))
                    .corner_radius(12)
                    .inner_margin(Margin::same(24)),
            )
            .show(root_ui.ctx(), |ui| {
                ui.set_min_width(660.0);
                ui.set_max_width(690.0);

                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("WILDFRONT BATTLEGROUNDS")
                            .size(28.0)
                            .strong()
                            .color(Color32::from_rgb(242, 248, 252)),
                    );
                    ui.label(
                        RichText::new("YEAR 2072 SURFACE EMERGENCE  •  170-MODEL VOXEL ENGINE & TOURNAMENT SYSTEM (v3.4.1 • 152 CARDS • 2-PLY AI & REPLAY)")
                            .size(11.5)
                            .strong()
                            .color(Color32::from_rgb(92, 224, 244)),
                    );
                });

                ui.add_space(10.0);
                Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(16, 26, 34, 220))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(42, 74, 90)))
                    .corner_radius(6)
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(WILDFRONT_THEME_2072)
                                .italics()
                                .size(12.2)
                                .color(Color32::from_rgb(206, 222, 232)),
                        );
                    });

                ui.add_space(12.0);
                // Scenario & Procedural Seed Picker (live-updates orbiting 3D background!)
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("🗺 Theater Scenario:").strong().color(Color32::WHITE));
                    egui::ComboBox::from_id_salt("main_menu_scenario")
                        .selected_text(current_scenario.title())
                        .show_ui(ui, |ui| {
                            for sid in ScenarioId::ALL {
                                if ui.selectable_label(current_scenario == sid, sid.title()).clicked() {
                                    act.select_scenario = Some(sid);
                                }
                            }
                        });
                    if ui.button("🎲 New Seed").clicked() {
                        act.roll_seed = true;
                    }
                    ui.label(RichText::new("🤖 AI:").strong().color(Color32::WHITE));
                    egui::ComboBox::from_id_salt("main_menu_ai_diff")
                        .selected_text(ai_difficulty.label())
                        .show_ui(ui, |ui| {
                            for d in wf_sim::AiDifficulty::ALL {
                                if ui.selectable_label(ai_difficulty == d, d.label()).clicked() {
                                    act.select_ai_difficulty = Some(d);
                                }
                            }
                        });
                    ui.label(
                        RichText::new(format!("Seed: 0x{:08X}", active_seed))
                            .monospace()
                            .color(Color32::from_rgb(168, 204, 218)),
                    );
                });

                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Active Decks — P1: {}   |   P2 / AI: {}",
                        p1_deck_name, p2_deck_name
                    ))
                    .small()
                    .color(Color32::from_rgb(176, 214, 198)),
                );

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(10.0);

                // Primary Play Modes Row
                ui.columns(2, |cols| {
                    // Left Column: Local Modes
                    cols[0].label(
                        RichText::new("LOCAL COMMAND MODES")
                            .strong()
                            .color(Color32::from_rgb(112, 232, 248)),
                    );
                    cols[0].add_space(6.0);
                    if self.has_started_match
                        && cols[0]
                            .add_sized(
                                [300.0, 34.0],
                                egui::Button::new(
                                    RichText::new("▶ Resume Current Match")
                                        .size(14.5)
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .fill(Color32::from_rgb(34, 128, 96)),
                            )
                            .clicked()
                    {
                        self.is_open = false;
                    }
                    if self.has_started_match {
                        cols[0].add_space(6.0);
                    }
                    if cols[0]
                        .add_sized(
                            [300.0, 36.0],
                            egui::Button::new(
                                RichText::new("🤖 Solo Skirmish (vs Tactical AI)")
                                    .size(14.5)
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(30, 104, 138)),
                        )
                        .clicked()
                    {
                        act.start_solo_ai = true;
                    }
                    cols[0].add_space(6.0);
                    if cols[0]
                        .add_sized(
                            [300.0, 36.0],
                            egui::Button::new(
                                RichText::new("👥 1v1 Local Hotseat Match")
                                    .size(14.5)
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(86, 74, 146)),
                        )
                        .clicked()
                    {
                        act.start_hotseat = true;
                    }

                    // Right Column: 1v1 TCP Lockstep Multiplayer
                    cols[1].label(
                        RichText::new("1v1 TCP LOCKSTEP MULTIPLAYER")
                            .strong()
                            .color(Color32::from_rgb(248, 192, 88)),
                    );
                    cols[1].add_space(6.0);
                    cols[1].horizontal(|ui| {
                        ui.label("Bind:");
                        ui.text_edit_singleline(&mut net.host_bind_input);
                        if matches!(net.status, NetStatus::HostingWait(_)) {
                            if ui.button("✖ Cancel").clicked() {
                                act.cancel_tcp = true;
                            }
                        } else if ui
                            .add(
                                egui::Button::new(RichText::new("📡 Host P1").strong().color(Color32::WHITE))
                                    .fill(Color32::from_rgb(168, 92, 28)),
                            )
                            .clicked()
                        {
                            act.host_tcp = true;
                        }
                    });
                    cols[1].add_space(4.0);
                    cols[1].horizontal(|ui| {
                        ui.label("Host IP:");
                        ui.text_edit_singleline(&mut net.join_addr_input);
                        if ui
                            .add(
                                egui::Button::new(RichText::new("🔌 Join P2").strong().color(Color32::WHITE))
                                    .fill(Color32::from_rgb(36, 124, 92)),
                            )
                            .clicked()
                        {
                            act.join_tcp = true;
                        }
                    });
                    cols[1].add_space(4.0);
                    cols[1].label(
                        RichText::new(&net.status_line)
                            .small()
                            .color(Color32::from_rgb(142, 232, 198)),
                    );
                });

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                // Utility Footer Buttons
                ui.horizontal(|ui| {
                    if ui.button("🃏 250-Pt Deck Builder (B)").clicked() {
                        act.open_deck_builder = true;
                    }
                    if ui.button("🖨 3D Print STLs (F8)").clicked() {
                        act.open_stl_studio = true;
                    }
                    if ui.button("🎲 Dice Roller (R)").clicked() {
                        act.open_dice_roller = true;
                    }
                    if ui.button("🗄 Database (F2)").clicked() {
                        act.open_db_manager = true;
                    }
                    if ui.button("🗺 Map Studio (F3)").clicked() {
                        act.open_map_editor = true;
                    }
                    if ui.button("🎬 Replay (F4)").clicked() {
                        act.open_replay_theater = true;
                    }
                    if ui.button("📖 Field Manual & Codex (F1)").clicked() {
                        act.open_codex = true;
                    }
                    if ui.button("📂 Load Quicksave (F9)").clicked() {
                        act.quickload = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("🚪 Quit Desktop").clicked() {
                            act.quit_app = true;
                        }
                    });
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Copyright CRV Science Network  •  GPL-3.0-or-later")
                            .small()
                            .color(Color32::from_rgb(156, 186, 200)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("ℹ About WildFront").clicked() {
                            act.open_about = true;
                        }
                    });
                });
            });

        act
    }
}
