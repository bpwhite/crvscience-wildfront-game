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

//! PhyloPic-style Organism Silhouette Renderer for *Wildfront Battlegrounds*.
//!
//! Draws crisp, high-contrast biological & mechanical silhouettes inside `egui::Painter`
//! for every card in the Database Manager (`F2`), Deck Builder (`B`), and Tactical Hand,
//! paired with each unit's scientific taxon & PhyloPic lookup URL.

use egui::{Color32, Pos2, Rect, Shape, Stroke, Ui, Vec2};
use wf_core::{CombatClass, OrganismFaction, UnitCardDef};

pub fn phylopic_genus(species: &str) -> &str {
    species.split_whitespace().next().unwrap_or("Homo")
}

pub fn phylopic_search_url(species: &str) -> String {
    format!("https://www.phylopic.org/images?query={}", phylopic_genus(species))
}

/// Renders a crisp PhyloPic-style organism silhouette inside an allocated box.
pub fn draw_unit_silhouette(ui: &mut Ui, card: &UnitCardDef, size: Vec2, tint: Color32) {
    let (rect, _resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(12, 18, 24, 210));
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), 130)),
        egui::StrokeKind::Inside,
    );

    let c = rect.center();
    let s = rect.width().min(rect.height()) * 0.38;

    let pt = |dx: f32, dy: f32| Pos2::new(c.x + dx * s, c.y + dy * s);
    let fill = tint;
    let stroke = Stroke::NONE;

    match card.faction {
        OrganismFaction::Mammal => {
            // Quadruped / Battlephant silhouette
            if card.combat_class == CombatClass::Heavy {
                let body = vec![
                    pt(-0.8, 0.1), pt(-0.6, -0.55), pt(0.2, -0.6), pt(0.65, -0.35),
                    pt(0.95, 0.25), pt(0.8, 0.45), pt(0.55, 0.05), pt(0.45, 0.75),
                    pt(0.2, 0.75), pt(0.2, 0.25), pt(-0.35, 0.25), pt(-0.4, 0.75),
                    pt(-0.65, 0.75), pt(-0.7, 0.2),
                ];
                painter.add(Shape::convex_polygon(body, fill, stroke));
            } else {
                let body = vec![
                    pt(-0.85, 0.2), pt(-0.55, -0.35), pt(0.25, -0.35), pt(0.65, -0.65),
                    pt(0.9, -0.45), pt(0.6, 0.05), pt(0.55, 0.75), pt(0.32, 0.75),
                    pt(0.3, 0.2), pt(-0.3, 0.2), pt(-0.4, 0.75), pt(-0.65, 0.75),
                ];
                painter.add(Shape::convex_polygon(body, fill, stroke));
            }
        }
        OrganismFaction::Reptile => {
            // Low-slung crocodile / serpent silhouette
            let body = vec![
                pt(-0.95, 0.25), pt(-0.45, -0.22), pt(0.35, -0.25), pt(0.95, 0.05),
                pt(0.9, 0.28), pt(0.35, 0.15), pt(0.25, 0.6), pt(0.05, 0.6),
                pt(0.05, 0.18), pt(-0.4, 0.18), pt(-0.5, 0.6), pt(-0.7, 0.6),
            ];
            painter.add(Shape::convex_polygon(body, fill, stroke));
        }
        OrganismFaction::Bird => {
            // Soaring raptor wingspan silhouette
            let wings = vec![
                pt(-0.95, -0.45), pt(-0.2, -0.15), pt(0.0, -0.65), pt(0.2, -0.15),
                pt(0.95, -0.45), pt(0.45, 0.25), pt(0.12, 0.15), pt(0.0, 0.8),
                pt(-0.12, 0.15), pt(-0.45, 0.25),
            ];
            painter.add(Shape::convex_polygon(wings, fill, stroke));
        }
        OrganismFaction::Plant => {
            // Carnivorous pitcher / spore canopy silhouette
            let stem = Rect::from_center_size(pt(0.0, 0.35), Vec2::new(0.22 * s, 0.85 * s));
            painter.rect_filled(stem, 2.0, fill);
            painter.circle_filled(pt(0.0, -0.35), 0.45 * s, fill);
            painter.circle_filled(pt(-0.52, -0.05), 0.28 * s, fill);
            painter.circle_filled(pt(0.52, -0.05), 0.28 * s, fill);
        }
        OrganismFaction::Arachnid => {
            // Arachnid / Scorpion body + legs
            painter.circle_filled(pt(0.0, 0.05), 0.32 * s, fill);
            painter.circle_filled(pt(0.0, -0.35), 0.2 * s, fill);
            for side in [-1.0_f32, 1.0_f32] {
                for leg in [-0.35_f32, -0.1, 0.15, 0.4] {
                    painter.line_segment(
                        [pt(0.2 * side, leg * 0.5), pt(0.85 * side, leg - 0.15)],
                        Stroke::new(2.2, fill),
                    );
                }
            }
        }
        OrganismFaction::Insect => {
            // Winged beetle / mantis silhouette
            painter.circle_filled(pt(0.0, 0.1), 0.34 * s, fill);
            painter.circle_filled(pt(0.0, -0.42), 0.18 * s, fill);
            painter.line_segment([pt(-0.15, -0.55), pt(-0.45, -0.9)], Stroke::new(1.8, fill));
            painter.line_segment([pt(0.15, -0.55), pt(0.45, -0.9)], Stroke::new(1.8, fill));
            for side in [-1.0_f32, 1.0_f32] {
                for leg in [-0.2_f32, 0.1, 0.4] {
                    painter.line_segment(
                        [pt(0.22 * side, leg * 0.6), pt(0.78 * side, leg + 0.1)],
                        Stroke::new(2.0, fill),
                    );
                }
            }
        }
        OrganismFaction::Fish => {
            // Pelagic shark / manta silhouette
            let body = vec![
                pt(-0.9, 0.0), pt(-0.3, -0.35), pt(0.05, -0.8), pt(0.25, -0.3),
                pt(0.85, -0.35), pt(0.65, 0.0), pt(0.85, 0.35), pt(0.25, 0.25),
                pt(-0.3, 0.35),
            ];
            painter.add(Shape::convex_polygon(body, fill, stroke));
        }
        OrganismFaction::Cnidarian => {
            // Bell + trailing tentacles silhouette
            painter.circle_filled(pt(0.0, -0.32), 0.48 * s, fill);
            for tx in [-0.35_f32, -0.12, 0.12, 0.35] {
                painter.line_segment([pt(tx, -0.05), pt(tx * 1.2, 0.8)], Stroke::new(2.0, fill));
            }
        }
        OrganismFaction::Amphibian => {
            // Frog / Salamander silhouette
            painter.circle_filled(pt(0.0, -0.1), 0.36 * s, fill);
            painter.circle_filled(pt(-0.22, -0.45), 0.12 * s, fill);
            painter.circle_filled(pt(0.22, -0.45), 0.12 * s, fill);
            for side in [-1.0_f32, 1.0_f32] {
                painter.line_segment([pt(0.25 * side, 0.15), pt(0.75 * side, 0.65)], Stroke::new(2.6, fill));
                painter.line_segment([pt(0.25 * side, -0.15), pt(0.65 * side, -0.1)], Stroke::new(2.2, fill));
            }
        }
        OrganismFaction::Human => {
            if card.combat_class.is_building() {
                let base = Rect::from_center_size(pt(0.0, 0.25), Vec2::new(1.3 * s, 0.8 * s));
                painter.rect_filled(base, 2.0, fill);
                let spire = Rect::from_center_size(pt(0.0, -0.35), Vec2::new(0.45 * s, 0.6 * s));
                painter.rect_filled(spire, 1.5, fill);
            } else {
                painter.circle_filled(pt(0.0, -0.48), 0.22 * s, fill);
                let torso = Rect::from_center_size(pt(0.0, 0.05), Vec2::new(0.48 * s, 0.68 * s));
                painter.rect_filled(torso, 3.0, fill);
                painter.line_segment([pt(-0.15, 0.38), pt(-0.22, 0.85)], Stroke::new(3.0, fill));
                painter.line_segment([pt(0.15, 0.38), pt(0.22, 0.85)], Stroke::new(3.0, fill));
            }
        }
    }
}
