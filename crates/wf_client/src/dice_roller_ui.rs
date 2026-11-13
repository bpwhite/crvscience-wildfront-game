//! Interactive In-Game Simulated Combat Dice Roller & Round Logistics Hub (`R`).
//!
//! Provides:
//! - Visual 2D/3D pip rendering for 1D6 Hit Roll & 1D6 Strength Damage Roll
//! - Live tumbling animation with procedural rolling sound / tick cues
//! - Exact mathematical combat breakdown: `Target = clamp(1 + Dist + Evd + Cover - Lock, 2..6)`,
//!   `Capped Roll = min(DmgD6, Attacker_STR)`, `Delta STR = clamp(Att_STR - Def_STR, -2..+3)`,
//!   `Raw Strike = max(1, Capped + Delta)`, and `Armor Absorption`
//! - Auto-populate from current target pair or custom manual test parameters
//! - Full round tracker, phase flow control, and rolling history log.

use egui::{Align2, Color32, Frame, Margin, Order, RichText, ScrollArea, Stroke, Ui, Window};
use wf_core::combat::{required_ranged_hit_roll, resolve_attack, CombatContext, CombatOutcome};
use wf_core::mapgen::SplitMix64;
use wf_core::OrganismFaction;
use crate::audio::{SfxCue, SpatialAudioEngine};
use crate::game_session::ClientMatchState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollPhase {
    Idle,
    Rolling { elapsed: f32, duration: f32 },
    Resolved,
}

#[derive(Debug, Clone)]
pub struct DiceHistoryEntry {
    pub round_num: u32,
    pub attacker_name: String,
    pub defender_name: String,
    pub hit_d6: u8,
    pub hit_target: u8,
    pub hit_succeeded: bool,
    pub dmg_d6: u8,
    pub capped_dmg: u8,
    pub delta_str: i16,
    pub raw_strike: u8,
    pub armor_absorbed: u8,
    pub str_damage: u8,
    pub defender_destroyed: bool,
}

pub struct DiceRollerStudio {
    pub is_open: bool,
    pub phase: RollPhase,
    pub anim_timer: f32,
    pub display_hit_pip: u8,
    pub display_dmg_pip: u8,
    pub manual_dist: u8,
    pub manual_att_str: u8,
    pub manual_def_str: u8,
    pub manual_def_arm: u8,
    pub manual_def_evd: u8,
    pub manual_in_cover: bool,
    pub manual_target_lock: bool,
    pub manual_is_melee: bool,
    pub manual_att_name: String,
    pub manual_def_name: String,
    pub last_outcome: Option<CombatOutcome>,
    pub last_calc_details: Option<(u8, i16, u8)>, // (capped, delta, raw)
    pub history: Vec<DiceHistoryEntry>,
    rng: SplitMix64,
}

impl Default for DiceRollerStudio {
    fn default() -> Self {
        Self {
            is_open: false,
            phase: RollPhase::Idle,
            anim_timer: 0.0,
            display_hit_pip: 6,
            display_dmg_pip: 5,
            manual_dist: 2,
            manual_att_str: 6,
            manual_def_str: 5,
            manual_def_arm: 2,
            manual_def_evd: 1,
            manual_in_cover: false,
            manual_target_lock: false,
            manual_is_melee: false,
            manual_att_name: "Mammal Battlephant".into(),
            manual_def_name: "Plant Spore Mortar".into(),
            last_outcome: None,
            last_calc_details: None,
            history: Vec::new(),
            rng: SplitMix64::new(0x2072_D1CE_9988),
        }
    }
}

impl DiceRollerStudio {
    pub fn open_with_selection(&mut self, session: &ClientMatchState) {
        self.is_open = true;
        if let (Some(sel), Some(hov)) = (session.selected_hex, session.hovered_hex) {
            if let (Some(att_idx), Some(def_idx)) = (session.entity_at(sel), session.entity_at(hov)) {
                let att = &session.entities[att_idx];
                let def = &session.entities[def_idx];
                self.manual_att_name = att.card.unit_name.clone();
                self.manual_def_name = def.card.unit_name.clone();
                self.manual_att_str = att.unit.current_str_hp;
                self.manual_def_str = def.unit.current_str_hp;
                self.manual_def_arm = def.unit.current_armor;
                self.manual_def_evd = def.unit.evade;
                let dist = sel.unsigned_distance_to(hov) as u8;
                self.manual_dist = dist;
                self.manual_is_melee = dist <= att.card.melee_range && (att.card.ranged_distance == 0 || dist == 1);
                self.manual_target_lock = def.unit.target_locked;
                self.manual_in_cover = session.scenario.map.tiles.get(&hov).map_or(false, |t| t.morphology.grants_ranged_cover());
            }
        }
    }

    pub fn trigger_roll(&mut self, audio: &mut SpatialAudioEngine) {
        self.phase = RollPhase::Rolling { elapsed: 0.0, duration: 0.65 };
        audio.play_ui(SfxCue::UiCardTurn);
    }

    pub fn tick(&mut self, dt: f32, audio: &mut SpatialAudioEngine) {
        if let RollPhase::Rolling { mut elapsed, duration } = self.phase {
            elapsed += dt;
            self.anim_timer += dt;
            // Rapidly tumble random dice faces during rolling animation
            self.display_hit_pip = ((self.rng.next_u64() % 6) + 1) as u8;
            self.display_dmg_pip = ((self.rng.next_u64() % 6) + 1) as u8;

            if elapsed >= duration {
                // Final decisive roll values
                let final_hit = ((self.rng.next_u64() % 6) + 1) as u8;
                let final_dmg = ((self.rng.next_u64() % 6) + 1) as u8;
                self.display_hit_pip = final_hit;
                self.display_dmg_pip = final_dmg;

                // Resolve using exact wf_core rules
                let ctx = CombatContext {
                    hex_distance: self.manual_dist,
                    is_melee: self.manual_is_melee,
                    attacker_can_fly: false,
                    attacker_stealth_ambush: false,
                    attacker_adjacent_commander: false,
                    attacker_pack_tactics_bonus: false,
                    defender_in_rough_cover: self.manual_in_cover,
                };
                let req_hit = if self.manual_is_melee {
                    1
                } else {
                    let mut ev = i16::from(self.manual_def_evd);
                    if self.manual_in_cover { ev += 1; }
                    let mut tgt = 1 + i16::from(self.manual_dist.max(1)) + ev;
                    if self.manual_target_lock { tgt -= 1; }
                    tgt.clamp(2, 6) as u8
                };

                let capped = final_dmg.min(self.manual_att_str);
                let delta = (i16::from(self.manual_att_str) - i16::from(self.manual_def_str)).clamp(-2, 3);
                let raw_strike = (i16::from(capped) + delta).max(1) as u8;

                let hit_succeeded = self.manual_is_melee || final_hit == 6 || (final_hit > 1 && final_hit >= req_hit);
                let (armor_abs, str_loss) = if hit_succeeded {
                    let abs = raw_strike.min(self.manual_def_arm);
                    let rem = raw_strike - abs;
                    let loss = rem.min(self.manual_def_str);
                    (abs, loss)
                } else {
                    (0, 0)
                };

                let outcome = CombatOutcome {
                    hit_threshold: req_hit,
                    hit_succeeded,
                    armor_absorbed: armor_abs,
                    strength_damage_dealt: str_loss,
                    defender_destroyed: str_loss >= self.manual_def_str,
                    triggers_melee_retaliation: self.manual_is_melee && str_loss < self.manual_def_str,
                };

                self.last_outcome = Some(outcome.clone());
                self.last_calc_details = Some((capped, delta, raw_strike));
                self.phase = RollPhase::Resolved;

                self.history.push(DiceHistoryEntry {
                    round_num: self.history.len() as u32 + 1,
                    attacker_name: self.manual_att_name.clone(),
                    defender_name: self.manual_def_name.clone(),
                    hit_d6: final_hit,
                    hit_target: req_hit,
                    hit_succeeded,
                    dmg_d6: final_dmg,
                    capped_dmg: capped,
                    delta_str: delta,
                    raw_strike,
                    armor_absorbed: armor_abs,
                    str_damage: str_loss,
                    defender_destroyed: outcome.defender_destroyed,
                });

                if hit_succeeded {
                    audio.play_ui(SfxCue::BallisticImpact);
                } else {
                    audio.play_ui(SfxCue::UiCardTurn);
                }
            } else {
                self.phase = RollPhase::Rolling { elapsed, duration };
            }
        }
    }

    fn draw_pip_face(ui: &mut Ui, pip: u8, color_primary: Color32, is_rolling: bool) {
        let (rect, _resp) = ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
        let painter = ui.painter();

        let bg_col = if is_rolling {
            Color32::from_rgb(26, 36, 48)
        } else {
            Color32::from_rgb(18, 28, 38)
        };
        painter.rect(
            rect,
            8.0,
            bg_col,
            Stroke::new(2.4, color_primary),
            egui::StrokeKind::Inside,
        );

        // Standard D6 pip coordinate mapping inside 64x64 square
        let cx = rect.center().x;
        let cy = rect.center().y;
        let l = rect.left() + 16.0;
        let r = rect.right() - 16.0;
        let t = rect.top() + 16.0;
        let b = rect.bottom() - 16.0;
        let r_pip = 4.2;

        let dot = |px: f32, py: f32| {
            painter.circle_filled(egui::pos2(px, py), r_pip, color_primary);
        };

        match pip {
            1 => dot(cx, cy),
            2 => { dot(l, t); dot(r, b); }
            3 => { dot(l, t); dot(cx, cy); dot(r, b); }
            4 => { dot(l, t); dot(r, t); dot(l, b); dot(r, b); }
            5 => { dot(l, t); dot(r, t); dot(cx, cy); dot(l, b); dot(r, b); }
            6 => { dot(l, t); dot(r, t); dot(l, cy); dot(r, cy); dot(l, b); dot(r, b); }
            _ => dot(cx, cy),
        }
    }

    pub fn show(
        &mut self,
        root_ui: &mut Ui,
        session: &ClientMatchState,
        audio: &mut SpatialAudioEngine,
    ) {
        if !self.is_open {
            return;
        }
        let mut open_flag = self.is_open;

        Window::new("🎲 IN-GAME SIMULATED DICE ROLLER & ROUND LOGISTICS HUB (R)")
            .open(&mut open_flag)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .order(Order::Foreground)
            .frame(
                Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(10, 18, 26, 250))
                    .stroke(Stroke::new(2.2, Color32::from_rgb(248, 196, 68)))
                    .corner_radius(12)
                    .inner_margin(Margin::same(18)),
            )
            .show(root_ui.ctx(), |ui| {
                ui.set_min_width(700.0);
                ui.set_max_width(740.0);

                // Header bar
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("ROUND & COMBAT DICE SIMULATOR")
                            .size(17.0)
                            .strong()
                            .color(Color32::from_rgb(250, 214, 88)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Sync With Selection").on_hover_text("Load current attacker/target stats from board").clicked() {
                            self.open_with_selection(session);
                            audio.play_ui(SfxCue::UiCardTurn);
                        }
                    });
                });
                ui.separator();
                ui.add_space(4.0);

                // Two columns: Left = Visual Dice Roller Stage | Right = Parameter Controls & History
                ui.columns(2, |cols| {
                    // LEFT COLUMN: DICE STAGE & FORMULA BREAKDOWN
                    Frame::NONE
                        .fill(Color32::from_rgb(16, 26, 36))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(46, 78, 96)))
                        .corner_radius(8)
                        .inner_margin(Margin::same(14))
                        .show(&mut cols[0], |ui| {
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "{} vs {}",
                                        self.manual_att_name, self.manual_def_name
                                    ))
                                    .strong()
                                    .size(14.0)
                                    .color(Color32::WHITE),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "Distance: {} hexes • Attacker STR: {} • Defender STR: {} (ARM {})",
                                        self.manual_dist, self.manual_att_str, self.manual_def_str, self.manual_def_arm
                                    ))
                                    .small()
                                    .color(Color32::from_rgb(168, 204, 218)),
                                );
                                ui.add_space(10.0);

                                // The Two Dice
                                let is_rolling = matches!(self.phase, RollPhase::Rolling { .. });
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 24.0;
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            RichText::new("🎯 HIT ROLL (1D6)")
                                                .strong()
                                                .color(Color32::from_rgb(112, 232, 248)),
                                        );
                                        ui.add_space(4.0);
                                        Self::draw_pip_face(
                                            ui,
                                            self.display_hit_pip,
                                            Color32::from_rgb(112, 232, 248),
                                            is_rolling,
                                        );
                                        ui.add_space(4.0);
                                        ui.label(
                                            RichText::new(format!("Face: {}", self.display_hit_pip))
                                                .monospace()
                                                .color(Color32::from_rgb(172, 218, 232)),
                                        );
                                    });

                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            RichText::new("💥 DAMAGE (1D6)")
                                                .strong()
                                                .color(Color32::from_rgb(248, 112, 92)),
                                        );
                                        ui.add_space(4.0);
                                        Self::draw_pip_face(
                                            ui,
                                            self.display_dmg_pip,
                                            Color32::from_rgb(248, 112, 92),
                                            is_rolling,
                                        );
                                        ui.add_space(4.0);
                                        ui.label(
                                            RichText::new(format!("Face: {}", self.display_dmg_pip))
                                                .monospace()
                                                .color(Color32::from_rgb(248, 172, 162)),
                                        );
                                    });
                                });

                                ui.add_space(12.0);
                                let roll_btn = egui::Button::new(
                                    RichText::new(if is_rolling { "🎲 Tumbling..." } else { "🎲 ROLL COMBAT DICE (Space)" })
                                        .strong()
                                        .size(14.0)
                                        .color(Color32::WHITE),
                                )
                                .fill(if is_rolling {
                                    Color32::from_rgb(72, 84, 96)
                                } else {
                                    Color32::from_rgb(32, 142, 92)
                                });
                                if ui.add_enabled(!is_rolling, roll_btn).clicked() {
                                    self.trigger_roll(audio);
                                }
                            });

                            // Formula breakdown card
                            if let (Some(out), Some((cap, delta, raw))) =
                                (&self.last_outcome, self.last_calc_details)
                            {
                                ui.add_space(10.0);
                                Frame::NONE
                                    .fill(Color32::from_rgb(12, 20, 28))
                                    .stroke(Stroke::new(
                                        1.2,
                                        if out.hit_succeeded {
                                            Color32::from_rgb(52, 212, 142)
                                        } else {
                                            Color32::from_rgb(242, 88, 88)
                                        },
                                    ))
                                    .corner_radius(6)
                                    .inner_margin(Margin::same(10))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(if out.hit_succeeded {
                                                    "✓ HIT CONNECTED"
                                                } else {
                                                    "✗ ATTACK MISSED"
                                                })
                                                .strong()
                                                .color(if out.hit_succeeded {
                                                    Color32::from_rgb(76, 242, 162)
                                                } else {
                                                    Color32::from_rgb(255, 96, 96)
                                                }),
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "Target: {}+ on D6",
                                                            out.hit_threshold
                                                        ))
                                                        .small()
                                                        .color(Color32::WHITE),
                                                    );
                                                },
                                            );
                                        });

                                        if out.hit_succeeded {
                                            ui.add_space(4.0);
                                            ui.label(
                                                RichText::new(format!(
                                                    "• Dmg Cap: min({}, Att_STR {}) = {}\n\
                                                     • Strength Diff: ({:+} clamp -2..+3) = {:+}\n\
                                                     • Raw Strike: max(1, {} + {:+}) = {}\n\
                                                     • Armor Absorbed: min({}, ARM {}) = -{}\n\
                                                     • Net STR Damage: {} STR LOST{}",
                                                    self.display_dmg_pip,
                                                    self.manual_att_str,
                                                    cap,
                                                    i16::from(self.manual_att_str) - i16::from(self.manual_def_str),
                                                    delta,
                                                    cap,
                                                    delta,
                                                    raw,
                                                    raw,
                                                    self.manual_def_arm,
                                                    out.armor_absorbed,
                                                    out.strength_damage_dealt,
                                                    if out.defender_destroyed { " (DESTROYED!)" } else { "" }
                                                ))
                                                .monospace()
                                                .size(9.5)
                                                .color(Color32::from_rgb(210, 232, 242)),
                                            );
                                        }
                                    });
                            }
                        });

                    // RIGHT COLUMN: PARAMETER TWEAKERS & HISTORY LOG
                    cols[1].label(
                        RichText::new("COMBAT PARAMETERS & MODIFIERS")
                            .strong()
                            .color(Color32::from_rgb(248, 196, 78)),
                    );
                    cols[1].add_space(4.0);

                    cols[1].horizontal(|ui| {
                        ui.label("Hex Distance:");
                        ui.add(egui::Slider::new(&mut self.manual_dist, 1..=5));
                    });
                    cols[1].horizontal(|ui| {
                        ui.label("Attacker STR:");
                        ui.add(egui::Slider::new(&mut self.manual_att_str, 1..=10));
                    });
                    cols[1].horizontal(|ui| {
                        ui.label("Defender STR:");
                        ui.add(egui::Slider::new(&mut self.manual_def_str, 1..=10));
                    });
                    cols[1].horizontal(|ui| {
                        ui.label("Defender Armor:");
                        ui.add(egui::Slider::new(&mut self.manual_def_arm, 0..=3));
                        ui.separator();
                        ui.label("Evade:");
                        ui.add(egui::Slider::new(&mut self.manual_def_evd, 0..=3));
                    });
                    cols[1].horizontal(|ui| {
                        ui.checkbox(&mut self.manual_in_cover, "Rough Cover (+1 Evd)");
                        ui.checkbox(&mut self.manual_target_lock, "Target Lock (-1 Target)");
                    });
                    cols[1].checkbox(&mut self.manual_is_melee, "Melee Engagement (Auto-Hits)");

                    cols[1].add_space(8.0);
                    cols[1].separator();
                    cols[1].add_space(4.0);

                    cols[1].label(
                        RichText::new(format!("ROLL HISTORY ({} Rolls)", self.history.len()))
                            .strong()
                            .color(Color32::from_rgb(176, 204, 218)),
                    );

                    ScrollArea::vertical()
                        .id_salt("dice_history_scroll")
                        .max_height(140.0)
                        .stick_to_bottom(true)
                        .show(&mut cols[1], |ui| {
                            if self.history.is_empty() {
                                ui.label(
                                    RichText::new("No rolls yet in this match. Press Space or click Roll.")
                                        .small()
                                        .color(Color32::GRAY),
                                );
                            } else {
                                for h in self.history.iter().rev() {
                                    ui.label(
                                        RichText::new(format!(
                                            "R#{}: {} vs {} -> Hit D6={}/{} ({}) | Dmg D6={} -> {} Raw - {} Arm = -{} STR{}",
                                            h.round_num,
                                            h.attacker_name,
                                            h.defender_name,
                                            h.hit_d6,
                                            h.hit_target,
                                            if h.hit_succeeded { "HIT" } else { "MISS" },
                                            h.dmg_d6,
                                            h.raw_strike,
                                            h.armor_absorbed,
                                            h.str_damage,
                                            if h.defender_destroyed { " [KO]" } else { "" }
                                        ))
                                        .monospace()
                                        .size(9.0)
                                        .color(if h.hit_succeeded {
                                            Color32::from_rgb(168, 242, 196)
                                        } else {
                                            Color32::from_rgb(242, 142, 142)
                                        }),
                                    );
                                }
                            }
                        });
                });
            });

        self.is_open = open_flag;
    }
}
