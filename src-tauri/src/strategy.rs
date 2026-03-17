use shakmaty::{attacks, Chess, Color, Position, Role, Square};

use crate::tactics::{piece_value, Finding, CRITICAL, IMPORTANT, POSITIONAL};

/// Analyzes material balance for both sides.
/// Returns a human-readable summary like "White is up a knight (+3.00)".
pub fn analyze_material(pos: &Chess) -> String {
    let board = pos.board();

    let white_material = count_material(board, Color::White);
    let black_material = count_material(board, Color::Black);
    let diff = white_material - black_material;

    let mut parts = Vec::new();

    if diff == 0 {
        parts.push("Material is equal".to_string());
    } else {
        let (side, advantage) = if diff > 0 {
            ("White", diff)
        } else {
            ("Black", -diff)
        };
        parts.push(format!(
            "{} is up material (+{})",
            side,
            advantage
        ));
    }

    parts.join(". ")
}

/// Counts total material value for one side.
fn count_material(board: &shakmaty::Board, color: Color) -> i32 {
    let pieces = board.by_color(color);
    let mut total = 0;
    for sq in pieces {
        if let Some(role) = board.role_at(sq) {
            total += piece_value(role);
        }
    }
    total
}

/// Analyzes pawn structure: doubled, isolated, and passed pawns.
pub fn analyze_pawn_structure(pos: &Chess) -> Vec<Finding> {
    let mut findings = Vec::new();
    let board = pos.board();

    for &color in &[Color::White, Color::Black] {
        let side = if color == Color::White {
            "White"
        } else {
            "Black"
        };
        let our_pawns = board.by_color(color) & board.by_role(Role::Pawn);
        let their_pawns = board.by_color(!color) & board.by_role(Role::Pawn);

        // Count pawns per file (0-7)
        let mut file_counts = [0u32; 8];
        for sq in our_pawns {
            let file = u32::from(sq) % 8;
            file_counts[file as usize] += 1;
        }

        // Doubled pawns: 2+ pawns on the same file
        for file in 0..8u32 {
            if file_counts[file as usize] >= 2 {
                let file_char = (b'a' + file as u8) as char;
                findings.push(Finding::new(POSITIONAL, format!(
                    "{} has doubled pawns on the {}-file",
                    side, file_char
                )));
            }
        }

        // Isolated pawns: no friendly pawns on adjacent files
        for sq in our_pawns {
            let file = u32::from(sq) % 8;
            let has_neighbor = (file > 0 && file_counts[(file - 1) as usize] > 0)
                || (file < 7 && file_counts[(file + 1) as usize] > 0);

            if !has_neighbor {
                findings.push(Finding::new(POSITIONAL, format!("{} has an isolated pawn on {}", side, sq)));
            }
        }

        // Passed pawns: no enemy pawns ahead on the same or adjacent files
        for sq in our_pawns {
            let file = u32::from(sq) % 8;
            let rank = u32::from(sq) / 8;
            let mut is_passed = true;

            for enemy_sq in their_pawns {
                let enemy_file = u32::from(enemy_sq) % 8;
                let enemy_rank = u32::from(enemy_sq) / 8;

                // Check if enemy pawn is on same or adjacent file
                if enemy_file.abs_diff(file) <= 1 {
                    // Check if enemy pawn is ahead of our pawn
                    let ahead = match color {
                        Color::White => enemy_rank > rank,
                        Color::Black => enemy_rank < rank,
                    };
                    if ahead {
                        is_passed = false;
                        break;
                    }
                }
            }

            if is_passed {
                let rank_desc = match color {
                    Color::White => rank + 1, // Display 1-indexed
                    Color::Black => rank + 1,
                };
                // Only report if pawn has advanced past the 3rd rank
                let threshold = match color {
                    Color::White => 2, // rank index 2 = 3rd rank
                    Color::Black => 5, // rank index 5 = 6th rank
                };
                let is_advanced = match color {
                    Color::White => rank >= threshold,
                    Color::Black => rank <= threshold,
                };
                if is_passed && is_advanced {
                    findings.push(Finding::new(IMPORTANT, format!(
                        "{} has a passed pawn on {} (rank {})",
                        side, sq, rank_desc
                    )));
                }
            }
        }
    }

    findings
}

/// Analyzes king safety for both sides.
pub fn analyze_king_safety(pos: &Chess) -> Vec<Finding> {
    let mut findings = Vec::new();
    let board = pos.board();
    let occupied = board.occupied();

    for &color in &[Color::White, Color::Black] {
        let side = if color == Color::White {
            "White"
        } else {
            "Black"
        };

        let king_bb = board.by_color(color) & board.by_role(Role::King);
        let king_sq = match king_bb.into_iter().next() {
            Some(sq) => sq,
            None => continue,
        };

        let king_file = u32::from(king_sq) % 8;
        let king_rank = u32::from(king_sq) / 8;

        // Check pawn shield (pawns in front of king on adjacent files)
        let our_pawns = board.by_color(color) & board.by_role(Role::Pawn);
        let shield_rank = match color {
            Color::White => king_rank + 1,
            Color::Black => king_rank.wrapping_sub(1),
        };

        let mut shield_count = 0;
        if shield_rank < 8 {
            for f in king_file.saturating_sub(1)..=(king_file + 1).min(7) {
                let shield_idx = f + shield_rank * 8;
                if shield_idx < 64 {
                    let shield_sq = Square::new(shield_idx);
                    if our_pawns.contains(shield_sq) {
                        shield_count += 1;
                    }
                }
            }
        }

        // Check if king is castled (on g/h or a/b files in the back rank)
        let back_rank = match color {
            Color::White => 0,
            Color::Black => 7,
        };
        let is_on_back_rank = king_rank == back_rank;
        let is_castled_position = is_on_back_rank && (king_file >= 6 || king_file <= 1);

        if is_castled_position && shield_count == 0 {
            findings.push(Finding::new(IMPORTANT, format!(
                "{}'s king has no pawn shield — potentially unsafe",
                side
            )));
        }

        // Count enemy attackers near king (within king's attack zone)
        let king_zone = attacks::king_attacks(king_sq);
        let enemy_pieces = board.by_color(!color) & !board.by_role(Role::Pawn);
        let mut attacker_count = 0;

        for enemy_sq in enemy_pieces {
            if let Some(role) = board.role_at(enemy_sq) {
                let attacks_bb = match role {
                    Role::Knight => attacks::knight_attacks(enemy_sq),
                    Role::Bishop => attacks::bishop_attacks(enemy_sq, occupied),
                    Role::Rook => attacks::rook_attacks(enemy_sq, occupied),
                    Role::Queen => attacks::queen_attacks(enemy_sq, occupied),
                    Role::King => attacks::king_attacks(enemy_sq),
                    Role::Pawn => continue,
                };
                // If the enemy piece attacks any square in the king zone
                if !(attacks_bb & king_zone).is_empty() {
                    attacker_count += 1;
                }
            }
        }

        if attacker_count >= 2 {
            findings.push(Finding::new(CRITICAL, format!(
                "{}'s king is under pressure — {} enemy pieces attack the king zone",
                side, attacker_count
            )));
        }

        // Check for open files near king
        let all_pawns = board.by_role(Role::Pawn);
        for f in king_file.saturating_sub(1)..=(king_file + 1).min(7) {
            let mut has_pawn_on_file = false;
            for rank in 0..8u32 {
                let idx = f + rank * 8;
                let sq = Square::new(idx);
                if all_pawns.contains(sq) {
                    has_pawn_on_file = true;
                    break;
                }
            }
            if !has_pawn_on_file {
                let file_char = (b'a' + f as u8) as char;
                findings.push(Finding::new(IMPORTANT, format!(
                    "Open {}-file near {}'s king",
                    file_char, side
                )));
            }
        }
    }

    findings
}

/// Analyzes piece activity: centralized pieces, rooks on open files.
pub fn analyze_piece_activity(pos: &Chess) -> Vec<Finding> {
    let mut findings = Vec::new();
    let board = pos.board();

    // Central squares: d4, d5, e4, e5 (indices: 27, 35, 28, 36)
    let central_squares = [
        Square::new(27), // d4
        Square::new(35), // d5
        Square::new(28), // e4
        Square::new(36), // e5
    ];

    for &color in &[Color::White, Color::Black] {
        let side = if color == Color::White {
            "White"
        } else {
            "Black"
        };

        // Check for centralized knights (knights on central squares are strong)
        let our_knights = board.by_color(color) & board.by_role(Role::Knight);
        for sq in our_knights {
            if central_squares.contains(&sq) {
                findings.push(Finding::new(POSITIONAL, format!(
                    "{} has a centralized knight on {}",
                    side, sq
                )));
            }
        }

        // Check for rooks on open files (no pawns of any color on the file)
        let our_rooks = board.by_color(color) & board.by_role(Role::Rook);
        let all_pawns = board.by_role(Role::Pawn);
        for sq in our_rooks {
            let file = u32::from(sq) % 8;
            let mut file_has_pawn = false;
            for rank in 0..8u32 {
                let idx = file + rank * 8;
                if all_pawns.contains(Square::new(idx)) {
                    file_has_pawn = true;
                    break;
                }
            }
            if !file_has_pawn {
                findings.push(Finding::new(POSITIONAL, format!(
                    "{} has a rook on the open {}-file",
                    side,
                    (b'a' + file as u8) as char
                )));
            }
        }

        // Check for undeveloped minor pieces (still on back rank)
        let back_rank = match color {
            Color::White => 0u32,
            Color::Black => 7u32,
        };
        let minors = (board.by_role(Role::Knight) | board.by_role(Role::Bishop))
            & board.by_color(color);
        let mut undeveloped = 0;
        for sq in minors {
            let rank = u32::from(sq) / 8;
            if rank == back_rank {
                undeveloped += 1;
            }
        }
        if undeveloped >= 2 {
            findings.push(Finding::new(POSITIONAL, format!(
                "{} has {} undeveloped minor pieces",
                side, undeveloped
            )));
        }
    }

    findings
}

/// Runs all strategic analyses and returns combined findings with priority tiers.
pub fn analyze_all_strategy(pos: &Chess) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(analyze_pawn_structure(pos));
    findings.extend(analyze_king_safety(pos));
    findings.extend(analyze_piece_activity(pos));
    findings
}
