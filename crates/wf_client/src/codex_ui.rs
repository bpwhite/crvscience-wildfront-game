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

//! Interactive In-Game Rules, Combat Math, Faction Doctrine & Hotkey Codex (`F1`).

use egui::{Align2, Color32, Frame, Margin, Order, RichText, ScrollArea, Stroke, Ui, Window};
use wf_core::AbilityCatalog;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexTab {
    LogisticsAndEconomy,
    CombatAndFogOfWar,
    FactionDoctrines,
    ControlsAndMultiplayer,
}

pub struct CodexOverlay {
    pub is_open: bool,
    pub active_tab: CodexTab,
    catalog: AbilityCatalog,
}

impl Default for CodexOverlay {
    fn default() -> Self {
        Self {
            is_open: false,
            active_tab: CodexTab::LogisticsAndEconomy,
            catalog: AbilityCatalog::canonical(),
        }
    }
}

impl CodexOverlay {
    pub fn show(&mut self, root_ui: &mut Ui) {
        if !self.is_open {
            return;
        }
        let mut open_flag = self.is_open;
        Window::new("📖 WILDRONT BATTLEGROUNDS — 2072 FIELD MANUAL & CODEX (F1)")
            .open(&mut open_flag)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .order(Order::Foreground)
            .frame(
                Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(11, 19, 26, 248))
                    .stroke(Stroke::new(2.0, Color32::from_rgb(76, 196, 224)))
                    .corner_radius(10)
                    .inner_margin(Margin::same(18)),
            )
            .show(root_ui.ctx(), |ui| {
                ui.set_min_width(680.0);
                ui.set_max_width(720.0);

                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (CodexTab::LogisticsAndEconomy, "⚙ Logistics & Nodes"),
                        (CodexTab::CombatAndFogOfWar, "🎲 D6 Combat & Stealth"),
                        (CodexTab::FactionDoctrines, "🧬 10 Faction Doctrines"),
                        (CodexTab::ControlsAndMultiplayer, "⌨ Hotkeys & TCP 1v1"),
                    ] {
                        if ui.selectable_label(self.active_tab == tab, RichText::new(label).strong()).clicked() {
                            self.active_tab = tab;
                        }
                    }
                });
                ui.separator();

                ScrollArea::vertical()
                    .id_salt("wf_codex_scroll")
                    .max_height(430.0)
                    .show(ui, |ui| match self.active_tab {
                        CodexTab::LogisticsAndEconomy => {
                            Self::section(
                                ui,
                                "Actions & Psychic Control Nodes (Cyan Aura)",
                                "• Each turn you start with 2 Base Actions. Every static Control Node on the board grants +1 Action/turn (up to 5 Actions max).
                                 • Spending 1 Action lets you Deploy a card from your hand, Move a unit, Attack, Cast an Ability, or Rest (+1 STR).
                                 • Engineered bio-units require a Psychic Control Node in range to operate (`Node Leash`). Commanders, Drones, Control Nodes, and Cnidarian Synapse units project mobile/static node fields. Units outside Node range become CONFUSED (`⚠ NO NODE`) until re-linked.",
                            );
                            Self::section(
                                ui,
                                "Contiguous Supply Chain & Encumbrance Ammo (Amber Aura)",
                                "• Your Command Center (10 STR, 2 non-refreshing structural ARM) generates +5 Home Munitions/turn (Player 2 starts Turn 1 with 8 Munitions & 6 cards; both players start with 2 Base Actions).
                                 • Any friendly buildings touching in a contiguous chain from your Command Center automatically extend your Home Supply aura (`🔗SUP`).
                                 • Units inside Home Supply draw ranged/ability Munitions directly from Home Base; units outside Supply spend their carried `Encumbrance` (`📦`) ammo pool. Medics spend Munitions to heal (`Field Bio-Mend`).
                                 • Laser weapons (`⚡LSR`) cost 0 Munitions to fire, but can only fire once per turn.",
                            );
                            Self::section(
                                ui,
                                "Deck Building & Hospital / Factory Graveyard Recovery",
                                "• Standard 250-point decks combine `Human` infrastructure with 1 Pure Bio-Type or 2 Hybrid Bio-Types across 152 unique cards (including 19 single-use Tactical Deployments and Remote Cargo Drops).
                                 • Active supplied Hospitals and Factories spend 1 Munition/tick (up to 2 bays/turn, +2 STR/tick) with a 1-revive-per-deck-copy clone cap (2x for Pure Insect Light/Med). Round 30+ triggers Orbital Fallout Storm (-1/-2 HQ STR/round).",
                            );
                        }
                        CodexTab::CombatAndFogOfWar => {
                            Self::section(
                                ui,
                                "Distance Hit Roll & Strength-Invariant Damage",
                                "• Ranged Hit Threshold on 1D6: `clamp(1 + HexDistance + TargetEvade + Cover - Locks - Bonuses, 2..6)`. Natural `6` always hits; natural `1` misses. Melee (`Range 1`) automatically connects.
                                 • Strength == Health (`❤ STR`): As a unit loses health, its maximum attack dice cap drops!
                                 • Damage Formula: `Raw = min(DmgD6, Attacker_STR) + clamp(Attacker_STR - Defender_STR, -2..+3)`. Defender `Armor` (`🛡`, refreshed on mobile units only; static building Armor never auto-refreshes) absorbs `Raw` first; flyers ignore negative STR delta (`>= 0`) and cannot be hit by ground melee.",
                            );
                            Self::section(
                                ui,
                                "Fog-of-War, Mountain Line-of-Sight, Radar Range & Stealth",
                                "• Active Sight illuminates surrounding hexes (`+1` on Highland plateaus; `4` base sight for flying units). `MountainPeak` crags block non-flying ground sight lines.
                                 • Previously scouted tiles remain in dimmed Slate Memory (showing static terrain props), while unscouted sectors stay under Deep Radar Shroud.
                                 • `Stealth` & `Burrowed` units (`👁STEALTH`) remain invisible to enemies even inside normal sight unless an enemy moves adjacent (`Dist 1`), scans them inside `Radar Range` (`📡`, e.g., `Radar Tower`, `Drone`, `Recon` scouts), or the stealthed unit attacks (revealing it until its next turn).",
                            );
                        }
                        CodexTab::FactionDoctrines => {
                            for fp in &self.catalog.faction_passives {
                                Frame::NONE
                                    .fill(Color32::from_rgb(19, 30, 38))
                                    .stroke(Stroke::new(1.0, Color32::from_rgb(48, 86, 104)))
                                    .corner_radius(6)
                                    .inner_margin(Margin::same(10))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new(format!(
                                                "{:?} — {}",
                                                fp.faction, fp.doctrine_name
                                            ))
                                            .strong()
                                            .size(14.0)
                                            .color(Color32::from_rgb(112, 232, 248)),
                                        );
                                        ui.label(
                                            RichText::new(format!("Synergy Bonus: {}", fp.bonus_effect))
                                                .small()
                                                .color(Color32::from_rgb(196, 236, 212)),
                                        );
                                        ui.label(
                                            RichText::new(format!("Tradeoff: {}", fp.tactical_tradeoff))
                                                .small()
                                                .color(Color32::from_rgb(228, 192, 168)),
                                        );
                                    });
                                ui.add_space(5.0);
                            }
                        }
                        CodexTab::ControlsAndMultiplayer => {
                            Self::section(
                                ui,
                                "Keyboard & Mouse Command Reference",
                                "• Left Click: Select hex/unit, deploy selected hand card, move, attack, or cast ability
                                 • Right Drag / Left Drag: Orbit 3D camera  |  Scroll Wheel: Smooth voxel LOD zoom
                                 • WASD / Arrows: Pan camera across battlefield
                                 • Space: End Turn (or reveal Hotseat Turn-Handoff curtain)
                                 • B: Deck Builder  |  F2: Database  |  F3: 3D Hex Map Studio  |  F4: Replay & Telemetry  |  R: Dice  |  L: Sky Lighting  |  F1: Codex
                                 • Tab: Supply/Node hex auras  |  H: Floating 3D Unit HP bars  |  P: 3D Terrain Props  |  F: Fog-of-War
                                 • M: Toggle Solo vs AI <-> 1v1 Local Hotseat  |  F8: 3D Print Tabletop .STL Studio  |  F5 / F9: Quicksave / Quickload (.ron)
                                 • 1–5 / N: Switch procedural scenarios or roll a fresh map seed  |  Esc: Cancel / Main Menu",
                            );
                            Self::section(
                                ui,
                                "Tabletop 3D Print .STL Studio (F8) — Recessed 45° Micro-Dice V-Cradles",
                                "• Press `F8` (or click `🖨 STL` in the HUD / Main Menu / Deck Builder) to export watertight 2-manifold `.STL` miniatures scaled 1:1 to `45.0mm` tabletop hexes.
                                 • Each Staunton pedestal automatically carves support-free 45° V-cradles sized from `master_roster.ron`: twin `7mm` D6 cradles (`STR` + `MUN`) for combat squads, a centered `10mm` mini-D10 cradle for `ENC 0` HQs/Nodes, or a `D6 + D10` pair for `ENC 7–10` Convoys/Depots, separated by a solid `>= 2.80mm` printable center wall.",
                            );
                            Self::section(
                                ui,
                                "Testing 1v1 TCP Lockstep Multiplayer (Single PC or LAN)",
                                "• Open two terminals and run `cargo run -p wf_client --release` in each.
                                 • In Window 1 (Main Menu): choose your Scenario & Deck, then click `📡 Host 1v1 TCP` (`0.0.0.0:20720`). You command Team 1.
                                 • In Window 2 (Main Menu): customize Player 2's Deck if desired, enter `127.0.0.1:20720` (or Host LAN IP), and click `🔌 Join 1v1 TCP`. You command Team 2.
                                 • Both clients synchronize the 64-bit scenario seed, custom decks, and `SplitMix64` combat dice in deterministic lockstep while maintaining private per-window Fog-of-War and Tactical Hands!",
                            );
                        }
                    });
            });
        self.is_open = open_flag;
    }

    fn section(ui: &mut Ui, title: &str, body: &str) {
        ui.label(
            RichText::new(title)
                .size(15.0)
                .strong()
                .color(Color32::from_rgb(108, 228, 246)),
        );
        ui.add_space(3.0);
        ui.label(
            RichText::new(body)
                .size(12.5)
                .color(Color32::from_rgb(216, 228, 236)),
        );
        ui.add_space(10.0);
    }
}
