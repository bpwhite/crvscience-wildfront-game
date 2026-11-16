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

//! Ingests `Wildfront_PC_Master_Stats_and_DeckBuilder.xlsx` (v3 133-Card Balanced Model)
//! into validated `RosterManifest` and `DeckList` artifacts.

use std::path::Path;
use anyhow::{anyhow, Context, Result};
use calamine::{open_workbook, Data, Reader, Xlsx};
use wf_core::{
    CombatClass, DeckEntry, DeckList, GraveyardFacility, OrganismFaction, OriginAllegiance,
    RosterManifest, UnitCardDef,
};

fn cell_str(row: &[Data], idx: usize) -> String {
    row.get(idx)
        .map(|d| match d {
            Data::String(s) => s.trim().to_string(),
            Data::Float(f) => format!("{}", *f as i64),
            Data::Int(i) => i.to_string(),
            _ => String::new(),
        })
        .unwrap_or_default()
}

fn cell_u8(row: &[Data], idx: usize) -> u8 {
    row.get(idx)
        .map(|d| match d {
            Data::Int(i) => (*i).clamp(0, 255) as u8,
            Data::Float(f) => f.round().clamp(0.0, 255.0) as u8,
            Data::String(s) => s.trim().parse::<f64>().unwrap_or(0.0).round() as u8,
            _ => 0,
        })
        .unwrap_or(0)
}

fn cell_u16(row: &[Data], idx: usize) -> u16 {
    row.get(idx)
        .map(|d| match d {
            Data::Int(i) => (*i).clamp(0, 65535) as u16,
            Data::Float(f) => f.round().clamp(0.0, 65535.0) as u16,
            Data::String(s) => s.trim().parse::<f64>().unwrap_or(0.0).round() as u16,
            _ => 0,
        })
        .unwrap_or(0)
}

fn cell_tenths_u16(row: &[Data], idx: usize) -> u16 {
    row.get(idx)
        .map(|d| match d {
            Data::Int(i) => ((*i) * 10).clamp(0, 65535) as u16,
            Data::Float(f) => (f * 10.0).round().clamp(0.0, 65535.0) as u16,
            _ => 0,
        })
        .unwrap_or(0)
}

pub fn compile_master_xlsx(xlsx_path: &Path) -> Result<(RosterManifest, DeckList)> {
    let mut wb: Xlsx<_> = open_workbook(xlsx_path)
        .with_context(|| format!("Failed to open workbook '{}'", xlsx_path.display()))?;

    let range = wb
        .worksheet_range("Master Roster")
        .context("Missing 'Master Roster' sheet")?;

    let mut units = Vec::new();
    for (row_idx, row) in range.rows().enumerate().skip(1) {
        let key = cell_str(row, 0);
        if key.is_empty() || key.starts_with("Roster Totals") {
            continue;
        }
        let combat_class = CombatClass::from_label(&cell_str(row, 5))
            .with_context(|| format!("Row {} invalid Combat Class", row_idx + 1))?;

        let unit = UnitCardDef {
            card_key: key,
            unit_name: cell_str(row, 1),
            origin: OriginAllegiance::from_label(&cell_str(row, 2))?,
            species: cell_str(row, 3),
            faction: OrganismFaction::from_label(&cell_str(row, 4))?,
            combat_class,
            graveyard_queue: GraveyardFacility::from_label(&cell_str(row, 6))?,
            signature_ability: cell_str(row, 7),
            strength_hp: cell_u8(row, 8),
            encumbrance: cell_u8(row, 9),
            armor: cell_u8(row, 10),
            shield: cell_u8(row, 11),
            movement: cell_u8(row, 12),
            evade: cell_u8(row, 13),
            melee_range: cell_u8(row, 14),
            ranged_distance: cell_u8(row, 15),
            is_laser: cell_u8(row, 16) > 0,
            node_leash_range: cell_u8(row, 17),
            emits_node: cell_u8(row, 18) > 0,
            node_emit_radius: cell_u8(row, 19),
            action_bonus: cell_u8(row, 20),
            supply_range: cell_u8(row, 21),
            radar_range: cell_u8(row, 22),
            heal_repair_range: cell_u8(row, 23),
            root_stacks: cell_u8(row, 24),
            stealth: cell_u8(row, 25) > 0,
            poison_stacks: cell_u8(row, 26),
            poison_immune: cell_u8(row, 27) > 0,
            can_fly: cell_u8(row, 28) > 0,
            saboteur: cell_u8(row, 29) > 0,
            can_burrow: cell_u8(row, 30) > 0,
            taunt_block: cell_u8(row, 31),
            amphibious_tier: cell_u8(row, 32),
            is_autonomous: cell_u8(row, 33) > 0,
            deck_points: cell_u16(row, 34),
            combat_ehp_tenths: cell_tenths_u16(row, 35),
            squad_model_count: combat_class.default_squad_models(),
            provenance_note: cell_str(row, 36),
        };
        units.push(unit);
    }

    let manifest = RosterManifest {
        schema_version: 3,
        source_workbook: xlsx_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Wildfront_PC_Master_Stats_and_DeckBuilder.xlsx")
            .to_string(),
        standard_deck_budget: 250,
        opening_hand_size: 5,
        base_actions_per_turn: 2,
        base_munitions_per_turn: 5,
        units,
    };
    manifest.validate()?;

    // Parse active starter deck from '250-Pt Deck Builder'
    let deck_sheet = wb
        .worksheet_range("250-Pt Deck Builder")
        .context("Missing '250-Pt Deck Builder' sheet")?;
    let rows_vec: Vec<&[Data]> = deck_sheet.rows().collect();
    if rows_vec.len() < 31 {
        return Err(anyhow!("'250-Pt Deck Builder' sheet truncated"));
    }
    let primary_faction = OrganismFaction::from_label(&cell_str(rows_vec[4], 0))?;
    let secondary_faction = OrganismFaction::from_label(&cell_str(rows_vec[4], 2))?;
    let point_budget = cell_u16(rows_vec[4], 4);

    let mut entries = Vec::new();
    for slot_row in rows_vec.iter().take(31).skip(11) {
        let card_key = cell_str(slot_row, 1);
        let qty = cell_u8(slot_row, 2);
        if !card_key.is_empty() && qty > 0 {
            entries.push(DeckEntry {
                card_key,
                quantity: qty,
            });
        }
    }

    let deck = DeckList {
        deck_name: format!("{:?} + {:?} 250-Pt Tournament Deck", primary_faction, secondary_faction),
        faction: OriginAllegiance::Australia,
        primary_faction,
        secondary_faction,
        is_pure_biotype: primary_faction == secondary_faction,
        point_budget,
        entries,
    };
    let report = deck.validate_against(&manifest);
    if !report.is_legal {
        return Err(anyhow!(
            "Starter deck failed tournament validation: {:?}",
            report.violations
        ));
    }

    Ok((manifest, deck))
}
