mod analysis;
mod engine;
mod moves;
mod strategy;
mod tactics;

use serde::Serialize;
use shakmaty::{fen::Fen, CastlingMode, Chess, Color, Position};
use tauri::Manager;
use tauri_plugin_shell::ShellExt;
use tokio::sync::Mutex;

use analysis::{analyze_position_features, compare_lines, generate_comparison_text, LineComparison, PositionReport};
use engine::{read_until, StockfishEngine};
use moves::{format_score, parse_analysis, parse_raw, uci_moves_to_san, PvLine};

/// The full analysis result sent to the frontend.
#[derive(Serialize)]
struct AnalysisResult {
    best_move: String,
    lines: Vec<PvLine>,
    /// If the user provided a move, this is the engine's best continuation after it
    user_line: Option<PvLine>,
    /// Tactical and strategic analysis of the current position
    position_report: PositionReport,
    /// LLM-ready comparison text (present when user_move is provided)
    comparison_text: Option<String>,
    /// Checkpoint-by-checkpoint comparison of engine vs user lines
    line_comparison: Option<LineComparison>,
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

/// Tauri command: analyzes a chess position given a FEN string.
/// If user_move is provided (UCI format like "e2e4"), also analyzes the position
/// after that move to show the engine's best continuation.
/// All moves in the response are formatted in SAN (e.g. "Nf3", "Rxc3+").
#[tauri::command]
async fn analyze_position(
    fen: String,
    user_move: Option<String>,
    state: tauri::State<'_, EngineState>,
) -> Result<AnalysisResult, String> {
    let pos = parse_fen(&fen)?;

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
    // Keep raw UCI data so we can walk the engine's best line later
    let (_, engine_raw_lines) = parse_raw(&output);
    let (best_move, mut lines) = parse_analysis(&output, &pos)?;

    // Stockfish scores are from side-to-move perspective.
    // Normalize all scores to White's perspective.
    if pos.turn() == Color::Black {
        for line in &mut lines {
            line.score_cp = -line.score_cp;
            line.score = format_score(line.score_cp);
        }
    }

    // --- Analysis 2 (optional): Best continuation after the user's move ---
    let (user_line, user_raw_uci) = if let Some(ref uci_str) = user_move {
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
        let (_, raw_lines) = parse_raw(&output);

        // Restore MultiPV 2 for the next main analysis call
        engine
            .child
            .write("setoption name MultiPV value 2\n".as_bytes())
            .map_err(|e| format!("Failed to reset MultiPV: {}", e))?;

        if let Some(raw) = raw_lines.first() {
            let mut full_uci = vec![uci_str.clone()];
            full_uci.extend(raw.moves.iter().cloned());

            let san = uci_moves_to_san(&pos, &full_uci).unwrap_or_default();
            // Stockfish reports from side-to-move after the user's move.
            // Normalize to White's perspective.
            let score_cp = if pos.turn() == Color::White {
                // User played as White, Stockfish now reports for Black → negate
                -raw.score_cp
            } else {
                // User played as Black, Stockfish now reports for White → keep as-is
                raw.score_cp
            };
            let score = format_score(score_cp);

            (Some(PvLine {
                rank: 0,
                score,
                score_cp,
                moves: san,
            }), full_uci)
        } else {
            (None, vec![])
        }
    } else {
        (None, vec![])
    };

    // --- Position feature analysis (tactics + strategy) ---
    let position_report = analyze_position_features(&pos);

    // --- Compare engine vs user lines at checkpoints ---
    let line_comparison = if user_move.is_some() && !user_raw_uci.is_empty() {
        // Engine's best line raw UCI moves
        let engine_raw_uci: Vec<String> = engine_raw_lines
            .first()
            .map(|r| r.moves.clone())
            .unwrap_or_default();
        Some(compare_lines(
            &pos,
            &engine_raw_uci,
            &user_raw_uci,
            &position_report.tactics_full,
            &position_report.strategy_full,
        ))
    } else {
        None
    };

    // --- Generate comparison text for LLM coaching (when user_move provided) ---
    let comparison_text = generate_comparison_text(
        &pos,
        &lines,
        user_line.as_ref(),
        &position_report,
        line_comparison.as_ref(),
    );

    Ok(AnalysisResult {
        best_move,
        lines,
        user_line,
        position_report,
        comparison_text,
        line_comparison,
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

            let (rx, child) = app
                .shell()
                .sidecar("stockfish")
                .map_err(|e| format!("Failed to create sidecar: {}", e))?
                .spawn()
                .map_err(|e| format!("Failed to spawn Stockfish: {}", e))?;

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