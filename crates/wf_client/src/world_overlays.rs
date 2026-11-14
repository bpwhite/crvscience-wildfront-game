//! World-space 3D -> 2D projected floating Unit HP/Armor/Munition/Status badges
//! and rising D6 Combat Roll & Ability popups using `egui 0.37.0` non-interactive `Area` layers.

use egui::{Align2, Area, Color32, Frame, Id, Margin, Order, Pos2, ProgressBar, Rect, RichText, Stroke, Ui};
use glam::{Mat4, Vec3};
use hexx::Hex;
use crate::game_session::ClientMatchState;
use crate::vfx::VfxManager;

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

        // Sort back-to-front (largest clip_w painted first)
        let mut projected_units: Vec<(f32, usize, Pos2)> = Vec::with_capacity(pick_boxes.len());
        for &(slot_idx, _hex, bmin, bmax) in pick_boxes {
            if slot_idx >= session.entities.len() {
                continue;
            }
            let top_world = Vec3::new(
                (bmin.x + bmax.x) * 0.5,
                bmax.y + 0.26,
                (bmin.z + bmax.z) * 0.5,
            );
            if let Some((pos, depth_w)) = project_world_to_egui(top_world, view_proj, full_size) {
                if viewport_rect.expand(-8.0).contains(pos) {
                    projected_units.push((depth_w, slot_idx, pos));
                }
            }
        }
        projected_units.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let compact_zoom = camera_distance > 44.0;
        for (_depth, slot_idx, screen_pos) in projected_units {
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
