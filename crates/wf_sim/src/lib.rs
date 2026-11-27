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

#![forbid(unsafe_code)]

//! Headless deterministic match simulator, 2-ply tactical lookahead AI planner,
//! action-stream replay verifier, and automated 102-match tournament benchmark suite
//! for *Wildfront Battlegrounds* (`v3.4.0`).

pub mod ai_planner;
pub mod sim_engine;
pub mod tournament;

pub use ai_planner::{
    compute_attack_ev, evaluate_board, select_best_action, AiDifficulty, AttackEvSummary,
};
pub use sim_engine::{
    HeadlessMatch, MatchReplayFile, RecordedActionStep, SimAction, SimEntity, TurnTelemetryPoint,
};
pub use tournament::{
    build_balance_scorecard, build_tournament_deck, export_balance_scorecard_csv,
    export_balance_scorecard_json, run_102_round_robin, scorecard_to_csv, scorecard_to_json,
    ArchetypeStanding, BalanceScorecard, ContinentBalanceRow, LineageBalanceRow, MatchSummaryRow,
    TournamentReport,
};
