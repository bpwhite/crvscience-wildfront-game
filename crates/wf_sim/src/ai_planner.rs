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

//! Multi-Difficulty Tactical AI Planner (`Recruit`, `Veteran`, `Grandmaster2Ply`)
//! with 2-Ply Alpha-Beta Beam Search over cloned `HeadlessMatch` states (`v3.4.0`).

use hexx::Hex;
use serde::{Deserialize, Serialize};
use wf_core::mapgen::SplitMix64;
use wf_core::CombatClass;
use crate::sim_engine::{HeadlessMatch, SimAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiDifficulty {
    Recruit,
    Veteran,
    Grandmaster2Ply,
}

impl Default for AiDifficulty {
    fn default() -> Self {
        Self::Grandmaster2Ply
    }
}

impl AiDifficulty {
    pub const ALL: [Self; 3] = [Self::Recruit, Self::Veteran, Self::Grandmaster2Ply];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Recruit => "Recruit (1-Ply Casual)",
            Self::Veteran => "Veteran (1-Ply EV Focus)",
            Self::Grandmaster2Ply => "Grandmaster (2-Ply Lookahead)",
        }
    }
}

/// Evaluates board state strictly from `perspective` (`1` or `2`).
pub fn evaluate_board(m: &HeadlessMatch, perspective: u8) -> i32 {
    if let Some(w) = m.winner_team {
        return if w == perspective {
            100_000 - (m.turn_number as i32 * 50)
        } else {
            -100_000 + (m.turn_number as i32 * 50)
        };
    }

    let opp = if perspective == 1 { 2 } else { 1 };
    let mut score: i32 = 0;

    let mut my_hq_hex = Hex::ZERO;
    let mut opp_hq_hex = Hex::ZERO;
    for e in &m.entities {
        if e.unit.current_str_hp == 0 {
            continue;
        }
        if e.card.combat_class == CombatClass::Installation {
            if e.team_id == perspective {
                my_hq_hex = e.hex;
            } else if e.team_id == opp {
                opp_hq_hex = e.hex;
            }
        }
    }

    let focal = m
        .scenario
        .objective_hexes
        .first()
        .map(|&(q, r)| Hex::new(q, r))
        .unwrap_or(opp_hq_hex);

    for e in &m.entities {
        if e.unit.current_str_hp == 0 {
            continue;
        }
        let sign = if e.team_id == perspective { 1 } else { -1 };
        if e.card.combat_class == CombatClass::Installation {
            let hq_val = i32::from(e.unit.current_str_hp) * 210
                + i32::from(e.unit.current_armor) * 95
                + i32::from(e.unit.fortified_stacks) * 85;
            score += sign * hq_val;
        } else {
            let mut u_val = i32::from(e.unit.current_str_hp) * 28
                + i32::from(e.unit.current_armor) * 18
                + i32::from(e.unit.fortified_stacks) * 24
                + i32::from(e.card.deck_points) * 2;
            if e.unit.emits_node {
                u_val += 26;
            }
            if e.unit.confused {
                u_val -= 24;
            }
            let target_hq = if e.team_id == perspective { opp_hq_hex } else { my_hq_hex };
            let dist_hq = e.hex.unsigned_distance_to(target_hq) as i32;
            let dist_obj = e.hex.unsigned_distance_to(focal) as i32;
            u_val += (12 - dist_hq.min(12)) * 8 + (10 - dist_obj.min(10)) * 6;
            score += sign * u_val;
        }
    }

    // Objective progress & Munitions economy advantage
    let my_obj = i32::from(m.obj_progress[(perspective - 1) as usize]);
    let opp_obj = i32::from(m.obj_progress[(opp - 1) as usize]);
    score += (my_obj - opp_obj) * 165;

    let my_mun = i32::from(m.economy.home_base_munitions.get(&perspective).copied().unwrap_or(0));
    let opp_mun = i32::from(m.economy.home_base_munitions.get(&opp).copied().unwrap_or(0));
    score += (my_mun - opp_mun) * 4;

    score
}

fn simulate_deterministic_preview(base: &HeadlessMatch, act: &SimAction) -> HeadlessMatch {
    let mut branch = base.clone();
    branch.record_replay = false;
    // Use fixed mid-roll seed on lookahead previews so candidate ranking is variance-free
    branch.dice_rng = SplitMix64::new(base.compute_state_hash() ^ 0xA1B2_C3D4);
    let _ = branch.apply_action(act.clone());
    branch
}

/// Selects the strongest tactical `SimAction` for `m.active_team` under `difficulty`.
pub fn select_best_action(
    m: &HeadlessMatch,
    difficulty: AiDifficulty,
    noise_rng: &mut SplitMix64,
) -> SimAction {
    let candidates = m.legal_actions();
    if candidates.len() <= 1 {
        return candidates.into_iter().next().unwrap_or(SimAction::EndTurn);
    }

    let me = m.active_team;
    let base_eval = evaluate_board(m, me);

    // Score all Ply-1 candidates
    let mut scored: Vec<(i32, SimAction, HeadlessMatch)> = Vec::with_capacity(candidates.len());
    for act in candidates {
        if matches!(act, SimAction::EndTurn) {
            scored.push((base_eval - 6, act, m.clone()));
            continue;
        }
        let next_state = simulate_deterministic_preview(m, &act);
        let mut s = evaluate_board(&next_state, me);
        if difficulty == AiDifficulty::Recruit {
            let jitter = ((noise_rng.next_u64() % 55) as i32) - 27;
            s += jitter;
        }
        scored.push((s, act, next_state));
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0));

    if difficulty != AiDifficulty::Grandmaster2Ply {
        return scored.into_iter().next().map(|x| x.1).unwrap_or(SimAction::EndTurn);
    }

    // Grandmaster 2-Ply Beam Search:
    // Inspect top 6 Ply-1 actions; roll forward one follow-up step (or opponent counter-action)
    // and choose the line maximizing minimax post-response advantage!
    let mut best_act = scored[0].1.clone();
    let mut best_2ply = i32::MIN;

    for (ply1_score, act, mut branch) in scored.into_iter().take(6) {
        if branch.winner_team == Some(me) {
            return act; // Immediate lethal win found!
        }
        let val_2ply = if matches!(act, SimAction::EndTurn) {
            ply1_score
        } else {
            // If we still have AP, take our best follow-up; then let opponent take their best counter-strike!
            if branch.active_team == me
                && branch.economy.actions_remaining.get(&me).copied().unwrap_or(0) > 0
            {
                let follow = select_best_action(&branch, AiDifficulty::Veteran, noise_rng);
                branch = simulate_deterministic_preview(&branch, &follow);
            }
            if branch.winner_team.is_none() && branch.active_team == me {
                let _ = branch.apply_action(SimAction::EndTurn);
            }
            if branch.winner_team.is_none() && branch.active_team != me {
                let opp_best = select_best_action(&branch, AiDifficulty::Veteran, noise_rng);
                branch = simulate_deterministic_preview(&branch, &opp_best);
            }
            evaluate_board(&branch, me)
        };

        if val_2ply > best_2ply {
            best_2ply = val_2ply;
            best_act = act;
        }
    }

    best_act
}
