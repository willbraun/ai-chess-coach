use std::collections::HashSet;

use serde::Serialize;
use shakmaty::{san::SanPlus, uci::UciMove, Chess, Position};

use crate::moves::PvLine;
use crate::strategy::{analyze_all_strategy, analyze_material};
use crate::tactics::{detect_all_tactics, detect_discovered_attack, get_attacked_pieces, Finding, POSITIONAL};

/// Full position report combining tactical and strategic analysis.
#[derive(Serialize, Clone)]
pub struct PositionReport {
    /// Material balance summary (e.g. "White is up a knight (+3.00)")
    pub material: String,
    /// Tactical findings — truncated by priority tier for the frontend
    pub tactics: Vec<Finding>,
    /// Strategic findings — truncated by priority tier for the frontend
    pub strategy: Vec<Finding>,
    /// One-line summary
    pub summary: String,
    /// Full (untruncated) tactics list — used for checkpoint diffing, not serialized
    #[serde(skip)]
    pub tactics_full: Vec<Finding>,
    /// Full (untruncated) strategy list — used for checkpoint diffing, not serialized
    #[serde(skip)]
    pub strategy_full: Vec<Finding>,
}

/// A snapshot of tactics/strategy changes at a checkpoint along a PV line.
/// Only items that differ from the previous checkpoint (or the original position
/// for the first checkpoint) are included.
#[derive(Serialize, Clone)]
pub struct Checkpoint {
    /// How many half-moves into the line this checkpoint is
    pub half_move: usize,
    /// The move (in SAN notation) that led to this checkpoint position
    pub move_san: String,
    /// Material balance at this checkpoint
    pub material: String,
    /// Tactical patterns that appeared since the previous checkpoint
    pub new_tactics: Vec<Finding>,
    /// Tactical patterns that disappeared since the previous checkpoint
    pub removed_tactics: Vec<Finding>,
    /// Strategic observations that appeared since the previous checkpoint
    pub new_strategy: Vec<Finding>,
    /// Strategic observations that disappeared since the previous checkpoint
    pub removed_strategy: Vec<Finding>,
    /// Pieces that became newly attacked since the previous checkpoint
    pub newly_attacked: Vec<Finding>,
}

/// Side-by-side comparison of the engine line vs the user line at checkpoints.
#[derive(Serialize, Clone)]
pub struct LineComparison {
    pub engine_checkpoints: Vec<Checkpoint>,
    pub user_checkpoints: Vec<Checkpoint>,
}

/// Analyzes a position for all tactical and strategic features.
pub fn analyze_position_features(pos: &Chess) -> PositionReport {
    let material = analyze_material(pos);
    let tactics_full = detect_all_tactics(pos);
    let strategy_full = analyze_all_strategy(pos);

    let summary = build_summary(&material, &tactics_full, &strategy_full);

    PositionReport {
        material,
        tactics: truncate_findings(&tactics_full),
        strategy: truncate_findings(&strategy_full),
        summary,
        tactics_full,
        strategy_full,
    }
}

/// Builds a one-line summary from the analysis components.
fn build_summary(material: &str, tactics: &[Finding], strategy: &[Finding]) -> String {
    let mut parts = vec![material.to_string()];

    if !tactics.is_empty() {
        parts.push(format!("{} tactical pattern(s) found", tactics.len()));
    }

    let pawn_issues: usize = strategy
        .iter()
        .filter(|f| f.text.contains("doubled") || f.text.contains("isolated"))
        .count();
    if pawn_issues > 0 {
        parts.push(format!("{} pawn structure issue(s)", pawn_issues));
    }

    parts.join(". ")
}

/// Computes items in `current` that are not in `previous` (compared by text).
fn vec_added(previous: &[Finding], current: &[Finding]) -> Vec<Finding> {
    let prev_set: HashSet<&str> = previous.iter().map(|f| f.text.as_str()).collect();
    current.iter().filter(|f| !prev_set.contains(f.text.as_str())).cloned().collect()
}

/// Computes items in `previous` that are not in `current` (compared by text).
fn vec_removed(previous: &[Finding], current: &[Finding]) -> Vec<Finding> {
    let curr_set: HashSet<&str> = current.iter().map(|f| f.text.as_str()).collect();
    previous.iter().filter(|f| !curr_set.contains(f.text.as_str())).cloned().collect()
}

/// Walks a PV line, analyzing tactics/strategy diffs at each half-move checkpoint.
/// Walks at least `min_moves`, then continues until material stabilizes.
fn analyze_checkpoints(
    pos: &Chess,
    uci_moves: &[String],
    base_tactics: &[Finding],
    base_strategy: &[Finding],
    min_moves: usize,
) -> Vec<Checkpoint> {
    let mut checkpoints = Vec::new();
    let mut current = pos.clone();

    // Track the previous checkpoint's full analysis for diffing
    let mut prev_tactics: Vec<Finding> = base_tactics.to_vec();
    let mut prev_strategy: Vec<Finding> = base_strategy.to_vec();
    let mut prev_material: Option<String> = None;
    let mut prev_attacked: Vec<Finding> = {
        let mut v = get_attacked_pieces(pos.board(), shakmaty::Color::White);
        v.extend(get_attacked_pieces(pos.board(), shakmaty::Color::Black));
        v
    };

    for (i, uci_str) in uci_moves.iter().enumerate() {
        let half_move = i + 1;

        let uci = match uci_str.parse::<UciMove>() {
            Ok(u) => u,
            Err(_) => break,
        };

        // Capture from_sq and mover color before playing, for discovered attack detection
        let from_sq_opt = match &uci {
            UciMove::Normal { from, .. } => Some(*from),
            _ => None,
        };
        let mover_color = current.turn();
        let board_before = current.board().clone();

        let m = match uci.to_move(&current) {
            Ok(m) => m,
            Err(_) => break,
        };
        let move_san = SanPlus::from_move_and_play_unchecked(&mut current, m).to_string();

        let material = analyze_material(&current);
        let tactics = detect_all_tactics(&current);
        let strategy = analyze_all_strategy(&current);
        let mut curr_attacked = get_attacked_pieces(current.board(), shakmaty::Color::White);
        curr_attacked.extend(get_attacked_pieces(current.board(), shakmaty::Color::Black));
        let newly_attacked = vec_added(&prev_attacked, &curr_attacked);

        // Diff the truncated (shown) sets — this ensures a finding only appears
        // as "new" or "removed" if it was actually visible to the user, not merely
        // present in the full list while a higher-priority tier dominated.
        let prev_shown_tactics = truncate_findings(&prev_tactics);
        let curr_shown_tactics = truncate_findings(&tactics);
        let prev_shown_strategy = truncate_findings(&prev_strategy);
        let curr_shown_strategy = truncate_findings(&strategy);

        let mut new_tactics = vec_added(&prev_shown_tactics, &curr_shown_tactics);
        let removed_tactics = vec_removed(&prev_shown_tactics, &curr_shown_tactics);
        let new_strategy = vec_added(&prev_shown_strategy, &curr_shown_strategy);
        let removed_strategy = vec_removed(&prev_shown_strategy, &curr_shown_strategy);

        // Append discovered attacks/checks for this specific move directly into
        // new_tactics. Intentionally NOT added to `prev_tactics` so they don't
        // appear as "removed" in the next checkpoint.
        if let Some(from_sq) = from_sq_opt {
            detect_discovered_attack(
                &board_before,
                current.board(),
                mover_color,
                from_sq,
                &mut new_tactics,
            );
        }

        // Check if material has stabilized (same as previous checkpoint)
        let material_stable = prev_material.as_ref() == Some(&material);

        checkpoints.push(Checkpoint {
            half_move,
            move_san,
            material: material.clone(),
            new_tactics,
            removed_tactics,
            new_strategy,
            removed_strategy,
            newly_attacked,
        });

        // Update previous for next checkpoint
        // (discovered attack findings intentionally excluded from prev_tactics)
        prev_tactics = tactics;
        prev_strategy = strategy;
        prev_attacked = curr_attacked;
        prev_material = Some(material);

        // Stop once we've reached the minimum and material has stabilized,
        // or when we hit the hard cap to avoid walking long tactical PV lines.
        let max_moves = min_moves + 4;
        if (half_move >= min_moves && material_stable) || half_move >= max_moves {
            break;
        }
    }

    checkpoints
}

/// Compares the engine's best line and the user's line by walking both
/// and analyzing tactics/strategy diffs at each half-move checkpoint.
/// Walks at least 6 half-moves, then continues until material stabilizes.
/// `base_tactics` and `base_strategy` come from the original position's analysis.
pub fn compare_lines(
    pos: &Chess,
    engine_uci: &[String],
    user_uci: &[String],
    base_tactics: &[Finding],
    base_strategy: &[Finding],
) -> LineComparison {
    let min_moves = 6;
    let engine_checkpoints = analyze_checkpoints(pos, engine_uci, base_tactics, base_strategy, min_moves);
    let user_checkpoints = analyze_checkpoints(pos, user_uci, base_tactics, base_strategy, min_moves);
    LineComparison {
        engine_checkpoints,
        user_checkpoints,
    }
}

/// Applies priority-tier truncation to a findings slice.
/// Shows only the highest tier present: if any Critical exist, show only those;
/// else if any Important exist, show only those; else show Positional.
pub fn truncate_findings(findings: &[Finding]) -> Vec<Finding> {
    use crate::tactics::{CRITICAL, IMPORTANT};
    for &tier in &[CRITICAL, IMPORTANT, POSITIONAL] {
        let tier_findings: Vec<Finding> = findings
            .iter()
            .filter(|f| f.priority == tier)
            .cloned()
            .collect();
        if !tier_findings.is_empty() {
            return tier_findings;
        }
    }
    vec![]
}

pub fn generate_comparison_text(
    pos: &Chess,
    best_lines: &[PvLine],
    user_line: Option<&PvLine>,
    report: &PositionReport,
    line_comparison: Option<&LineComparison>,
) -> Option<String> {
    let user = user_line?;
    let engine_best = best_lines.first()?;

    let mut text = String::new();

    text.push_str("=== Chess Position Analysis ===\n\n");

    // Position overview
    text.push_str(&format!("Material: {}\n", report.material));
    text.push_str(&format!("Side to move: {}\n\n", pos.turn()));

    // Tactical findings in the starting position
    if !report.tactics.is_empty() {
        text.push_str("Tactical patterns in current position:\n");
        for t in &report.tactics {
            text.push_str(&format!("- {}\n", t.text));
        }
        text.push('\n');
    }

    // Strategic findings in the starting position
    if !report.strategy.is_empty() {
        text.push_str("Strategic observations in current position:\n");
        for s in &report.strategy {
            text.push_str(&format!("- {}\n", s.text));
        }
        text.push('\n');
    }

    // Engine's best line
    text.push_str(&format!(
        "Engine's best line (eval {}): {}\n",
        engine_best.score, engine_best.moves
    ));

    // Second engine line if available
    if let Some(second) = best_lines.get(1) {
        text.push_str(&format!(
            "Engine's 2nd line (eval {}): {}\n",
            second.score, second.moves
        ));
    }

    // User's line
    text.push_str(&format!(
        "\nUser's move (eval {}): {}\n",
        user.score, user.moves
    ));

    // Score comparison
    let diff = engine_best.score_cp - user.score_cp;
    if diff.abs() < 20 {
        text.push_str("\nThe user's move is nearly as good as the engine's best.\n");
    } else if diff > 0 {
        text.push_str(&format!(
            "\nThe user's move loses approximately {:.2} pawns worth of evaluation compared to the engine's best.\n",
            diff as f64 / 100.0
        ));
    } else {
        text.push_str("\nThe user's move may be better than expected — verify the position.\n");
    }

    // Checkpoint-by-checkpoint comparison along both lines
    if let Some(cmp) = line_comparison {
        text.push_str("\n=== Engine Line Checkpoints ===\n");
        for cp in &cmp.engine_checkpoints {
            let has_changes = !cp.new_tactics.is_empty() || !cp.removed_tactics.is_empty()
                || !cp.new_strategy.is_empty() || !cp.removed_strategy.is_empty()
                || !cp.newly_attacked.is_empty();
            text.push_str(&format!("\nAfter {}:\n", cp.move_san));
            if !has_changes {
                text.push_str("  No tactical or strategic changes.\n");
            }
            for t in &cp.new_tactics {
                text.push_str(&format!("  New tactic: {}\n", t.text));
            }
            for t in &cp.removed_tactics {
                text.push_str(&format!("  No longer on the board: {}\n", t.text));
            }
            for s in &cp.new_strategy {
                text.push_str(&format!("  New observation: {}\n", s.text));
            }
            for s in &cp.removed_strategy {
                text.push_str(&format!("  No longer relevant: {}\n", s.text));
            }
            for a in &cp.newly_attacked {
                text.push_str(&format!("  Newly attacked: {}\n", a.text));
            }
        }

        text.push_str("\n=== User Line Checkpoints ===\n");
        for cp in &cmp.user_checkpoints {
            let has_changes = !cp.new_tactics.is_empty() || !cp.removed_tactics.is_empty()
                || !cp.new_strategy.is_empty() || !cp.removed_strategy.is_empty()
                || !cp.newly_attacked.is_empty();
            text.push_str(&format!("\nAfter {}:\n", cp.move_san));
            if !has_changes {
                text.push_str("  No tactical or strategic changes.\n");
            }
            for t in &cp.new_tactics {
                text.push_str(&format!("  New tactic: {}\n", t.text));
            }
            for t in &cp.removed_tactics {
                text.push_str(&format!("  No longer on the board: {}\n", t.text));
            }
            for s in &cp.new_strategy {
                text.push_str(&format!("  New observation: {}\n", s.text));
            }
            for s in &cp.removed_strategy {
                text.push_str(&format!("  No longer relevant: {}\n", s.text));
            }
            for a in &cp.newly_attacked {
                text.push_str(&format!("  Newly attacked: {}\n", a.text));
            }
        }
    }

    text.push_str("\nPlease explain why the engine's move is better (or confirm the user's move is fine). ");
    text.push_str("Use the checkpoint analysis to highlight concrete tactical and strategic differences between the two lines. ");
    text.push_str("Keep the explanation concise and instructive.");

    Some(text)
}
