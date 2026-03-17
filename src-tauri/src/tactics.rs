use shakmaty::{attacks, Bitboard, Board, Chess, Color, Position, Role, Square};

// Standard piece values in centipawns
pub const PAWN_VALUE: i32 = 100;
pub const KNIGHT_VALUE: i32 = 300;
pub const BISHOP_VALUE: i32 = 320;
pub const ROOK_VALUE: i32 = 500;
pub const QUEEN_VALUE: i32 = 900;

/// Returns the centipawn value of a piece by its role.
pub fn piece_value(role: Role) -> i32 {
    match role {
        Role::Pawn => PAWN_VALUE,
        Role::Knight => KNIGHT_VALUE,
        Role::Bishop => BISHOP_VALUE,
        Role::Rook => ROOK_VALUE,
        Role::Queen => QUEEN_VALUE,
        Role::King => 0,
    }
}

/// Human-readable name for a piece role.
pub fn piece_name(role: Role) -> &'static str {
    match role {
        Role::Pawn => "pawn",
        Role::Knight => "knight",
        Role::Bishop => "bishop",
        Role::Rook => "rook",
        Role::Queen => "queen",
        Role::King => "king",
    }
}

/// Human-readable name for a color.
fn color_name(color: Color) -> &'static str {
    match color {
        Color::White => "White",
        Color::Black => "Black",
    }
}

/// Computes the squares a pawn of `color` at `sq` attacks (diagonal captures).
/// Uses the reverse trick: squares attacked by a white pawn are the same squares
/// from which a black pawn would attack, and vice versa.
fn pawn_attacks_from(color: Color, sq: Square) -> Bitboard {
    let idx = u32::from(sq) as i32;
    let file = idx % 8;
    let mut result = Bitboard::EMPTY;

    // White pawns attack diagonally up (+7 = left-up, +9 = right-up)
    // Black pawns attack diagonally down (-9 = left-down, -7 = right-down)
    let offsets: &[i32] = match color {
        Color::White => &[7, 9],
        Color::Black => &[-9, -7],
    };

    for &offset in offsets {
        let target = idx + offset;
        if target >= 0 && target < 64 {
            let target_file = target % 8;
            // Ensure we didn't wrap around the board (file distance must be 1)
            if (target_file - file).abs() == 1 {
                result = result | Bitboard::from(Square::new(target as u32));
            }
        }
    }
    result
}

/// Returns a bitboard of all pieces of `by_color` that attack `sq`.
/// Uses the reverse-attack trick: e.g., to find knights attacking sq,
/// compute knight_attacks(sq) and intersect with actual knights.
fn attackers_to(board: &Board, sq: Square, by_color: Color) -> Bitboard {
    let occupied = board.occupied();
    let their = board.by_color(by_color);

    let knights = attacks::knight_attacks(sq) & their & board.by_role(Role::Knight);
    let kings = attacks::king_attacks(sq) & their & board.by_role(Role::King);

    // Bishop/Queen on diagonals
    let diag = attacks::bishop_attacks(sq, occupied)
        & their
        & (board.by_role(Role::Bishop) | board.by_role(Role::Queen));

    // Rook/Queen on ranks/files
    let straight = attacks::rook_attacks(sq, occupied)
        & their
        & (board.by_role(Role::Rook) | board.by_role(Role::Queen));

    // For pawns: a pawn of by_color attacks sq from positions where the
    // OPPOSITE color's pawn at sq would attack back to them
    let pawns = pawn_attacks_from(!by_color, sq) & their & board.by_role(Role::Pawn);

    knights | kings | diag | straight | pawns
}

/// Detects hanging pieces for a single color: pieces attacked by the opponent
/// but not defended by their own side.
fn detect_hanging(board: &Board, color: Color, findings: &mut Vec<String>) {
    let pieces = board.by_color(color) & !board.by_role(Role::King);
    let opponent = !color;
    for sq in pieces {
        let role = match board.role_at(sq) {
            Some(r) => r,
            None => continue,
        };
        let attackers = attackers_to(board, sq, opponent);
        let defenders = attackers_to(board, sq, color);
        if !attackers.is_empty() && defenders.is_empty() {
            findings.push(format!(
                "{}'s {} on {} is hanging — attacked but undefended",
                color_name(color),
                piece_name(role),
                sq
            ));
        }
    }
}

/// Detects forks: a piece attacking 2+ enemy pieces where material can be won.
/// A fork is meaningful when the forking piece is worth less than the most
/// valuable attacked piece, or the attacked pieces can't all escape.
fn detect_forks(board: &Board, color: Color, findings: &mut Vec<String>) {
    let occupied = board.occupied();
    let our_pieces = board.by_color(color);
    let their_pieces = board.by_color(!color) & !board.by_role(Role::King);
    let side = color_name(color);

    for sq in our_pieces {
        let role = match board.role_at(sq) {
            Some(r) => r,
            None => continue,
        };

        // Compute what this piece attacks
        let piece_attacks = match role {
            Role::Pawn => pawn_attacks_from(color, sq),
            Role::Knight => attacks::knight_attacks(sq),
            Role::Bishop => attacks::bishop_attacks(sq, occupied),
            Role::Rook => attacks::rook_attacks(sq, occupied),
            Role::Queen => attacks::queen_attacks(sq, occupied),
            Role::King => attacks::king_attacks(sq),
        };

        // Which enemy pieces are attacked?
        let attacked_enemies = piece_attacks & their_pieces;
        if attacked_enemies.count() >= 2 {
            let forker_value = piece_value(role);
            let mut attacked_names = Vec::new();

            for target_sq in attacked_enemies {
                if let Some(target_role) = board.role_at(target_sq) {
                    // Only include targets where capturing actually wins material:
                    // the target is undefended, or worth more than the forking piece.
                    let target_defenders = attackers_to(board, target_sq, !color);
                    let is_undefended = target_defenders.is_empty();
                    let profitable = piece_value(target_role) > forker_value;
                    if is_undefended || profitable {
                        attacked_names.push(format!("{} on {}", piece_name(target_role), target_sq));
                    }
                }
            }

            if attacked_names.len() >= 2 {
                findings.push(format!(
                    "{}'s {} on {} forks {}",
                    side,
                    piece_name(role),
                    sq,
                    attacked_names.join(" and ")
                ));
            }
        }

        // Also check if we fork the king + another piece
        let attacked_king = piece_attacks & board.by_color(!color) & board.by_role(Role::King);
        if !attacked_king.is_empty() && !attacked_enemies.is_empty() {
            let mut targets = Vec::new();
            for target_sq in attacked_enemies {
                if let Some(target_role) = board.role_at(target_sq) {
                    targets.push(format!("{} on {}", piece_name(target_role), target_sq));
                }
            }
            if !targets.is_empty() {
                findings.push(format!(
                    "{}'s {} on {} attacks the king and {}",
                    side,
                    piece_name(role),
                    sq,
                    targets.join(", ")
                ));
            }
        }
    }
}

/// Detects absolute pins: an enemy sliding piece pins one of our pieces to our king.
/// A piece is pinned when it's the only piece between an enemy slider and our king
/// on the relevant ray (diagonal for bishop/queen, straight for rook/queen).
fn detect_pins(board: &Board, us: Color, them: Color, findings: &mut Vec<String>) {
    let occupied = board.occupied();
    let our_pieces = board.by_color(us);
    let their_pieces = board.by_color(them);
    let pinned_side = color_name(us);

    // Find our king
    let our_king_sq = match (board.by_role(Role::King) & our_pieces).into_iter().next() {
        Some(sq) => sq,
        None => return,
    };

    // Enemy diagonal sliders (bishop + queen)
    let their_diag = (board.by_role(Role::Bishop) | board.by_role(Role::Queen)) & their_pieces;
    // Enemy straight sliders (rook + queen)
    let their_straight = (board.by_role(Role::Rook) | board.by_role(Role::Queen)) & their_pieces;

    // Check diagonal pins
    for attacker_sq in their_diag {
        // Verify attacker and king are on the same diagonal
        if !attacks::bishop_attacks(attacker_sq, Bitboard::EMPTY)
            .contains(our_king_sq)
        {
            continue;
        }
        let between = attacks::between(attacker_sq, our_king_sq);
        let blockers = between & occupied;
        if blockers.count() == 1 {
            let blocker_sq = blockers.into_iter().next().unwrap();
            if our_pieces.contains(blocker_sq) {
                if let (Some(blocker_role), Some(attacker_role)) =
                    (board.role_at(blocker_sq), board.role_at(attacker_sq))
                {
                    findings.push(format!(
                        "{}'s {} on {} is pinned to the king by {} on {}",
                        pinned_side,
                        piece_name(blocker_role),
                        blocker_sq,
                        piece_name(attacker_role),
                        attacker_sq
                    ));
                }
            }
        }
    }

    // Check straight pins
    for attacker_sq in their_straight {
        if !attacks::rook_attacks(attacker_sq, Bitboard::EMPTY)
            .contains(our_king_sq)
        {
            continue;
        }
        let between = attacks::between(attacker_sq, our_king_sq);
        let blockers = between & occupied;
        if blockers.count() == 1 {
            let blocker_sq = blockers.into_iter().next().unwrap();
            if our_pieces.contains(blocker_sq) {
                if let (Some(blocker_role), Some(attacker_role)) =
                    (board.role_at(blocker_sq), board.role_at(attacker_sq))
                {
                    findings.push(format!(
                        "{}'s {} on {} is pinned to the king by {} on {}",
                        pinned_side,
                        piece_name(blocker_role),
                        blocker_sq,
                        piece_name(attacker_role),
                        attacker_sq
                    ));
                }
            }
        }
    }
}

/// Detects skewers: a sliding piece attacks a valuable piece, and behind it
/// on the same ray sits a less valuable piece that will be captured if the
/// front piece moves.
fn detect_skewers(board: &Board, color: Color, findings: &mut Vec<String>) {
    let occupied = board.occupied();
    let our_pieces = board.by_color(color);
    let their_pieces = board.by_color(!color);

    // Our diagonal sliders
    let our_diag = (board.by_role(Role::Bishop) | board.by_role(Role::Queen)) & our_pieces;
    let our_straight = (board.by_role(Role::Rook) | board.by_role(Role::Queen)) & our_pieces;

    for attacker_sq in our_diag {
        check_skewer_ray(board, attacker_sq, color, their_pieces, occupied, true, findings);
    }
    for attacker_sq in our_straight {
        check_skewer_ray(board, attacker_sq, color, their_pieces, occupied, false, findings);
    }
}

/// Helper: checks all enemy pieces on a slider's ray for skewer patterns.
/// A skewer is only valid when:
///   1. The front piece can't simply capture the attacker (it doesn't attack
///      the attacker's square).
///   2. The front piece is under real pressure — either it's undefended, or the
///      attacker is worth less than the front piece (profitable capture threat).
fn check_skewer_ray(
    board: &Board,
    attacker_sq: Square,
    attacker_color: Color,
    their_pieces: Bitboard,
    occupied: Bitboard,
    diagonal: bool,
    findings: &mut Vec<String>,
) {
    let attack_bb = if diagonal {
        attacks::bishop_attacks(attacker_sq, occupied)
    } else {
        attacks::rook_attacks(attacker_sq, occupied)
    };

    let attacker_role = match board.role_at(attacker_sq) {
        Some(r) => r,
        None => return,
    };

    // For each enemy piece we attack, check if there's another piece behind it
    let attacked = attack_bb & their_pieces;
    for front_sq in attacked {
        let front_role = match board.role_at(front_sq) {
            Some(r) => r,
            None => continue,
        };

        // 1. If the front piece can attack the skewering piece, it can just
        //    capture instead of moving out of the way — not a real skewer.
        let front_attacks = match front_role {
            Role::Pawn => pawn_attacks_from(!attacker_color, front_sq),
            Role::Knight => attacks::knight_attacks(front_sq),
            Role::Bishop => attacks::bishop_attacks(front_sq, occupied),
            Role::Rook => attacks::rook_attacks(front_sq, occupied),
            Role::Queen => attacks::queen_attacks(front_sq, occupied),
            Role::King => attacks::king_attacks(front_sq),
        };
        if front_attacks.contains(attacker_sq) {
            continue;
        }

        // 2. The front piece must be under real pressure — either undefended,
        //    or the attacker is worth less (profitable capture threat).
        let front_defenders = attackers_to(board, front_sq, !attacker_color);
        let front_is_defended = !front_defenders.is_empty();
        let profitable_capture = piece_value(attacker_role) < piece_value(front_role);
        if front_is_defended && !profitable_capture {
            continue;
        }

        // Remove the front piece and see what's behind it on the same ray
        let occ_without_front = occupied & !Bitboard::from(front_sq);
        let extended = if diagonal {
            attacks::bishop_attacks(attacker_sq, occ_without_front)
        } else {
            attacks::rook_attacks(attacker_sq, occ_without_front)
        };

        // Look for a piece behind the front piece on the same ray direction
        let behind = extended & their_pieces & !Bitboard::from(front_sq);
        for back_sq in behind {
            // Verify it's on the same ray (attacker -> front -> back)
            let between_ab = attacks::between(attacker_sq, back_sq);
            if !between_ab.contains(front_sq) {
                continue; // Not on the same ray direction
            }

            if let Some(back_role) = board.role_at(back_sq) {
                // Skewer: front piece is more valuable (or equal) — it must move,
                // exposing the back piece
                if piece_value(front_role) >= piece_value(back_role) {
                    findings.push(format!(
                        "{} on {} skewers {} on {} through to {} on {}",
                        piece_name(attacker_role),
                        attacker_sq,
                        piece_name(front_role),
                        front_sq,
                        piece_name(back_role),
                        back_sq
                    ));
                }
            }
        }
    }
}

/// Runs all tactical pattern detections on the current position.
/// Returns a list of human-readable tactical findings.
pub fn detect_all_tactics(pos: &Chess) -> Vec<String> {
    let mut findings = Vec::new();
    let board = pos.board();

    // Run all detections for both sides so we never miss patterns
    for &color in &[Color::White, Color::Black] {
        detect_hanging(board, color, &mut findings);
        detect_forks(board, color, &mut findings);
        detect_pins(board, color, !color, &mut findings);
        detect_skewers(board, color, &mut findings);
    }

    findings
}
