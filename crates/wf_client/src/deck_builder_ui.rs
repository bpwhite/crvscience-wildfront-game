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

//! Interactive `egui 0.37` 250-Point Two-Faction Tournament Deck Builder Screen.
//!
//! Features:
//! - Select any 2 Bio-Factions (`Mammal`, `Reptile`, `Plant`, `Bird`, `Arachnid`, `Insect`,
//!   `Fish`, `Cnidarian`, `Amphibian`) alongside always-unlocked `Human` Core Infrastructure
//! - Search & filter across all 133 balanced cards (`Master Roster`)
//! - Live `DeckList::validate_against(&RosterManifest)` legality verification (250-pt budget,
//!   1 HQ + 1 Supply Depot, <=1 Commander, <=3 Control Nodes, 2-Faction Rule)
//! - Bio (`Hospital`) & Mech (`Factory`) Graveyard queue synergy advisor
//! - 4 Tournament Presets + `.ron` Disk Save/Load + 1-Click `Launch Match With This Deck`

use std::fs;
use std::path::Path;
use egui::{Align2, Color32, Frame, Margin, ProgressBar, RichText, ScrollArea, Stroke, Ui, Vec2};
use ron::ser::{to_string_pretty, PrettyConfig};
use wf_core::{
    CombatClass, DeckEntry, DeckList, FactionCardDef, GraveyardFacility, OrganismFaction,
    OriginAllegiance, RosterManifest, UnitCardDef,
};
use wf_assets::stl_exporter::{export_card_to_stl, export_deck_to_stl_folder, sanitize_filename_slug, TabletopPrintConfig};
use crate::audio::{SfxCue, SpatialAudioEngine};
use crate::silhouettes::draw_unit_silhouette;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassFilter {
    All,
    CoreBuildings,
    Heavy,
    Medium,
    Light,
    SupportAndMedic,
    TrapsAndArtifacts,
}

impl ClassFilter {
    pub const ALL: [(Self, &'static str); 7] = [
        (Self::All, "All Classes"),
        (Self::CoreBuildings, "Buildings & Nodes"),
        (Self::Heavy, "Heavy"),
        (Self::Medium, "Medium"),
        (Self::Light, "Light"),
        (Self::SupportAndMedic, "Support & Medic"),
        (Self::TrapsAndArtifacts, "Traps, Artifacts & Tactical Deployments"),
    ];

    pub fn matches(&self, cls: CombatClass) -> bool {
        match self {
            Self::All => true,
            Self::CoreBuildings => matches!(
                cls,
                CombatClass::Installation
                    | CombatClass::SupplyDepot
                    | CombatClass::ControlNode
                    | CombatClass::SupportInstallation
            ),
            Self::Heavy => matches!(cls, CombatClass::Heavy | CombatClass::Commander),
            Self::Medium => cls == CombatClass::Medium,
            Self::Light => cls == CombatClass::Light,
            Self::SupportAndMedic => matches!(cls, CombatClass::Support | CombatClass::Medic),
            Self::TrapsAndArtifacts => matches!(cls, CombatClass::Trap | CombatClass::Artifact | CombatClass::TacticalDeployment),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonTarget {
    OpponentSlot1,
    PresetPlantCnidarian,
    PresetMammalReptile,
    PresetFishAmphibian,
    PresetBirdArachnid,
    PresetPurePlant,
    PresetPureInsect,
    CustomSavedRon,
}

impl ComparisonTarget {
    pub const ALL: [(Self, &'static str); 8] = [
        (Self::OpponentSlot1, "P2 / AI Opponent Deck"),
        (Self::PresetPlantCnidarian, "Preset: Plant + Cnidarian (Australia)"),
        (Self::PresetMammalReptile, "Preset: Mammal + Reptile (Africa)"),
        (Self::PresetFishAmphibian, "Preset: Fish + Amphibian (North America)"),
        (Self::PresetBirdArachnid, "Preset: Bird + Arachnid (South America)"),
        (Self::PresetPurePlant, "Preset: Pure Plant Dominion (Australia)"),
        (Self::PresetPureInsect, "Preset: Pure Insect Swarm (Asia)"),
        (Self::CustomSavedRon, "Custom Saved Deck (.ron)"),
    ];
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeckMetrics {
    pub survivability: f32,
    pub mobility: f32,
    pub firepower: f32,
    pub logistics: f32,
    pub tactics: f32,
    pub bio_synergy: f32,
    pub total_ehp: f32,
    pub avg_move: f32,
    pub total_firepower: u32,
    pub munitions_capacity: u32,
    pub tactical_control: u32,
    pub bio_synergy_score: u32,
    pub total_points: u16,
    pub total_cards: u16,
}

pub fn compute_deck_metrics(deck: &DeckList, roster: &RosterManifest) -> DeckMetrics {
    let lookup = roster.index_by_key();
    let mut total_cards = 0u16;
    let mut total_points = 0u16;
    let mut total_ehp = 0.0f32;
    let mut move_sum = 0.0f32;
    let mut mobile_unit_count = 0usize;
    let mut total_firepower = 0u32;
    let mut total_enc = 0u32;
    let mut tactical_control = 0u32;
    let mut bio_synergy_score = 0u32;

    for entry in &deck.entries {
        if entry.quantity == 0 {
            continue;
        }
        let qty = entry.quantity as u32;
        let qty_u16 = entry.quantity as u16;
        total_cards += qty_u16;

        if let Some(card) = lookup.get(entry.card_key.as_str()) {
            total_points += card.deck_points * qty_u16;
            total_ehp += (card.combat_ehp_tenths as f32 / 10.0) * qty as f32;

            if !card.combat_class.is_building() {
                let mut m = card.movement as f32;
                if card.can_fly { m += 0.8; }
                if card.can_burrow { m += 0.4; }
                move_sum += m * qty as f32;
                mobile_unit_count += entry.quantity as usize;
            }

            let fp = card.strength_hp as u32
                + (card.ranged_distance as u32 * 2)
                + if card.is_laser { 3 } else { 0 }
                + (card.melee_range as u32 * 2);
            total_firepower += fp * qty;

            let mut logis = card.encumbrance as u32;
            if card.combat_class == CombatClass::SupplyDepot { logis += 12; }
            if card.card_key.contains("Supply Truck") { logis += 6; }
            total_enc += logis * qty;

            let mut tech = 0u32;
            if card.combat_class == CombatClass::ControlNode { tech += 6; }
            if card.radar_range > 0 { tech += card.radar_range as u32 * 2; }
            if card.stealth { tech += 4; }
            if card.saboteur { tech += 4; }
            if card.combat_class == CombatClass::Trap { tech += 3; }
            if card.combat_class == CombatClass::TacticalDeployment { tech += 5; }
            if card.card_key.contains("Radar Laser") { tech += 5; }
            tactical_control += tech * qty;

            let mut syn = 0u32;
            if card.card_key == "Hospital" { syn += 8; }
            if card.card_key == "Factory" { syn += 6; }
            if card.combat_class == CombatClass::Medic { syn += 5; }
            syn += card.poison_stacks as u32 * 3;
            syn += card.root_stacks as u32 * 3;
            bio_synergy_score += syn * qty;
        }
    }

    match deck.faction {
        OriginAllegiance::Europe => { total_ehp += 25.0; }
        OriginAllegiance::Africa => { total_enc += 8; }
        OriginAllegiance::Asia => { tactical_control += 8; }
        OriginAllegiance::Australia => { tactical_control += 6; move_sum += 2.0; }
        OriginAllegiance::NorthAmerica => { total_firepower += 10; }
        OriginAllegiance::SouthAmerica => { bio_synergy_score += 10; }
        _ => {}
    }

    if deck.is_pure_biotype() {
        bio_synergy_score += 15;
        match deck.primary_faction {
            OrganismFaction::Mammal => { move_sum += 3.0; total_firepower += 8; }
            OrganismFaction::Reptile => { total_ehp += 30.0; }
            OrganismFaction::Plant => { total_ehp += 20.0; bio_synergy_score += 10; }
            OrganismFaction::Bird => { move_sum += 5.0; total_firepower += 6; }
            OrganismFaction::Arachnid => { bio_synergy_score += 12; tactical_control += 6; }
            OrganismFaction::Insect => { total_enc += 10; total_cards += 2; }
            OrganismFaction::Fish => { move_sum += 3.0; total_firepower += 8; }
            OrganismFaction::Cnidarian => { tactical_control += 8; bio_synergy_score += 10; }
            OrganismFaction::Amphibian => { bio_synergy_score += 14; }
            _ => {}
        }
    }

    let avg_move = if mobile_unit_count > 0 {
        move_sum / mobile_unit_count as f32
    } else {
        2.0
    };

    let survivability = (total_ehp / 380.0).clamp(0.12, 1.0);
    let mobility = (avg_move / 4.2).clamp(0.15, 1.0);
    let firepower = (total_firepower as f32 / 110.0).clamp(0.15, 1.0);
    let logistics = (total_enc as f32 / 45.0).clamp(0.15, 1.0);
    let tactics = (tactical_control as f32 / 32.0).clamp(0.12, 1.0);
    let bio_synergy = (bio_synergy_score as f32 / 35.0).clamp(0.12, 1.0);

    DeckMetrics {
        survivability,
        mobility,
        firepower,
        logistics,
        tactics,
        bio_synergy,
        total_ehp,
        avg_move,
        total_firepower,
        munitions_capacity: total_enc,
        tactical_control,
        bio_synergy_score,
        total_points,
        total_cards,
    }
}

pub fn render_radar_chart(
    ui: &mut Ui,
    deck_a_metrics: &DeckMetrics,
    _deck_a_name: &str,
    deck_b_opt: Option<(&DeckMetrics, &str)>,
) {
    let desired_size = Vec2::new(300.0, 220.0);
    let (rect, _response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
    let painter = ui.painter_at(rect);

    let center = rect.center();
    let max_radius = 75.0f32;

    let axis_names = [
        "SURVIVABILITY",
        "MOBILITY",
        "FIREPOWER",
        "LOGISTICS",
        "TACTICAL TECH",
        "BIO-SYNERGY",
    ];

    for ring_idx in 1..=3 {
        let frac = ring_idx as f32 / 3.0;
        let r = max_radius * frac;
        let mut ring_pts = Vec::with_capacity(6);
        for k in 0..6 {
            let angle = -std::f32::consts::FRAC_PI_2 + (k as f32) * (std::f32::consts::PI / 3.0);
            ring_pts.push(center + Vec2::new(r * angle.cos(), r * angle.sin()));
        }
        for k in 0..6 {
            let p1 = ring_pts[k];
            let p2 = ring_pts[(k + 1) % 6];
            painter.line_segment([p1, p2], Stroke::new(1.0, Color32::from_rgba_unmultiplied(65, 95, 115, 90)));
        }
    }

    for k in 0..6 {
        let angle = -std::f32::consts::FRAC_PI_2 + (k as f32) * (std::f32::consts::PI / 3.0);
        let dir = Vec2::new(angle.cos(), angle.sin());
        let outer_pt = center + dir * max_radius;
        painter.line_segment([center, outer_pt], Stroke::new(1.0, Color32::from_rgba_unmultiplied(80, 118, 140, 110)));

        let label_pos = center + dir * (max_radius + 12.0);
        let align = match k {
            0 => Align2::CENTER_BOTTOM,
            1 => Align2::LEFT_CENTER,
            2 => Align2::LEFT_CENTER,
            3 => Align2::CENTER_TOP,
            4 => Align2::RIGHT_CENTER,
            5 => Align2::RIGHT_CENTER,
            _ => Align2::CENTER_CENTER,
        };
        painter.text(
            label_pos,
            align,
            axis_names[k],
            egui::FontId::proportional(8.5),
            Color32::from_rgb(160, 195, 215),
        );
    }

    let get_scores = |m: &DeckMetrics| [
        m.survivability,
        m.mobility,
        m.firepower,
        m.logistics,
        m.tactics,
        m.bio_synergy,
    ];

    if let Some((deck_b, _name_b)) = deck_b_opt {
        let scores_b = get_scores(deck_b);
        let mut pts_b = Vec::with_capacity(6);
        for k in 0..6 {
            let angle = -std::f32::consts::FRAC_PI_2 + (k as f32) * (std::f32::consts::PI / 3.0);
            let r = max_radius * scores_b[k];
            pts_b.push(center + Vec2::new(r * angle.cos(), r * angle.sin()));
        }
        let poly_shape = egui::Shape::convex_polygon(
            pts_b.clone(),
            Color32::from_rgba_unmultiplied(240, 150, 40, 50),
            Stroke::new(2.0, Color32::from_rgb(250, 170, 50)),
        );
        painter.add(poly_shape);
        for pt in pts_b {
            painter.circle_filled(pt, 3.0, Color32::from_rgb(255, 190, 80));
        }
    }

    let scores_a = get_scores(deck_a_metrics);
    let mut pts_a = Vec::with_capacity(6);
    for k in 0..6 {
        let angle = -std::f32::consts::FRAC_PI_2 + (k as f32) * (std::f32::consts::PI / 3.0);
        let r = max_radius * scores_a[k];
        pts_a.push(center + Vec2::new(r * angle.cos(), r * angle.sin()));
    }
    let poly_shape = egui::Shape::convex_polygon(
        pts_a.clone(),
        Color32::from_rgba_unmultiplied(40, 210, 240, 60),
        Stroke::new(2.0, Color32::from_rgb(50, 230, 255)),
    );
    painter.add(poly_shape);
    for pt in pts_a {
        painter.circle_filled(pt, 3.5, Color32::from_rgb(120, 245, 255));
    }
}

pub struct DeckBuilderScreen {
    pub is_open: bool,
    pub target_slot: usize,
    pub working_deck: DeckList,
    pub search_query: String,
    pub show_only_legal_factions: bool,
    pub class_filter: ClassFilter,
    pub requested_db_card: Option<String>,
    pub status_banner: String,
    pub show_deck_comparison: bool,
    pub comparison_target: ComparisonTarget,
    pub show_command_cards_detail: bool,
}

const BIO_FACTIONS: [OrganismFaction; 9] = [
    OrganismFaction::Mammal,
    OrganismFaction::Reptile,
    OrganismFaction::Plant,
    OrganismFaction::Bird,
    OrganismFaction::Arachnid,
    OrganismFaction::Insect,
    OrganismFaction::Fish,
    OrganismFaction::Cnidarian,
    OrganismFaction::Amphibian,
];

impl DeckBuilderScreen {
    pub fn new(initial_deck: DeckList) -> Self {
        Self {
            is_open: false,
            target_slot: 0,
            working_deck: initial_deck,
            search_query: String::new(),
            show_only_legal_factions: true,
            class_filter: ClassFilter::All,
            requested_db_card: None,
            status_banner: "Build any 250-point Human + 2 Bio-Faction tournament deck.".into(),
            show_deck_comparison: false,
            comparison_target: ComparisonTarget::OpponentSlot1,
            show_command_cards_detail: false,
        }
    }

    pub fn build_preset_fish_amphibian(roster: &RosterManifest) -> DeckList {
        Self::assemble_greedy_preset(
            roster,
            "Fish + Amphibian 250-Pt Pelagic Deck",
            OriginAllegiance::NorthAmerica,
            OrganismFaction::Fish,
            OrganismFaction::Amphibian,
            "Command Center (North America)",
            "Commander (North America)",
            "Factory",
            &[
                "Optic-Laser Shark",
                "Hellbender Salamander",
                "Armada Manta Ray",
                "Voltaic Eel Disruptor",
                "Axolotl Bio-Mender",
                "Tidehopper Mudskipper",
                "Venom Dart",
                "Recon Trout",
                "Cryo-Hibernation Vat",
            ],
        )
    }

    pub fn build_preset_bird_arachnid(roster: &RosterManifest) -> DeckList {
        Self::assemble_greedy_preset(
            roster,
            "Bird + Arachnid 250-Pt Aerial-Web Deck",
            OriginAllegiance::SouthAmerica,
            OrganismFaction::Bird,
            OrganismFaction::Arachnid,
            "Command Center (South America)",
            "Commander (South America)",
            "Hospital",
            &[
                "Carrier Pigeon Dreadnought",
                "Deathstalker Scorpion",
                "Legspinner Tarantula",
                "Dive-Strike Peregrine",
                "Skyhunter Falcon",
                "Bombardier Ravens",
                "Stalker Wolf Spider",
                "Funnel-Web Trapper",
                "Avian Aerie Splice",
            ],
        )
    }

    pub fn build_preset_pure_plant(roster: &RosterManifest) -> DeckList {
        let mut deck = wf_sim::build_tournament_deck(
            roster,
            OriginAllegiance::Australia,
            OrganismFaction::Plant,
            OrganismFaction::Plant,
        );
        deck.deck_name = "Pure Plant Dominion (Australia)".into();
        deck
    }

    pub fn build_preset_pure_insect(roster: &RosterManifest) -> DeckList {
        let mut deck = wf_sim::build_tournament_deck(
            roster,
            OriginAllegiance::Asia,
            OrganismFaction::Insect,
            OrganismFaction::Insect,
        );
        deck.deck_name = "Pure Insect Swarm (Asia)".into();
        deck
    }

    /// Automatically constructs an optimal, tournament-legal 250-point deck
    /// tailored to the currently selected primary and secondary bio-factions.
    pub fn auto_generate_faction_synergy_deck(&mut self, roster: &RosterManifest) {
        let f1 = self.working_deck.primary_faction;
        let is_pure = self.working_deck.is_pure_biotype();
        if is_pure {
            self.working_deck.secondary_faction = f1;
        }
        let f2 = self.working_deck.secondary_faction;
        let lookup = roster.index_by_key();

        // 1. Pick Continent HQ & Commander matching chosen continent faction
        let hq_key = match self.working_deck.faction {
            OriginAllegiance::Australia => "Command Center (Australia)",
            OriginAllegiance::Africa => "Command Center (Africa)",
            OriginAllegiance::Asia => "Command Center (Asia)",
            OriginAllegiance::Europe => "Command Center (Europe)",
            OriginAllegiance::NorthAmerica => "Command Center (North America)",
            OriginAllegiance::SouthAmerica => "Command Center (South America)",
            OriginAllegiance::Neutral => "Command Center (Australia)",
        };
        let cmd_key = match self.working_deck.faction {
            OriginAllegiance::Australia => "Commander (Australia)",
            OriginAllegiance::Africa => "Commander (Africa)",
            OriginAllegiance::Asia => "Commander (Asia)",
            OriginAllegiance::Europe => "Commander (Europe)",
            OriginAllegiance::NorthAmerica => "Commander (North America)",
            OriginAllegiance::SouthAmerica => "Commander (South America)",
            OriginAllegiance::Neutral => "Commander (Australia)",
        };


        let mut entries = vec![
            DeckEntry { card_key: hq_key.into(), quantity: 1 },
            DeckEntry { card_key: "Supply Depot".into(), quantity: 1 },
            DeckEntry { card_key: cmd_key.into(), quantity: 1 },
            DeckEntry { card_key: "Control Node".into(), quantity: 1 },
        ];
        if f1 == OrganismFaction::Fish || (!is_pure && f2 == OrganismFaction::Fish) {
            entries.push(DeckEntry { card_key: "Hydro-Breach Canal".into(), quantity: 2 });
        }

        let mut current_pts: u16 = entries
            .iter()
            .filter_map(|e| lookup.get(e.card_key.as_str()).map(|c| c.deck_points * u16::from(e.quantity)))
            .sum();

        // 2. Select eligible cards matching f1 or f2
        let mut eligible: Vec<&UnitCardDef> = roster
            .units
            .iter()
            .filter(|u| {
                if is_pure {
                    u.faction == f1
                } else {
                    u.faction == f1 || u.faction == f2
                }
            })
            .collect();

        // Sort candidates by tactical efficiency: Heavies & Anchors first, then Medics, then Mediums, then Lights
        eligible.sort_by(|a, b| {
            let role_prio = |c: CombatClass| match c {
                CombatClass::Heavy => 1,
                CombatClass::Medic => 2,
                CombatClass::Medium => 3,
                CombatClass::Support => 4,
                CombatClass::Light => 5,
                CombatClass::Trap => 6,
                CombatClass::Artifact => 7,
                _ => 8,
            };
            let prio_a = role_prio(a.combat_class);
            let prio_b = role_prio(b.combat_class);
            prio_a.cmp(&prio_b).then_with(|| b.deck_points.cmp(&a.deck_points))
        });

        // 3. Count Graveyard queue tendencies to pick Hospital vs Factory
        let bio_count = eligible.iter().filter(|u| matches!(u.graveyard_queue, GraveyardFacility::HospitalBio)).count();
        let mech_count = eligible.iter().filter(|u| matches!(u.graveyard_queue, GraveyardFacility::FactoryMech)).count();

        if bio_count >= mech_count {
            if let Some(hosp) = lookup.get("Hospital") {
                if current_pts + hosp.deck_points <= 250 {
                    entries.push(DeckEntry { card_key: "Hospital".into(), quantity: 1 });
                    current_pts += hosp.deck_points;
                }
            }
        } else {
            if let Some(fact) = lookup.get("Factory") {
                if current_pts + fact.deck_points <= 250 {
                    entries.push(DeckEntry { card_key: "Factory".into(), quantity: 1 });
                    current_pts += fact.deck_points;
                }
            }
        }

        // 4. Greedily fill deck with high-synergy unit cards
        for card in eligible {
            if current_pts + card.deck_points <= 250 {
                entries.push(DeckEntry {
                    card_key: card.card_key.clone(),
                    quantity: 1,
                });
                current_pts += card.deck_points;
            }
        }

        // 5. If points remain, double up on key line units (Medium/Light/Medic) up to class cap
        for entry in &mut entries {
            if let Some(card) = lookup.get(entry.card_key.as_str()) {
                let cap = Self::max_copies_for_class(card.combat_class);
                while entry.quantity < cap && current_pts + card.deck_points <= 250 {
                    entry.quantity += 1;
                    current_pts += card.deck_points;
                }
            }
        }

        // 6. If points still remain, add second Control Node if affordable
        if let Some(node_entry) = entries.iter_mut().find(|e| e.card_key == "Control Node") {
            if let Some(card) = lookup.get("Control Node") {
                if node_entry.quantity < 3 && current_pts + card.deck_points <= 250 {
                    node_entry.quantity += 1;
                    current_pts += card.deck_points;
                }
            }
        }

        let faction = self.working_deck.faction;
        self.working_deck = DeckList {
            deck_name: format!("{:?} + {:?} 250-Pt Synergy Deck", f1, f2),
            faction,
            is_pure_biotype: is_pure,
            primary_faction: f1,
            secondary_faction: f2,
            point_budget: 250,
            entries,
        };
        self.status_banner = format!(
            "⚡ Generated optimal {:?} + {:?} deck ({} pts / 250 pts)!",
            f1, f2, current_pts
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn assemble_greedy_preset(
        roster: &RosterManifest,
        name: &str,
        faction: OriginAllegiance,
        f1: OrganismFaction,
        f2: OrganismFaction,
        hq_key: &str,
        cmd_key: &str,
        fac_key: &str,
        candidates: &[&str],
    ) -> DeckList {
        let lookup = roster.index_by_key();
        let mut entries = vec![
            DeckEntry { card_key: hq_key.into(), quantity: 1 },
            DeckEntry { card_key: "Supply Depot".into(), quantity: 1 },
            DeckEntry { card_key: cmd_key.into(), quantity: 1 },
            DeckEntry { card_key: "Control Node".into(), quantity: 1 },
            DeckEntry { card_key: fac_key.into(), quantity: 1 },
        ];
        let mut pts: u16 = entries
            .iter()
            .filter_map(|e| lookup.get(e.card_key.as_str()).map(|c| c.deck_points * u16::from(e.quantity)))
            .sum();

        for &ck in candidates {
            if let Some(card) = lookup.get(ck) {
                if pts + card.deck_points <= 250 {
                    entries.push(DeckEntry {
                        card_key: ck.into(),
                        quantity: 1,
                    });
                    pts += card.deck_points;
                }
            }
        }
        for entry in &mut entries {
            if let Some(card) = lookup.get(entry.card_key.as_str()) {
                let cap = Self::max_copies_for_class(card.combat_class);
                while entry.quantity < cap && pts + card.deck_points <= 250 {
                    entry.quantity += 1;
                    pts += card.deck_points;
                }
            }
        }
        DeckList {
            deck_name: name.into(),
            faction,
            is_pure_biotype: f1 == f2,
            primary_faction: f1,
            secondary_faction: f2,
            point_budget: 250,
            entries,
        }
    }

    fn max_copies_for_class(cls: CombatClass) -> u8 {
        match cls {
            CombatClass::Installation | CombatClass::SupplyDepot | CombatClass::Commander => 1,
            CombatClass::ControlNode => 3,
            CombatClass::Heavy | CombatClass::SupportInstallation | CombatClass::TacticalDeployment => 2,
            _ => 3,
        }
    }

    fn add_card_copy(&mut self, card_key: &str, cls: CombatClass) {
        let cap = Self::max_copies_for_class(cls);
        if let Some(existing) = self.working_deck.entries.iter_mut().find(|e| e.card_key == card_key) {
            if existing.quantity < cap {
                existing.quantity += 1;
            }
            return;
        }
        self.working_deck.entries.push(DeckEntry {
            card_key: card_key.to_string(),
            quantity: 1,
        });
    }

    fn adjust_entry_qty(&mut self, card_key: &str, delta: i8) {
        if let Some(pos) = self.working_deck.entries.iter().position(|e| e.card_key == card_key) {
            let cur = self.working_deck.entries[pos].quantity as i8 + delta;
            if cur <= 0 {
                self.working_deck.entries.remove(pos);
            } else {
                self.working_deck.entries[pos].quantity = cur as u8;
            }
        }
    }

    fn prune_illegal_faction_cards(&mut self, roster: &RosterManifest) {
        let lookup = roster.index_by_key();
        let f1 = self.working_deck.primary_faction;
        let is_pure = self.working_deck.is_pure_biotype();
        let f2 = self.working_deck.secondary_faction;
        self.working_deck.entries.retain(|e| {
            lookup.get(e.card_key.as_str()).map_or(false, |c| {
                if is_pure {
                    c.faction == OrganismFaction::Human || c.faction == f1
                } else {
                    c.faction == OrganismFaction::Human || c.faction == f1 || c.faction == f2
                }
            })
        });
    }

    /// Renders the full-screen Deck Builder overlay when `self.is_open` is true.
    /// Returns `Some(DeckList)` when the player clicks `Launch Match With This Deck`.
    pub fn show(
        &mut self,
        root_ui: &mut Ui,
        roster: &RosterManifest,
        starter_presets: &[DeckList; 2],
        audio: &mut SpatialAudioEngine,
    ) -> Option<(usize, DeckList)> {
        if !self.is_open {
            return None;
        }
        let mut launched_deck: Option<(usize, DeckList)> = None;
        let report = self.working_deck.validate_against(roster);
        let lookup = roster.index_by_key();

        egui::CentralPanel::default()
            .frame(
                Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(11, 18, 24, 246))
                    .inner_margin(Margin::same(16)),
            )
            .show(root_ui, |ui| {
                // Header Row: Title, Presets, Save/Load RON & Close
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("🃏 250-POINT TWO-FACTION TOURNAMENT DECK BUILDER")
                            .strong()
                            .size(18.0)
                            .color(Color32::from_rgb(112, 232, 248)),
                    );
                    ui.separator();

                    if ui.button("Preset: Plant+Cnidarian").clicked() {
                        self.working_deck = starter_presets[0].clone();
                        audio.play_ui(SfxCue::UiCardTurn);
                    }
                    if ui.button("Preset: Mammal+Reptile").clicked() {
                        self.working_deck = starter_presets[1].clone();
                        audio.play_ui(SfxCue::UiCardTurn);
                    }
                    if ui.button("Preset: Fish+Amphibian").clicked() {
                        self.working_deck = Self::build_preset_fish_amphibian(roster);
                        audio.play_ui(SfxCue::UiCardTurn);
                    }
                    if ui.button("Preset: Bird+Arachnid").clicked() {
                        self.working_deck = Self::build_preset_bird_arachnid(roster);
                        audio.play_ui(SfxCue::UiCardTurn);
                    }
                    if ui.button("Preset: Pure Plant").on_hover_text("Mono-Bio-Type: Ancient Overgrowth Canopy Doctrine").clicked() {
                        self.working_deck = Self::build_preset_pure_plant(roster);
                        audio.play_ui(SfxCue::BioHealChime);
                    }
                    if ui.button("Preset: Pure Insect").on_hover_text("Mono-Bio-Type: Hive Mind Resurgence Doctrine").clicked() {
                        self.working_deck = Self::build_preset_pure_insect(roster);
                        audio.play_ui(SfxCue::BioHealChime);
                    }
                    ui.separator();
                    let comp_btn = egui::Button::new(
                        RichText::new(if self.show_deck_comparison { "📊 Close Comparison" } else { "📊 Deck Comparison & Radar" })
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(if self.show_deck_comparison { Color32::from_rgb(175, 75, 35) } else { Color32::from_rgb(45, 115, 160) });
                    if ui.add(comp_btn).clicked() {
                        self.show_deck_comparison = !self.show_deck_comparison;
                        audio.play_ui(SfxCue::UiCardTurn);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("✖ Close (B)").clicked() {
                            self.is_open = false;
                            audio.play_ui(SfxCue::UiCardTurn);
                        }
                        if ui.button("📂 Load .ron").clicked() {
                            let p = crate::paths::resolve_asset_path("compiled/custom_user_deck.ron")
                                .unwrap_or_else(|| crate::paths::writable_app_path("assets/compiled/custom_user_deck.ron"));
                            if let Ok(raw) = fs::read_to_string(&p) {
                                if let Ok(loaded) = ron::from_str::<DeckList>(&raw) {
                                    self.working_deck = loaded;
                                    self.status_banner = format!("Loaded {}.", p.display());
                                    audio.play_ui(SfxCue::UiCardTurn);
                                }
                            } else {
                                self.status_banner = "No custom_user_deck.ron saved yet—click Save .ron first!".into();
                            }
                        }
                        if ui.button("🖨 Export Deck STLs").on_hover_text("Export every card in this deck as a watertight 45mm-hex .STL with 2.80mm thick-wall V-cradles").clicked() {
                            let cfg = TabletopPrintConfig::default();
                            let slug = sanitize_filename_slug(&self.working_deck.deck_name);
                            let out_dir = crate::paths::writable_app_path("exports/stl/decks").join(slug);
                            let vox_dir = crate::paths::resolve_asset_path("vox")
                                .unwrap_or_else(|| crate::paths::writable_app_path("assets/vox"));
                            match export_deck_to_stl_folder(&self.working_deck, roster, &vox_dir, &out_dir, &cfg) {
                                Ok(reps) => {
                                    self.status_banner = format!("✓ Exported {} printable .STL miniatures + PRINT_MANIFEST.txt to {}!", reps.len(), out_dir.display());
                                    audio.play_ui(SfxCue::BioHealChime);
                                }
                                Err(e) => {
                                    self.status_banner = format!("⚠ STL export error: {e}");
                                }
                            }
                        }
                        if ui.button("💾 Save .ron").clicked() {
                            let pretty = PrettyConfig::new().depth_limit(3);
                            if let Ok(serialized) = to_string_pretty(&self.working_deck, pretty) {
                                let p = crate::paths::writable_app_path("assets/compiled/custom_user_deck.ron");
                                let _ = crate::paths::ensure_parent_dir(&p);
                                match fs::write(&p, serialized) {
                                    Ok(()) => {
                                        self.status_banner = format!("Saved custom deck to {}!", p.display());
                                        audio.play_ui(SfxCue::UiCardTurn);
                                    }
                                    Err(e) => {
                                        self.status_banner = format!("⚠ Save failed: {e}");
                                    }
                                }
                            }
                        }
                    });
                });

                ui.add_space(6.0);

                // 3 Command Cards Architecture Header (1 Faction Card + 1-2 Bio-Type Cards)
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Slot:").strong());
                    if ui.selectable_label(self.target_slot == 0, "P1 Deck").clicked() {
                        self.target_slot = 0;
                    }
                    if ui.selectable_label(self.target_slot == 1, "P2 / AI Deck").clicked() {
                        self.target_slot = 1;
                    }
                    ui.separator();
                    ui.label(RichText::new("Deck Name:").strong());
                    ui.text_edit_singleline(&mut self.working_deck.deck_name);

                    ui.separator();
                    // CARD 1: FACTION COMMAND CARD (CONTINENT)
                    ui.label(RichText::new("Card 1: Faction (Continent):").strong().color(Color32::from_rgb(120, 220, 240)));
                    egui::ComboBox::from_id_salt("db_continent_fac")
                        .selected_text(format!("{:?}", self.working_deck.faction))
                        .show_ui(ui, |ui| {
                            for orig in FactionCardDef::ALL {
                                if ui
                                    .selectable_label(self.working_deck.faction == orig, format!("{:?}", orig))
                                    .clicked()
                                {
                                    self.working_deck.faction = orig;
                                }
                            }
                        });

                    ui.separator();
                    // CARDS 2 & 3: BIO-TYPE MODE TOGGLE
                    let is_pure = self.working_deck.is_pure_biotype();
                    if ui.selectable_label(is_pure, "🌿 Pure Bio-Type (1 Organism)").on_hover_text("Commit to 1 Bio-Type to unlock powerful Apex Mono-Organism Doctrine").clicked() {
                        self.working_deck.is_pure_biotype = true;
                        self.working_deck.secondary_faction = self.working_deck.primary_faction;
                    }
                    if ui.selectable_label(!is_pure, "🧬 Hybrid Bio-Type (2 Organisms)").on_hover_text("Combine 2 Organism Bio-Types for dual synergy doctrines").clicked() {
                        self.working_deck.is_pure_biotype = false;
                    }

                    ui.label(RichText::new("Primary Bio-Type:").strong());
                    egui::ComboBox::from_id_salt("db_prim_fac")
                        .selected_text(format!("{:?}", self.working_deck.primary_faction))
                        .show_ui(ui, |ui| {
                            for fac in BIO_FACTIONS {
                                if ui
                                    .selectable_label(self.working_deck.primary_faction == fac, format!("{:?}", fac))
                                    .clicked()
                                {
                                    self.working_deck.primary_faction = fac;
                                    if self.working_deck.is_pure_biotype {
                                        self.working_deck.secondary_faction = fac;
                                    }
                                }
                            }
                        });

                    if !self.working_deck.is_pure_biotype() {
                        ui.label(RichText::new("Secondary Bio-Type:").strong());
                        egui::ComboBox::from_id_salt("db_sec_fac")
                            .selected_text(format!("{:?}", self.working_deck.secondary_faction))
                            .show_ui(ui, |ui| {
                                for fac in BIO_FACTIONS {
                                    if ui
                                        .selectable_label(self.working_deck.secondary_faction == fac, format!("{:?}", fac))
                                        .clicked()
                                    {
                                        self.working_deck.secondary_faction = fac;
                                    }
                                }
                            });
                    }

                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("⚡ Auto-Generate 250-Pt Deck")
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(26, 128, 92)),
                        )
                        .on_hover_text("Automatically build an optimal, tournament-legal 250-point deck for chosen faction and bio-types")
                        .clicked()
                    {
                        self.auto_generate_faction_synergy_deck(roster);
                        audio.play_ui(SfxCue::BioHealChime);
                    }

                    if ui.button("🧹 Trim Off-Faction").clicked() {
                        self.prune_illegal_faction_cards(roster);
                        audio.play_ui(SfxCue::UiCardTurn);
                    }

                    if ui.selectable_label(self.show_command_cards_detail, "📜 Command Cards View").clicked() {
                        self.show_command_cards_detail = !self.show_command_cards_detail;
                    }
                });

                // Detailed Command Cards Display Panel
                if self.show_command_cards_detail {
                    let fac_card = self.working_deck.faction_card();
                    let bio1_card = self.working_deck.primary_biotype_card();
                    let bio2_opt = self.working_deck.secondary_biotype_card();

                    ui.add_space(4.0);
                    ui.columns(if bio2_opt.is_some() { 3 } else { 2 }, |cols| {
                        // Card 1: Faction Command Card
                        cols[0].group(|ui| {
                            ui.label(RichText::new("CARD 1 • CONTINENTAL FACTION").strong().color(Color32::from_rgb(112, 232, 248)));
                            ui.label(RichText::new(&fac_card.title).strong().color(Color32::WHITE));
                            ui.label(RichText::new(format!("Redoubt: {}", fac_card.bunker_stronghold)).small().italics().color(Color32::from_rgb(170, 200, 215)));
                            ui.separator();
                            ui.label(RichText::new(format!("Doctrine: {}", fac_card.command_doctrine)).strong().color(Color32::from_rgb(246, 192, 78)));
                            ui.label(RichText::new(&fac_card.passive_summary).small().color(Color32::from_rgb(200, 220, 230)));
                            ui.label(RichText::new(format!("Active Ability: {} (Cost: {} Act / {} Mun)", fac_card.active_ability_name, fac_card.active_action_cost, fac_card.active_munition_cost)).small().color(Color32::from_rgb(120, 240, 160)));
                            ui.label(RichText::new(format!("↳ {}", fac_card.active_effect_summary)).small().italics().color(Color32::from_rgb(160, 210, 200)));
                        });

                        // Card 2: Primary Bio-Type Card
                        cols[1].group(|ui| {
                            let is_pure = self.working_deck.is_pure_biotype();
                            ui.label(RichText::new(if is_pure { "CARD 2 & 3 • PURE BIO-TYPE CARD" } else { "CARD 2 • PRIMARY BIO-TYPE" }).strong().color(Color32::from_rgb(130, 240, 140)));
                            ui.label(RichText::new(&bio1_card.title).strong().color(Color32::WHITE));
                            ui.separator();
                            if is_pure {
                                ui.label(RichText::new(format!("Apex Doctrine: {}", bio1_card.pure_doctrine_name)).strong().color(Color32::from_rgb(246, 210, 60)));
                                ui.label(RichText::new(&bio1_card.pure_doctrine_effect).small().color(Color32::from_rgb(230, 240, 220)));
                            } else {
                                ui.label(RichText::new(format!("Doctrine: {}", bio1_card.hybrid_doctrine_name)).strong().color(Color32::from_rgb(180, 230, 140)));
                                ui.label(RichText::new(&bio1_card.hybrid_doctrine_effect).small().color(Color32::from_rgb(200, 225, 215)));
                            }
                            ui.label(RichText::new(format!("Primal Surge: {} (Cost: {} Act / {} Mun)", bio1_card.primal_surge_name, bio1_card.primal_surge_cost.0, bio1_card.primal_surge_cost.1)).small().color(Color32::from_rgb(140, 230, 210)));
                            ui.label(RichText::new(format!("↳ {}", bio1_card.primal_surge_effect)).small().italics().color(Color32::from_rgb(160, 205, 195)));
                        });

                        // Card 3: Secondary Bio-Type Card (if hybrid)
                        if let Some(bio2) = bio2_opt {
                            cols[2].group(|ui| {
                                ui.label(RichText::new("CARD 3 • SECONDARY BIO-TYPE").strong().color(Color32::from_rgb(240, 170, 120)));
                                ui.label(RichText::new(&bio2.title).strong().color(Color32::WHITE));
                                ui.separator();
                                ui.label(RichText::new(format!("Doctrine: {}", bio2.hybrid_doctrine_name)).strong().color(Color32::from_rgb(246, 180, 120)));
                                ui.label(RichText::new(&bio2.hybrid_doctrine_effect).small().color(Color32::from_rgb(225, 215, 205)));
                                ui.label(RichText::new(format!("Primal Surge: {} (Cost: {} Act / {} Mun)", bio2.primal_surge_name, bio2.primal_surge_cost.0, bio2.primal_surge_cost.1)).small().color(Color32::from_rgb(230, 190, 140)));
                                ui.label(RichText::new(format!("↳ {}", bio2.primal_surge_effect)).small().italics().color(Color32::from_rgb(205, 185, 175)));
                            });
                        }
                    });
                }

                ui.add_space(6.0);
                let frac = (report.points_used as f32 / self.working_deck.point_budget.max(1) as f32).clamp(0.0, 1.0);
                ui.add(
                    ProgressBar::new(frac)
                        .text(format!(
                            "Points Used: {} / {} pts  ({:+} remaining)  •  Cards: {}  •  Max Actions/Turn: {}  •  {}",
                            report.points_used,
                            self.working_deck.point_budget,
                            report.points_remaining,
                            report.total_cards,
                            report.max_actions_per_turn,
                            if report.is_legal { "TOURNAMENT LEGAL (PASS)" } else { "NEEDS ADJUSTMENT" }
                        ))
                        .fill(if report.points_used <= self.working_deck.point_budget {
                            Color32::from_rgb(38, 148, 96)
                        } else {
                            Color32::from_rgb(198, 52, 52)
                        }),
                );
                ui.label(RichText::new(&self.status_banner).small().color(Color32::from_rgb(168, 204, 218)));
                // DECK COMPARISON & 6-AXIS RADAR CHART PANEL
                let deck_a_metrics = compute_deck_metrics(&self.working_deck, roster);

                if self.show_deck_comparison {
                    ui.separator();
                    Frame::NONE
                        .fill(Color32::from_rgb(14, 24, 32))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(46, 82, 104)))
                        .corner_radius(6)
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("📊 SIDE-BY-SIDE DECK COMPARISON & 6-AXIS RADAR CHART")
                                        .strong()
                                        .color(Color32::from_rgb(112, 232, 248)),
                                );
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    egui::ComboBox::from_id_salt("db_comp_target")
                                        .selected_text(match self.comparison_target {
                                            ComparisonTarget::OpponentSlot1 => "P2 / AI Opponent Deck",
                                            ComparisonTarget::PresetPlantCnidarian => "Preset: Plant + Cnidarian (Australia)",
                                            ComparisonTarget::PresetMammalReptile => "Preset: Mammal + Reptile (Africa)",
                                            ComparisonTarget::PresetFishAmphibian => "Preset: Fish + Amphibian (North America)",
                                            ComparisonTarget::PresetBirdArachnid => "Preset: Bird + Arachnid (South America)",
                                            ComparisonTarget::PresetPurePlant => "Preset: Pure Plant Dominion (Australia)",
                                            ComparisonTarget::PresetPureInsect => "Preset: Pure Insect Swarm (Asia)",
                                            ComparisonTarget::CustomSavedRon => "Custom Saved Deck (.ron)",
                                        })
                                        .show_ui(ui, |ui| {
                                            for (tgt, label) in ComparisonTarget::ALL {
                                                if ui.selectable_label(self.comparison_target == tgt, label).clicked() {
                                                    self.comparison_target = tgt;
                                                }
                                            }
                                        });
                                    ui.label(RichText::new("Compare Deck Against:").strong());
                                });
                            });

                            let deck_b: DeckList = match self.comparison_target {
                                ComparisonTarget::OpponentSlot1 => starter_presets[1].clone(),
                                ComparisonTarget::PresetPlantCnidarian => starter_presets[0].clone(),
                                ComparisonTarget::PresetMammalReptile => starter_presets[1].clone(),
                                ComparisonTarget::PresetFishAmphibian => Self::build_preset_fish_amphibian(roster),
                                ComparisonTarget::PresetBirdArachnid => Self::build_preset_bird_arachnid(roster),
                                ComparisonTarget::PresetPurePlant => Self::build_preset_pure_plant(roster),
                                ComparisonTarget::PresetPureInsect => Self::build_preset_pure_insect(roster),
                                ComparisonTarget::CustomSavedRon => {
                                    let p = crate::paths::resolve_asset_path("compiled/custom_user_deck.ron")
                                        .unwrap_or_else(|| crate::paths::writable_app_path("assets/compiled/custom_user_deck.ron"));
                                    fs::read_to_string(p)
                                        .ok()
                                        .and_then(|r| ron::from_str::<DeckList>(&r).ok())
                                        .unwrap_or_else(|| starter_presets[1].clone())
                                }
                            };
                            let deck_b_metrics = compute_deck_metrics(&deck_b, roster);

                            ui.columns(2, |c_cols| {
                                // Left Column: 6-Axis Radar Spider Chart
                                c_cols[0].vertical_centered(|ui| {
                                    render_radar_chart(
                                        ui,
                                        &deck_a_metrics,
                                        &self.working_deck.deck_name,
                                        Some((&deck_b_metrics, &deck_b.deck_name)),
                                    );
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("■ Current Deck (Cyan)").strong().color(Color32::from_rgb(50, 230, 255)));
                                        ui.separator();
                                        ui.label(RichText::new("■ Target Deck (Amber)").strong().color(Color32::from_rgb(255, 175, 50)));
                                    });
                                });

                                // Right Column: Comparative Metrics Breakdown Table
                                c_cols[1].vertical(|ui| {
                                    egui::Grid::new("db_comp_grid")
                                        .striped(true)
                                        .spacing([12.0, 4.0])
                                        .show(ui, |ui| {
                                            ui.label(RichText::new("METRIC").strong().color(Color32::from_rgb(180, 210, 225)));
                                            ui.label(RichText::new("CURRENT DECK").strong().color(Color32::from_rgb(50, 230, 255)));
                                            ui.label(RichText::new("TARGET DECK").strong().color(Color32::from_rgb(255, 175, 50)));
                                            ui.label(RichText::new("DELTA (Δ)").strong().color(Color32::WHITE));
                                            ui.end_row();

                                            // Faction
                                            ui.label("Faction Command");
                                            ui.label(format!("{:?}", self.working_deck.faction));
                                            ui.label(format!("{:?}", deck_b.faction));
                                            ui.label("-");
                                            ui.end_row();

                                            // Bio-Type Mode
                                            ui.label("Bio-Type Cards");
                                            let a_bio = if self.working_deck.is_pure_biotype() {
                                                format!("Pure {:?}", self.working_deck.primary_faction)
                                            } else {
                                                format!("{:?} + {:?}", self.working_deck.primary_faction, self.working_deck.secondary_faction)
                                            };
                                            let b_bio = if deck_b.is_pure_biotype() {
                                                format!("Pure {:?}", deck_b.primary_faction)
                                            } else {
                                                format!("{:?} + {:?}", deck_b.primary_faction, deck_b.secondary_faction)
                                            };
                                            ui.label(a_bio);
                                            ui.label(b_bio);
                                            ui.label("-");
                                            ui.end_row();

                                            // Points Used
                                            ui.label("Point Budget");
                                            ui.label(format!("{} / 250", deck_a_metrics.total_points));
                                            ui.label(format!("{} / 250", deck_b_metrics.total_points));
                                            let d_pts = deck_a_metrics.total_points as i32 - deck_b_metrics.total_points as i32;
                                            ui.label(RichText::new(format!("{:+} pts", d_pts)).color(if d_pts <= 0 { Color32::from_rgb(112, 242, 164) } else { Color32::from_rgb(255, 140, 140) }));
                                            ui.end_row();

                                            // Total Cards
                                            ui.label("Total Cards");
                                            ui.label(format!("{}", deck_a_metrics.total_cards));
                                            ui.label(format!("{}", deck_b_metrics.total_cards));
                                            let d_cds = deck_a_metrics.total_cards as i32 - deck_b_metrics.total_cards as i32;
                                            ui.label(format!("{:+} cards", d_cds));
                                            ui.end_row();

                                            // Total EHP
                                            ui.label("Total EHP (Health)");
                                            ui.label(format!("{:.1}", deck_a_metrics.total_ehp));
                                            ui.label(format!("{:.1}", deck_b_metrics.total_ehp));
                                            let d_ehp = deck_a_metrics.total_ehp - deck_b_metrics.total_ehp;
                                            ui.label(RichText::new(format!("{:+.1}", d_ehp)).color(if d_ehp >= 0.0 { Color32::from_rgb(112, 242, 164) } else { Color32::from_rgb(255, 140, 140) }));
                                            ui.end_row();

                                            // Avg Mobility
                                            ui.label("Avg Mobility");
                                            ui.label(format!("{:.2} hex/turn", deck_a_metrics.avg_move));
                                            ui.label(format!("{:.2} hex/turn", deck_b_metrics.avg_move));
                                            let d_mov = deck_a_metrics.avg_move - deck_b_metrics.avg_move;
                                            ui.label(RichText::new(format!("{:+.2}", d_mov)).color(if d_mov >= 0.0 { Color32::from_rgb(112, 242, 164) } else { Color32::from_rgb(255, 140, 140) }));
                                            ui.end_row();

                                            // Strike Firepower
                                            ui.label("Strike Firepower");
                                            ui.label(format!("{}", deck_a_metrics.total_firepower));
                                            ui.label(format!("{}", deck_b_metrics.total_firepower));
                                            let d_fp = deck_a_metrics.total_firepower as i32 - deck_b_metrics.total_firepower as i32;
                                            ui.label(RichText::new(format!("{:+}", d_fp)).color(if d_fp >= 0 { Color32::from_rgb(112, 242, 164) } else { Color32::from_rgb(255, 140, 140) }));
                                            ui.end_row();

                                            // Logistics Capacity
                                            ui.label("Munitions Supply");
                                            ui.label(format!("{} ENC", deck_a_metrics.munitions_capacity));
                                            ui.label(format!("{} ENC", deck_b_metrics.munitions_capacity));
                                            let d_enc = deck_a_metrics.munitions_capacity as i32 - deck_b_metrics.munitions_capacity as i32;
                                            ui.label(RichText::new(format!("{:+}", d_enc)).color(if d_enc >= 0 { Color32::from_rgb(112, 242, 164) } else { Color32::from_rgb(255, 140, 140) }));
                                            ui.end_row();

                                            // Tactical Tech
                                            ui.label("Tactical Tech");
                                            ui.label(format!("{}", deck_a_metrics.tactical_control));
                                            ui.label(format!("{}", deck_b_metrics.tactical_control));
                                            let d_tech = deck_a_metrics.tactical_control as i32 - deck_b_metrics.tactical_control as i32;
                                            ui.label(RichText::new(format!("{:+}", d_tech)).color(if d_tech >= 0 { Color32::from_rgb(112, 242, 164) } else { Color32::from_rgb(255, 140, 140) }));
                                            ui.end_row();

                                            // Bio-Synergy
                                            ui.label("Bio-Synergy");
                                            ui.label(format!("{}", deck_a_metrics.bio_synergy_score));
                                            ui.label(format!("{}", deck_b_metrics.bio_synergy_score));
                                            let d_syn = deck_a_metrics.bio_synergy_score as i32 - deck_b_metrics.bio_synergy_score as i32;
                                            ui.label(RichText::new(format!("{:+}", d_syn)).color(if d_syn >= 0 { Color32::from_rgb(112, 242, 164) } else { Color32::from_rgb(255, 140, 140) }));
                                            ui.end_row();
                                        });
                                });
                            });
                        });
                    ui.add_space(4.0);
                }

                ui.separator();

                // Two-Column Split: Left = 152-Card Catalog Browser | Right = Active Deck & Legality Diagnostics
                ui.columns(2, |cols| {
                    // LEFT COLUMN: FILTERABLE 152-CARD CATALOG
                    cols[0].horizontal_wrapped(|ui| {
                        ui.label(RichText::new("🔍 Search:").strong());
                        ui.text_edit_singleline(&mut self.search_query);
                        ui.checkbox(&mut self.show_only_legal_factions, "Legal Factions Only");
                    });
                    cols[0].horizontal_wrapped(|ui| {
                        for (cf, label) in ClassFilter::ALL {
                            if ui.selectable_label(self.class_filter == cf, label).clicked() {
                                self.class_filter = cf;
                            }
                        }
                    });
                    cols[0].add_space(4.0);

                    let q_low = self.search_query.trim().to_lowercase();
                    let f1 = self.working_deck.primary_faction;
                    let f2 = self.working_deck.secondary_faction;
                    let is_pure = self.working_deck.is_pure_biotype();

                    ScrollArea::vertical()
                        .id_salt("db_catalog_scroll")
                        .max_height(if self.show_deck_comparison { 360.0 } else { 520.0 })
                        .show(&mut cols[0], |ui| {
                            for card in &roster.units {
                                let fac_legal = if is_pure {
                                    card.faction == OrganismFaction::Human || card.faction == f1
                                } else {
                                    card.faction == OrganismFaction::Human || card.faction == f1 || card.faction == f2
                                };
                                if self.show_only_legal_factions && !fac_legal {
                                    continue;
                                }
                                if !self.class_filter.matches(card.combat_class) {
                                    continue;
                                }
                                if !q_low.is_empty()
                                    && !card.card_key.to_lowercase().contains(&q_low)
                                    && !card.signature_ability.to_lowercase().contains(&q_low)
                                    && !format!("{:?}", card.faction).to_lowercase().contains(&q_low)
                                {
                                    continue;
                                }

                                Frame::NONE
                                    .fill(Color32::from_rgb(20, 31, 39))
                                    .stroke(Stroke::new(
                                        1.0,
                                        if fac_legal {
                                            Color32::from_rgb(46, 76, 92)
                                        } else {
                                            Color32::from_rgb(132, 48, 48)
                                        },
                                    ))
                                    .corner_radius(4)
                                    .inner_margin(Margin::symmetric(8, 5))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            draw_unit_silhouette(ui, card, Vec2::new(26.0, 26.0), Color32::from_rgb(130, 220, 240));
                                            if ui.small_button("➕ Add").clicked() {
                                                self.add_card_copy(&card.card_key, card.combat_class);
                                                audio.play_ui(SfxCue::UiCardTurn);
                                            }
                                            if ui.small_button("🔍 DB").on_hover_text("Open detailed tactical dossier in Database Manager (F2)").clicked() {
                                                self.requested_db_card = Some(card.card_key.clone());
                                            }
                                            if ui.small_button("🖨 STL").on_hover_text("Export watertight 3D-printable .STL with 45° micro-dice V-cradles").clicked() {
                                                let cfg = TabletopPrintConfig::default();
                                                let out_p = crate::paths::writable_app_path("exports/stl/cards")
                                                    .join(format!("{}__STR{}_ENC{}.stl", sanitize_filename_slug(&card.card_key), card.strength_hp, card.encumbrance));
                                                let vox_dir = crate::paths::resolve_asset_path("vox")
                                                    .unwrap_or_else(|| crate::paths::writable_app_path("assets/vox"));
                                                match export_card_to_stl(card, &vox_dir, &out_p, &cfg) {
                                                    Ok(rep) => {
                                                        self.status_banner = format!("✓ Exported {} ({})", rep.output_path.display(), rep.resolved_cradle_desc);
                                                        audio.play_ui(SfxCue::BioHealChime);
                                                    }
                                                    Err(e) => {
                                                        self.status_banner = format!("⚠ STL export error: {e}");
                                                    }
                                                }
                                            }
                                            ui.label(
                                                RichText::new(format!("{} ({}p)", card.card_key, card.deck_points))
                                                    .strong()
                                                    .color(Color32::WHITE),
                                            );
                                            ui.label(
                                                RichText::new(format!(
                                                    "[{:?} • {:?}] STR {} ARM {} MOV {} RNG {} ENC {}",
                                                    card.faction,
                                                    card.combat_class,
                                                    card.strength_hp,
                                                    card.armor,
                                                    card.movement,
                                                    card.ranged_distance,
                                                    card.encumbrance
                                                ))
                                                .small()
                                                .color(Color32::from_rgb(176, 198, 210)),
                                            );
                                        });
                                        ui.label(
                                            RichText::new(format!("   ↳ {}", card.signature_ability))
                                                .small()
                                                .italics()
                                                .color(Color32::from_rgb(130, 188, 206)),
                                        );
                                    });
                                ui.add_space(3.0);
                            }
                        });

                    // RIGHT COLUMN: ACTIVE DECK SLOTS + GRAVEYARD SYNERGY + LAUNCH BUTTON
                    cols[1].horizontal(|ui| {
                        ui.label(
                            RichText::new("ACTIVE DECKLIST & TOURNAMENT DIAGNOSTICS")
                                .strong()
                                .color(Color32::from_rgb(230, 242, 248)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let launch_btn = egui::Button::new(
                                RichText::new("⚔ Launch Match With This Deck")
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(if report.is_legal {
                                Color32::from_rgb(28, 152, 92)
                            } else {
                                Color32::from_rgb(75, 82, 88)
                            });
                            if ui.add_enabled(report.is_legal, launch_btn).clicked() {
                                audio.play_ui(SfxCue::UiCardTurn);
                                self.is_open = false;
                                launched_deck = Some((self.target_slot, self.working_deck.clone()));
                            }
                        });
                    });

                    // Graveyard Facility Synergy Check
                    let mut bio_units = 0usize;
                    let mut mech_units = 0usize;
                    let mut has_hosp = false;
                    let mut has_fact = false;
                    for e in &self.working_deck.entries {
                        if e.quantity == 0 {
                            continue;
                        }
                        if e.card_key == "Hospital" {
                            has_hosp = true;
                        }
                        if e.card_key == "Factory" {
                            has_fact = true;
                        }
                        if let Some(c) = lookup.get(e.card_key.as_str()) {
                            match c.graveyard_queue {
                                GraveyardFacility::HospitalBio => bio_units += usize::from(e.quantity),
                                GraveyardFacility::FactoryMech => mech_units += usize::from(e.quantity),
                                _ => {}
                            }
                        }
                    }
                    cols[1].label(
                        RichText::new(format!(
                            "Graveyard Queues: {} Bio cards ({})  |  {} Mech/Tech cards ({})",
                            bio_units,
                            if has_hosp { "Hospital Ready ✓" } else { "No Hospital" },
                            mech_units,
                            if has_fact { "Factory Ready ✓" } else { "No Factory" }
                        ))
                        .small()
                        .color(Color32::from_rgb(168, 210, 194)),
                    );

                    if !report.violations.is_empty() {
                        for v in &report.violations {
                            cols[1].label(
                                RichText::new(format!("⚠ {}", v))
                                    .small()
                                    .color(Color32::from_rgb(255, 118, 118)),
                            );
                        }
                    } else {
                        cols[1].label(
                            RichText::new("✓ All 250-Pt PC Tournament Rules Satisfied (HQ, Depot, Commander, Nodes & 2-Faction Cap)")
                                .small()
                                .color(Color32::from_rgb(112, 242, 164)),
                        );
                    }
                    cols[1].add_space(4.0);

                    let entries_snapshot = self.working_deck.entries.clone();
                    ScrollArea::vertical()
                        .id_salt("db_deck_scroll")
                        .max_height(if self.show_deck_comparison { 285.0 } else { 445.0 })
                        .show(&mut cols[1], |ui| {
                            for entry in entries_snapshot {
                                let Some(card) = lookup.get(entry.card_key.as_str()) else {
                                    continue;
                                };
                                let line_pts = card.deck_points * u16::from(entry.quantity);
                                let is_pure = self.working_deck.is_pure_biotype();
                                let fac_ok = if is_pure {
                                    card.faction == OrganismFaction::Human || card.faction == self.working_deck.primary_faction
                                } else {
                                    card.faction == OrganismFaction::Human
                                        || card.faction == self.working_deck.primary_faction
                                        || card.faction == self.working_deck.secondary_faction
                                };

                                Frame::NONE
                                    .fill(Color32::from_rgb(22, 34, 42))
                                    .stroke(Stroke::new(
                                        1.0,
                                        if fac_ok {
                                            Color32::from_rgb(48, 84, 98)
                                        } else {
                                            Color32::from_rgb(210, 68, 68)
                                        },
                                    ))
                                    .corner_radius(4)
                                    .inner_margin(Margin::symmetric(8, 4))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            if ui.small_button("➖").clicked() {
                                                self.adjust_entry_qty(&entry.card_key, -1);
                                                audio.play_ui(SfxCue::UiCardTurn);
                                            }
                                            ui.label(RichText::new(format!("{}x", entry.quantity)).strong());
                                            if ui.small_button("➕").clicked() {
                                                let cap = Self::max_copies_for_class(card.combat_class);
                                                if entry.quantity < cap {
                                                    self.adjust_entry_qty(&entry.card_key, 1);
                                                    audio.play_ui(SfxCue::UiCardTurn);
                                                }
                                            }
                                            ui.label(
                                                RichText::new(&card.card_key)
                                                    .strong()
                                                    .color(Color32::WHITE),
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(format!("{} pts", line_pts))
                                                            .strong()
                                                            .color(Color32::from_rgb(246, 192, 78)),
                                                    );
                                                    ui.label(
                                                        RichText::new(format!("{:?}", card.faction))
                                                            .small()
                                                            .color(Color32::from_rgb(164, 194, 208)),
                                                    );
                                                },
                                            );
                                        });
                                    });
                                ui.add_space(2.0);
                            }
                        });
                });
            });

        launched_deck
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn load_roster() -> RosterManifest {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let raw = fs::read_to_string(root.join("assets/compiled/master_roster.ron"))
            .expect("Missing master_roster.ron");
        ron::from_str(&raw).expect("Invalid master_roster.ron")
    }

    #[test]
    fn all_preset_and_auto_generated_decks_validate_legally() {
        let roster = load_roster();
        for (label, deck) in [
            ("Fish+Amphibian", DeckBuilderScreen::build_preset_fish_amphibian(&roster)),
            ("Bird+Arachnid", DeckBuilderScreen::build_preset_bird_arachnid(&roster)),
            ("Pure Plant", DeckBuilderScreen::build_preset_pure_plant(&roster)),
            ("Pure Insect", DeckBuilderScreen::build_preset_pure_insect(&roster)),
        ] {
            let rep = deck.validate_against(&roster);
            assert!(rep.is_legal, "{label} failed deck validation: {:?}", rep.violations);
        }

        let initial = DeckBuilderScreen::build_preset_pure_plant(&roster);
        let mut screen = DeckBuilderScreen::new(initial);
        screen.auto_generate_faction_synergy_deck(&roster);
        let auto_rep = screen.working_deck.validate_against(&roster);
        assert!(auto_rep.is_legal, "Auto-generated synergy deck failed validation: {:?}", auto_rep.violations);
    }
}
