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

//! Interactive In-Game Unit & Faction Database Management System (`F2`).
//!
//! Features:
//! - Complete interactive inspection of all 152 cards & 170 voxel models across all 10 Factions & Human Core
//! - Multi-column sorting (Name, Points, STR, ENC, ARM, MOV, EVD, Range)
//! - Detailed tactical dossier breakdown (Combat Class, Graveyard Queue, Origin continent,
//!   Species binomial name, Signature Ability & mechanics, 3D Voxel Rig name, and 3D Print V-cradle specs)
//! - Faction Doctrine & Synergy matrix (Passives, active synergies, and tactical trade-offs)
//! - Direct "Add to Deck" action that injects inspected cards straight into the active Deck Builder

use egui::{Align2, Color32, Frame, Margin, Order, RichText, ScrollArea, Stroke, Ui, Window};
use wf_core::{
    AbilityCatalog, BioTypeCardDef, CombatClass, DeckList, FactionCardDef, OrganismFaction, RosterManifest, UnitCardDef,
};
use wf_core::sqlite_db::{delete_unit_from_sqlite, save_unit_to_sqlite};
use crate::audio::{SfxCue, SpatialAudioEngine};
use crate::paths::{ensure_parent_dir, resolve_asset_path, writable_app_path};
use crate::silhouettes::{draw_unit_silhouette, phylopic_genus, phylopic_search_url};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseTab {
    UnitCatalog,
    FactionDoctrines,
    CommandCards,
    ClassMatrix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Points,
    Strength,
    Encumbrance,
    Armor,
    Movement,
    Evade,
}

pub struct DatabaseManagerStudio {
    pub is_open: bool,
    pub active_tab: DatabaseTab,
    pub selected_card_key: String,
    pub search_query: String,
    pub faction_filter: Option<OrganismFaction>,
    pub class_filter: Option<CombatClass>,
    pub sort_column: SortColumn,
    pub sort_descending: bool,
    pub edit_mode: bool,
    pub db_status_msg: String,
    catalog: AbilityCatalog,
}

impl Default for DatabaseManagerStudio {
    fn default() -> Self {
        Self {
            is_open: false,
            active_tab: DatabaseTab::UnitCatalog,
            selected_card_key: "Battlephant".to_string(),
            search_query: String::new(),
            faction_filter: None,
            class_filter: None,
            sort_column: SortColumn::Points,
            sort_descending: true,
            edit_mode: false,
            db_status_msg: "SQLite DB: assets/compiled/wildfront_roster.db".to_string(),
            catalog: AbilityCatalog::canonical(),
        }
    }
}

impl DatabaseManagerStudio {
    pub fn open_for_unit(&mut self, card_key: &str) {
        self.selected_card_key = card_key.to_string();
        self.active_tab = DatabaseTab::UnitCatalog;
        self.is_open = true;
    }

    fn faction_color(fac: OrganismFaction) -> Color32 {
        match fac {
            OrganismFaction::Human => Color32::from_rgb(56, 132, 184),
            OrganismFaction::Mammal => Color32::from_rgb(202, 118, 52),
            OrganismFaction::Reptile => Color32::from_rgb(86, 164, 72),
            OrganismFaction::Plant => Color32::from_rgb(44, 178, 102),
            OrganismFaction::Bird => Color32::from_rgb(92, 172, 234),
            OrganismFaction::Arachnid => Color32::from_rgb(158, 82, 178),
            OrganismFaction::Insect => Color32::from_rgb(208, 154, 42),
            OrganismFaction::Fish => Color32::from_rgb(38, 168, 188),
            OrganismFaction::Cnidarian => Color32::from_rgb(206, 78, 164),
            OrganismFaction::Amphibian => Color32::from_rgb(84, 192, 142),
        }
    }

    fn dice_hardware_note(c: &UnitCardDef) -> String {
        let s = c.strength_hp;
        let e = c.encumbrance;
        if e == 0 {
            if s > 6 {
                format!("Single 10mm D10 (STR 1–{})", s)
            } else {
                format!("Single 7mm D6 (STR 1–{})", s)
            }
        } else if e > 6 {
            format!("7mm D6 (STR {}) + 10mm D10 (MUN {})", s, e)
        } else {
            format!("Dual 7mm D6 (STR {} + MUN {})", s, e)
        }
    }

    pub fn show(
        &mut self,
        root_ui: &mut Ui,
        roster: &mut RosterManifest,
        active_deck: &mut DeckList,
        audio: &mut SpatialAudioEngine,
    ) {
        if !self.is_open {
            return;
        }
        let mut open_flag = self.is_open;

        Window::new("🗄 IN-GAME UNIT & FACTION DATABASE MANAGEMENT SYSTEM (F2)")
            .open(&mut open_flag)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .order(Order::Foreground)
            .frame(
                Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(10, 17, 24, 252))
                    .stroke(Stroke::new(2.2, Color32::from_rgb(92, 224, 244)))
                    .corner_radius(12)
                    .inner_margin(Margin::same(18)),
            )
            .show(root_ui.ctx(), |ui| {
                ui.set_min_width(880.0);
                ui.set_max_width(920.0);

                // Top Navigation Bar
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("2072 TACTICAL CODEX & FACTION DATABASE")
                            .size(17.0)
                            .strong()
                            .color(Color32::from_rgb(112, 238, 255)),
                    );
                    ui.separator();

                    for (tab, lbl) in [
                        (DatabaseTab::UnitCatalog, "📋 152-Card Catalog"),
                        (DatabaseTab::FactionDoctrines, "🧬 10 Faction Doctrines"),
                        (DatabaseTab::CommandCards, "🎴 3 Command Cards"),
                        (DatabaseTab::ClassMatrix, "⚔ Combat Class Matrix"),
                    ] {
                        if ui.selectable_label(self.active_tab == tab, RichText::new(lbl).strong()).clicked() {
                            self.active_tab = tab;
                            audio.play_ui(SfxCue::UiCardTurn);
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("Total Units: {}", roster.units.len()))
                                .small()
                                .color(Color32::from_rgb(168, 204, 218)),
                        );
                    });
                });
                ui.separator();
                ui.add_space(4.0);

                match self.active_tab {
                    DatabaseTab::UnitCatalog => {
                        self.render_unit_catalog(ui, roster, active_deck, audio);
                    }
                    DatabaseTab::FactionDoctrines => {
                        self.render_faction_doctrines(ui);
                    }
                    DatabaseTab::CommandCards => {
                        ScrollArea::vertical().id_salt("dbm_command_cards_scroll").show(ui, |ui| {
                            ui.label(
                                RichText::new("THE 3 MAIN COMMAND CARDS ARCHITECTURE")
                                    .strong()
                                    .size(16.0)
                                    .color(Color32::from_rgb(112, 232, 248)),
                            );
                            ui.label(
                                RichText::new("In Wildfront Battlegrounds (2072), every army roster is defined by exactly 3 Command Cards:\n                                • 1 Faction Card (Continental Bunker Allegiance): Grants global command traits, continent active ability, and infrastructure boosts.\n                                • 1-2 Bio-Type Cards (Engineered Organisms): A player may choose Pure 1 Organism (Apex Mono-Doctrine) OR Hybrid 2 Organisms (Versatile Synergies).")
                                    .small()
                                    .color(Color32::from_rgb(180, 210, 225)),
                            );
                            ui.add_space(8.0);

                            ui.label(RichText::new("--- PART 1: THE 6 CONTINENTAL FACTION CARDS ---").strong().color(Color32::from_rgb(246, 192, 78)));
                            for orig in FactionCardDef::ALL {
                                let fc = FactionCardDef::get(orig);
                                Frame::NONE
                                    .fill(Color32::from_rgb(18, 28, 36))
                                    .stroke(Stroke::new(1.0, Color32::from_rgb(46, 80, 98)))
                                    .corner_radius(4)
                                    .inner_margin(Margin::same(8))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(format!("{:?}", orig)).strong().color(Color32::from_rgb(112, 232, 248)));
                                            ui.label(RichText::new(&fc.title).strong().color(Color32::WHITE));
                                            ui.label(RichText::new(format!("({})", fc.bunker_stronghold)).small().italics().color(Color32::from_rgb(160, 190, 205)));
                                        });
                                        ui.label(RichText::new(format!("Command Doctrine: {}", fc.command_doctrine)).strong().color(Color32::from_rgb(246, 192, 78)));
                                        ui.label(RichText::new(format!("Passive Effect: {}", fc.passive_summary)).small().color(Color32::from_rgb(210, 230, 240)));
                                        ui.label(RichText::new(format!("Active Ability: {} (Cost: {} Act / {} Mun) — {}", fc.active_ability_name, fc.active_action_cost, fc.active_munition_cost, fc.active_effect_summary)).small().color(Color32::from_rgb(120, 240, 160)));
                                        ui.label(RichText::new(format!("Lore: {}", fc.narrative_fluff)).small().italics().color(Color32::from_rgb(140, 175, 190)));
                                    });
                                ui.add_space(4.0);
                            }

                            ui.add_space(8.0);
                            ui.label(RichText::new("--- PART 2: THE 9 ORGANISM BIO-TYPE CARDS (PURE VS HYBRID) ---").strong().color(Color32::from_rgb(130, 240, 140)));
                            for bio in BioTypeCardDef::ALL {
                                let bc = BioTypeCardDef::get(bio);
                                Frame::NONE
                                    .fill(Color32::from_rgb(20, 32, 28))
                                    .stroke(Stroke::new(1.0, Color32::from_rgb(46, 98, 70)))
                                    .corner_radius(4)
                                    .inner_margin(Margin::same(8))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(format!("{:?}", bio)).strong().color(Self::faction_color(bio)));
                                            ui.label(RichText::new(&bc.title).strong().color(Color32::WHITE));
                                            ui.label(RichText::new(format!("Affinities: {}", bc.affinity_biomes.join(", "))).small().color(Color32::from_rgb(150, 210, 180)));
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(format!("Hybrid Doctrine: {}", bc.hybrid_doctrine_name)).strong().color(Color32::from_rgb(180, 230, 150)));
                                            ui.label(RichText::new(&bc.hybrid_doctrine_effect).small().color(Color32::from_rgb(200, 230, 210)));
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(format!("Pure Apex Doctrine: {}", bc.pure_doctrine_name)).strong().color(Color32::from_rgb(250, 210, 60)));
                                            ui.label(RichText::new(&bc.pure_doctrine_effect).small().color(Color32::from_rgb(240, 235, 200)));
                                        });
                                        ui.label(RichText::new(format!("Primal Surge: {} (Cost: {} Act / {} Mun) — {}", bc.primal_surge_name, bc.primal_surge_cost.0, bc.primal_surge_cost.1, bc.primal_surge_effect)).small().color(Color32::from_rgb(120, 240, 210)));
                                    });
                                ui.add_space(4.0);
                            }
                        });
                    }
                    DatabaseTab::ClassMatrix => {
                        self.render_class_matrix(ui, roster);
                    }
                }
            });

        self.is_open = open_flag;
    }

    fn render_unit_catalog(
        &mut self,
        ui: &mut Ui,
        roster: &mut RosterManifest,
        active_deck: &mut DeckList,
        audio: &mut SpatialAudioEngine,
    ) {
        // Filter & Search Controls
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("🔍 Search:").strong());
            ui.text_edit_singleline(&mut self.search_query);

            ui.separator();
            ui.label("Faction:");
            let fac_lbl = self.faction_filter.map_or("All Factions", |f| f.slug());
            egui::ComboBox::from_id_salt("db_fac_filter")
                .selected_text(fac_lbl)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(self.faction_filter.is_none(), "All Factions").clicked() {
                        self.faction_filter = None;
                    }
                    for f in OrganismFaction::ALL {
                        if ui.selectable_label(self.faction_filter == Some(f), format!("{:?}", f)).clicked() {
                            self.faction_filter = Some(f);
                        }
                    }
                });

            ui.separator();
            ui.label("Class:");
            let cls_lbl = self.class_filter.map_or("All Classes".to_string(), |c| format!("{:?}", c));
            egui::ComboBox::from_id_salt("db_cls_filter")
                .selected_text(cls_lbl)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(self.class_filter.is_none(), "All Classes").clicked() {
                        self.class_filter = None;
                    }
                    for c in [
                        CombatClass::Installation,
                        CombatClass::SupplyDepot,
                        CombatClass::ControlNode,
                        CombatClass::Commander,
                        CombatClass::Heavy,
                        CombatClass::Medium,
                        CombatClass::Light,
                        CombatClass::Support,
                        CombatClass::Medic,
                        CombatClass::Trap,
                        CombatClass::Artifact,
                        CombatClass::TacticalDeployment,
                        CombatClass::SupportInstallation,
                    ] {
                        if ui.selectable_label(self.class_filter == Some(c), format!("{:?}", c)).clicked() {
                            self.class_filter = Some(c);
                        }
                    }
                });

            ui.separator();
            ui.label("Sort:");
            for (col, label) in [
                (SortColumn::Points, "Pts"),
                (SortColumn::Strength, "STR"),
                (SortColumn::Encumbrance, "ENC"),
                (SortColumn::Armor, "ARM"),
                (SortColumn::Movement, "MOV"),
                (SortColumn::Evade, "EVD"),
                (SortColumn::Name, "Name"),
            ] {
                let is_active = self.sort_column == col;
                let txt = if is_active {
                    format!("{} {}", label, if self.sort_descending { "▼" } else { "▲" })
                } else {
                    label.to_string()
                };
                if ui.selectable_label(is_active, txt).clicked() {
                    if is_active {
                        self.sort_descending = !self.sort_descending;
                    } else {
                        self.sort_column = col;
                        self.sort_descending = true;
                    }
                }
            }
        });
        ui.add_space(4.0);

        // Filter and Sort Unit List
        let q = self.search_query.trim().to_lowercase();
        let mut filtered: Vec<UnitCardDef> = roster
            .units
            .iter()
            .filter(|u| {
                if let Some(f) = self.faction_filter {
                    if u.faction != f { return false; }
                }
                if let Some(c) = self.class_filter {
                    if u.combat_class != c { return false; }
                }
                if !q.is_empty() {
                    let in_key = u.card_key.to_lowercase().contains(&q);
                    let in_spec = u.species.to_lowercase().contains(&q);
                    let in_ab = u.signature_ability.to_lowercase().contains(&q);
                    let in_fac = format!("{:?}", u.faction).to_lowercase().contains(&q);
                    if !in_key && !in_spec && !in_ab && !in_fac { return false; }
                }
                true
            })
            .cloned()
            .collect();

        filtered.sort_by(|a, b| {
            let ord = match self.sort_column {
                SortColumn::Name => a.card_key.cmp(&b.card_key),
                SortColumn::Points => a.deck_points.cmp(&b.deck_points),
                SortColumn::Strength => a.strength_hp.cmp(&b.strength_hp),
                SortColumn::Encumbrance => a.encumbrance.cmp(&b.encumbrance),
                SortColumn::Armor => a.armor.cmp(&b.armor),
                SortColumn::Movement => a.movement.cmp(&b.movement),
                SortColumn::Evade => a.evade.cmp(&b.evade),
            };
            if self.sort_descending { ord.reverse() } else { ord }
        });

        // Split Layout: Left = Unit Master Table | Right = Selected Unit Tactical Dossier
        ui.columns(2, |cols| {
            // LEFT COLUMN: FILTERED UNIT MASTER LIST
            Frame::NONE
                .fill(Color32::from_rgb(15, 24, 32))
                .stroke(Stroke::new(1.0, Color32::from_rgb(42, 72, 88)))
                .corner_radius(8)
                .inner_margin(Margin::same(8))
                .show(&mut cols[0], |ui| {
                    ui.label(
                        RichText::new(format!("FILTERED UNITS ({})", filtered.len()))
                            .strong()
                            .color(Color32::from_rgb(172, 218, 232)),
                    );
                    ui.add_space(4.0);

                    ScrollArea::vertical()
                        .id_salt("db_unit_master_scroll")
                        .max_height(460.0)
                        .show(ui, |ui| {
                            for card in &filtered {
                                let is_selected = self.selected_card_key == card.card_key;
                                let badge_col = Self::faction_color(card.faction);

                                Frame::NONE
                                    .fill(if is_selected {
                                        Color32::from_rgb(26, 44, 58)
                                    } else {
                                        Color32::from_rgb(18, 28, 36)
                                    })
                                    .stroke(Stroke::new(
                                        if is_selected { 1.8 } else { 1.0 },
                                        if is_selected {
                                            Color32::from_rgb(92, 224, 244)
                                        } else {
                                            Color32::from_rgb(38, 64, 78)
                                        },
                                    ))
                                    .corner_radius(6)
                                    .inner_margin(Margin::symmetric(8, 6))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            if ui
                                                .selectable_label(
                                                    is_selected,
                                                    RichText::new(&card.card_key).strong().color(Color32::WHITE),
                                                )
                                                .clicked()
                                            {
                                                self.selected_card_key = card.card_key.clone();
                                                audio.play_ui(SfxCue::UiCardTurn);
                                            }
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(format!("{}p", card.deck_points))
                                                            .strong()
                                                            .color(Color32::from_rgb(248, 196, 78)),
                                                    );
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "STR {} • ENC {} • MOV {}",
                                                            card.strength_hp, card.encumbrance, card.movement
                                                        ))
                                                        .small()
                                                        .color(Color32::from_rgb(168, 198, 210)),
                                                    );
                                                    ui.label(
                                                        RichText::new(format!("{:?}", card.faction))
                                                            .small()
                                                            .strong()
                                                            .color(badge_col),
                                                    );
                                                },
                                            );
                                        });
                                    });
                                ui.add_space(2.0);
                            }
                        });
                });

                            if let Some(sel_idx) = roster.units.iter().position(|u| u.card_key == self.selected_card_key) {
                                let mut card_clone = roster.units[sel_idx].clone();
                                let fc = Self::faction_color(card_clone.faction);
                                let mut modified = false;
                                let mut delete_requested = false;
                                let mut clone_requested = false;

                                ScrollArea::vertical().id_salt("dbm_dossier_scroll").show(&mut cols[1], |ui| {
                                    ui.horizontal(|ui| {
                                        draw_unit_silhouette(ui, &card_clone, egui::vec2(64.0, 64.0), fc);
                                        ui.vertical(|ui| {
                                            ui.label(
                                                RichText::new(&card_clone.unit_name)
                                                    .strong()
                                                    .size(18.0)
                                                    .color(Color32::WHITE),
                                            );
                                            ui.label(
                                                RichText::new(format!("{} • {:?} • {:?}", card_clone.species, card_clone.faction, card_clone.origin))
                                                    .small()
                                                    .italics()
                                                    .color(fc),
                                            );
                                            ui.label(
                                                RichText::new(format!("PhyloPic Taxon: {}  |  Points: {} pts  |  EHP: {:.1}", phylopic_genus(&card_clone.species), card_clone.deck_points, f32::from(card_clone.combat_ehp_tenths) / 10.0))
                                                    .small()
                                                    .color(Color32::from_rgb(246, 192, 78)),
                                            );
                                            ui.hyperlink_to("🔗 PhyloPic Silhouette Lookup", phylopic_search_url(&card_clone.species));
                                        });
                                    });

                                    ui.add_space(6.0);
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.selectable_label(self.edit_mode, "✏ Edit Card Stats (SQLite)").clicked() {
                                            self.edit_mode = !self.edit_mode;
                                        }
                                        if ui.button("➕ Add to Active Deck").clicked() {
                                            if let Some(ex) = active_deck.entries.iter_mut().find(|e| e.card_key == card_clone.card_key) {
                                                ex.quantity = ex.quantity.saturating_add(1);
                                            } else {
                                                active_deck.entries.push(wf_core::DeckEntry {
                                                    card_key: card_clone.card_key.clone(),
                                                    quantity: 1,
                                                });
                                            }
                                            audio.play_ui(SfxCue::UiCardTurn);
                                            self.db_status_msg = format!("Added {} to active Deck Builder!", card_clone.card_key);
                                        }
                                        if ui.button("🧬 Clone Card").clicked() {
                                            clone_requested = true;
                                        }
                                        if ui.button("🗑 Delete").clicked() {
                                            delete_requested = true;
                                        }
                                    });

                                    ui.label(RichText::new(&self.db_status_msg).small().color(Color32::from_rgb(130, 235, 190)));
                                    ui.separator();

                                    if self.edit_mode {
                                        ui.label(RichText::new("LIVE SQLITE CARD EDITOR (AUTO-BALANCES POINTS)").strong().color(Color32::from_rgb(112, 232, 248)));
                                        egui::Grid::new("dbm_edit_grid").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                                            ui.label("Unit Name:");
                                            if ui.text_edit_singleline(&mut card_clone.unit_name).changed() { modified = true; }
                                            ui.end_row();

                                            ui.label("Species (Binomial):");
                                            if ui.text_edit_singleline(&mut card_clone.species).changed() { modified = true; }
                                            ui.end_row();

                                            ui.label("Signature Ability:");
                                            if ui.text_edit_singleline(&mut card_clone.signature_ability).changed() { modified = true; }
                                            ui.end_row();

                                            ui.label("STR (HP):");
                                            if ui.add(egui::DragValue::new(&mut card_clone.strength_hp).range(1..=30)).changed() { modified = true; }
                                            ui.end_row();

                                            ui.label("ENC (Ammo):");
                                            if ui.add(egui::DragValue::new(&mut card_clone.encumbrance).range(0..=15)).changed() { modified = true; }
                                            ui.end_row();

                                            ui.label("Armor:");
                                            if ui.add(egui::DragValue::new(&mut card_clone.armor).range(0..=10)).changed() { modified = true; }
                                            ui.end_row();

                                            ui.label("Shield:");
                                            if ui.add(egui::DragValue::new(&mut card_clone.shield).range(0..=10)).changed() { modified = true; }
                                            ui.end_row();

                                            ui.label("Movement:");
                                            if ui.add(egui::DragValue::new(&mut card_clone.movement).range(0..=10)).changed() { modified = true; }
                                            ui.end_row();

                                            ui.label("Evade:");
                                            if ui.add(egui::DragValue::new(&mut card_clone.evade).range(0..=6)).changed() { modified = true; }
                                            ui.end_row();

                                            ui.label("Melee Range:");
                                            if ui.add(egui::DragValue::new(&mut card_clone.melee_range).range(0..=3)).changed() { modified = true; }
                                            ui.end_row();

                                            ui.label("Ranged Distance:");
                                            if ui.add(egui::DragValue::new(&mut card_clone.ranged_distance).range(0..=8)).changed() { modified = true; }
                                            ui.end_row();

                                            ui.label("Radar Range:");
                                            if ui.add(egui::DragValue::new(&mut card_clone.radar_range).range(0..=6)).changed() { modified = true; }
                                            ui.end_row();

                                            ui.label("Heal / Repair:");
                                            if ui.add(egui::DragValue::new(&mut card_clone.heal_repair_range).range(0..=5)).changed() { modified = true; }
                                            ui.end_row();

                                            ui.label("Poison / Root:");
                                            ui.horizontal(|ui| {
                                                if ui.add(egui::DragValue::new(&mut card_clone.poison_stacks).range(0..=5).prefix("P:")).changed() { modified = true; }
                                                if ui.add(egui::DragValue::new(&mut card_clone.root_stacks).range(0..=5).prefix("R:")).changed() { modified = true; }
                                            });
                                            ui.end_row();
                                        });

                                        ui.horizontal_wrapped(|ui| {
                                            if ui.checkbox(&mut card_clone.is_laser, "Laser").changed() { modified = true; }
                                            if ui.checkbox(&mut card_clone.can_fly, "Flight").changed() { modified = true; }
                                            if ui.checkbox(&mut card_clone.stealth, "Stealth").changed() { modified = true; }
                                            if ui.checkbox(&mut card_clone.can_burrow, "Burrow").changed() { modified = true; }
                                            if ui.checkbox(&mut card_clone.saboteur, "Saboteur").changed() { modified = true; }
                                            if ui.checkbox(&mut card_clone.is_autonomous, "Autonomous").changed() { modified = true; }
                                            if ui.checkbox(&mut card_clone.poison_immune, "PoisonImmune").changed() { modified = true; }
                                        });

                                        if modified {
                                            card_clone.deck_points = card_clone.computed_weighted_points();
                                            card_clone.combat_ehp_tenths = u16::from(card_clone.strength_hp) * 10
                                                + u16::from(card_clone.armor) * 20
                                                + u16::from(card_clone.shield) * 20
                                                + u16::from(card_clone.evade) * 15;
                                            roster.units[sel_idx] = card_clone.clone();
                                        }

                                        ui.add_space(4.0);
                                        if ui.button("💾 Save Changes to SQLite DB + Sync .ron").clicked() {
                                            let db_p = resolve_asset_path("compiled/wildfront_roster.db")
                                                .unwrap_or_else(|| writable_app_path("assets/compiled/wildfront_roster.db"));
                                            let _ = ensure_parent_dir(&db_p);
                                            match save_unit_to_sqlite(&db_p, &card_clone) {
                                                Ok(_) => {
                                                    if let Ok(pretty) = ron::ser::to_string_pretty(roster, ron::ser::PrettyConfig::default()) {
                                                        let ron_out = writable_app_path("assets/compiled/master_roster.ron");
                                                        let _ = ensure_parent_dir(&ron_out);
                                                        let _ = std::fs::write(ron_out, pretty);
                                                    }
                                                    self.db_status_msg = format!("✓ Saved '{}' ({} pts) to {}!", card_clone.card_key, card_clone.deck_points, db_p.display());
                                                    audio.play_ui(SfxCue::BioHealChime);
                                                }
                                                Err(e) => {
                                                    self.db_status_msg = format!("⚠ SQLite save error: {e}");
                                                }
                                            }
                                        }
                                    } else {
                                        // Read-only tactical dossier
                                        ui.label(RichText::new(format!("Combat Class: {:?}  |  Graveyard: {:?}", card_clone.combat_class, card_clone.graveyard_queue)).color(Color32::from_rgb(180, 215, 225)));
                                        ui.label(RichText::new(format!("Signature Ability: {}", card_clone.signature_ability)).strong().color(Color32::from_rgb(120, 240, 175)));
                                        ui.add_space(4.0);
                                        ui.label(RichText::new(format!(
                                            "STR {}  |  ENC {}  |  ARM {}  |  SHD {}  |  MOV {}  |  EVD {}  |  MEL {}  |  RNG {}",
                                            card_clone.strength_hp, card_clone.encumbrance, card_clone.armor, card_clone.shield,
                                            card_clone.movement, card_clone.evade, card_clone.melee_range, card_clone.ranged_distance
                                        )).strong());
                                        ui.label(RichText::new(format!("Voxel Rig: {}  |  Models/Hex: {}  |  Dice: {}", card_clone.default_vox_slug(), card_clone.squad_model_count, Self::dice_hardware_note(&card_clone))).small().color(Color32::from_rgb(160, 190, 205)));
                                        ui.label(RichText::new(format!("Provenance: {}", card_clone.provenance_note)).small().italics().color(Color32::from_rgb(140, 170, 185)));
                                    }
                                });

                                if clone_requested {
                                    let mut cloned = card_clone.clone();
                                    let new_key = format!("{} (Custom)", cloned.card_key);
                                    cloned.card_key = new_key.clone();
                                    cloned.unit_name = format!("{} (Custom)", cloned.unit_name);
                                    let db_p = resolve_asset_path("compiled/wildfront_roster.db")
                                        .unwrap_or_else(|| writable_app_path("assets/compiled/wildfront_roster.db"));
                                    let _ = ensure_parent_dir(&db_p);
                                    let _ = save_unit_to_sqlite(&db_p, &cloned);
                                    roster.units.push(cloned);
                                    self.selected_card_key = new_key;
                                    self.edit_mode = true;
                                    self.db_status_msg = "Cloned card into SQLite DB—ready to edit!".to_string();
                                } else if delete_requested && roster.units.len() > 1 {
                                    let key_del = card_clone.card_key.clone();
                                    if let Some(db_p) = resolve_asset_path("compiled/wildfront_roster.db") {
                                        let _ = delete_unit_from_sqlite(&db_p, &key_del);
                                    }
                                    roster.units.retain(|u| u.card_key != key_del);
                                    self.selected_card_key = roster.units[0].card_key.clone();
                                    self.db_status_msg = format!("Deleted '{}' from SQLite DB.", key_del);
                                }
            }

        });
    }

    fn render_faction_doctrines(&mut self, ui: &mut Ui) {
        ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
            for fp in &self.catalog.faction_passives {
                let badge_col = Self::faction_color(fp.faction);
                Frame::NONE
                    .fill(Color32::from_rgb(16, 26, 36))
                    .stroke(Stroke::new(1.4, badge_col))
                    .corner_radius(8)
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{:?} FACTION — {}", fp.faction, fp.doctrine_name))
                                    .size(15.0)
                                    .strong()
                                    .color(badge_col),
                            );
                        });
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(format!("Synergy Bonus: {}", fp.bonus_effect))
                                .strong()
                                .color(Color32::from_rgb(122, 242, 184)),
                        );
                        ui.label(
                            RichText::new(format!("Tactical Tradeoff: {}", fp.tactical_tradeoff))
                                .small()
                                .color(Color32::from_rgb(248, 172, 142)),
                        );
                    });
                ui.add_space(6.0);
            }
        });
    }

    fn render_class_matrix(&mut self, ui: &mut Ui, roster: &RosterManifest) {
        ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
            ui.label(
                RichText::new("COMBAT CLASS ROLES & DEPLOYMENT CAPACITIES")
                    .size(15.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            ui.add_space(6.0);

            for (cls, desc, cap, color) in [
                (CombatClass::Installation, "Command Centers & Node Hubs. Generates +5 Munitions/turn.", "1 per deck", Color32::from_rgb(92, 224, 244)),
                (CombatClass::SupplyDepot, "Munitions reservoir & high-volume hauler supply relays.", "1 per deck", Color32::from_rgb(248, 196, 78)),
                (CombatClass::ControlNode, "Static psychic control emitter (+1 Turn Action per node).", "Up to 3", Color32::from_rgb(112, 238, 255)),
                (CombatClass::Commander, "Aura projection (+1 Ranged Hit to adjacent allies, Node relay).", "1 per deck", Color32::from_rgb(248, 112, 112)),
                (CombatClass::Heavy, "Armored line-breakers and heavy siege artillery (STR 6–10).", "Up to 2 copies", Color32::from_rgb(242, 142, 62)),
                (CombatClass::Medium, "Core combat squads and tactical line infantry (STR 4–5).", "Up to 3 copies", Color32::from_rgb(168, 204, 218)),
                (CombatClass::Light, "Fast skirmishers, scouts, stealth infiltrators and sappers (STR 3).", "Up to 3 copies", Color32::from_rgb(142, 242, 184)),
                (CombatClass::Medic, "Field triage & decontamination (+2 STR healing to damaged allies).", "Up to 3 copies", Color32::from_rgb(92, 242, 142)),
                (CombatClass::Support, "Ammo convoys, logistics carriers and mobile supply links.", "Up to 3 copies", Color32::from_rgb(248, 214, 96)),
                (CombatClass::Trap, "Root snares, venom thornapples, and stealth static hazards.", "Up to 3 copies", Color32::from_rgb(196, 132, 255)),
                (CombatClass::Artifact, "Single-use field consumables (Remote Cargo Drop, Splice Vats).", "Up to 3 copies", Color32::from_rgb(244, 142, 212)),
                (CombatClass::TacticalDeployment, "Single-use Tactical Deployments (cost deck points; exiled on resolution, never regenerated).", "Up to 2 copies", Color32::from_rgb(255, 118, 232)),
                (CombatClass::SupportInstallation, "Hospitals, Factories, Radar Towers and Shield Domes.", "Up to 2 copies", Color32::from_rgb(112, 196, 244)),
            ] {
                let count = roster.units.iter().filter(|u| u.combat_class == cls).count();
                Frame::NONE
                    .fill(Color32::from_rgb(15, 24, 32))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(42, 72, 88)))
                    .corner_radius(6)
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{:?}", cls)).size(13.5).strong().color(color));
                            ui.label(RichText::new(format!("({count} units in catalog)")).small().color(Color32::from_rgb(142, 184, 198)));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(format!("Deck Cap: {cap}")).strong().color(Color32::from_rgb(248, 196, 78)));
                            });
                        });
                        ui.label(RichText::new(desc).small().color(Color32::from_rgb(196, 216, 228)));
                    });
                ui.add_space(4.0);
            }
        });
    }
}
