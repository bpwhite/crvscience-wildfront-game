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

//! World-space 3D -> 2D projected floating Unit HP/Armor/Munition/Status badges
//! and rising D6 Combat Roll & Ability popups using `egui 0.37.0` non-interactive `Area` layers.

use egui::{Align2, Area, Color32, Frame, Id, Margin, Order, Pos2, ProgressBar, Rect, RichText, Stroke, Ui, Vec2};
use glam::{Mat4, Vec3};
use hexx::Hex;
use crate::game_session::ClientMatchState;
use crate::vfx::VfxManager;

/// Input descriptor for deterministic screen-space 3D unit plate AABB de-overlapping.
#[derive(Debug, Clone, Copy)]
pub struct PlateLayoutInput {
    pub slot_idx: usize,
    pub uid: u64,
    pub depth_w: f32,
    pub priority_tier: u8,
    pub anchor_pos: Pos2,
    pub size: Vec2,
}

/// Resolved screen-space placement for a 3D unit plate (`Align2::CENTER_BOTTOM` at `screen_pos`).
#[derive(Debug, Clone, Copy)]
pub struct ResolvedPlateLayout {
    pub slot_idx: usize,
    pub uid: u64,
    pub depth_w: f32,
    pub anchor_pos: Pos2,
    pub screen_pos: Pos2,
    #[allow(dead_code)]
    pub rect: Rect,
}

fn clamp_center_bottom_pos(pos: Pos2, size: Vec2, safe: Rect) -> Pos2 {
    let half_w = size.x * 0.5;
    let min_x = (safe.left() + half_w).min(safe.center().x);
    let max_x = (safe.right() - half_w).max(safe.center().x);
    let min_y = (safe.top() + size.y).min(safe.bottom());
    let max_y = safe.bottom().max(min_y);
    egui::pos2(pos.x.clamp(min_x, max_x), pos.y.clamp(min_y, max_y))
}

fn rect_from_center_bottom(pos: Pos2, size: Vec2) -> Rect {
    Rect::from_min_size(egui::pos2(pos.x - size.x * 0.5, pos.y - size.y), size)
}

fn overlap_area(a: Rect, b: Rect) -> f32 {
    let inter = a.intersect(b);
    if inter.is_positive() {
        inter.width() * inter.height()
    } else {
        0.0
    }
}

/// Resolves deterministic non-overlapping, viewport-clamped screen-space AABBs for projected 3D unit plates.
/// Returns plates ordered back-to-front (largest `depth_w` first) for painting.
pub fn resolve_unit_plate_rects(
    inputs: &[PlateLayoutInput],
    viewport_rect: Rect,
) -> Vec<ResolvedPlateLayout> {
    let safe_bounds = if viewport_rect.width() > 24.0 && viewport_rect.height() > 24.0 {
        viewport_rect.shrink(6.0)
    } else {
        viewport_rect
    };

    let mut order: Vec<usize> = (0..inputs.len()).collect();
    order.sort_by(|&ia, &ib| {
        let a = &inputs[ia];
        let b = &inputs[ib];
        a.priority_tier
            .cmp(&b.priority_tier)
            .then_with(|| a.depth_w.partial_cmp(&b.depth_w).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| a.uid.cmp(&b.uid))
    });

    let mut placed_rects: Vec<Rect> = Vec::with_capacity(inputs.len());
    let mut resolved: Vec<ResolvedPlateLayout> = Vec::with_capacity(inputs.len());
    const GAP_PX: f32 = 3.0;

    for idx in order {
        let inp = inputs[idx];
        let step_x = (inp.size.x * 0.55).max(52.0);
        let step_y = (inp.size.y + 6.0).max(24.0);

        let mut best_pos = clamp_center_bottom_pos(inp.anchor_pos, inp.size, safe_bounds);
        let mut best_rect = rect_from_center_bottom(best_pos, inp.size);
        let mut best_score = (f32::INFINITY, f32::INFINITY);

        'search: for ring in 0_i32..=12 {
            let mut ring_best_zero: Option<(f32, Pos2, Rect)> = None;
            for ky in -ring..=ring {
                for mx in -ring..=ring {
                    if ring > 0 && ky.abs().max(mx.abs()) != ring {
                        continue;
                    }
                    let raw = egui::pos2(
                        inp.anchor_pos.x + mx as f32 * step_x,
                        inp.anchor_pos.y + ky as f32 * step_y,
                    );
                    let cand_pos = clamp_center_bottom_pos(raw, inp.size, safe_bounds);
                    let cand_rect = rect_from_center_bottom(cand_pos, inp.size);
                    let padded = cand_rect.expand(GAP_PX);

                    let mut total_overlap = 0.0_f32;
                    for pr in &placed_rects {
                        total_overlap += overlap_area(padded, *pr);
                    }
                    let dist_sq = cand_pos.distance_sq(inp.anchor_pos);
                    if total_overlap <= 0.001 {
                        if ring_best_zero.map_or(true, |(best_d, _, _)| dist_sq < best_d) {
                            ring_best_zero = Some((dist_sq, cand_pos, cand_rect));
                        }
                    } else if total_overlap < best_score.0 - 0.1
                        || ((total_overlap - best_score.0).abs() <= 0.1 && dist_sq < best_score.1)
                    {
                        best_score = (total_overlap, dist_sq);
                        best_pos = cand_pos;
                        best_rect = cand_rect;
                    }
                }
            }
            if let Some((dist_sq, pos, rect)) = ring_best_zero {
                best_score = (0.0, dist_sq);
                best_pos = pos;
                best_rect = rect;
                break 'search;
            }
        }

        if best_score.0 > 0.001 {
            let half_w = inp.size.x * 0.5;
            let min_cx = (safe_bounds.left() + half_w).min(safe_bounds.center().x);
            let max_cx = (safe_bounds.right() - half_w).max(safe_bounds.center().x);
            let min_cy = (safe_bounds.top() + inp.size.y).min(safe_bounds.bottom());
            let max_cy = safe_bounds.bottom().max(min_cy);
            let scan_dx = ((inp.size.x + GAP_PX) * 0.5).max(32.0);
            let scan_dy = ((inp.size.y + GAP_PX) * 0.5).max(24.0);
            let cols = (((max_cx - min_cx) / scan_dx).ceil() as i32).max(0);
            let rows = (((max_cy - min_cy) / scan_dy).ceil() as i32).max(0);
            for r in 0..=rows {
                let cy = (min_cy + r as f32 * scan_dy).min(max_cy);
                for c in 0..=cols {
                    let cx = (min_cx + c as f32 * scan_dx).min(max_cx);
                    let cand_pos = egui::pos2(cx, cy);
                    let cand_rect = rect_from_center_bottom(cand_pos, inp.size);
                    let padded = cand_rect.expand(GAP_PX);
                    let mut total_overlap = 0.0_f32;
                    for pr in &placed_rects {
                        total_overlap += overlap_area(padded, *pr);
                    }
                    let dist_sq = cand_pos.distance_sq(inp.anchor_pos);
                    if total_overlap < best_score.0 - 0.1
                        || ((total_overlap - best_score.0).abs() <= 0.1 && dist_sq < best_score.1)
                    {
                        best_score = (total_overlap, dist_sq);
                        best_pos = cand_pos;
                        best_rect = cand_rect;
                    }
                }
            }
        }

        placed_rects.push(best_rect);
        resolved.push(ResolvedPlateLayout {
            slot_idx: inp.slot_idx,
            uid: inp.uid,
            depth_w: inp.depth_w,
            anchor_pos: inp.anchor_pos,
            screen_pos: best_pos,
            rect: best_rect,
        });
    }

    // Paint back-to-front so nearer units always layer cleanly on top
    resolved.sort_by(|a, b| {
        b.depth_w
            .partial_cmp(&a.depth_w)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.uid.cmp(&a.uid))
    });
    resolved
}

/// Projects a 3D world position into logical `egui` screen coordinates `(Pos2, clip_w)`.
/// Returns `None` if behind the camera or outside the normalized depth range `[0, 1]`.
pub fn project_world_to_egui(
    world_pos: Vec3,
    view_proj: Mat4,
    logical_screen_size: egui::Vec2,
) -> Option<(Pos2, f32)> {
    let clip = view_proj * world_pos.extend(1.0);
    if clip.w <= 0.15 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    if ndc.z < -0.05 || ndc.z > 1.05 || ndc.x.abs() > 1.15 || ndc.y.abs() > 1.15 {
        return None;
    }
    let sx = (ndc.x * 0.5 + 0.5) * logical_screen_size.x;
    let sy = (0.5 - ndc.y * 0.5) * logical_screen_size.y;
    Some((egui::pos2(sx, sy), clip.w))
}

fn str_bar_color(team_id: u8, frac: f32) -> Color32 {
    if team_id == 1 {
        if frac > 0.55 {
            Color32::from_rgb(46, 214, 136)
        } else if frac > 0.28 {
            Color32::from_rgb(245, 184, 52)
        } else {
            Color32::from_rgb(238, 74, 74)
        }
    } else if team_id == 2 {
        if frac > 0.55 {
            Color32::from_rgb(236, 96, 72)
        } else if frac > 0.28 {
            Color32::from_rgb(242, 148, 48)
        } else {
            Color32::from_rgb(198, 42, 52)
        }
    } else {
        Color32::from_rgb(238, 198, 68)
    }
}

/// Renders floating unit status plates and rising D6 combat roll callouts inside `viewport_rect`.
#[allow(clippy::too_many_arguments)]
pub fn draw_world_space_overlays(
    root_ui: &mut Ui,
    viewport_rect: Rect,
    view_proj: Mat4,
    camera_distance: f32,
    session: &ClientMatchState,
    vfx: &VfxManager,
    pick_boxes: &[(usize, Hex, Vec3, Vec3)],
    show_unit_badges: bool,
) {
    let ctx = root_ui.ctx().clone();
    let full_size = ctx.input(|i| {
        i.viewport()
            .inner_rect
            .map(|r| r.size())
            .unwrap_or_else(|| viewport_rect.size())
    });

    // 1. FLOATING WORLD-SPACE UNIT HP / ARMOR / MUNITION / NODE BADGES
    if show_unit_badges {
        let p1_supply = session.team_supply_hexes(1);
        let p2_supply = session.team_supply_hexes(2);

        let compact_zoom = camera_distance > 44.0;
        let mut layout_inputs: Vec<PlateLayoutInput> = Vec::with_capacity(pick_boxes.len());
        for &(slot_idx, _hex, bmin, bmax) in pick_boxes {
            if slot_idx >= session.entities.len() {
                continue;
            }
            let ent = &session.entities[slot_idx];
            if ent.unit.current_str_hp == 0 {
                continue;
            }
            let top_world = Vec3::new(
                (bmin.x + bmax.x) * 0.5,
                bmax.y + 0.26,
                (bmin.z + bmax.z) * 0.5,
            );
            if let Some((pos, depth_w)) = project_world_to_egui(top_world, view_proj, full_size) {
                if viewport_rect.expand(-8.0).contains(pos) {
                    let is_selected = session.selected_hex == Some(ent.hex);
                    let is_hovered = session.hovered_hex == Some(ent.hex);
                    let priority_tier = if is_selected {
                        0
                    } else if is_hovered {
                        1
                    } else {
                        2
                    };
                    let area_id = Id::unique(("wf_unit_world_plate", ent.uid));
                    let is_cloaked = (ent.card.stealth || ent.unit.burrowed)
                        && !ent.unit.target_locked
                        && !ent.unit.revealed_until_turn_start;
                    let has_status_row = ent.unit.confused
                        || ent.unit.poisoned_stacks > 0
                        || ent.unit.rooted_turns > 0
                        || ent.unit.target_locked
                        || is_cloaked
                        || (ent.card.radar_range > 0 && (is_selected || is_hovered));
                    let est_size = ctx
                        .memory(|m| m.area_rect(area_id))
                        .map(|r| r.size())
                        .filter(|s| s.x > 20.0 && s.y > 12.0)
                        .unwrap_or_else(|| {
                            if compact_zoom && !is_selected && !is_hovered {
                                egui::vec2(88.0, 22.0)
                            } else {
                                let est_w = (ent.card.unit_name.len() as f32 * 6.5 + 32.0).clamp(112.0, 178.0);
                                let est_h = if has_status_row { 74.0 } else { 58.0 };
                                egui::vec2(est_w, est_h)
                            }
                        });
                    layout_inputs.push(PlateLayoutInput {
                        slot_idx,
                        uid: ent.uid,
                        depth_w,
                        priority_tier,
                        anchor_pos: pos,
                        size: est_size,
                    });
                }
            }
        }

        let resolved_plates = resolve_unit_plate_rects(&layout_inputs, viewport_rect);
        let bg_painter = ctx.layer_painter(egui::LayerId::background());
        for plate in resolved_plates {
            let slot_idx = plate.slot_idx;
            let screen_pos = plate.screen_pos;
            let ent = &session.entities[slot_idx];
            if ent.unit.current_str_hp == 0 {
                continue;
            }
            let is_selected = session.selected_hex == Some(ent.hex);
            let is_hovered = session.hovered_hex == Some(ent.hex);
            let in_supply = match ent.team_id {
                1 => p1_supply.contains(&ent.hex),
                2 => p2_supply.contains(&ent.hex),
                _ => false,
            };

            let border_col = if is_selected {
                Color32::from_rgb(116, 255, 184)
            } else if is_hovered {
                Color32::from_rgb(228, 248, 255)
            } else if ent.unit.confused {
                Color32::from_rgb(255, 82, 82)
            } else if ent.team_id == 1 {
                Color32::from_rgb(48, 188, 214)
            } else if ent.team_id == 2 {
                Color32::from_rgb(218, 86, 68)
            } else {
                Color32::from_rgb(224, 184, 58)
            };

            if plate.anchor_pos.distance(screen_pos) > 12.0 {
                bg_painter.line_segment(
                    [plate.anchor_pos, screen_pos],
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(border_col.r(), border_col.g(), border_col.b(), 150)),
                );
            }

            let hp_frac = (f32::from(ent.unit.current_str_hp)
                / f32::from(ent.unit.max_str_hp.max(1)))
            .clamp(0.0, 1.0);
            let bar_col = str_bar_color(ent.team_id, hp_frac);

            Area::new(Id::unique(("wf_unit_world_plate", ent.uid)))
                .order(Order::Background)
                .interactable(false)
                .movable(false)
                .fade_in(false)
                .pivot(Align2::CENTER_BOTTOM)
                .fixed_pos(screen_pos)
                .show(&ctx, |ui| {
                    Frame::NONE
                        .fill(Color32::from_rgba_unmultiplied(12, 20, 26, 218))
                        .stroke(Stroke::new(if is_selected { 2.0 } else { 1.1 }, border_col))
                        .corner_radius(5)
                        .inner_margin(Margin::symmetric(6, 3))
                        .show(ui, |ui| {
                            if compact_zoom && !is_selected && !is_hovered {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    ui.label(
                                        RichText::new(format!(
                                            "❤{}/{} 🛡{}",
                                            ent.unit.current_str_hp,
                                            ent.unit.max_str_hp,
                                            ent.unit.current_armor
                                        ))
                                        .size(10.0)
                                        .strong()
                                        .color(bar_col),
                                    );
                                    if ent.unit.confused {
                                        ui.label(
                                            RichText::new("⚠")
                                                .size(10.0)
                                                .color(Color32::from_rgb(255, 92, 92)),
                                        );
                                    }
                                });
                            } else {
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 4.0;
                                        let team_tag = match ent.team_id {
                                            1 => "P1",
                                            2 => "AI",
                                            _ => "OBJ",
                                        };
                                        ui.label(
                                            RichText::new(team_tag)
                                                .size(9.5)
                                                .strong()
                                                .color(border_col),
                                        );
                                        ui.label(
                                            RichText::new(&ent.card.unit_name)
                                                .size(10.5)
                                                .strong()
                                                .color(Color32::from_rgb(240, 246, 250)),
                                        );
                                    });

                                    ui.add(
                                        ProgressBar::new(hp_frac)
                                            .desired_width(96.0)
                                            .desired_height(5.0)
                                            .fill(bar_col)
                                            .corner_radius(2),
                                    );

                                    let ammo_txt = if ent.unit.is_laser {
                                        if ent.unit.laser_fired_this_turn {
                                            "⚡CD".to_string()
                                        } else {
                                            "⚡LSR".to_string()
                                        }
                                    } else if in_supply {
                                        "🔗SUP".to_string()
                                    } else {
                                        format!("📦{}", ent.unit.carried_munitions)
                                    };
                                    let fort_txt = if ent.unit.fortified_stacks > 0 {
                                        format!("+F{}", ent.unit.fortified_stacks)
                                    } else {
                                        String::new()
                                    };

                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 5.0;
                                        ui.label(
                                            RichText::new(format!(
                                                "❤{}/{} 🛡{}{} {}",
                                                ent.unit.current_str_hp,
                                                ent.unit.max_str_hp,
                                                ent.unit.current_armor,
                                                fort_txt,
                                                ammo_txt
                                            ))
                                            .size(9.5)
                                            .color(Color32::from_rgb(205, 220, 230)),
                                        );
                                    });

                                    let is_cloaked = (ent.card.stealth || ent.unit.burrowed)
                                        && !ent.unit.target_locked
                                        && !ent.unit.revealed_until_turn_start;
                                    if ent.unit.confused
                                        || ent.unit.poisoned_stacks > 0
                                        || ent.unit.rooted_turns > 0
                                        || ent.unit.target_locked
                                        || is_cloaked
                                        || (ent.card.radar_range > 0 && (is_selected || is_hovered))
                                    {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 4.0;
                                            if ent.unit.confused {
                                                ui.label(
                                                    RichText::new("⚠ NO NODE")
                                                        .size(9.0)
                                                        .strong()
                                                        .color(Color32::from_rgb(255, 96, 96)),
                                                );
                                            }
                                            if ent.unit.poisoned_stacks > 0 {
                                                ui.label(
                                                    RichText::new(format!("☠PSN×{}", ent.unit.poisoned_stacks))
                                                        .size(9.0)
                                                        .color(Color32::from_rgb(162, 250, 72)),
                                                );
                                            }
                                            if ent.unit.rooted_turns > 0 {
                                                ui.label(
                                                    RichText::new(format!("🕸ROOT({})", ent.unit.rooted_turns))
                                                        .size(9.0)
                                                        .color(Color32::from_rgb(198, 152, 255)),
                                                );
                                            }
                                            if ent.unit.target_locked {
                                                ui.label(
                                                    RichText::new("🎯LOCK")
                                                        .size(9.0)
                                                        .color(Color32::from_rgb(84, 224, 255)),
                                                );
                                            }
                                            if is_cloaked {
                                                ui.label(
                                                    RichText::new("👁STEALTH")
                                                        .size(9.0)
                                                        .strong()
                                                        .color(Color32::from_rgb(196, 132, 255)),
                                                );
                                            }
                                            if ent.card.radar_range > 0 && (is_selected || is_hovered) {
                                                ui.label(
                                                    RichText::new(format!("📡R{}", ent.card.radar_range))
                                                        .size(9.0)
                                                        .color(Color32::from_rgb(96, 232, 255)),
                                                );
                                            }
                                        });
                                    }
                                });
                            }
                        });
                });
        }
    }

    // 2. RISING ANIMATED D6 COMBAT ROLL & TACTICAL EVENT POPUPS
    for popup in &vfx.popups {
        // Hold combat result badge until the 3D tumbling dice settle at ~0.65s!
        if popup.elapsed < 0.65 {
            continue;
        }
        let eff_elapsed = popup.elapsed - 0.65;
        let eff_lifetime = (popup.lifetime - 0.65).max(0.1);
        let progress = (eff_elapsed / eff_lifetime).clamp(0.0, 1.0);
        let float_world = popup.world_pos + Vec3::new(0.0, 0.85 + eff_elapsed * 0.95, 0.0);
        let Some((pos, _)) = project_world_to_egui(float_world, view_proj, full_size) else {
            continue;
        };
        if !viewport_rect.contains(pos) {
            continue;
        }
        let fade = if progress < 0.68 {
            1.0
        } else {
            ((1.0 - progress) / 0.32).clamp(0.0, 1.0)
        };
        let alpha = (fade * 245.0) as u8;
        let [ar, ag, ab] = popup.accent_rgb;
        let accent = Color32::from_rgba_unmultiplied(ar, ag, ab, alpha);
        let sub_col = Color32::from_rgba_unmultiplied(232, 240, 246, alpha);
        let bg = Color32::from_rgba_unmultiplied(10, 16, 22, (fade * 228.0) as u8);

        Area::new(Id::unique(("wf_combat_d6_popup", popup.id)))
            .order(Order::Middle)
            .interactable(false)
            .movable(false)
            .fade_in(false)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(pos)
            .show(&ctx, |ui| {
                Frame::NONE
                    .fill(bg)
                    .stroke(Stroke::new(1.6, accent))
                    .corner_radius(6)
                    .inner_margin(Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.5;
                            ui.label(
                                RichText::new(&popup.headline)
                                    .size(12.0)
                                    .strong()
                                    .color(accent),
                            );
                            if !popup.subline.is_empty() {
                                ui.label(
                                    RichText::new(&popup.subline)
                                        .size(10.5)
                                        .strong()
                                        .color(sub_col),
                                );
                            }
                        });
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_plate_aabb_solver_eliminates_overlap_and_clamps_to_viewport() {
        let viewport = Rect::from_min_size(egui::pos2(310.0, 50.0), egui::vec2(1290.0, 720.0));
        let inputs = vec![
            PlateLayoutInput {
                slot_idx: 0,
                uid: 101,
                depth_w: 30.0,
                priority_tier: 2,
                anchor_pos: egui::pos2(600.0, 400.0),
                size: egui::vec2(128.0, 60.0),
            },
            PlateLayoutInput {
                slot_idx: 1,
                uid: 102,
                depth_w: 29.0,
                priority_tier: 0, // Selected unit gets priority 0
                anchor_pos: egui::pos2(604.0, 402.0),
                size: egui::vec2(128.0, 60.0),
            },
            PlateLayoutInput {
                slot_idx: 2,
                uid: 103,
                depth_w: 31.0,
                priority_tier: 2,
                anchor_pos: egui::pos2(598.0, 398.0),
                size: egui::vec2(128.0, 60.0),
            },
            PlateLayoutInput {
                slot_idx: 3,
                uid: 104,
                depth_w: 25.0,
                priority_tier: 2,
                // Near top-left corner so clamping must pull it inside viewport
                anchor_pos: egui::pos2(320.0, 60.0),
                size: egui::vec2(128.0, 60.0),
            },
        ];

        let resolved = resolve_unit_plate_rects(&inputs, viewport);
        assert_eq!(resolved.len(), 4);

        // Selected unit (uid 102) stays at its original anchor
        let sel = resolved.iter().find(|r| r.uid == 102).unwrap();
        assert!((sel.screen_pos.x - 604.0).abs() < 0.01);
        assert!((sel.screen_pos.y - 402.0).abs() < 0.01);

        let safe = viewport.shrink(6.0);
        for (i, a) in resolved.iter().enumerate() {
            assert!(
                safe.contains_rect(a.rect),
                "Plate {} rect {:?} exceeded safe viewport {:?}",
                a.uid,
                a.rect,
                safe
            );
            for b in resolved.iter().skip(i + 1) {
                let area = overlap_area(a.rect, b.rect);
                assert!(
                    area <= 0.001,
                    "Plates {} ({:?}) and {} ({:?}) overlapped by {} px^2",
                    a.uid,
                    a.rect,
                    b.uid,
                    b.rect,
                    area
                );
            }
        }
    }

    #[test]
    fn unit_plate_aabb_solver_deoverlaps_corner_clustered_12_and_14_wide() {
        let viewport = Rect::from_min_size(egui::pos2(310.0, 50.0), egui::vec2(1290.0, 720.0));
        let safe = viewport.shrink(6.0);

        for (count, size) in [
            (12usize, egui::vec2(128.0, 60.0)),
            (14usize, egui::vec2(178.0, 74.0)),
        ] {
            let inputs: Vec<PlateLayoutInput> = (0..count)
                .map(|i| PlateLayoutInput {
                    slot_idx: i,
                    uid: (i + 1) as u64,
                    depth_w: 20.0 + i as f32 * 0.1,
                    priority_tier: if i == 0 { 0 } else { 2 },
                    anchor_pos: egui::pos2(320.0, 60.0),
                    size,
                })
                .collect();

            let resolved = resolve_unit_plate_rects(&inputs, viewport);
            assert_eq!(resolved.len(), count);
            for (i, a) in resolved.iter().enumerate() {
                assert!(
                    safe.contains_rect(a.rect),
                    "count={count}: Plate {} ({:?}) out of safe bounds {:?}",
                    a.uid,
                    a.rect,
                    safe
                );
                for b in resolved.iter().skip(i + 1) {
                    let area = overlap_area(a.rect, b.rect);
                    assert!(
                        area <= 0.001,
                        "count={count}: Plates {} ({:?}) and {} ({:?}) overlapped by {} px^2",
                        a.uid,
                        a.rect,
                        b.uid,
                        b.rect,
                        area
                    );
                }
            }
        }
    }
}
