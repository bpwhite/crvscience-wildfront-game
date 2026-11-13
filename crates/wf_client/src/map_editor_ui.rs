//! In-Game Live 3D Hex Map & Scenario Editor Studio (`F3`) for *Wildfront Battlegrounds* (`v3.4.0`).
//!
//! Lets designers paint `Morphology`, `Elevation` (`0..=3`), `Biome`, and `Objective Tags`
//! directly onto the live GPU-raymarched 3D hexagonal battlefield with optional
//! **180° Rotational** or **Axial Mirror** tournament symmetry locks, connectivity checks,
//! and `.ron` custom scenario export/import.

use std::fs;
use std::path::Path;
use egui::{Align2, Color32, ComboBox, Frame, Margin, Order, RichText, Slider, Stroke, Ui, Window};
use hexx::Hex;
use ron::ser::{to_string_pretty, PrettyConfig};
use wf_core::mapgen::SymmetryMode;
use wf_core::{Biome, GeneratedScenario, Morphology, VictoryCondition};
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
            self.status_line = format!(
                "Painted {} hex(es) at ({}, {}) -> {:?} (Elev {}).",
                edited, clicked_hex.x, clicked_hex.y, self.selected_morphology, self.selected_elevation
            );
            return true;
        }
        false
    }

    fn validate_scenario(scen: &GeneratedScenario) -> String {
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
        format!(
            "PASS — {} total hexes ({} ground/water passable, {} aquatic/ford channels, {} objective beacons).",
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

        Window::new("🗺 HEX MAP & SCENARIO STUDIO — LIVE 3D TERRAIN SCULPTOR (F3)")
            .open(&mut open_flag)
            .anchor(Align2::TOP_RIGHT, [-14.0, 54.0])
            .collapsible(true)
            .resizable(false)
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
                        let r_max = i32::from(session.scenario.map.radius);
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
                                }
                            }
                        }
                        self.status_line = "Carved navigable North-South DeepRiver + Central ShallowFord!".into();
                        audio.play_ui(SfxCue::BioHealChime);
                        mutated_map = true;
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
                        let p = Path::new(&self.custom_file_path);
                        let res = (|| -> Result<(), String> {
                            if let Some(parent) = p.parent() {
                                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                            session.scenario.symmetry = match self.symmetry_brush {
                                SymmetryBrush::AxialMirror => SymmetryMode::AxialMirror,
                                _ => SymmetryMode::Rotational180,
                            };
                            let raw = to_string_pretty(&session.scenario, PrettyConfig::new().depth_limit(4))
                                .map_err(|e| e.to_string())?;
                            fs::write(p, raw).map_err(|e| e.to_string())
                        })();
                        match res {
                            Ok(()) => {
                                self.status_line = format!("Saved custom scenario to {}", self.custom_file_path);
                                audio.play_ui(SfxCue::BioHealChime);
                            }
                            Err(e) => self.status_line = format!("Save error: {e}"),
                        }
                    }
                    if ui.button("📂 Load Scenario .RON").clicked() {
                        let p = Path::new(&self.custom_file_path);
                        match fs::read_to_string(p)
                            .map_err(|e| e.to_string())
                            .and_then(|raw| ron::from_str::<GeneratedScenario>(&raw).map_err(|e| e.to_string()))
                        {
                            Ok(loaded) => {
                                session.load_custom_generated_scenario(loaded);
                                self.status_line = format!("Loaded & reset playtest from {}", self.custom_file_path);
                                audio.play_ui(SfxCue::UiCardTurn);
                                mutated_map = true;
                            }
                            Err(e) => self.status_line = format!("Load error: {e}"),
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
