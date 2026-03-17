use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tokio::sync::mpsc::Receiver;

/// Holds the running Stockfish process and its stdout receiver.
/// We use tokio's async Mutex because the lock is held across .await points.
pub struct StockfishEngine {
    pub child: CommandChild,
    pub rx: Receiver<CommandEvent>,
    pub initialized: bool,
}

/// Reads lines from the Stockfish stdout receiver until a line starts with `marker`.
/// Returns all collected lines (including the marker line).
pub async fn read_until(
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
