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

//! In-Game 3D-Print `.STL` Export Studio (`F8`) for *Wildfront Battlegrounds*.
//!
//! Exports watertight, slicer-ready binary `.STL` miniatures directly from the live PC client
//! with parametric **45-degree Support-Free Recessed Micro-Dice V-Cradles** and a configurable,
//! print-safe **Center Divider Wall** (`>= 2.00mm`, default `2.80mm` = 7x `0.40mm` nozzle walls).

use std::path::Path;
use egui::{Align2, Color32, Frame, Margin, Order, RichText, ScrollArea, Slider, Stroke, Ui, Window};
use wf_assets::stl_exporter::{
    export_card_to_stl, export_deck_to_stl_folder, export_terrain_hexes_with_config, export_vox_directory_with_config,
    resolve_cradle_layout, sanitize_filename_slug, CradleMode, TabletopPrintConfig,
    MIN_PRINTABLE_CENTER_WALL_MM,
};
use wf_core::{DeckList, RosterManifest};
use crate::audio::{SfxCue, SpatialAudioEngine};

pub struct StlExportStudio {
    pub is_open: bool,
    pub config: TabletopPrintConfig,
    pub selected_card_key: String,
    pub filter_query: String,
    pub status_message: String,
}

impl Default for StlExportStudio {
    fn default() -> Self {
        Self {
            is_open: false,
            config: TabletopPrintConfig::default(),
            selected_card_key: "Battlephant".to_string(),
            filter_query: String::new(),
            status_message: "Ready — exports watertight .STL miniatures with 45° support-free micro-dice V-cradles to exports/stl/.".to_string(),
        }
    }
}

impl StlExportStudio {
    pub fn focus_card(&mut self, card_key: &str) {
        self.selected_card_key = card_key.to_string();
        self.is_open = true;
    }

    pub fn show(
        &mut self,
        root_ui: &mut Ui,
        roster: &RosterManifest,
        active_deck: &DeckList,
        audio: &mut SpatialAudioEngine,
    ) {
        if !self.is_open {
            return;
        }
        let mut open_flag = self.is_open;
        let lookup = roster.index_by_key();

        Window::new("🖨 3D PRINT TABLETOP .STL STUDIO — RECESSED MICRO-DICE V-CRADLES (F8)")
            .open(&mut open_flag)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .order(Order::Foreground)
            .frame(
                Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(11, 19, 26, 248))
                    .stroke(Stroke::new(2.0, Color32::from_rgb(92, 218, 242)))
                    .corner_radius(10)
                    .inner_margin(Margin::same(18)),
            )
            .show(root_ui.ctx(), |ui| {
                ui.set_min_width(720.0);
                ui.set_max_width(750.0);

                ui.label(
                    RichText::new(
                        "Generates single-piece watertight 2-manifold .STL miniatures scaled 1:1 to your tabletop hex grid, \
                         carving 45° support-free V-cradles with a reinforced solid center wall between STR and MUN dice.",
                    )
                    .small()
                    .color(Color32::from_rgb(184, 212, 224)),
                );
                ui.add_space(8.0);

                // 1. Slicer & Wall-Thickness Parameters
                Frame::NONE
                    .fill(Color32::from_rgb(18, 29, 37))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(46, 84, 102)))
                    .corner_radius(6)
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Hex Flat-to-Flat Scale:").strong().color(Color32::WHITE));
                            ui.add(
                                Slider::new(&mut self.config.target_hex_flat_mm, 32.0..=65.0)
                                    .step_by(0.5)
                                    .suffix(" mm"),
                            );
                            if ui.small_button("Reset 45mm").clicked() {
                                self.config.target_hex_flat_mm = 45.0;
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Center Wall Between Dice:")
                                    .strong()
                                    .color(Color32::from_rgb(116, 244, 178)),
                            );
                            ui.add(
                                Slider::new(
                                    &mut self.config.center_wall_mm,
                                    MIN_PRINTABLE_CENTER_WALL_MM..=4.50,
                                )
                                .step_by(0.10)
                                .suffix(" mm"),
                            );
                            let perims_04 = self.config.center_wall_mm / 0.40;
                            ui.label(
                                RichText::new(format!("({perims_04:.1} walls @ 0.4mm nozzle ✓)"))
                                    .small()
                                    .color(Color32::from_rgb(142, 236, 186)),
                            );
                        });

                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Outer Side Wall:").strong().color(Color32::WHITE));
                            ui.add(
                                Slider::new(&mut self.config.outer_wall_mm, 1.80..=3.60)
                                    .step_by(0.10)
                                    .suffix(" mm"),
                            );
                            ui.separator();
                            ui.label(RichText::new("Die Clearance:").strong().color(Color32::WHITE));
                            ui.add(
                                Slider::new(&mut self.config.die_clearance_mm, 0.20..=0.70)
                                    .step_by(0.05)
                                    .suffix(" mm"),
                            );
                        });

                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new("Cradle Routing:").strong().color(Color32::WHITE));
                            for (mode, lbl) in [
                                (CradleMode::AutoFromCardStats, "Auto From Card STR/ENC"),
                                (CradleMode::ForceDualD6, "Force Dual 7mm D6"),
                                (CradleMode::ForceSingleD10, "Force Single 10mm D10"),
                                (CradleMode::SculptOnlyNoCradles, "Sculpt Only (No Cradles)"),
                            ] {
                                if ui.selectable_label(self.config.cradle_mode == mode, lbl).clicked() {
                                    self.config.cradle_mode = mode;
                                }
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.checkbox(
                                &mut self.config.include_front_chevron,
                                "Front -Y Facing Chevron",
                            );
                            ui.separator();
                            ui.checkbox(
                                &mut self.config.include_magnet_pocket,
                                "Underside 6.2×2.2mm Magnet Recess",
                            );
                        });
                    });

                ui.add_space(8.0);

                // 2. Unit Card Selector + Live Cradle Preview
                ui.columns(2, |cols| {
                    cols[0].horizontal(|ui| {
                        ui.label(RichText::new("🔍 Card:").strong());
                        ui.text_edit_singleline(&mut self.filter_query);
                    });
                    cols[0].add_space(4.0);
                    let q = self.filter_query.trim().to_lowercase();
                    ScrollArea::vertical()
                        .id_salt("stl_studio_card_list")
                        .max_height(190.0)
                        .show(&mut cols[0], |ui| {
                            for card in &roster.units {
                                if !q.is_empty()
                                    && !card.card_key.to_lowercase().contains(&q)
                                    && !format!("{:?}", card.faction).to_lowercase().contains(&q)
                                {
                                    continue;
                                }
                                let is_sel = self.selected_card_key == card.card_key;
                                let row_txt = format!(
                                    "{}  [STR {} • ENC {}]",
                                    card.card_key, card.strength_hp, card.encumbrance
                                );
                                if ui.selectable_label(is_sel, row_txt).clicked() {
                                    self.selected_card_key = card.card_key.clone();
                                }
                            }
                        });

                    if let Some(card) = lookup.get(self.selected_card_key.as_str()) {
                        let slug = card.default_vox_slug();
                        let preview_layout =
                            resolve_cradle_layout(Some(card), slug, self.config.cradle_mode);
                        let wall_preview =
                            self.config.center_wall_mm.max(MIN_PRINTABLE_CENTER_WALL_MM);

                        Frame::NONE
                            .fill(Color32::from_rgb(20, 33, 42))
                            .stroke(Stroke::new(1.0, Color32::from_rgb(56, 102, 124)))
                            .corner_radius(6)
                            .inner_margin(Margin::same(10))
                            .show(&mut cols[1], |ui| {
                                ui.label(
                                    RichText::new(&card.card_key)
                                        .size(15.5)
                                        .strong()
                                        .color(Color32::WHITE),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "{:?} • {:?}  |  Rig: {}.vox",
                                        card.faction, card.combat_class, slug
                                    ))
                                    .small()
                                    .color(Color32::from_rgb(162, 198, 214)),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(format!(
                                        "Stats: STR {}  •  ENC {}  •  ARM {}",
                                        card.strength_hp, card.encumbrance, card.armor
                                    ))
                                    .strong()
                                    .color(Color32::from_rgb(248, 204, 96)),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(preview_layout.summary_label(wall_preview))
                                        .small()
                                        .color(Color32::from_rgb(122, 242, 184)),
                                );
                                ui.add_space(8.0);

                                if ui
                                    .add_sized(
                                        [310.0, 32.0],
                                        egui::Button::new(
                                            RichText::new("🖨 Export Selected Card (.STL)")
                                                .strong()
                                                .color(Color32::WHITE),
                                        )
                                        .fill(Color32::from_rgb(28, 138, 96)),
                                    )
                                    .clicked()
                                {
                                    let fname = format!(
                                        "{}__STR{}_ENC{}.stl",
                                        sanitize_filename_slug(&card.card_key),
                                        card.strength_hp,
                                        card.encumbrance
                                    );
                                    let out_path = Path::new("exports/stl/cards").join(fname);
                                    match export_card_to_stl(
                                        card,
                                        Path::new("assets/vox"),
                                        &out_path,
                                        &self.config,
                                    ) {
                                        Ok(rep) => {
                                            audio.play_ui(SfxCue::BioHealChime);
                                            self.status_message = format!(
                                                "✓ Saved {} ({} tris | {:.1}×{:.1}×{:.1}mm | {})",
                                                rep.output_path.display(),
                                                rep.triangle_count,
                                                rep.footprint_mm.0,
                                                rep.footprint_mm.1,
                                                rep.footprint_mm.2,
                                                rep.resolved_cradle_desc
                                            );
                                        }
                                        Err(e) => {
                                            self.status_message = format!("⚠ Export failed: {e}");
                                        }
                                    }
                                }
                            });
                    }
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);

                // 3. Batch Deck & Full Master Library Export Row
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!(
                                    "📦 Export Active Deck ({}) + Manifest",
                                    active_deck.deck_name
                                ))
                                .strong()
                                .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(36, 108, 148)),
                        )
                        .clicked()
                    {
                        let deck_slug = sanitize_filename_slug(&active_deck.deck_name);
                        let out_dir = Path::new("exports/stl/decks").join(deck_slug);
                        match export_deck_to_stl_folder(
                            active_deck,
                            roster,
                            Path::new("assets/vox"),
                            &out_dir,
                            &self.config,
                        ) {
                            Ok(reps) => {
                                audio.play_ui(SfxCue::BioHealChime);
                                self.status_message = format!(
                                    "✓ Exported {} unique card .STLs + PRINT_MANIFEST.txt to {}",
                                    reps.len(),
                                    out_dir.display()
                                );
                            }
                            Err(e) => {
                                self.status_message = format!("⚠ Deck STL export failed: {e}");
                            }
                        }
                    }

                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("🃏 Export All 152 Cards (.STL)")
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(28, 128, 112)),
                        )
                        .clicked()
                    {
                        let out_dir = Path::new("exports/stl/all_152_cards");
                        let mut ok_cnt = 0usize;
                        for card in &roster.units {
                            let out_p = out_dir.join(format!("{}__STR{}_ENC{}.stl", sanitize_filename_slug(&card.card_key), card.strength_hp, card.encumbrance));
                            if export_card_to_stl(card, Path::new("assets/vox"), &out_p, &self.config).is_ok() {
                                ok_cnt += 1;
                            }
                        }
                        audio.play_ui(SfxCue::BioHealChime);
                        self.status_message = format!("✓ Exported {}/{} card .STLs with STR/ENC V-cradles to {}!", ok_cnt, roster.units.len(), out_dir.display());
                    }

                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("🏛 Export All 170 Master Rigs (.STL)")
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(92, 76, 144)),
                        )
                        .clicked()
                    {
                        let out_dir = Path::new("exports/stl/master_rigs");
                        match export_vox_directory_with_config(
                            Path::new("assets/vox"),
                            out_dir,
                            &self.config,
                        ) {
                            Ok(cnt) => {
                                audio.play_ui(SfxCue::BioHealChime);
                                self.status_message = format!(
                                    "✓ Exported all {cnt} master .STL rigs to {} (center wall >= {:.2}mm)",
                                    out_dir.display(),
                                    self.config.center_wall_mm.max(MIN_PRINTABLE_CENTER_WALL_MM)
                                );
                            }
                            Err(e) => {
                                self.status_message = format!("⚠ Batch STL export failed: {e}");
                            }
                        }
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("🗺 Export All 12 Terrain Hexes + Canal (.STL)")
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(18, 118, 152)),
                        )
                        .clicked()
                    {
                        let out_dir = Path::new("exports/stl/terrain_hexes_45mm");
                        match export_terrain_hexes_with_config(
                            Path::new("assets/vox"),
                            out_dir,
                            &self.config,
                        ) {
                            Ok(cnt) => {
                                audio.play_ui(SfxCue::BioHealChime);
                                self.status_message = format!(
                                    "✓ Exported all {cnt} Modular {:.1}mm Terrain Hex & Hydro-Breach Canal .STLs to {}!",
                                    self.config.target_hex_flat_mm,
                                    out_dir.display()
                                );
                            }
                            Err(e) => {
                                self.status_message = format!("⚠ Terrain Hex STL export failed: {e}");
                            }
                        }
                    }
                });

                ui.add_space(6.0);
                ui.label(
                    RichText::new(&self.status_message)
                        .small()
                        .strong()
                        .color(Color32::from_rgb(132, 242, 188)),
                );
            });

        self.is_open = open_flag;
    }
}
