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

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use egui::{Align2, Color32, Frame, Margin, Order, ProgressBar, RichText, ScrollArea, Slider, Stroke, Ui, Window};
use wf_assets::stl_exporter::{
    export_card_to_stl, export_deck_to_stl_folder_cancellable, export_terrain_hexes_cancellable,
    export_vox_directory_cancellable, resolve_cradle_layout, sanitize_filename_slug, CradleMode,
    TabletopPrintConfig, MIN_PRINTABLE_CENTER_WALL_MM,
};
use wf_core::{DeckList, RosterManifest};
use crate::audio::{SfxCue, SpatialAudioEngine};
use crate::paths::{ensure_parent_dir, resolve_asset_path, writable_app_path};

#[derive(Debug, Clone)]
struct BatchStlProgress {
    done: usize,
    total: usize,
    current_label: String,
}

#[derive(Debug)]
struct BatchStlOutcome {
    cancelled: bool,
    result: Result<String, String>,
}

struct BatchStlJob {
    label: String,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<BatchStlProgress>>,
    rx: Receiver<BatchStlOutcome>,
}

pub struct StlExportStudio {
    pub is_open: bool,
    pub config: TabletopPrintConfig,
    pub selected_card_key: String,
    pub filter_query: String,
    pub status_message: String,
    active_job: Option<BatchStlJob>,
}

impl Default for StlExportStudio {
    fn default() -> Self {
        Self {
            is_open: false,
            config: TabletopPrintConfig::default(),
            selected_card_key: "Battlephant".to_string(),
            filter_query: String::new(),
            status_message: "Ready — exports watertight .STL miniatures with 45° support-free micro-dice V-cradles to exports/stl/.".to_string(),
            active_job: None,
        }
    }
}

impl StlExportStudio {
    pub fn focus_card(&mut self, card_key: &str) {
        self.selected_card_key = card_key.to_string();
        self.is_open = true;
    }

    pub fn is_batch_running(&self) -> bool {
        self.active_job.is_some()
    }

    fn spawn_batch_job<F>(&mut self, label: impl Into<String>, initial_total: usize, work: F)
    where
        F: FnOnce(Arc<AtomicBool>, Arc<Mutex<BatchStlProgress>>) -> BatchStlOutcome + Send + 'static,
    {
        if self.active_job.is_some() {
            return;
        }
        let label = label.into();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(BatchStlProgress {
            done: 0,
            total: initial_total,
            current_label: "Preparing...".to_string(),
        }));
        let (tx, rx) = channel();
        let cancel_w = cancel.clone();
        let prog_w = progress.clone();
        thread::spawn(move || {
            let outcome = work(cancel_w, prog_w);
            let _ = tx.send(outcome);
        });
        self.active_job = Some(BatchStlJob {
            label,
            cancel,
            progress,
            rx,
        });
    }

    fn poll_active_job(&mut self, audio: &mut SpatialAudioEngine) {
        let Some(job) = self.active_job.as_ref() else {
            return;
        };
        if let Ok(outcome) = job.rx.try_recv() {
            match outcome.result {
                Ok(msg) => {
                    self.status_message = msg;
                    if !outcome.cancelled {
                        audio.play_ui(SfxCue::BioHealChime);
                    }
                }
                Err(err) => {
                    self.status_message = format!("⚠ Batch STL export failed: {err}");
                }
            }
            self.active_job = None;
        }
    }

    pub fn start_deck_batch(&mut self, active_deck: DeckList, roster: RosterManifest, vox_dir: PathBuf, out_dir: PathBuf) {
        let cfg = self.config;
        let total_hint = active_deck.entries.iter().filter(|e| e.quantity > 0).count();
        self.spawn_batch_job(format!("Deck: {}", active_deck.deck_name), total_hint, move |cancel, progress| {
            let mut on_prog = |done: usize, total: usize, item: &str| {
                if let Ok(mut p) = progress.lock() {
                    p.done = done;
                    p.total = total;
                    p.current_label = item.to_string();
                }
            };
            match export_deck_to_stl_folder_cancellable(
                &active_deck,
                &roster,
                &vox_dir,
                &out_dir,
                &cfg,
                &cancel,
                &mut on_prog,
            ) {
                Ok((reps, true)) => BatchStlOutcome {
                    cancelled: true,
                    result: Ok(format!(
                        "⏹ Cancelled deck STL export ({} files written to {}).",
                        reps.len(),
                        out_dir.display()
                    )),
                },
                Ok((reps, false)) => BatchStlOutcome {
                    cancelled: false,
                    result: Ok(format!(
                        "✓ Exported {} unique card .STLs + PRINT_MANIFEST.txt to {}",
                        reps.len(),
                        out_dir.display()
                    )),
                },
                Err(e) => BatchStlOutcome {
                    cancelled: false,
                    result: Err(e.to_string()),
                },
            }
        });
    }

    pub fn start_all_cards_batch(&mut self, roster: RosterManifest, vox_dir: PathBuf, out_dir: PathBuf) {
        let cfg = self.config;
        let total = roster.units.len();
        self.spawn_batch_job(format!("All {} Cards", total), total, move |cancel, progress| {
            if let Err(e) = std::fs::create_dir_all(&out_dir) {
                return BatchStlOutcome {
                    cancelled: false,
                    result: Err(e.to_string()),
                };
            }
            let mut ok_cnt = 0usize;
            let mut fail_cnt = 0usize;
            let mut first_err: Option<String> = None;
            for (idx, card) in roster.units.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    return BatchStlOutcome {
                        cancelled: true,
                        result: Ok(format!(
                            "⏹ Cancelled card batch export ({ok_cnt}/{total} written to {}).",
                            out_dir.display()
                        )),
                    };
                }
                if let Ok(mut p) = progress.lock() {
                    p.done = idx;
                    p.total = total;
                    p.current_label = card.card_key.clone();
                }
                let out_p = out_dir.join(format!(
                    "{}__STR{}_ENC{}.stl",
                    sanitize_filename_slug(&card.card_key),
                    card.strength_hp,
                    card.encumbrance
                ));
                match export_card_to_stl(card, &vox_dir, &out_p, &cfg) {
                    Ok(_) => ok_cnt += 1,
                    Err(e) => {
                        fail_cnt += 1;
                        if first_err.is_none() {
                            first_err = Some(format!("{}: {e}", card.card_key));
                        }
                    }
                }
                if let Ok(mut p) = progress.lock() {
                    p.done = idx + 1;
                }
            }
            if fail_cnt > 0 {
                let err_detail = first_err.unwrap_or_else(|| "unknown export error".to_string());
                return BatchStlOutcome {
                    cancelled: false,
                    result: Err(format!(
                        "{fail_cnt}/{total} card .STL exports failed ({ok_cnt} succeeded; first error: {err_detail})"
                    )),
                };
            }
            BatchStlOutcome {
                cancelled: false,
                result: Ok(format!(
                    "✓ Exported {ok_cnt}/{total} card .STLs with STR/ENC V-cradles to {}!",
                    out_dir.display()
                )),
            }
        });
    }

    pub fn start_master_rigs_batch(&mut self, vox_dir: PathBuf, out_dir: PathBuf) {
        let cfg = self.config;
        self.spawn_batch_job("Master .VOX Rigs", 170, move |cancel, progress| {
            let mut on_prog = |done: usize, total: usize, item: &str| {
                if let Ok(mut p) = progress.lock() {
                    p.done = done;
                    p.total = total;
                    p.current_label = item.to_string();
                }
            };
            match export_vox_directory_cancellable(&vox_dir, &out_dir, &cfg, &cancel, &mut on_prog) {
                Ok((cnt, true)) => BatchStlOutcome {
                    cancelled: true,
                    result: Ok(format!(
                        "⏹ Cancelled master rig export ({cnt} rigs written to {}).",
                        out_dir.display()
                    )),
                },
                Ok((cnt, false)) => BatchStlOutcome {
                    cancelled: false,
                    result: Ok(format!(
                        "✓ Exported all {cnt} master .STL rigs to {} (center wall >= {:.2}mm)",
                        out_dir.display(),
                        cfg.center_wall_mm.max(MIN_PRINTABLE_CENTER_WALL_MM)
                    )),
                },
                Err(e) => BatchStlOutcome {
                    cancelled: false,
                    result: Err(e.to_string()),
                },
            }
        });
    }

    pub fn start_terrain_hexes_batch(&mut self, vox_dir: PathBuf, out_dir: PathBuf) {
        let cfg = self.config;
        self.spawn_batch_job("Terrain Hexes + Canal", 12, move |cancel, progress| {
            let mut on_prog = |done: usize, total: usize, item: &str| {
                if let Ok(mut p) = progress.lock() {
                    p.done = done;
                    p.total = total;
                    p.current_label = item.to_string();
                }
            };
            match export_terrain_hexes_cancellable(&vox_dir, &out_dir, &cfg, &cancel, &mut on_prog) {
                Ok((cnt, true)) => BatchStlOutcome {
                    cancelled: true,
                    result: Ok(format!(
                        "⏹ Cancelled terrain hex export ({cnt} tiles written to {}).",
                        out_dir.display()
                    )),
                },
                Ok((cnt, false)) => BatchStlOutcome {
                    cancelled: false,
                    result: Ok(format!(
                        "✓ Exported all {cnt} Modular {:.1}mm Terrain Hex & Hydro-Breach Canal .STLs to {}!",
                        cfg.target_hex_flat_mm,
                        out_dir.display()
                    )),
                },
                Err(e) => BatchStlOutcome {
                    cancelled: false,
                    result: Err(e.to_string()),
                },
            }
        });
    }

    pub fn show(
        &mut self,
        root_ui: &mut Ui,
        roster: &RosterManifest,
        active_deck: &DeckList,
        audio: &mut SpatialAudioEngine,
    ) {
        self.poll_active_job(audio);
        if !self.is_open {
            return;
        }
        if self.active_job.is_some() {
            root_ui.ctx().request_repaint_after(Duration::from_millis(50));
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
                                    let default_dir = writable_app_path("exports/stl/cards");
                                    let _ = ensure_parent_dir(&default_dir.join("placeholder"));
                                    if let Some(out_path) = rfd::FileDialog::new()
                                        .set_title("Save WildFront Miniature (.STL)")
                                        .set_directory(&default_dir)
                                        .set_file_name(&fname)
                                        .add_filter("STL Mesh", &["stl"])
                                        .save_file()
                                    {
                                        let vox_dir = resolve_asset_path("vox")
                                            .unwrap_or_else(|| writable_app_path("assets/vox"));
                                        match export_card_to_stl(
                                            card,
                                            &vox_dir,
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
                                }
                            });
                    }
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);

                // 3. Batch Deck & Full Master Library Export Row (Offloaded to Worker Thread)
                let busy = self.is_batch_running();
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !busy,
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
                        let default_dir = writable_app_path("exports/stl/decks").join(deck_slug);
                        let _ = ensure_parent_dir(&default_dir.join("placeholder"));
                        if let Some(out_dir) = rfd::FileDialog::new()
                            .set_title("Choose Output Folder for Active Deck .STLs")
                            .set_directory(&default_dir)
                            .pick_folder()
                        {
                            let vox_dir = resolve_asset_path("vox")
                                .unwrap_or_else(|| writable_app_path("assets/vox"));
                            self.start_deck_batch(active_deck.clone(), roster.clone(), vox_dir, out_dir);
                        }
                    }

                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(
                                RichText::new("🃏 Export All 152 Cards (.STL)")
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(28, 128, 112)),
                        )
                        .clicked()
                    {
                        let default_dir = writable_app_path("exports/stl/all_152_cards");
                        let _ = ensure_parent_dir(&default_dir.join("placeholder"));
                        if let Some(out_dir) = rfd::FileDialog::new()
                            .set_title("Choose Output Folder for All Card .STLs")
                            .set_directory(&default_dir)
                            .pick_folder()
                        {
                            let vox_dir = resolve_asset_path("vox")
                                .unwrap_or_else(|| writable_app_path("assets/vox"));
                            self.start_all_cards_batch(roster.clone(), vox_dir, out_dir);
                        }
                    }

                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(
                                RichText::new("🏛 Export All 170 Master Rigs (.STL)")
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(92, 76, 144)),
                        )
                        .clicked()
                    {
                        let default_dir = writable_app_path("exports/stl/master_rigs");
                        let _ = ensure_parent_dir(&default_dir.join("placeholder"));
                        if let Some(out_dir) = rfd::FileDialog::new()
                            .set_title("Choose Output Folder for All Master .STL Rigs")
                            .set_directory(&default_dir)
                            .pick_folder()
                        {
                            let vox_dir = resolve_asset_path("vox")
                                .unwrap_or_else(|| writable_app_path("assets/vox"));
                            self.start_master_rigs_batch(vox_dir, out_dir);
                        }
                    }

                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(
                                RichText::new("🗺 Export All 12 Terrain Hexes + Canal (.STL)")
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(18, 118, 152)),
                        )
                        .clicked()
                    {
                        let default_dir = writable_app_path("exports/stl/terrain_hexes_45mm");
                        let _ = ensure_parent_dir(&default_dir.join("placeholder"));
                        if let Some(out_dir) = rfd::FileDialog::new()
                            .set_title("Choose Output Folder for Terrain Hex .STLs")
                            .set_directory(&default_dir)
                            .pick_folder()
                        {
                            let vox_dir = resolve_asset_path("vox")
                                .unwrap_or_else(|| writable_app_path("assets/vox"));
                            self.start_terrain_hexes_batch(vox_dir, out_dir);
                        }
                    }
                });

                if let Some(job) = self.active_job.as_ref() {
                    ui.add_space(6.0);
                    let (done, total, cur_lbl) = job
                        .progress
                        .lock()
                        .map(|p| (p.done, p.total, p.current_label.clone()))
                        .unwrap_or((0, 1, String::new()));
                    let frac = if total > 0 {
                        (done as f32 / total as f32).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    ui.horizontal(|ui| {
                        ui.add(
                            ProgressBar::new(frac)
                                .desired_width(530.0)
                                .text(format!("{} — {}/{} ({})", job.label, done, total, cur_lbl))
                                .fill(Color32::from_rgb(48, 188, 214)),
                        );
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("✖ Cancel Export").strong().color(Color32::WHITE),
                                )
                                .fill(Color32::from_rgb(188, 56, 56)),
                            )
                            .clicked()
                        {
                            job.cancel.store(true, Ordering::Relaxed);
                            self.status_message = "Cancelling batch STL export...".to_string();
                        }
                    });
                }

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn threaded_batch_stl_export_reports_progress_and_supports_cancel() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let vox_dir = root.join("assets/vox");
        let tmp_dir = std::env::temp_dir().join("wf_stl_studio_batch_test");
        let _ = std::fs::remove_dir_all(&tmp_dir);

        let mut studio = StlExportStudio::default();
        studio.start_terrain_hexes_batch(vox_dir.clone(), tmp_dir.clone());
        assert!(studio.is_batch_running());

        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut audio = SpatialAudioEngine::new();
        while studio.is_batch_running() && std::time::Instant::now() < deadline {
            studio.poll_active_job(&mut audio);
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!studio.is_batch_running(), "Terrain batch STL export did not finish in time");
        assert!(
            studio.status_message.starts_with("✓ Exported all"),
            "Unexpected batch status: {}",
            studio.status_message
        );

        // Now verify cancellation stops a master rigs batch promptly
        let cancel_dir = std::env::temp_dir().join("wf_stl_studio_cancel_test");
        let _ = std::fs::remove_dir_all(&cancel_dir);
        studio.start_master_rigs_batch(vox_dir, cancel_dir.clone());
        if let Some(job) = studio.active_job.as_ref() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        let deadline_cancel = std::time::Instant::now() + Duration::from_secs(15);
        while studio.is_batch_running() && std::time::Instant::now() < deadline_cancel {
            studio.poll_active_job(&mut audio);
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!studio.is_batch_running(), "Cancelled batch job must exit promptly");
        assert!(
            studio.status_message.contains("Cancelled"),
            "Expected cancelled status, got: {}",
            studio.status_message
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);
        let _ = std::fs::remove_dir_all(&cancel_dir);
    }

    #[test]
    fn start_all_cards_batch_surfaces_errors_on_missing_vox_dir() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let roster: RosterManifest = ron::from_str(
            &std::fs::read_to_string(root.join("assets/compiled/master_roster.ron")).unwrap(),
        )
        .unwrap();
        let missing_vox_dir = std::env::temp_dir().join("wf_stl_missing_vox_dir_nonexistent");
        let _ = std::fs::remove_dir_all(&missing_vox_dir);
        let out_dir = std::env::temp_dir().join("wf_stl_all_cards_err_out");
        let _ = std::fs::remove_dir_all(&out_dir);

        let mut studio = StlExportStudio::default();
        studio.start_all_cards_batch(roster, missing_vox_dir, out_dir.clone());
        assert!(studio.is_batch_running());

        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut audio = SpatialAudioEngine::new();
        while studio.is_batch_running() && std::time::Instant::now() < deadline {
            studio.poll_active_job(&mut audio);
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!studio.is_batch_running());
        assert!(
            studio.status_message.contains("Batch STL export failed"),
            "Expected surfaced failure message on missing vox_dir, got: {}",
            studio.status_message
        );
        let _ = std::fs::remove_dir_all(&out_dir);
    }
}
