//! `wf_assets` CLI: compiles `Wildfront_PC_Master_Stats_and_DeckBuilder.xlsx`,
//! exports `abilities_catalog.ron`, bakes `.vox` brickmaps, generates seeded
//! procedural scenario `.ron` maps, and optionally exports watertight 3D-printable
//! `.STL` miniatures with Recessed 45° Micro-Dice V-Cradles (`--export-stl`).

use std::fs;
use std::path::PathBuf;
use anyhow::Result;
use clap::Parser;
use ron::ser::{to_string_pretty, PrettyConfig};
use wf_assets::stl_exporter::{
    export_vox_directory_with_config, TabletopPrintConfig, DEFAULT_CENTER_WALL_MM,
    DEFAULT_HEX_FLAT_MM,
};
use wf_assets::vox_baker::bake_vox_directory;
use wf_assets::xlsx_compiler::compile_master_xlsx;
use wf_core::{generate_scenario, AbilityCatalog, OrganismFaction, ScenarioId};

#[derive(Parser, Debug)]
#[command(name = "compile_roster", about = "Compiles Wildfront Excel stats, abilities, .vox brickmaps, scenarios & 3D-print STLs")]
struct Cli {
    #[arg(long, default_value = "data/Wildfront_PC_Master_Stats_and_DeckBuilder.xlsx")]
    xlsx: PathBuf,
    #[arg(long, default_value = "assets/vox")]
    vox_dir: PathBuf,
    #[arg(long, default_value = "assets/compiled")]
    out_dir: PathBuf,
    #[arg(long, default_value_t = 2072_0814)]
    seed: u64,
    #[arg(long)]
    export_stl: bool,
    #[arg(long, default_value_t = DEFAULT_HEX_FLAT_MM)]
    hex_mm: f32,
    #[arg(long, default_value_t = DEFAULT_CENTER_WALL_MM)]
    center_wall_mm: f32,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    fs::create_dir_all(&cli.out_dir)?;
    fs::create_dir_all(cli.out_dir.join("factions"))?;
    fs::create_dir_all(cli.out_dir.join("scenarios"))?;
    fs::create_dir_all(cli.out_dir.join("vox"))?;

    let pretty = PrettyConfig::new().depth_limit(4).separate_tuple_members(true);

    // 1. Compile 152-card Master Roster & Starter Deck
    let (manifest, deck) = compile_master_xlsx(&cli.xlsx)?;
    fs::write(
        cli.out_dir.join("master_roster.ron"),
        to_string_pretty(&manifest, pretty.clone())?,
    )?;
    fs::write(
        cli.out_dir.join("starter_deck_plant_cnidarian.ron"),
        to_string_pretty(&deck, pretty.clone())?,
    )?;

    for fac in OrganismFaction::ALL {
        let subset: Vec<_> = manifest.units.iter().filter(|u| u.faction == fac).cloned().collect();
        fs::write(
            cli.out_dir.join("factions").join(format!("{}.ron", fac.slug())),
            to_string_pretty(&subset, pretty.clone())?,
        )?;
    }

    // 2. Export balanced Ability & Faction Synergy Catalog
    let abilities = AbilityCatalog::canonical();
    fs::write(
        cli.out_dir.join("abilities_catalog.ron"),
        to_string_pretty(&abilities, pretty.clone())?,
    )?;

    // 3. Generate all 5 procedural scenarios from `cli.seed`
    for (idx, scen_id) in ScenarioId::ALL.iter().enumerate() {
        let scen = generate_scenario(*scen_id, cli.seed.wrapping_add(idx as u64 * 1009));
        fs::write(
            cli.out_dir.join("scenarios").join(format!("scenario_{}.ron", idx + 1)),
            to_string_pretty(&scen, pretty.clone())?,
        )?;
    }

    // 4. Bake any `.vox` rigs present
    if cli.vox_dir.exists() {
        let baked = bake_vox_directory(&cli.vox_dir, &cli.out_dir.join("vox"))?;
        println!(
            "[wf_assets] Compiled {} balanced cards, {} abilities, 5 procedural scenarios, and {} .vox brickmaps.",
            manifest.units.len(),
            abilities.abilities.len(),
            baked
        );
    }

    // 5. Optional 3D-printable `.STL` batch export with Recessed 45° Micro-Dice V-Cradles
    if cli.export_stl {
        let stl_dir = cli.out_dir.join("stl");
        let cfg = TabletopPrintConfig {
            target_hex_flat_mm: cli.hex_mm,
            center_wall_mm: cli.center_wall_mm,
            ..TabletopPrintConfig::default()
        };
        let exported = export_vox_directory_with_config(&cli.vox_dir, &stl_dir, &cfg)?;
        println!(
            "[wf_assets] Exported {} watertight {:.1}mm-hex .stl meshes (center wall >= {:.2}mm) to {}",
            exported,
            cfg.target_hex_flat_mm,
            cfg.center_wall_mm,
            stl_dir.display()
        );
    }
    Ok(())
}
