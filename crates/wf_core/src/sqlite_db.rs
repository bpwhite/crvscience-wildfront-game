//! SQLite persistence layer (`rusqlite` bundled) for *Wildfront Battlegrounds*.
//!
//! Stores and updates all 133+ unit cards, 6 Faction Cards, 9 Bio-Type Cards,
//! and PhyloPic silhouette metadata in `assets/compiled/wildfront_roster.db`.

use std::path::Path;
use rusqlite::{params, Connection, Result as SqlResult};
use crate::schema::{
    CombatClass, GraveyardFacility, OrganismFaction, OriginAllegiance, RosterManifest, UnitCardDef,
};

pub const DEFAULT_SQLITE_DB_PATH: &str = "assets/compiled/wildfront_roster.db";

pub fn load_roster_from_sqlite(path: &Path) -> SqlResult<RosterManifest> {
    let conn = Connection::open(path)?;
    let mut stmt = conn.prepare(
        "SELECT card_key, unit_name, origin, species, faction, combat_class, graveyard_queue,
                signature_ability, strength_hp, encumbrance, armor, shield, movement, evade,
                melee_range, ranged_distance, is_laser, node_leash_range, emits_node,
                node_emit_radius, action_bonus, supply_range, radar_range, heal_repair_range,
                root_stacks, stealth, poison_stacks, poison_immune, can_fly, saboteur,
                can_burrow, taunt_block, amphibious_tier, is_autonomous, deck_points,
                combat_ehp_tenths, squad_model_count, provenance_note
         FROM units ORDER BY card_key ASC",
    )?;

    let rows = stmt.query_map([], |row| {
        let origin_s: String = row.get(2)?;
        let fac_s: String = row.get(4)?;
        let cls_s: String = row.get(5)?;
        let gy_s: String = row.get(6)?;

        Ok(UnitCardDef {
            card_key: row.get(0)?,
            unit_name: row.get(1)?,
            origin: OriginAllegiance::from_label(&origin_s).unwrap_or(OriginAllegiance::Neutral),
            species: row.get(3)?,
            faction: OrganismFaction::from_label(&fac_s).unwrap_or(OrganismFaction::Human),
            combat_class: CombatClass::from_label(&cls_s).unwrap_or(CombatClass::Medium),
            graveyard_queue: GraveyardFacility::from_label(&gy_s).unwrap_or(GraveyardFacility::HospitalBio),
            signature_ability: row.get(7)?,
            strength_hp: row.get::<_, u32>(8)? as u8,
            encumbrance: row.get::<_, u32>(9)? as u8,
            armor: row.get::<_, u32>(10)? as u8,
            shield: row.get::<_, u32>(11)? as u8,
            movement: row.get::<_, u32>(12)? as u8,
            evade: row.get::<_, u32>(13)? as u8,
            melee_range: row.get::<_, u32>(14)? as u8,
            ranged_distance: row.get::<_, u32>(15)? as u8,
            is_laser: row.get::<_, u32>(16)? > 0,
            node_leash_range: row.get::<_, u32>(17)? as u8,
            emits_node: row.get::<_, u32>(18)? > 0,
            node_emit_radius: row.get::<_, u32>(19)? as u8,
            action_bonus: row.get::<_, u32>(20)? as u8,
            supply_range: row.get::<_, u32>(21)? as u8,
            radar_range: row.get::<_, u32>(22)? as u8,
            heal_repair_range: row.get::<_, u32>(23)? as u8,
            root_stacks: row.get::<_, u32>(24)? as u8,
            stealth: row.get::<_, u32>(25)? > 0,
            poison_stacks: row.get::<_, u32>(26)? as u8,
            poison_immune: row.get::<_, u32>(27)? > 0,
            can_fly: row.get::<_, u32>(28)? > 0,
            saboteur: row.get::<_, u32>(29)? > 0,
            can_burrow: row.get::<_, u32>(30)? > 0,
            taunt_block: row.get::<_, u32>(31)? as u8,
            amphibious_tier: row.get::<_, u32>(32)? as u8,
            is_autonomous: row.get::<_, u32>(33)? > 0,
            deck_points: row.get::<_, u32>(34)? as u16,
            combat_ehp_tenths: row.get::<_, u32>(35)? as u16,
            squad_model_count: row.get::<_, u32>(36)? as u8,
            provenance_note: row.get(37)?,
        })
    })?;

    let mut units = Vec::new();
    for r in rows {
        units.push(r?);
    }

    Ok(RosterManifest {
        schema_version: 3,
        source_workbook: "assets/compiled/wildfront_roster.db".to_string(),
        standard_deck_budget: 250,
        opening_hand_size: 5,
        base_actions_per_turn: 2,
        base_munitions_per_turn: 2,
        units,
    })
}

pub fn save_unit_to_sqlite(path: &Path, u: &UnitCardDef) -> SqlResult<()> {
    let conn = Connection::open(path)?;
    let genus = u.species.split_whitespace().next().unwrap_or("Homo");
    let slug: String = u
        .card_key
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let phylo_url = format!("https://www.phylopic.org/images?query={}", genus);
    let sil_path = format!("assets/silhouettes/{}.svg", slug.trim_matches('_'));

    conn.execute(
        "INSERT INTO units (
            card_key, unit_name, origin, species, faction, combat_class, graveyard_queue,
            signature_ability, strength_hp, encumbrance, armor, shield, movement, evade,
            melee_range, ranged_distance, is_laser, node_leash_range, emits_node,
            node_emit_radius, action_bonus, supply_range, radar_range, heal_repair_range,
            root_stacks, stealth, poison_stacks, poison_immune, can_fly, saboteur,
            can_burrow, taunt_block, amphibious_tier, is_autonomous, deck_points,
            combat_ehp_tenths, squad_model_count, provenance_note,
            phylopic_taxon, phylopic_uuid, phylopic_search_url, silhouette_path
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7,
            ?8, ?9, ?10, ?11, ?12, ?13, ?14,
            ?15, ?16, ?17, ?18, ?19,
            ?20, ?21, ?22, ?23, ?24,
            ?25, ?26, ?27, ?28, ?29, ?30,
            ?31, ?32, ?33, ?34, ?35,
            ?36, ?37, ?38,
            ?39, '', ?40, ?41
        )
        ON CONFLICT(card_key) DO UPDATE SET
            unit_name = excluded.unit_name,
            origin = excluded.origin,
            species = excluded.species,
            faction = excluded.faction,
            combat_class = excluded.combat_class,
            graveyard_queue = excluded.graveyard_queue,
            signature_ability = excluded.signature_ability,
            strength_hp = excluded.strength_hp,
            encumbrance = excluded.encumbrance,
            armor = excluded.armor,
            shield = excluded.shield,
            movement = excluded.movement,
            evade = excluded.evade,
            melee_range = excluded.melee_range,
            ranged_distance = excluded.ranged_distance,
            is_laser = excluded.is_laser,
            node_leash_range = excluded.node_leash_range,
            emits_node = excluded.emits_node,
            node_emit_radius = excluded.node_emit_radius,
            action_bonus = excluded.action_bonus,
            supply_range = excluded.supply_range,
            radar_range = excluded.radar_range,
            heal_repair_range = excluded.heal_repair_range,
            root_stacks = excluded.root_stacks,
            stealth = excluded.stealth,
            poison_stacks = excluded.poison_stacks,
            poison_immune = excluded.poison_immune,
            can_fly = excluded.can_fly,
            saboteur = excluded.saboteur,
            can_burrow = excluded.can_burrow,
            taunt_block = excluded.taunt_block,
            amphibious_tier = excluded.amphibious_tier,
            is_autonomous = excluded.is_autonomous,
            deck_points = excluded.deck_points,
            combat_ehp_tenths = excluded.combat_ehp_tenths,
            squad_model_count = excluded.squad_model_count,
            provenance_note = excluded.provenance_note,
            phylopic_taxon = excluded.phylopic_taxon,
            phylopic_search_url = excluded.phylopic_search_url,
            silhouette_path = excluded.silhouette_path",
        params![
            u.card_key,
            u.unit_name,
            format!("{:?}", u.origin),
            u.species,
            format!("{:?}", u.faction),
            format!("{:?}", u.combat_class),
            format!("{:?}", u.graveyard_queue),
            u.signature_ability,
            u32::from(u.strength_hp),
            u32::from(u.encumbrance),
            u32::from(u.armor),
            u32::from(u.shield),
            u32::from(u.movement),
            u32::from(u.evade),
            u32::from(u.melee_range),
            u32::from(u.ranged_distance),
            u32::from(u8::from(u.is_laser)),
            u32::from(u.node_leash_range),
            u32::from(u8::from(u.emits_node)),
            u32::from(u.node_emit_radius),
            u32::from(u.action_bonus),
            u32::from(u.supply_range),
            u32::from(u.radar_range),
            u32::from(u.heal_repair_range),
            u32::from(u.root_stacks),
            u32::from(u8::from(u.stealth)),
            u32::from(u.poison_stacks),
            u32::from(u8::from(u.poison_immune)),
            u32::from(u8::from(u.can_fly)),
            u32::from(u8::from(u.saboteur)),
            u32::from(u8::from(u.can_burrow)),
            u32::from(u.taunt_block),
            u32::from(u.amphibious_tier),
            u32::from(u8::from(u.is_autonomous)),
            u32::from(u.deck_points),
            u32::from(u.combat_ehp_tenths),
            u32::from(u.squad_model_count),
            u.provenance_note,
            genus,
            phylo_url,
            sil_path
        ],
    )?;
    Ok(())
}

pub fn delete_unit_from_sqlite(path: &Path, card_key: &str) -> SqlResult<()> {
    let conn = Connection::open(path)?;
    conn.execute("DELETE FROM units WHERE card_key = ?1", params![card_key])?;
    Ok(())
}
