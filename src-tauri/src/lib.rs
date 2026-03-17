use serde::Serialize;
use shakmaty::{
    fen::Fen,
    san::SanPlus,
    uci::UciMove,
    CastlingMode, Chess, Color, Position,
};
use tauri::Manager;
use tauri_plugin_shell::{
    process::{CommandChild, CommandEvent},
    ShellExt,
};
use tokio::sync::{mpsc::Receiver, Mutex};

/// Holds the running Stockfish process and its stdout receiver.
/// We use tokio's async Mutex because the lock is held across .await points.
struct StockfishEngine {
    child: CommandChild,
    rx: Receiver<CommandEvent>,
    initialized: bool,
}

/// A single principal variation line returned by Stockfish.
#[derive(Serialize, Clone)]
struct PvLine {
    rank: u32,
    score: String,
    score_cp: i32,
    moves: String,
}

/// Internal struct for raw Stockfish output before SAN conversion.
#[derive(Clone)]
struct RawPvLine {
    rank: u32,
    score: String,
    score_cp: i32,
    moves: Vec<String>,
}

/// The full analysis result sent to the frontend.
#[derive(Serialize)]
struct AnalysisResult {
    best_move: String,
    lines: Vec<PvLine>,
    /// If the user provided a move, this is the engine's best continuation after it
    user_line: Option<PvLine>,
}

// Type alias for the engine state managed by Tauri
type EngineState = Mutex<StockfishEngine>;

/// Parses a FEN string into a shakmaty Chess position.
fn parse_fen(fen: &str) -> Result<Chess, String> {
    let parsed: Fen = fen.parse().map_err(|e| format!("Invalid FEN: {}", e))?;
    parsed
        .into_position(CastlingMode::Standard)
        .map_err(|e| format!("Invalid position: {}", e))
}

/// Converts a sequence of UCI move strings into a single numbered move string
/// like "1. e4 e5 2. Nf3 Nc6". Uses the position's turn and fullmove number
/// to start numbering correctly mid-game.
fn uci_moves_to_san(pos: &Chess, uci_moves: &[String]) -> Result<String, String> {
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
fn uci_to_san_single(pos: &Chess, uci_str: &str) -> Result<String, String> {
    let uci: UciMove = uci_str
        .parse()
        .map_err(|e| format!("Invalid UCI move '{}': {}", uci_str, e))?;
    let m = uci
        .to_move(pos)
        .map_err(|e| format!("Illegal UCI move '{}': {}", uci_str, e))?;
    let san_plus = SanPlus::from_move(pos.clone(), m);
    Ok(san_plus.to_string())
}

/// Reads lines from the Stockfish stdout receiver until a line starts with `marker`.
/// Returns all collected lines (including the marker line).
async fn read_until(
    rx: &mut Receiver<CommandEvent>,
    marker: &str,
) -> Result<Vec<String>, String> {

    let mut lines = Vec::new();
    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(bytes) => {
                let line = String::from_utf8_lossy(&bytes).to_string();
                let is_marker = line.starts_with(marker);
                lines.push(line);
                if is_marker {
                    return Ok(lines);
                }
            }
            CommandEvent::Error(err) => {
                return Err(format!("Stockfish error: {}", err));
            }
            CommandEvent::Terminated(status) => {
                return Err(format!("Stockfish terminated unexpectedly: {:?}", status));
            }
            _ => {}
        }
    }
    Err("Stockfish channel closed unexpectedly".to_string())
}

/// Formats a centipawn score as a human-readable string.
/// Normal scores: "+0.35", "-1.20". Mate scores (|cp| >= 100000): "M3", "-M3".
fn format_score(cp: i32) -> String {
    if cp.abs() >= 100_000 {
        // Mate score — the "move count" was lost, but the sign is correct
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
fn parse_info_line(line: &str) -> Option<RawPvLine> {
    let tokens: Vec<&str> = line.split_whitespace().collect();

    // Must contain "pv" and "multipv"
    let multipv_idx = tokens.iter().position(|&t| t == "multipv")?;
    let score_idx = tokens.iter().position(|&t| t == "score")?;
    let pv_idx = tokens.iter().position(|&t| t == "pv")?;

    let rank: u32 = tokens.get(multipv_idx + 1)?.parse().ok()?;

    // Parse score — either "score cp <n>" or "score mate <n>"
    let score_type = *tokens.get(score_idx + 1)?;
    let score_value: i32 = tokens.get(score_idx + 2)?.parse().ok()?;

    let (score_cp, score_str) = match score_type {
        "cp" => {
            // Convert centipawns to a formatted string like "+0.35" or "-1.20"
            let sign = if score_value >= 0 { "+" } else { "" };
            let formatted = format!("{}{:.2}", sign, score_value as f64 / 100.0);
            (score_value, formatted)
        }
        "mate" => {
            // Mate in N moves — format as "M3" or "-M3"
            let formatted = if score_value > 0 {
                format!("M{}", score_value)
            } else {
                format!("-M{}", score_value.abs())
            };
            // Use a very large cp value to indicate mate
            let cp = if score_value > 0 { 100_000 } else { -100_000 };
            (cp, formatted)
        }
        _ => return None,
    };

    // Everything after "pv" is the move sequence
    let moves: Vec<String> = tokens[pv_idx + 1..].iter().map(|s| s.to_string()).collect();

    Some(RawPvLine {
        rank,
        score: score_str,
        score_cp,
        moves,
    })
}

/// Parses all Stockfish output lines into raw PV lines and the best move UCI string.
fn parse_raw(lines: &[String]) -> (String, Vec<RawPvLine>) {
    use std::collections::HashMap;

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

/// Parses all Stockfish output lines into an AnalysisResult.
/// Keeps only the deepest info line per multipv rank.
/// Converts UCI moves to numbered SAN notation using the given position.
fn parse_analysis(lines: &[String], pos: &Chess) -> Result<(String, Vec<PvLine>), String> {
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

/// Tauri command: analyzes a chess position given a FEN string.
/// If user_move is provided (UCI format like "e2e4"), also analyzes the position
/// after that move to show the engine's best continuation.
/// All moves in the response are formatted in SAN (e.g. "Nf3", "Rxc3+").
#[tauri::command]
async fn analyze_position(
    fen: String,
    user_move: Option<String>,
    // State<'_> lets Tauri inject the managed EngineState
    state: tauri::State<'_, EngineState>,
) -> Result<AnalysisResult, String> {
    // Parse the FEN into a shakmaty position for move conversion
    let pos = parse_fen(&fen)?;

    // Lock the async mutex — we hold this across .await calls
    let mut engine = state.lock().await;

    // Lazy initialization: send UCI handshake on first call
    if !engine.initialized {
        engine
            .child
            .write("uci\n".as_bytes())
            .map_err(|e| format!("Failed to send uci: {}", e))?;
        read_until(&mut engine.rx, "uciok").await?;

        engine
            .child
            .write("setoption name MultiPV value 2\n".as_bytes())
            .map_err(|e| format!("Failed to set MultiPV: {}", e))?;

        engine
            .child
            .write("isready\n".as_bytes())
            .map_err(|e| format!("Failed to send isready: {}", e))?;
        read_until(&mut engine.rx, "readyok").await?;

        engine.initialized = true;
    }

    // --- Analysis 1: Engine's top 2 lines from the current position ---
    let pos_cmd = format!("position fen {}\n", fen);
    engine
        .child
        .write(pos_cmd.as_bytes())
        .map_err(|e| format!("Failed to send position: {}", e))?;

    engine
        .child
        .write("go depth 20\n".as_bytes())
        .map_err(|e| format!("Failed to send go: {}", e))?;

    let output = read_until(&mut engine.rx, "bestmove").await?;
    let (best_move, lines) = parse_analysis(&output, &pos)?;

    // --- Analysis 2 (optional): Best continuation after the user's move ---
    let user_line = if let Some(ref uci_str) = user_move {
        // Tell Stockfish to analyze the position after the user's move
        // Use MultiPV 1 since we only need the best continuation
        engine
            .child
            .write("setoption name MultiPV value 1\n".as_bytes())
            .map_err(|e| format!("Failed to set MultiPV: {}", e))?;

        let pos_cmd = format!("position fen {} moves {}\n", fen, uci_str);
        engine
            .child
            .write(pos_cmd.as_bytes())
            .map_err(|e| format!("Failed to send position: {}", e))?;

        engine
            .child
            .write("go depth 20\n".as_bytes())
            .map_err(|e| format!("Failed to send go: {}", e))?;

        let output = read_until(&mut engine.rx, "bestmove").await?;

        // Parse raw UCI lines (we need the raw UCI moves to prepend the user's move)
        let (_, raw_lines) = parse_raw(&output);

        // Restore MultiPV 2 for the next main analysis call
        engine
            .child
            .write("setoption name MultiPV value 2\n".as_bytes())
            .map_err(|e| format!("Failed to reset MultiPV: {}", e))?;

        // Prepend the user's move to the continuation UCI moves, then convert
        // the full sequence to numbered SAN from the original position
        if let Some(raw) = raw_lines.first() {
            // Build the full UCI move list: user's move + engine continuation
            let mut full_uci = vec![uci_str.clone()];
            full_uci.extend(raw.moves.iter().cloned());

            let san = uci_moves_to_san(&pos, &full_uci).unwrap_or_default();
            let score_cp = -raw.score_cp;
            let score = format_score(score_cp);

            Some(PvLine {
                rank: 0,
                score,
                score_cp,
                moves: san,
            })
        } else {
            None
        }
    } else {
        None
    };

    Ok(AnalysisResult {
        best_move,
        lines,
        user_line,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Spawn Stockfish as a sidecar process — it stays alive for the app's lifetime
            let (rx, child) = app
                .shell()
                .sidecar("stockfish")
                .map_err(|e| format!("Failed to create sidecar: {}", e))?
                .spawn()
                .map_err(|e| format!("Failed to spawn Stockfish: {}", e))?;

            // Store the engine in Tauri's managed state so commands can access it
            app.manage(Mutex::new(StockfishEngine {
                child,
                rx,
                initialized: false,
            }));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![analyze_position])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}