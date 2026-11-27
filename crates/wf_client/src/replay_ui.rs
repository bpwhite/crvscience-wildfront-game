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

//! In-Game Turn-by-Turn Replay Scrubber, Tactical Undo & Live Telemetry Theater (`F4`) (`v3.4.0`).

use std::fs;
use egui::{
    pos2, Align2, Color32, Frame, Margin, Order, Pos2, Rect, RichText, Slider, Stroke, Ui, Vec2,
    Window,
};
use ron::ser::{to_string_pretty, PrettyConfig};
use serde::{Deserialize, Serialize};
use crate::audio::{SfxCue, SpatialAudioEngine};
use crate::game_session::{ClientMatchState, SavedMatchSnapshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedTimelineBundle {
    pub engine_version: String,
    pub scenario_title: String,
    pub keyframes: Vec<SavedMatchSnapshot>,
}

pub struct ReplayTheaterStudio {
    pub is_open: bool,
    pub cursor_idx: usize,
    pub export_path: String,
    pub status_line: String,
}

impl Default for ReplayTheaterStudio {
    fn default() -> Self {
        Self {
            is_open: false,
            cursor_idx: 0,
            export_path: "replays/match_timeline_2072.ron".to_string(),
            status_line: "Scrub any recorded turn keyframe to inspect board state or branch play!".to_string(),
        }
    }
}

impl ReplayTheaterStudio {
    pub fn sync_to_latest(&mut self, session: &ClientMatchState) {
        if !session.timeline_frames.is_empty() {
            self.cursor_idx = session.timeline_frames.len() - 1;
        }
    }

    fn team_ehp_and_mun(snap: &SavedMatchSnapshot, team_id: u8) -> (f32, f32) {
        let ehp: f32 = snap
            .entities
            .iter()
            .filter(|e| e.team_id == team_id && e.unit.current_str_hp > 0)
            .map(|e| {
                f32::from(e.unit.current_str_hp)
                    + f32::from(e.unit.current_armor) * 1.25
                    + f32::from(e.unit.fortified_stacks) * 1.5
            })
            .sum();
        let mun = f32::from(snap.economy.home_base_munitions.get(&team_id).copied().unwrap_or(0));
        (ehp, mun)
    }

    fn draw_telemetry_chart(ui: &mut Ui, frames: &[SavedMatchSnapshot], active_idx: usize) {
        let desired = Vec2::new(560.0, 138.0);
        let (rect, _resp) = ui.allocate_exact_size(desired, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, Color32::from_rgb(14, 24, 32));
        painter.rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, Color32::from_rgb(44, 78, 96)),
            egui::StrokeKind::Inside,
        );

        if frames.is_empty() {
            return;
        }

        let samples: Vec<(f32, f32)> = frames
            .iter()
            .map(|s| (Self::team_ehp_and_mun(s, 1).0, Self::team_ehp_and_mun(s, 2).0))
            .collect();
        let max_y = samples
            .iter()
            .fold(20.0_f32, |acc, &(a, b)| acc.max(a).max(b));

        let pad_l = 12.0;
        let pad_r = 12.0;
        let pad_t = 14.0;
        let pad_b = 14.0;
        let plot = Rect::from_min_max(
            pos2(rect.left() + pad_l, rect.top() + pad_t),
            pos2(rect.right() - pad_r, rect.bottom() - pad_b),
        );

        let n = samples.len();
        let x_at = |i: usize| -> f32 {
            if n <= 1 {
                plot.center().x
            } else {
                plot.left() + (i as f32 / (n - 1) as f32) * plot.width()
            }
        };
        let y_at = |val: f32| -> f32 {
            plot.bottom() - (val / max_y).clamp(0.0, 1.0) * plot.height()
        };

        // Midline reference
        let mid_y = (plot.top() + plot.bottom()) * 0.5;
        painter.line_segment(
            [pos2(plot.left(), mid_y), pos2(plot.right(), mid_y)],
            Stroke::new(0.8, Color32::from_rgba_unmultiplied(120, 150, 170, 55)),
        );

        // P1 Cyan & P2 Coral EHP curves
        let mut p1_pts: Vec<Pos2> = Vec::with_capacity(n);
        let mut p2_pts: Vec<Pos2> = Vec::with_capacity(n);
        for (i, &(v1, v2)) in samples.iter().enumerate() {
            let x = x_at(i);
            p1_pts.push(pos2(x, y_at(v1)));
            p2_pts.push(pos2(x, y_at(v2)));
        }
        if p1_pts.len() >= 2 {
            painter.add(egui::Shape::line(
                p1_pts.clone(),
                Stroke::new(2.2, Color32::from_rgb(56, 214, 244)),
            ));
            painter.add(egui::Shape::line(
                p2_pts.clone(),
                Stroke::new(2.2, Color32::from_rgb(248, 108, 92)),
            ));
        }

        // Scrub cursor vertical marker
        let cur_i = active_idx.min(n - 1);
        let cx = x_at(cur_i);
        painter.line_segment(
            [pos2(cx, plot.top()), pos2(cx, plot.bottom())],
            Stroke::new(1.6, Color32::from_rgb(250, 216, 88)),
        );
        painter.circle_filled(p1_pts[cur_i], 4.0, Color32::from_rgb(56, 214, 244));
        painter.circle_filled(p2_pts[cur_i], 4.0, Color32::from_rgb(248, 108, 92));
    }

    pub fn show(
        &mut self,
        root_ui: &mut Ui,
        session: &mut ClientMatchState,
        active_seed: &mut u64,
        audio: &mut SpatialAudioEngine,
    ) -> bool {
        if !self.is_open {
            return false;
        }
        let mut open_flag = self.is_open;
        let mut scene_restored = false;

        let total_frames = session.timeline_frames.len();
        if total_frames > 0 && self.cursor_idx >= total_frames {
            self.cursor_idx = total_frames - 1;
        }

        Window::new("🎬 REPLAY SCRUBBER, TACTICAL UNDO & TELEMETRY THEATER (F4)")
            .open(&mut open_flag)
            .anchor(Align2::CENTER_CENTER, [0.0, 20.0])
            .collapsible(false)
            .resizable(false)
            .order(Order::Foreground)
            .frame(
                Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(11, 19, 26, 246))
                    .stroke(Stroke::new(2.0, Color32::from_rgb(248, 192, 88)))
                    .corner_radius(10)
                    .inner_margin(Margin::same(18)),
            )
            .show(root_ui.ctx(), |ui| {
                ui.set_min_width(575.0);
                ui.set_max_width(595.0);

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(" Army Combat EHP Telemetry (Cyan = P1  |  Coral = P2  |  Gold = Scrubber)")
                            .strong()
                            .color(Color32::from_rgb(220, 236, 246)),
                    );
                });
                ui.add_space(4.0);
                Self::draw_telemetry_chart(ui, &session.timeline_frames, self.cursor_idx);

                ui.add_space(8.0);
                if total_frames > 0 {
                    let max_idx = total_frames - 1;
                    let prev_idx = self.cursor_idx;
                    ui.horizontal(|ui| {
                        if ui.button("⏮ Start").clicked() {
                            self.cursor_idx = 0;
                        }
                        if ui.button("◀ Prev Keyframe").clicked() && self.cursor_idx > 0 {
                            self.cursor_idx -= 1;
                        }
                        ui.add(
                            Slider::new(&mut self.cursor_idx, 0..=max_idx)
                                .text(format!("/ {} Keyframes", total_frames)),
                        );
                        if ui.button("Next ▶").clicked() && self.cursor_idx < max_idx {
                            self.cursor_idx += 1;
                        }
                        if ui.button("⏭ Latest").clicked() {
                            self.cursor_idx = max_idx;
                        }
                    });

                    if self.cursor_idx != prev_idx {
                        let snap = session.timeline_frames[self.cursor_idx].clone();
                        *active_seed = snap.active_seed;
                        session.restore_snapshot_preserving_timeline(snap);
                        audio.play_ui(SfxCue::UiCardTurn);
                        scene_restored = true;
                    }

                    let cur_snap = &session.timeline_frames[self.cursor_idx];
                    let (p1_ehp, p1_mun) = Self::team_ehp_and_mun(cur_snap, 1);
                    let (p2_ehp, p2_mun) = Self::team_ehp_and_mun(cur_snap, 2);
                    ui.label(
                        RichText::new(format!(
                            "Keyframe #{} — Turn {} (Active Team P{})  |  P1: {:.0} EHP, {:.0} MUN  |  P2: {:.0} EHP, {:.0} MUN",
                            self.cursor_idx + 1,
                            cur_snap.turn_number,
                            cur_snap.active_team,
                            p1_ehp,
                            p1_mun,
                            p2_ehp,
                            p2_mun
                        ))
                        .color(Color32::from_rgb(168, 224, 242)),
                    );

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(RichText::new("🌿 Branch & Play From This Turn").strong())
                            .on_hover_text("Truncates future keyframes after this point so you can test an alternate tactical line!")
                            .clicked()
                        {
                            session.timeline_frames.truncate(self.cursor_idx + 1);
                            let snap = session.timeline_frames[self.cursor_idx].clone();
                            *active_seed = snap.active_seed;
                            session.restore_snapshot_preserving_timeline(snap);
                            self.status_line = format!(
                                "Branched live match from Keyframe #{} (Turn {})!",
                                self.cursor_idx + 1,
                                session.turn_number
                            );
                            audio.play_ui(SfxCue::BioHealChime);
                            scene_restored = true;
                        }
                    });
                } else {
                    ui.label("No timeline keyframes recorded yet — advance a turn to populate telemetry.");
                }

                ui.add_space(6.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Timeline File:");
                    ui.text_edit_singleline(&mut self.export_path);
                });
                ui.horizontal(|ui| {
                    if ui.button("💾 Export Timeline (.RON)").clicked() {
                        let bundle = SavedTimelineBundle {
                            engine_version: env!("CARGO_PKG_VERSION").into(),
                            scenario_title: session.scenario.title.clone(),
                            keyframes: session.timeline_frames.clone(),
                        };
                        let default_dir = crate::paths::writable_app_path("replays");
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Export Match Replay Timeline (.RON)")
                            .set_directory(&default_dir)
                            .set_file_name("match_timeline_2072.ron")
                            .add_filter("RON Timeline", &["ron"])
                            .save_file()
                        {
                            self.export_path = p.display().to_string();
                            let res = (|| -> Result<(), String> {
                                crate::paths::ensure_parent_dir(&p).map_err(|e| e.to_string())?;
                                let raw = to_string_pretty(&bundle, PrettyConfig::new().depth_limit(4))
                                    .map_err(|e| e.to_string())?;
                                fs::write(&p, raw).map_err(|e| e.to_string())
                            })();
                            match res {
                                Ok(()) => {
                                    self.status_line = format!(
                                        "✓ Exported {} keyframes to {}",
                                        session.timeline_frames.len(),
                                        p.display()
                                    );
                                    audio.play_ui(SfxCue::BioHealChime);
                                }
                                Err(e) => self.status_line = format!("⚠ Export failed: {e}"),
                            }
                        }
                    }
                    if ui.button("📂 Import Timeline (.RON)").clicked() {
                        let default_dir = crate::paths::writable_app_path("replays");
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Import Match Replay Timeline (.RON)")
                            .set_directory(&default_dir)
                            .add_filter("RON Timeline", &["ron"])
                            .pick_file()
                        {
                            self.export_path = p.display().to_string();
                            match fs::read_to_string(&p)
                                .map_err(|e| e.to_string())
                                .and_then(|raw| ron::from_str::<SavedTimelineBundle>(&raw).map_err(|e| e.to_string()))
                            {
                                Ok(bundle) if !bundle.keyframes.is_empty() => {
                                    session.timeline_frames = bundle.keyframes;
                                    self.cursor_idx = session.timeline_frames.len() - 1;
                                    let snap = session.timeline_frames[self.cursor_idx].clone();
                                    *active_seed = snap.active_seed;
                                    session.restore_snapshot_preserving_timeline(snap);
                                    self.status_line = format!(
                                        "✓ Imported {} keyframes ({})!",
                                        session.timeline_frames.len(),
                                        bundle.scenario_title
                                    );
                                    audio.play_ui(SfxCue::UiCardTurn);
                                    scene_restored = true;
                                }
                                Ok(_) => self.status_line = "⚠ Timeline file contained 0 keyframes.".into(),
                                Err(e) => self.status_line = format!("⚠ Import failed: {e}"),
                            }
                        }
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("📊 Export Balance Scorecard (.JSON)").clicked() {
                        let default_dir = crate::paths::writable_app_path("reports");
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Export Faction & Lineage EHP Scorecard (.JSON)")
                            .set_directory(&default_dir)
                            .set_file_name("wildfront_balance_scorecard.json")
                            .add_filter("JSON Scorecard", &["json"])
                            .save_file()
                        {
                            let sc = wf_sim::build_balance_scorecard(&session.roster, None);
                            match wf_sim::export_balance_scorecard_json(&p, &sc) {
                                Ok(()) => {
                                    self.status_line = format!("✓ Exported balance scorecard JSON to {}", p.display());
                                    audio.play_ui(SfxCue::BioHealChime);
                                }
                                Err(e) => self.status_line = format!("⚠ Scorecard export failed: {e}"),
                            }
                        }
                    }
                    if ui.button("📊 Export Balance Scorecard (.CSV)").clicked() {
                        let default_dir = crate::paths::writable_app_path("reports");
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Export Faction & Lineage EHP Scorecard (.CSV)")
                            .set_directory(&default_dir)
                            .set_file_name("wildfront_balance_scorecard.csv")
                            .add_filter("CSV Scorecard", &["csv"])
                            .save_file()
                        {
                            let sc = wf_sim::build_balance_scorecard(&session.roster, None);
                            match wf_sim::export_balance_scorecard_csv(&p, &sc) {
                                Ok(()) => {
                                    self.status_line = format!("✓ Exported balance scorecard CSV to {}", p.display());
                                    audio.play_ui(SfxCue::BioHealChime);
                                }
                                Err(e) => self.status_line = format!("⚠ Scorecard export failed: {e}"),
                            }
                        }
                    }
                });

                ui.add_space(4.0);
                ui.label(RichText::new(&self.status_line).small().color(Color32::from_rgb(206, 224, 236)));
            });

        self.is_open = open_flag;
        scene_restored
    }
}
