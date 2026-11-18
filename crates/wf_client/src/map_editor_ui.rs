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

//! In-Game Live 3D Hex Map & Scenario Editor Studio (`F3`) for *Wildfront Battlegrounds* (`v3.4.1`).
//!
//! Lets designers paint `Morphology`, `Elevation` (`0..=3`), `Biome`, and `Objective Tags`
//! directly onto the live GPU-raymarched 3D hexagonal battlefield with optional
//! **180° Rotational** or **Axial Mirror** tournament symmetry locks, Undo/Redo history,
//! directional `cliff_edges` mask synchronization, BFS HQ reachability validation,
//! and `.ron` custom scenario export/import.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use egui::{Color32, ComboBox, Frame, Margin, Order, RichText, Slider, Stroke, Ui, Window};
use hexx::Hex;
use ron::ser::{to_string_pretty, PrettyConfig};
use wf_core::mapgen::SymmetryMode;
use wf_core::{Biome, GeneratedScenario, HexTile, Morphology, VictoryCondition};
use crate::audio::{SfxCue, SpatialAudioEngine};
use crate::game_session::ClientMatchState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymmetryBrush {
    FreeformSingleHex,
    Rotational180,
    AxialMirror,
}

impl SymmetryBrush {
    pub fn label(&self) -> &'static str {
        match self {
            Self::FreeformSingleHex => "Freeform (1 Hex)",
            Self::Rotational180 => "180° Rotational Mirror",
            Self::AxialMirror => "Axial Mirror",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MapEditorSnapshot {
    tiles: HashMap<Hex, HexTile>,
    objective_hexes: Vec<(i32, i32)>,
    description: String,
}

pub struct MapEditorStudio {
    pub is_open: bool,
    pub brush_paint_enabled: bool,
    pub selected_morphology: Morphology,
    pub selected_biome: Biome,
    pub selected_elevation: u8,
    pub selected_objective_tag: u8,
    pub symmetry_brush: SymmetryBrush,
    pub custom_file_path: String,
    pub status_line: String,
    pub undo_stack: Vec<MapEditorSnapshot>,
    pub redo_stack: Vec<MapEditorSnapshot>,
}

impl Default for MapEditorStudio {
    fn default() -> Self {
        Self {
            is_open: false,
            brush_paint_enabled: true,
            selected_morphology: Morphology::ShallowFord,
            selected_biome: Biome::AshwoodBasin,
            selected_elevation: 0,
            selected_objective_tag: 0,
            symmetry_brush: SymmetryBrush::Rotational180,
            custom_file_path: "scenarios/custom_arena_2072.ron".to_string(),
            status_line: "Click any 3D battlefield hex while Brush Mode is ON to sculpt terrain live!".to_string(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }
}

const ALL_MORPHOLOGIES: [(Morphology, &str); 10] = [
    (Morphology::Flat, "Flat Sector"),
    (Morphology::RoughCover, "Rough Cover (+1 Ranged EVD)"),
    (Morphology::ShallowFord, "Shallow Ford / Canal"),
    (Morphology::DeepRiver, "Deep River Channel"),
    (Morphology::OpenOcean, "Open Ocean Trench"),
    (Morphology::ArmoredDam, "Armored Dam Sluice"),
    (Morphology::BunkerRuins, "Pre-War Bunker Ruins"),
    (Morphology::RadiationFallout, "Radiation Fallout Vent"),
    (Morphology::MountainPeak, "Mountain Peak (Blocks LoS)"),
    (Morphology::RavineChasm, "Ravine Chasm (Flyers Only)"),
];

const ALL_BIOMES: [Biome; 6] = [
    Biome::IrradiatedBadlands,
    Biome::SavannahDustbowl,
    Biome::AshwoodBasin,
    Biome::OvergrownBioJungle,
    Biome::SunkenAtoll,
    Biome::RuinedBunkerCaldera,
];

impl MapEditorStudio {
    fn capture_snapshot(session: &ClientMatchState, description: impl Into<String>) -> MapEditorSnapshot {
        MapEditorSnapshot {
            tiles: session.scenario.map.tiles.clone(),
            objective_hexes: session.scenario.objective_hexes.clone(),
            description: description.into(),
        }
    }

    fn push_undo(&mut self, snap: MapEditorSnapshot) {
        const MAX_UNDO_DEPTH: usize = 64;
        if self.undo_stack.len() >= MAX_UNDO_DEPTH {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snap);
        self.redo_stack.clear();
    }

    pub fn undo(&mut self, session: &mut ClientMatchState) -> bool {
        let Some(prev) = self.undo_stack.pop() else {
            return false;
        };
        let cur = Self::capture_snapshot(session, prev.description.clone());
        self.redo_stack.push(cur);
        session.scenario.map.tiles = prev.tiles;
        session.scenario.objective_hexes = prev.objective_hexes;
        session.scenario.map.recompute_cliff_masks();
        self.status_line = format!("↩ Undid: {}", prev.description);
        true
    }

    pub fn redo(&mut self, session: &mut ClientMatchState) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        let cur = Self::capture_snapshot(session, next.description.clone());
        self.undo_stack.push(cur);
        session.scenario.map.tiles = next.tiles;
        session.scenario.objective_hexes = next.objective_hexes;
        session.scenario.map.recompute_cliff_masks();
        self.status_line = format!("↪ Redid: {}", next.description);
        true
    }

    /// Applies the active terrain brush to `clicked_hex` (and its symmetric twin if enabled).
    pub fn paint_hex(&mut self, session: &mut ClientMatchState, clicked_hex: Hex) -> bool {
        if !self.is_open || !self.brush_paint_enabled {
            return false;
        }
        let mut targets = vec![clicked_hex];
        match self.symmetry_brush {
            SymmetryBrush::FreeformSingleHex => {}
            SymmetryBrush::Rotational180 => {
                let twin = Hex::new(-clicked_hex.x, -clicked_hex.y);
                if twin != clicked_hex {
                    targets.push(twin);
                }
            }
            SymmetryBrush::AxialMirror => {
                let twin = Hex::new(-clicked_hex.x - clicked_hex.y, clicked_hex.y);
                if twin != clicked_hex {
                    targets.push(twin);
                }
            }
        }

        let before_snap = Self::capture_snapshot(
            session,
            format!("Paint ({}, {}) -> {:?}", clicked_hex.x, clicked_hex.y, self.selected_morphology),
        );
        let mut edited = 0usize;
        for h in targets {
            // Protect occupied HQ hexes from impassable Mountains/Chasms
            let has_hq = session.entities.iter().any(|e| {
                e.unit.current_str_hp > 0 && e.hex == h && e.card.combat_class.is_building()
            });
            if has_hq
                && matches!(
                    self.selected_morphology,
                    Morphology::MountainPeak
                        | Morphology::RavineChasm
                        | Morphology::DeepRiver
                        | Morphology::OpenOcean
                )
            {
                self.status_line = format!(
                    "Protected occupied building at ({}, {}) from impassable terrain.",
                    h.x, h.y
                );
                continue;
            }
            if let Some(tile) = session.scenario.map.tiles.get_mut(&h) {
                tile.morphology = self.selected_morphology;
                tile.biome = self.selected_biome;
                tile.elevation = if self.selected_morphology == Morphology::MountainPeak {
                    3
                } else if matches!(
                    self.selected_morphology,
                    Morphology::DeepRiver | Morphology::OpenOcean | Morphology::RavineChasm
                ) {
                    0
                } else {
                    self.selected_elevation.min(2)
                };
                tile.objective_tag = self.selected_objective_tag;

                let coord = (h.x, h.y);
                if self.selected_objective_tag > 0 {
                    if !session.scenario.objective_hexes.contains(&coord) {
                        session.scenario.objective_hexes.push(coord);
                    }
                } else {
                    session.scenario.objective_hexes.retain(|&c| c != coord);
                }
                edited += 1;
            }
        }

        if edited > 0 {
            self.push_undo(before_snap);
            session.scenario.map.recompute_cliff_masks();
            self.status_line = format!(
                "Painted {} hex(es) at ({}, {}) -> {:?} (Elev {}).",
                edited, clicked_hex.x, clicked_hex.y, self.selected_morphology, self.selected_elevation
            );
            return true;
        }
        false
    }

    pub fn carve_central_ford_river(&mut self, session: &mut ClientMatchState) -> bool {
        let before_snap = Self::capture_snapshot(session, "Carve Central Ford River");
        let r_max = i32::from(session.scenario.map.radius);
        let mut changed = false;
        for r in -r_max..=r_max {
            let h = Hex::new(0, r);
            if session.entity_at(h).is_none() {
                if let Some(t) = session.scenario.map.tiles.get_mut(&h) {
                    t.morphology = if r.abs() <= 1 {
                        Morphology::ShallowFord
                    } else {
                        Morphology::DeepRiver
                    };
                    t.elevation = 0;
                    changed = true;
                }
            }
        }
        if changed {
            self.push_undo(before_snap);
            session.scenario.map.recompute_cliff_masks();
            self.status_line = "Carved navigable North-South DeepRiver + Central ShallowFord!".into();
        }
        changed
    }

    /// Audits map terrain counts AND runs a ground BFS from Team 1 HQ to Team 2 HQ
    /// (plus objective beacons) respecting elevation cliffs, peaks, chasms, and deep water.
    pub fn validate_scenario(scen: &GeneratedScenario) -> String {
        let total = scen.map.tiles.len();
        let water = scen
            .map
            .tiles
            .values()
            .filter(|t| matches!(t.morphology, Morphology::DeepRiver | Morphology::OpenOcean | Morphology::ShallowFord))
            .count();
        let passable = scen
            .map
            .tiles
            .values()
            .filter(|t| !matches!(t.morphology, Morphology::MountainPeak | Morphology::RavineChasm))
            .count();

        let Some(p1_hq) = scen
            .placements
            .iter()
            .find(|p| p.team_id == 1 && p.card_key.contains("Command Center"))
            .map(|p| Hex::new(p.q, p.r))
        else {
            return "FAIL — Missing Team 1 Command Center placement.".to_string();
        };
        let Some(p2_hq) = scen
            .placements
            .iter()
            .find(|p| p.team_id == 2 && p.card_key.contains("Command Center"))
            .map(|p| Hex::new(p.q, p.r))
        else {
            return "FAIL — Missing Team 2 Command Center placement.".to_string();
        };

        let (Some(p1_tile), Some(p2_tile)) = (scen.map.tiles.get(&p1_hq), scen.map.tiles.get(&p2_hq))
        else {
            return format!(
                "FAIL — Missing HQ anchor tile(s): P1 ({}, {}), P2 ({}, {}).",
                p1_hq.x, p1_hq.y, p2_hq.x, p2_hq.y
            );
        };
        let is_ground_impassable = |m: Morphology| {
            matches!(
                m,
                Morphology::MountainPeak
                    | Morphology::RavineChasm
                    | Morphology::DeepRiver
                    | Morphology::OpenOcean
            )
        };
        if is_ground_impassable(p1_tile.morphology) || is_ground_impassable(p2_tile.morphology) {
            return format!(
                "FAIL — HQ sits on ground-impassable terrain (P1 ({}, {}): {:?}, P2 ({}, {}): {:?}).",
                p1_hq.x,
                p1_hq.y,
                p1_tile.morphology,
                p2_hq.x,
                p2_hq.y,
                p2_tile.morphology
            );
        }

        let mut visited: HashSet<Hex> = HashSet::new();
        let mut queue: VecDeque<Hex> = VecDeque::new();
        visited.insert(p1_hq);
        queue.push_back(p1_hq);

        while let Some(curr) = queue.pop_front() {
            for nb in curr.all_neighbors() {
                if !visited.contains(&nb) && scen.map.can_traverse(curr, nb, false, false, 0) {
                    visited.insert(nb);
                    queue.push_back(nb);
                }
            }
        }

        if !visited.contains(&p2_hq) {
            return format!(
                "FAIL — P1 HQ ({}, {}) cannot reach P2 HQ ({}, {}) on ground ({} hexes reachable from P1; blocked by cliffs/peaks/deep water).",
                p1_hq.x, p1_hq.y, p2_hq.x, p2_hq.y, visited.len()
            );
        }

        let unreachable_obj: Vec<(i32, i32)> = scen
            .objective_hexes
            .iter()
            .copied()
            .filter(|&(ox, oy)| !visited.contains(&Hex::new(ox, oy)))
            .collect();
        if !unreachable_obj.is_empty() {
            return format!(
                "FAIL — P1/P2 HQs connected ({} hexes), but {} objective beacon(s) unreachable on ground (e.g. {:?}).",
                visited.len(),
                unreachable_obj.len(),
                unreachable_obj[0]
            );
        }

        format!(
            "PASS — HQ-to-HQ ground BFS verified ({} reachable hexes, {} total, {} ground/ford passable, {} aquatic/ford channels, {} reachable objective beacons).",
            visited.len(),
            total,
            passable,
            water,
            scen.objective_hexes.len()
        )
    }

    pub fn show(
        &mut self,
        root_ui: &mut Ui,
        session: &mut ClientMatchState,
        audio: &mut SpatialAudioEngine,
    ) -> bool {
        if !self.is_open {
            return false;
        }
        let mut open_flag = self.is_open;
        let mut mutated_map = false;

        let win_w = root_ui
            .ctx()
            .input(|i| i.viewport().inner_rect.map(|r| r.width()).unwrap_or(1480.0));
        let default_x = (win_w - 446.0).max(16.0);

        Window::new("🗺 HEX MAP & SCENARIO STUDIO — LIVE 3D TERRAIN SCULPTOR (F3)")
            .open(&mut open_flag)
            .default_pos([default_x, 54.0])
            .collapsible(true)
            .resizable(false)
            .fade_in(false)
            .order(Order::Foreground)
            .frame(
                Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(11, 20, 27, 244))
                    .stroke(Stroke::new(2.0, Color32::from_rgb(72, 214, 178)))
                    .corner_radius(10)
                    .inner_margin(Margin::same(16)),
            )
            .show(root_ui.ctx(), |ui| {
                ui.set_min_width(410.0);
                ui.set_max_width(430.0);

                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut self.brush_paint_enabled,
                        RichText::new("🖌 Click-to-Paint 3D Viewport Hexes")
                            .strong()
                            .color(Color32::from_rgb(132, 244, 204)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let redo_n = self.redo_stack.len();
                        let undo_n = self.undo_stack.len();
                        if ui
                            .add_enabled(redo_n > 0, egui::Button::new(format!("↪ Redo ({redo_n})")))
                            .clicked()
                            && self.redo(session)
                        {
                            audio.play_ui(SfxCue::UiCardTurn);
                            mutated_map = true;
                        }
                        if ui
                            .add_enabled(undo_n > 0, egui::Button::new(format!("↩ Undo ({undo_n})")))
                            .clicked()
                            && self.undo(session)
                        {
                            audio.play_ui(SfxCue::UiCardTurn);
                            mutated_map = true;
                        }
                    });
                });
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    ui.label(RichText::new("Symmetry Lock:").strong());
                    ComboBox::from_id_salt("map_ed_sym")
                        .selected_text(self.symmetry_brush.label())
                        .show_ui(ui, |ui| {
                            for sb in [
                                SymmetryBrush::FreeformSingleHex,
                                SymmetryBrush::Rotational180,
                                SymmetryBrush::AxialMirror,
                            ] {
                                ui.selectable_value(&mut self.symmetry_brush, sb, sb.label());
                            }
                        });
                });

                ui.horizontal(|ui| {
                    ui.label(RichText::new("Morphology:").strong());
                    let cur_lbl = ALL_MORPHOLOGIES
                        .iter()
                        .find(|(m, _)| *m == self.selected_morphology)
                        .map(|(_, l)| *l)
                        .unwrap_or("Flat");
                    ComboBox::from_id_salt("map_ed_morph")
                        .selected_text(cur_lbl)
                        .show_ui(ui, |ui| {
                            for (m, lbl) in ALL_MORPHOLOGIES {
                                ui.selectable_value(&mut self.selected_morphology, m, lbl);
                            }
                        });
                });

                ui.horizontal(|ui| {
                    ui.label(RichText::new("Biome Tint:").strong());
                    ComboBox::from_id_salt("map_ed_biome")
                        .selected_text(self.selected_biome.display_name())
                        .show_ui(ui, |ui| {
                            for b in ALL_BIOMES {
                                ui.selectable_value(&mut self.selected_biome, b, b.display_name());
                            }
                        });
                });

                ui.horizontal(|ui| {
                    ui.label(RichText::new("Plateau Elevation:").strong());
                    ui.add(Slider::new(&mut self.selected_elevation, 0..=2).text("Tier"));
                    ui.label(RichText::new("Obj Tag:").strong());
                    ui.add(Slider::new(&mut self.selected_objective_tag, 0..=2));
                });

                ui.add_space(6.0);
                ui.separator();
                ui.label(RichText::new("Scenario Rules & Victory Clock").strong().color(Color32::WHITE));
                ui.horizontal(|ui| {
                    ui.label("Title:");
                    ui.text_edit_singleline(&mut session.scenario.title);
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Raze HQ").clicked() {
                        session.scenario.victory = VictoryCondition::DestroyEnemyCommandCenter;
                    }
                    if ui.button("7-Round Spire").clicked() {
                        session.scenario.victory = VictoryCondition::HoldCentralControlSpire { target_turns: 7 };
                    }
                    if ui.button("12-Round Vault").clicked() {
                        session.scenario.victory = VictoryCondition::HoldVaultTurns { target_turns: 12 };
                    }
                    if ui.button("18-Mun Convoy").clicked() {
                        session.scenario.victory = VictoryCondition::SalvageMunitionsOrRaze { target_munitions: 18 };
                    }
                });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("🌊 Carve Central Ford River").clicked() {
                        if self.carve_central_ford_river(session) {
                            audio.play_ui(SfxCue::BioHealChime);
                            mutated_map = true;
                        }
                    }
                    if ui.button("✅ Audit Map").clicked() {
                        self.status_line = Self::validate_scenario(&session.scenario);
                    }
                });

                ui.add_space(6.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("RON:");
                    ui.text_edit_singleline(&mut self.custom_file_path);
                });
                ui.horizontal(|ui| {
                    if ui.button("💾 Export Scenario .RON").clicked() {
                        let default_dir = crate::paths::writable_app_path("scenarios");
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Export Custom WildFront Scenario (.RON)")
                            .set_directory(&default_dir)
                            .set_file_name("custom_arena_2072.ron")
                            .add_filter("RON Scenario", &["ron"])
                            .save_file()
                        {
                            self.custom_file_path = p.display().to_string();
                            let res = (|| -> Result<(), String> {
                                crate::paths::ensure_parent_dir(&p).map_err(|e| e.to_string())?;
                                session.scenario.symmetry = match self.symmetry_brush {
                                    SymmetryBrush::AxialMirror => SymmetryMode::AxialMirror,
                                    _ => SymmetryMode::Rotational180,
                                };
                                let raw = to_string_pretty(&session.scenario, PrettyConfig::new().depth_limit(4))
                                    .map_err(|e| e.to_string())?;
                                fs::write(&p, raw).map_err(|e| e.to_string())
                            })();
                            match res {
                                Ok(()) => {
                                    self.status_line = format!("✓ Saved custom scenario to {}", p.display());
                                    audio.play_ui(SfxCue::BioHealChime);
                                }
                                Err(e) => self.status_line = format!("⚠ Save error: {e}"),
                            }
                        }
                    }
                    if ui.button("📂 Load Scenario .RON").clicked() {
                        let default_dir = crate::paths::writable_app_path("scenarios");
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Load Custom WildFront Scenario (.RON)")
                            .set_directory(&default_dir)
                            .add_filter("RON Scenario", &["ron"])
                            .pick_file()
                        {
                            self.custom_file_path = p.display().to_string();
                            match fs::read_to_string(&p)
                                .map_err(|e| e.to_string())
                                .and_then(|raw| ron::from_str::<GeneratedScenario>(&raw).map_err(|e| e.to_string()))
                            {
                                Ok(mut loaded) => {
                                    loaded.map.recompute_cliff_masks();
                                    session.load_custom_generated_scenario(loaded);
                                    self.undo_stack.clear();
                                    self.redo_stack.clear();
                                    self.status_line = format!("✓ Loaded & reset playtest from {}", p.display());
                                    audio.play_ui(SfxCue::UiCardTurn);
                                    mutated_map = true;
                                }
                                Err(e) => self.status_line = format!("⚠ Load error: {e}"),
                            }
                        }
                    }
                });

                ui.add_space(6.0);
                ui.label(
                    RichText::new(&self.status_line)
                        .small()
                        .color(Color32::from_rgb(178, 228, 240)),
                );
            });

        self.is_open = open_flag;
        mutated_map
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use wf_core::{DeckList, RosterManifest, ScenarioId};

    fn test_session() -> ClientMatchState {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let roster: RosterManifest = ron::from_str(
            &fs::read_to_string(root.join("assets/compiled/master_roster.ron")).unwrap(),
        )
        .unwrap();
        let d0: DeckList = ron::from_str(
            &fs::read_to_string(root.join("assets/compiled/starter_deck_plant_cnidarian.ron")).unwrap(),
        )
        .unwrap();
        let d1: DeckList = ron::from_str(
            &fs::read_to_string(root.join("assets/compiled/starter_deck_mammal_reptile.ron")).unwrap(),
        )
        .unwrap();
        ClientMatchState::new(roster, [d0, d1], ScenarioId::EmergenceSkirmish, 0x2072_0814, 0)
    }

    #[test]
    fn map_editor_undo_redo_and_cliff_edges_sync() {
        let mut session = test_session();
        let mut ed = MapEditorStudio::default();
        ed.is_open = true;
        ed.brush_paint_enabled = true;
        ed.symmetry_brush = SymmetryBrush::FreeformSingleHex;

        // Clear center + neighbor elevations to 0 first
        let target = Hex::new(0, 0);
        let neighbor = Hex::new(1, 0);
        for h in [target, neighbor] {
            if let Some(t) = session.scenario.map.tiles.get_mut(&h) {
                t.morphology = Morphology::Flat;
                t.elevation = 0;
            }
        }
        session.scenario.map.recompute_cliff_masks();
        assert_eq!(session.scenario.map.tiles[&target].cliff_edges, 0);

        // Paint elevation-2 BunkerRuins at (0, 0) -> must set cliff_edges on target & neighbor
        ed.selected_morphology = Morphology::BunkerRuins;
        ed.selected_elevation = 2;
        assert!(ed.paint_hex(&mut session, target));
        assert_eq!(session.scenario.map.tiles[&target].elevation, 2);
        assert_ne!(session.scenario.map.tiles[&target].cliff_edges, 0);
        assert_ne!(session.scenario.map.tiles[&neighbor].cliff_edges, 0);

        // Undo -> restores elevation 0 and clears cliff_edges
        assert!(ed.undo(&mut session));
        assert_eq!(session.scenario.map.tiles[&target].elevation, 0);
        assert_eq!(session.scenario.map.tiles[&target].cliff_edges, 0);

        // Redo -> reapplies elevation 2 and cliff_edges
        assert!(ed.redo(&mut session));
        assert_eq!(session.scenario.map.tiles[&target].elevation, 2);
        assert_ne!(session.scenario.map.tiles[&target].cliff_edges, 0);
    }

    #[test]
    fn validate_scenario_checks_bfs_hq_reachability() {
        let mut session = test_session();
        let ok_msg = MapEditorStudio::validate_scenario(&session.scenario);
        assert!(ok_msg.starts_with("PASS"), "Expected PASS on generated scenario, got: {ok_msg}");

        // Bisect the entire map along q = 0 with impassable MountainPeak crags
        let p1 = Hex::new(session.scenario.placements[0].q, session.scenario.placements[0].r);
        let p2 = Hex::new(session.scenario.placements[1].q, session.scenario.placements[1].r);
        for (&h, tile) in session.scenario.map.tiles.iter_mut() {
            if h.x == 0 && h != p1 && h != p2 {
                tile.morphology = Morphology::MountainPeak;
                tile.elevation = 3;
            }
        }
        session.scenario.map.recompute_cliff_masks();

        let fail_msg = MapEditorStudio::validate_scenario(&session.scenario);
        assert!(
            fail_msg.starts_with("FAIL"),
            "Expected FAIL when HQs are separated by MountainPeak wall, got: {fail_msg}"
        );
    }

    #[test]
    fn validate_scenario_fails_on_missing_or_impassable_hq() {
        let mut session = test_session();
        session.scenario.placements.clear();
        let empty_msg = MapEditorStudio::validate_scenario(&session.scenario);
        assert!(
            empty_msg.starts_with("FAIL") && empty_msg.contains("Missing Team 1 Command Center"),
            "Expected FAIL when placements are empty, got: {empty_msg}"
        );

        let mut session_non_hq = test_session();
        session_non_hq
            .scenario
            .placements
            .retain(|p| !(p.team_id == 2 && p.card_key.contains("Command Center")));
        session_non_hq.scenario.placements.push(wf_core::ScenarioPlacement {
            team_id: 2,
            card_key: "Supply Depot".into(),
            vox_slug: "supply_depot".into(),
            q: 4,
            r: -2,
            yaw_radians: 0.0,
        });
        let missing_p2_msg = MapEditorStudio::validate_scenario(&session_non_hq.scenario);
        assert!(
            missing_p2_msg.starts_with("FAIL") && missing_p2_msg.contains("Missing Team 2 Command Center"),
            "Expected FAIL when Team 2 Command Center is missing, got: {missing_p2_msg}"
        );

        for bad_morph in [
            Morphology::DeepRiver,
            Morphology::OpenOcean,
            Morphology::MountainPeak,
            Morphology::RavineChasm,
        ] {
            let mut s = test_session();
            let p1 = Hex::new(s.scenario.placements[0].q, s.scenario.placements[0].r);
            s.scenario.map.tiles.get_mut(&p1).unwrap().morphology = bad_morph;
            let bad_msg = MapEditorStudio::validate_scenario(&s.scenario);
            assert!(
                bad_msg.starts_with("FAIL") && bad_msg.contains("ground-impassable"),
                "Expected FAIL for P1 HQ on {bad_morph:?}, got: {bad_msg}"
            );
        }
    }
}
