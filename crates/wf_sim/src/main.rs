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

//! Command-line Headless Match Simulator, Replay Verifier & 102-Match Tournament Benchmark (`wf_sim`).

use std::fs;
use std::path::PathBuf;
use anyhow::Result;
use clap::{Parser, Subcommand};
use ron::ser::{to_string_pretty, PrettyConfig};
use wf_core::{OrganismFaction, OriginAllegiance, RosterManifest, ScenarioId};
use wf_sim::tournament::play_headless_match_to_end;
use wf_sim::{
    build_balance_scorecard, build_tournament_deck, export_balance_scorecard_csv,
    export_balance_scorecard_json, run_102_round_robin, AiDifficulty, HeadlessMatch,
};

#[derive(Parser)]
#[command(name = "wf_sim", version, about = "Wildfront Battlegrounds v3.4.1 Headless Sim, 2-Ply AI & Replay CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Simulate a single headless match to completion and optionally export a deterministic `.ron` replay
    Match {
        #[arg(long, default_value = "Australia")]
        p1_origin: String,
        #[arg(long, default_value = "Mammal")]
        p1_primary: String,
        #[arg(long, default_value = "Mammal")]
        p1_secondary: String,
        #[arg(long, default_value = "Africa")]
        p2_origin: String,
        #[arg(long, default_value = "Fish")]
        p2_primary: String,
        #[arg(long, default_value = "Fish")]
        p2_secondary: String,
        #[arg(long, default_value_t = 1)]
        scenario: u8,
        #[arg(long, default_value_t = 0x2072_0814)]
        seed: u64,
        #[arg(long, default_value = "grandmaster")]
        ai: String,
        #[arg(long)]
        save_replay: Option<PathBuf>,
    },
    /// Run the full 102-match symmetric Pure Bio-Type + Continental Round-Robin Tournament
    Tournament {
        #[arg(long, default_value = "veteran")]
        ai: String,
        #[arg(long, default_value_t = 45)]
        max_rounds: u32,
        #[arg(long)]
        export_ron: Option<PathBuf>,
        #[arg(long)]
        export_scorecard_json: Option<PathBuf>,
        #[arg(long)]
        export_scorecard_csv: Option<PathBuf>,
    },
    /// Export faction & lineage EHP/win-rate balance analytics scorecard (JSON / CSV)
    Scorecard {
        #[arg(long, default_value = "veteran")]
        ai: String,
        #[arg(long, default_value_t = 45)]
        max_rounds: u32,
        #[arg(long, default_value_t = false)]
        roster_only: bool,
        #[arg(long)]
        export_json: Option<PathBuf>,
        #[arg(long)]
        export_csv: Option<PathBuf>,
    },
    /// Deterministically re-simulate and verify every turn state hash of a saved `.ron` replay
    VerifyReplay {
        replay_path: PathBuf,
    },
}

fn parse_ai(label: &str) -> AiDifficulty {
    match label.to_ascii_lowercase().as_str() {
        "recruit" | "easy" => AiDifficulty::Recruit,
        "veteran" | "normal" => AiDifficulty::Veteran,
        _ => AiDifficulty::Grandmaster2Ply,
    }
}

fn load_roster() -> Result<RosterManifest> {
    let db_path = PathBuf::from(wf_core::sqlite_db::DEFAULT_SQLITE_DB_PATH);
    if db_path.exists() {
        if let Ok(r) = wf_core::sqlite_db::load_roster_from_sqlite(&db_path) {
            if !r.units.is_empty() {
                return Ok(r);
            }
        }
    }
    let raw = fs::read_to_string("assets/compiled/master_roster.ron")?;
    Ok(ron::from_str(&raw)?)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let roster = load_roster()?;

    match cli.command {
        Commands::Match {
            p1_origin,
            p1_primary,
            p1_secondary,
            p2_origin,
            p2_primary,
            p2_secondary,
            scenario,
            seed,
            ai,
            save_replay,
        } => {
            let o1 = OriginAllegiance::from_label(&p1_origin)?;
            let b1a = OrganismFaction::from_label(&p1_primary)?;
            let b1b = OrganismFaction::from_label(&p1_secondary)?;
            let o2 = OriginAllegiance::from_label(&p2_origin)?;
            let b2a = OrganismFaction::from_label(&p2_primary)?;
            let b2b = OrganismFaction::from_label(&p2_secondary)?;
            let diff = parse_ai(&ai);
            let scen = ScenarioId::from_index(scenario);

            let d1 = build_tournament_deck(&roster, o1, b1a, b1b);
            let d2 = build_tournament_deck(&roster, o2, b2a, b2b);

            let m = play_headless_match_to_end(
                &roster,
                d1,
                d2,
                scen,
                seed,
                diff,
                50,
                save_replay.is_some(),
            );
            println!(
                "[wf_sim] {} | Winner: P{:?} in {} turns — {} (Final State Hash: 0x{:016X})",
                scen.title(),
                m.winner_team,
                m.turn_number,
                m.win_reason,
                m.compute_state_hash()
            );
            if let Some(rp) = save_replay {
                m.save_replay_ron(&rp)?;
                println!("[wf_sim] Saved deterministic replay ({} steps) to {}", m.steps.len(), rp.display());
            }
        }
        Commands::Tournament {
            ai,
            max_rounds,
            export_ron,
            export_scorecard_json,
            export_scorecard_csv,
        } => {
            let diff = parse_ai(&ai);
            let report = run_102_round_robin(&roster, diff, max_rounds);
            println!(
                "=== WILDFRONT v3.4.1 102-MATCH TOURNAMENT REPORT ({}) ===",
                report.ai_difficulty
            );
            println!(
                "Decisive: {}/{} | P1 Wins: {} | P2 Wins: {} | Median Turns: {:.1}",
                report.decisive_matches,
                report.total_matches,
                report.p1_wins,
                report.p2_wins,
                report.median_turns
            );
            for st in &report.standings {
                println!(
                    "  [{:<13}] {:<18} {:>2}/{:<2} ({:>5.1}%)  Avg {:.1}t",
                    st.category, st.name, st.wins, st.games, st.win_rate_pct, st.avg_turns
                );
            }
            if let Some(out_p) = export_ron {
                if let Some(parent) = out_p.parent() {
                    fs::create_dir_all(parent)?;
                }
                let raw = to_string_pretty(&report, PrettyConfig::new().depth_limit(4))?;
                fs::write(&out_p, raw)?;
                println!("[wf_sim] Exported tournament report to {}", out_p.display());
            }
            if export_scorecard_json.is_some() || export_scorecard_csv.is_some() {
                let sc = build_balance_scorecard(&roster, Some(&report));
                if let Some(ref jp) = export_scorecard_json {
                    export_balance_scorecard_json(jp, &sc).map_err(|e| anyhow::anyhow!(e))?;
                    println!("[wf_sim] Exported balance scorecard JSON to {}", jp.display());
                }
                if let Some(ref cp) = export_scorecard_csv {
                    export_balance_scorecard_csv(cp, &sc).map_err(|e| anyhow::anyhow!(e))?;
                    println!("[wf_sim] Exported balance scorecard CSV to {}", cp.display());
                }
            }
        }
        Commands::Scorecard {
            ai,
            max_rounds,
            roster_only,
            export_json,
            export_csv,
        } => {
            let report = if roster_only {
                None
            } else {
                Some(run_102_round_robin(&roster, parse_ai(&ai), max_rounds))
            };
            let sc = build_balance_scorecard(&roster, report.as_ref());
            println!(
                "=== WILDFRONT v3.4.1 FACTION & LINEAGE BALANCE SCORECARD ({} cards, {}) ===",
                sc.total_cards, sc.ai_difficulty
            );
            for r in &sc.lineages {
                println!(
                    "  [Lineage {:<10}] cards={:>2} avg_pts={:>5.1} avg_EHP={:>5.1} ({:.2}/pt) W={:>2}/{} ({:>5.1}%)",
                    r.lineage, r.card_count, r.avg_deck_points, r.avg_ehp, r.avg_ehp_per_point, r.wins, r.matches_played, r.win_rate_pct
                );
            }
            if let Some(ref jp) = export_json {
                export_balance_scorecard_json(jp, &sc).map_err(|e| anyhow::anyhow!(e))?;
                println!("[wf_sim] Exported balance scorecard JSON to {}", jp.display());
            }
            if let Some(ref cp) = export_csv {
                export_balance_scorecard_csv(cp, &sc).map_err(|e| anyhow::anyhow!(e))?;
                println!("[wf_sim] Exported balance scorecard CSV to {}", cp.display());
            }
        }
        Commands::VerifyReplay { replay_path } => {
            let verified = HeadlessMatch::verify_replay_file(roster, &replay_path)?;
            println!(
                "[wf_sim] VERIFIED OK: {} steps across {} turns matched bit-for-bit (Seed 0x{:08X}, Winner P{:?}).",
                verified.steps.len(),
                verified.total_turns,
                verified.seed,
                verified.winner_team
            );
        }
    }
    Ok(())
}
