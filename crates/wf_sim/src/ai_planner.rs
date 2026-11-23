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
use wf_core::combat::{resolve_attack, CombatContext};
use wf_core::logistics::BoardUnit;
use wf_core::mapgen::SplitMix64;
use wf_core::{CombatClass, OrganismFaction, OriginAllegiance, UnitCardDef};
use crate::sim_engine::{HeadlessMatch, SimAction};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttackEvSummary {
    pub hit_probability: f64,
    pub expected_armor_absorbed: f64,
    pub expected_str_damage: f64,
    pub expected_total_damage: f64,
    pub kill_probability: f64,
    pub expected_retal_armor_absorbed: f64,
    pub expected_retal_str_damage: f64,
    pub expected_fortify_consumed: f64,
    pub expected_poison_added: f64,
    pub expected_root_probability: f64,
}

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

fn hq_and_focal_hexes(m: &HeadlessMatch, perspective: u8) -> (Hex, Hex, Hex) {
    let opp = if perspective == 1 { 2 } else { 1 };
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
    (my_hq_hex, opp_hq_hex, focal)
}

#[allow(clippy::too_many_arguments)]
fn entity_eval_score(
    team_id: u8,
    hex: Hex,
    card: &UnitCardDef,
    unit: &BoardUnit,
    perspective: u8,
    my_hq_hex: Hex,
    opp_hq_hex: Hex,
    focal: Hex,
) -> i32 {
    if unit.current_str_hp == 0 {
        return 0;
    }
    let sign = if team_id == perspective { 1 } else { -1 };
    if card.combat_class == CombatClass::Installation {
        let hq_val = i32::from(unit.current_str_hp) * 210
            + i32::from(unit.current_armor) * 95
            + i32::from(unit.fortified_stacks) * 85;
        sign * hq_val
    } else {
        let mut u_val = i32::from(unit.current_str_hp) * 28
            + i32::from(unit.current_armor) * 18
            + i32::from(unit.fortified_stacks) * 24
            + i32::from(card.deck_points) * 2;
        if unit.emits_node {
            u_val += 26;
        }
        if unit.confused {
            u_val -= 24;
        }
        let target_hq = if team_id == perspective { opp_hq_hex } else { my_hq_hex };
        let dist_hq = hex.unsigned_distance_to(target_hq) as i32;
        let dist_obj = hex.unsigned_distance_to(focal) as i32;
        u_val += (12 - dist_hq.min(12)) * 8 + (10 - dist_obj.min(10)) * 6;
        sign * u_val
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
    let (my_hq_hex, opp_hq_hex, focal) = hq_and_focal_hexes(m, perspective);

    for e in &m.entities {
        score += entity_eval_score(
            e.team_id,
            e.hex,
            &e.card,
            &e.unit,
            perspective,
            my_hq_hex,
            opp_hq_hex,
            focal,
        );
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

/// Computes exact analytical combat Expected Value across all 36 `(hit_d6, dmg_d6)` faces
/// (and 6 `ret_d6` faces when melee retaliation triggers).
pub fn compute_attack_ev(
    base: &HeadlessMatch,
    attacker_uid: u64,
    defender_uid: u64,
) -> Option<AttackEvSummary> {
    compute_attack_ev_with_eval(base, attacker_uid, defender_uid, base.active_team)
        .map(|(summary, _)| summary)
}

fn compute_attack_ev_with_eval(
    base: &HeadlessMatch,
    attacker_uid: u64,
    defender_uid: u64,
    perspective: u8,
) -> Option<(AttackEvSummary, f64)> {
    if base.winner_team.is_some() {
        return None;
    }
    let att_i = base.entity_by_uid(attacker_uid)?;
    let def_i = base.entity_by_uid(defender_uid)?;
    if att_i == def_i {
        return None;
    }
    let att = &base.entities[att_i];
    let def = &base.entities[def_i];
    let actor = att.team_id;
    let dist = att.hex.unsigned_distance_to(def.hex) as u8;
    let is_melee = dist <= att.card.melee_range;

    let in_cover = base
        .scenario
        .map
        .tiles
        .get(&def.hex)
        .map_or(false, |t| t.morphology.grants_ranged_cover())
        && !matches!(att.card.faction, OrganismFaction::Arachnid | OrganismFaction::Insect);
    let adj_cmd = base.entities.iter().any(|e| {
        e.team_id == actor
            && e.unit.current_str_hp > 0
            && e.card.combat_class == CombatClass::Commander
            && e.hex.unsigned_distance_to(att.hex) == 1
    });
    let pack_bonus = att.card.faction == OrganismFaction::Mammal
        && base.entities.iter().any(|e| {
            e.team_id == actor
                && e.uid != attacker_uid
                && e.unit.current_str_hp > 0
                && e.card.faction == OrganismFaction::Mammal
                && e.hex.unsigned_distance_to(att.hex) == 1
        });

    let ctx = CombatContext {
        hex_distance: dist,
        is_melee,
        attacker_can_fly: att.card.can_fly,
        attacker_stealth_ambush: att.card.stealth,
        attacker_adjacent_commander: adj_cmd,
        attacker_pack_tactics_bonus: pack_bonus,
        defender_in_rough_cover: in_cover,
    };

    let (my_hq_hex, opp_hq_hex, focal) = hq_and_focal_hexes(base, perspective);
    let base_eval = f64::from(evaluate_board(base, perspective));
    let att_before = entity_eval_score(
        att.team_id,
        att.hex,
        &att.card,
        &att.unit,
        perspective,
        my_hq_hex,
        opp_hq_hex,
        focal,
    );
    let def_before = entity_eval_score(
        def.team_id,
        def.hex,
        &def.card,
        &def.unit,
        perspective,
        my_hq_hex,
        opp_hq_hex,
        focal,
    );

    let m_cost = u8::from(!att.unit.is_laser);
    let home_mun = base.economy.home_base_munitions.get(&actor).copied().unwrap_or(0);
    let mun_delta = if m_cost > 0
        && base.team_supply_hexes(actor).contains(&att.hex)
        && home_mun >= u16::from(m_cost)
    {
        if actor == perspective {
            -4.0 * f64::from(m_cost)
        } else {
            4.0 * f64::from(m_cost)
        }
    } else {
        0.0
    };

    let mut att_tpl = att.unit.clone();
    att_tpl.card_key = String::new();
    let mut def_tpl = def.unit.clone();
    def_tpl.card_key = String::new();

    let na_laser_bonus = base.decks[(actor.saturating_sub(1)).min(1) as usize].faction
        == OriginAllegiance::NorthAmerica;
    let turn_pen = f64::from(base.turn_number) * 50.0;

    let mut hit_prob = 0.0_f64;
    let mut exp_arm_abs = 0.0_f64;
    let mut exp_str_dmg = 0.0_f64;
    let mut kill_prob = 0.0_f64;
    let mut exp_retal_arm = 0.0_f64;
    let mut exp_retal_str = 0.0_f64;
    let mut exp_fort_cons = 0.0_f64;
    let mut exp_poison_add = 0.0_f64;
    let mut exp_root_prob = 0.0_f64;
    let mut exp_board_score = 0.0_f64;

    for hit_d6 in 1u8..=6 {
        for dmg_d6 in 1u8..=6 {
            let mut a_u = att_tpl.clone();
            let mut d_u = def_tpl.clone();
            let out = resolve_attack(&mut a_u, &mut d_u, ctx, hit_d6, dmg_d6);

            if out.hit_succeeded && a_u.is_laser && hit_d6 == 6 && na_laser_bonus {
                d_u.current_str_hp = d_u.current_str_hp.saturating_sub(1);
            }
            if out.hit_succeeded && att.card.poison_stacks > 0 && !def.card.poison_immune {
                d_u.poisoned_stacks = (d_u.poisoned_stacks + att.card.poison_stacks).min(3);
            }
            if out.hit_succeeded && att.card.root_stacks > 0 {
                d_u.rooted_turns = d_u.rooted_turns.max(1);
            }

            let str_lost = f64::from(def_tpl.current_str_hp.saturating_sub(d_u.current_str_hp));
            let arm_abs = f64::from(out.armor_absorbed);
            let fort_cons = if def_tpl.fortified_stacks > d_u.fortified_stacks { 1.0 } else { 0.0 };
            let pois_add = f64::from(d_u.poisoned_stacks.saturating_sub(def_tpl.poisoned_stacks));
            let root_hit = if out.hit_succeeded && att.card.root_stacks > 0 { 1.0 } else { 0.0 };
            let def_ko = if d_u.current_str_hp == 0 { 1.0 } else { 0.0 };
            let hit_ok = if out.hit_succeeded { 1.0 } else { 0.0 };

            let def_after = entity_eval_score(
                def.team_id,
                def.hex,
                &def.card,
                &d_u,
                perspective,
                my_hq_hex,
                opp_hq_hex,
                focal,
            );

            if out.triggers_melee_retaliation && d_u.current_str_hp > 0 {
                let w = 1.0 / 216.0;
                for ret_d6 in 1u8..=6 {
                    let mut a_r = a_u.clone();
                    let ret_raw = ret_d6.min(d_u.current_str_hp);
                    let abs_r = ret_raw.min(a_r.current_armor);
                    a_r.current_armor -= abs_r;
                    let ret_str_lost = (ret_raw - abs_r).min(a_r.current_str_hp);
                    a_r.current_str_hp -= ret_str_lost;

                    hit_prob += w * hit_ok;
                    exp_arm_abs += w * arm_abs;
                    exp_str_dmg += w * str_lost;
                    kill_prob += w * def_ko;
                    exp_fort_cons += w * fort_cons;
                    exp_poison_add += w * pois_add;
                    exp_root_prob += w * root_hit;
                    exp_retal_arm += w * f64::from(abs_r);
                    exp_retal_str += w * f64::from(ret_str_lost);

                    let branch_eval = if def.card.combat_class == CombatClass::Installation
                        && d_u.current_str_hp == 0
                    {
                        if actor == perspective { 100_000.0 - turn_pen } else { -100_000.0 + turn_pen }
                    } else if att.card.combat_class == CombatClass::Installation
                        && a_r.current_str_hp == 0
                    {
                        if def.team_id == perspective { 100_000.0 - turn_pen } else { -100_000.0 + turn_pen }
                    } else {
                        let att_after = entity_eval_score(
                            att.team_id,
                            att.hex,
                            &att.card,
                            &a_r,
                            perspective,
                            my_hq_hex,
                            opp_hq_hex,
                            focal,
                        );
                        base_eval
                            + mun_delta
                            + f64::from((att_after - att_before) + (def_after - def_before))
                    };
                    exp_board_score += w * branch_eval;
                }
            } else {
                let w = 1.0 / 36.0;
                hit_prob += w * hit_ok;
                exp_arm_abs += w * arm_abs;
                exp_str_dmg += w * str_lost;
                kill_prob += w * def_ko;
                exp_fort_cons += w * fort_cons;
                exp_poison_add += w * pois_add;
                exp_root_prob += w * root_hit;

                let branch_eval = if def.card.combat_class == CombatClass::Installation
                    && d_u.current_str_hp == 0
                {
                    if actor == perspective { 100_000.0 - turn_pen } else { -100_000.0 + turn_pen }
                } else {
                    let att_after = entity_eval_score(
                        att.team_id,
                        att.hex,
                        &att.card,
                        &a_u,
                        perspective,
                        my_hq_hex,
                        opp_hq_hex,
                        focal,
                    );
                    base_eval
                        + mun_delta
                        + f64::from((att_after - att_before) + (def_after - def_before))
                };
                exp_board_score += w * branch_eval;
            }
        }
    }

    Some((
        AttackEvSummary {
            hit_probability: hit_prob,
            expected_armor_absorbed: exp_arm_abs,
            expected_str_damage: exp_str_dmg,
            expected_total_damage: exp_arm_abs + exp_str_dmg,
            kill_probability: kill_prob,
            expected_retal_armor_absorbed: exp_retal_arm,
            expected_retal_str_damage: exp_retal_str,
            expected_fortify_consumed: exp_fort_cons,
            expected_poison_added: exp_poison_add,
            expected_root_probability: exp_root_prob,
        },
        exp_board_score,
    ))
}

fn simulate_deterministic_preview(base: &HeadlessMatch, act: &SimAction) -> HeadlessMatch {
    let mut branch = base.clone();
    branch.record_replay = false;
    if let SimAction::AttackUnit {
        attacker_uid,
        defender_uid,
    } = act
    {
        let actor = branch.active_team;
        let ap = branch.economy.actions_remaining.get(&actor).copied().unwrap_or(0);
        let (Some(att_i), Some(def_i)) = (
            branch.entity_by_uid(*attacker_uid),
            branch.entity_by_uid(*defender_uid),
        ) else {
            return branch;
        };
        if ap == 0 || att_i == def_i || branch.entities[att_i].team_id != actor {
            return branch;
        }
        let m_cost = u8::from(!branch.entities[att_i].unit.is_laser);
        if !branch.spend_munition_for_unit(att_i, m_cost) {
            return branch;
        }
        *branch.economy.actions_remaining.entry(actor).or_insert(1) -= 1;
        if branch.entities[att_i].unit.is_laser {
            branch.entities[att_i].unit.laser_fired_this_turn = true;
        }

        if let Some(ev) = compute_attack_ev(base, *attacker_uid, *defender_uid) {
            if ev.expected_fortify_consumed >= 0.5 {
                let d_u = &mut branch.entities[def_i].unit;
                d_u.fortified_stacks = d_u.fortified_stacks.saturating_sub(1);
            } else {
                let arm_abs = (ev.expected_armor_absorbed.round() as u8)
                    .min(branch.entities[def_i].unit.current_armor);
                let str_loss = (ev.expected_str_damage.round() as u8)
                    .min(branch.entities[def_i].unit.current_str_hp);
                branch.entities[def_i].unit.current_armor -= arm_abs;
                branch.entities[def_i].unit.current_str_hp -= str_loss;

                if ev.expected_poison_added >= 0.5 {
                    let add_p = ev.expected_poison_added.round() as u8;
                    let d_u = &mut branch.entities[def_i].unit;
                    d_u.poisoned_stacks = (d_u.poisoned_stacks + add_p).min(3);
                }
                if ev.expected_root_probability >= 0.5 {
                    let d_u = &mut branch.entities[def_i].unit;
                    d_u.rooted_turns = d_u.rooted_turns.max(1);
                }
                let ret_arm = (ev.expected_retal_armor_absorbed.round() as u8)
                    .min(branch.entities[att_i].unit.current_armor);
                let ret_str = (ev.expected_retal_str_damage.round() as u8)
                    .min(branch.entities[att_i].unit.current_str_hp);
                branch.entities[att_i].unit.current_armor -= ret_arm;
                branch.entities[att_i].unit.current_str_hp -= ret_str;
            }
        }
        branch.enqueue_destroyed_units();
        branch.refresh_confusion_states();
        branch.check_immediate_hq_victory();
        return branch;
    }

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

    // Score all Ply-1 candidates using exact analytical D6 EV (millipoints preserve fractional ties)
    let mut scored: Vec<(i64, SimAction, HeadlessMatch)> = Vec::with_capacity(candidates.len());
    for act in candidates {
        if matches!(act, SimAction::EndTurn) {
            scored.push((i64::from(base_eval - 6) * 1_000, act, m.clone()));
            continue;
        }
        let next_state = simulate_deterministic_preview(m, &act);
        let mut score_milli = match &act {
            SimAction::AttackUnit {
                attacker_uid,
                defender_uid,
            } => {
                if let Some((_, ev_score)) =
                    compute_attack_ev_with_eval(m, *attacker_uid, *defender_uid, me)
                {
                    (ev_score * 1_000.0).round() as i64
                } else {
                    i64::from(evaluate_board(&next_state, me)) * 1_000
                }
            }
            _ => i64::from(evaluate_board(&next_state, me)) * 1_000,
        };
        if difficulty == AiDifficulty::Recruit {
            let jitter = ((noise_rng.next_u64() % 55) as i64) - 27;
            score_milli += jitter * 1_000;
        }
        scored.push((score_milli, act, next_state));
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0));

    if difficulty != AiDifficulty::Grandmaster2Ply {
        return scored.into_iter().next().map(|x| x.1).unwrap_or(SimAction::EndTurn);
    }

    // Grandmaster 2-Ply Beam Search:
    // Inspect top 6 Ply-1 actions; roll forward one follow-up step (or opponent counter-action)
    // and choose the line maximizing minimax post-response advantage!
    let mut best_act = scored[0].1.clone();
    let mut best_2ply = i64::MIN;

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
            i64::from(evaluate_board(&branch, me)) * 1_000
        };

        if val_2ply > best_2ply {
            best_2ply = val_2ply;
            best_act = act;
        }
    }

    best_act
}
