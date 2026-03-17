use serde::Serialize;
use shakmaty::{san::SanPlus, uci::UciMove, Chess, Color, Position};
use std::collections::HashMap;

/// A single principal variation line returned by Stockfish.
#[derive(Serialize, Clone)]
pub struct PvLine {
    pub rank: u32,
    pub score: String,
    pub score_cp: i32,
    pub moves: String,
}

/// Internal struct for raw Stockfish output before SAN conversion.
#[derive(Clone)]
pub struct RawPvLine {
    pub rank: u32,
    pub score: String,
    pub score_cp: i32,
    pub moves: Vec<String>,
}

/// Converts a sequence of UCI move strings into a single numbered move string
/// like "1. e4 e5 2. Nf3 Nc6". Uses the position's turn and fullmove number
/// to start numbering correctly mid-game.
pub fn uci_moves_to_san(pos: &Chess, uci_moves: &[String]) -> Result<String, String> {
    let mut current = pos.clone();
    let mut result = String::new();
    let mut fullmove = pos.fullmoves().get();
    let mut is_white_turn = current.turn() == Color::White;

    // If the line starts on Black's move, prefix with the move number and "..."
    if !is_white_turn && !uci_moves.is_empty() {
        result.push_str(&format!("{}... ", fullmove));
    }

    for uci_str in uci_moves {
        let uci: UciMove = uci_str
            .parse()
            .map_err(|e| format!("Invalid UCI move '{}': {}", uci_str, e))?;
        let m = uci
            .to_move(&current)
            .map_err(|e| format!("Illegal UCI move '{}': {}", uci_str, e))?;
        let san_plus = SanPlus::from_move_and_play_unchecked(&mut current, m);

        if is_white_turn {
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(&format!("{}. {}", fullmove, san_plus));
        } else {
            result.push_str(&format!(" {}", san_plus));
            fullmove += 1;
        }
        is_white_turn = !is_white_turn;
    }

    Ok(result)
}

/// Converts a single UCI move string to SAN in the context of a position.
pub fn uci_to_san_single(pos: &Chess, uci_str: &str) -> Result<String, String> {
    let uci: UciMove = uci_str
        .parse()
        .map_err(|e| format!("Invalid UCI move '{}': {}", uci_str, e))?;
    let m = uci
        .to_move(pos)
        .map_err(|e| format!("Illegal UCI move '{}': {}", uci_str, e))?;
    let san_plus = SanPlus::from_move(pos.clone(), m);
    Ok(san_plus.to_string())
}

/// Formats a centipawn score as a human-readable string.
/// Normal scores: "+0.35", "-1.20". Mate scores (|cp| >= 100000): "M3", "-M3".
pub fn format_score(cp: i32) -> String {
    if cp.abs() >= 100_000 {
        if cp > 0 {
            "M?".to_string()
        } else {
            "-M?".to_string()
        }
    } else {
        let sign = if cp >= 0 { "+" } else { "" };
        format!("{}{:.2}", sign, cp as f64 / 100.0)
    }
}

/// Parses a single UCI "info" line into a RawPvLine.
/// Example: "info depth 20 multipv 1 score cp 35 ... pv e2e4 e7e5 ..."
pub fn parse_info_line(line: &str) -> Option<RawPvLine> {
    let tokens: Vec<&str> = line.split_whitespace().collect();

    let multipv_idx = tokens.iter().position(|&t| t == "multipv")?;
    let score_idx = tokens.iter().position(|&t| t == "score")?;
    let pv_idx = tokens.iter().position(|&t| t == "pv")?;

    let rank: u32 = tokens.get(multipv_idx + 1)?.parse().ok()?;

    let score_type = *tokens.get(score_idx + 1)?;
    let score_value: i32 = tokens.get(score_idx + 2)?.parse().ok()?;

    let (score_cp, score_str) = match score_type {
        "cp" => {
            let sign = if score_value >= 0 { "+" } else { "" };
            let formatted = format!("{}{:.2}", sign, score_value as f64 / 100.0);
            (score_value, formatted)
        }
        "mate" => {
            let formatted = if score_value > 0 {
                format!("M{}", score_value)
            } else {
                format!("-M{}", score_value.abs())
            };
            let cp = if score_value > 0 { 100_000 } else { -100_000 };
            (cp, formatted)
        }
        _ => return None,
    };

    let moves: Vec<String> = tokens[pv_idx + 1..].iter().map(|s| s.to_string()).collect();

    Some(RawPvLine {
        rank,
        score: score_str,
        score_cp,
        moves,
    })
}

/// Parses all Stockfish output lines into raw PV lines and the best move UCI string.
pub fn parse_raw(lines: &[String]) -> (String, Vec<RawPvLine>) {
    let mut best_lines: HashMap<u32, RawPvLine> = HashMap::new();
    let mut best_move_uci = String::new();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("info") && trimmed.contains(" pv ") {
            if let Some(pv) = parse_info_line(trimmed) {
                best_lines.insert(pv.rank, pv);
            }
        } else if trimmed.starts_with("bestmove") {
            best_move_uci = trimmed
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_string();
        }
    }

    let mut sorted: Vec<RawPvLine> = best_lines.into_values().collect();
    sorted.sort_by_key(|l| l.rank);

    (best_move_uci, sorted)
}

/// Parses all Stockfish output lines into PvLines with SAN notation.
/// Keeps only the deepest info line per multipv rank.
pub fn parse_analysis(lines: &[String], pos: &Chess) -> Result<(String, Vec<PvLine>), String> {
    let (best_move_uci, sorted) = parse_raw(lines);

    let best_move = if best_move_uci.is_empty() {
        String::new()
    } else {
        uci_to_san_single(pos, &best_move_uci).unwrap_or(best_move_uci)
    };

    let result: Vec<PvLine> = sorted
        .into_iter()
        .map(|raw| {
            let san = uci_moves_to_san(pos, &raw.moves).unwrap_or_default();
            PvLine {
                rank: raw.rank,
                score: raw.score,
                score_cp: raw.score_cp,
                moves: san,
            }
        })
        .collect();

    Ok((best_move, result))
}
