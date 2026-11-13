//! Headless deterministic match simulator, 2-ply tactical lookahead AI planner,
//! action-stream replay verifier, and automated 102-match tournament benchmark suite
//! for *Wildfront Battlegrounds* (`v3.4.0`).

pub mod ai_planner;
pub mod sim_engine;
pub mod tournament;

pub use ai_planner::{evaluate_board, select_best_action, AiDifficulty};
pub use sim_engine::{
    HeadlessMatch, MatchReplayFile, RecordedActionStep, SimAction, SimEntity, TurnTelemetryPoint,
};
pub use tournament::{
    build_tournament_deck, run_102_round_robin, ArchetypeStanding, MatchSummaryRow,
    TournamentReport,
};
