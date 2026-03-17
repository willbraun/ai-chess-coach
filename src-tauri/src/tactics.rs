use serde::Serialize;
use shakmaty::{attacks, Bitboard, Board, Chess, Color, Position, Role, Square};

// Priority tiers for LLM prompt filtering
pub const CRITICAL: u8 = 1; // Directly wins/loses material or forces mate
pub const IMPORTANT: u8 = 2; // Creates winning pressure or structural advantage
pub const POSITIONAL: u8 = 3; // General quality observations

/// A tactical or strategic finding with a priority tier for LLM filtering.
#[derive(Serialize, Clone)]
pub struct Finding {
    pub text: String,
    pub priority: u8,
}

impl Finding {
    pub fn new(priority: u8, text: String) -> Self {
        Self { text, priority }
    }
}

// Standard piece values
pub const PAWN_VALUE: i32 = 1;
pub const KNIGHT_VALUE: i32 = 3;
pub const BISHOP_VALUE: i32 = 3;
pub const ROOK_VALUE: i32 = 5;
pub const QUEEN_VALUE: i32 = 9;

/// Returns the pawn value of a piece by its role.
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

/// Returns true if `piece_sq` (belonging to `piece_color`) is absolutely pinned
/// such that it cannot legally capture at `target_sq`.
/// A piece is absolutely pinned when removing it from the board exposes its king
/// to attack by an enemy slider, AND `target_sq` is not on the pin ray.
fn is_pinned_against_capture(
    board: &Board,
    piece_sq: Square,
    piece_color: Color,
    target_sq: Square,
) -> bool {
    let king_sq = match (board.by_color(piece_color) & board.by_role(Role::King))
        .into_iter()
        .next()
    {
        Some(sq) => sq,
        None => return false,
    };

    let enemy = board.by_color(!piece_color);
    // Remove the piece and see if the king becomes attacked by a slider
    let occupied_without = board.occupied() & !Bitboard::from(piece_sq);

    let diag_sliders = (board.by_role(Role::Bishop) | board.by_role(Role::Queen)) & enemy;
    let straight_sliders = (board.by_role(Role::Rook) | board.by_role(Role::Queen)) & enemy;

    let king_diag = attacks::bishop_attacks(king_sq, occupied_without);
    let king_straight = attacks::rook_attacks(king_sq, occupied_without);

    let pinner_sq = (king_diag & diag_sliders)
        .into_iter()
        .next()
        .or_else(|| (king_straight & straight_sliders).into_iter().next());

    match pinner_sq {
        None => false, // not pinned at all
        Some(pinner) => {
            // Pinned. The only legal moves are along the pin ray (king..pinner inclusive).
            // If target_sq is the pinner or between king and pinner, the capture is legal.
            target_sq != pinner && !attacks::between(king_sq, pinner).contains(target_sq)
        }
    }
}

/// Detects hanging pieces for a single color: pieces attacked by the opponent
/// but not defended by their own side.
fn detect_hanging(pos: &Chess, color: Color, findings: &mut Vec<Finding>) {
    let board = pos.board();
    let pieces = board.by_color(color) & !board.by_role(Role::King);
    let opponent = !color;
    for sq in pieces {
        let role = match board.role_at(sq) {
            Some(r) => r,
            None => continue,
        };
        let attackers = attackers_to(board, sq, opponent);
        let defenders = attackers_to(board, sq, color);
        // Filter out attackers that are absolutely pinned and cannot legally capture here
        let effective_attackers = attackers
            .into_iter()
            .filter(|&attacker_sq| !is_pinned_against_capture(board, attacker_sq, opponent, sq))
            .count();
        if effective_attackers > 0 && defenders.is_empty() {
            findings.push(Finding::new(CRITICAL, format!(
                "{}'s {} on {} is hanging",
                color_name(color),
                piece_name(role),
                sq
            )));
        }
    }
}

/// Detects forks: a piece attacking 2+ enemy pieces where material can be won.
/// A fork is meaningful when the forking piece is worth less than the most
/// valuable attacked piece, or the attacked pieces can't all escape.
fn detect_forks(board: &Board, color: Color, findings: &mut Vec<Finding>) {
    let occupied = board.occupied();
    let our_pieces = board.by_color(color);
    let their_pieces = board.by_color(!color);
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

            let mut attacked_targets: Vec<(i32, String)> = Vec::new();

            for target_sq in attacked_enemies {
                if let Some(target_role) = board.role_at(target_sq) {
                    // The king is always a valid fork target (opponent must deal with check)
                    if target_role == Role::King {
                        attacked_targets.push((1000, format!("king on {}", target_sq)));
                        continue;
                    }
                    // For non-king targets, only include where capturing wins material:
                    // undefended, worth more than the forker, or overwhelmed.
                    let target_defenders = attackers_to(board, target_sq, !color);
                    let our_attackers = attackers_to(board, target_sq, color);
                    let is_undefended = target_defenders.is_empty();
                    let profitable = piece_value(target_role) > forker_value;
                    let overwhelmed = our_attackers.count() > target_defenders.count();
                    if is_undefended || profitable || overwhelmed {
                        attacked_targets.push((piece_value(target_role), format!("{} on {}", piece_name(target_role), target_sq)));
                    }
                }
            }

            // Sort by piece value descending (king first)
            attacked_targets.sort_by(|a, b| b.0.cmp(&a.0));
            let attacked_names: Vec<String> = attacked_targets.into_iter().map(|(_, name)| name).collect();

            if attacked_names.len() >= 2 {
                findings.push(Finding::new(CRITICAL, format!(
                    "{}'s {} on {} forks {}",
                    side,
                    piece_name(role),
                    sq,
                    attacked_names.join(" and ")
                )));
            }
        }
    }
}

/// Detects pins: a sliding piece pins one of the opponent's pieces to a more
/// valuable piece behind it on the same ray.
/// Absolute pins (to the king) and relative pins (to the queen or rook) are detected.
fn detect_pins(board: &Board, us: Color, them: Color, findings: &mut Vec<Finding>) {
    let occupied = board.occupied();
    let our_pieces = board.by_color(us);
    let their_pieces = board.by_color(them);
    let pinned_side = color_name(us);

    // High-value pieces that can be pinned to (king, queen, rook)
    let pin_targets = (board.by_role(Role::King) | board.by_role(Role::Queen) | board.by_role(Role::Rook))
        & our_pieces;

    // Enemy diagonal sliders (bishop + queen)
    let their_diag = (board.by_role(Role::Bishop) | board.by_role(Role::Queen)) & their_pieces;
    // Enemy straight sliders (rook + queen)
    let their_straight = (board.by_role(Role::Rook) | board.by_role(Role::Queen)) & their_pieces;

    // Check diagonal pins
    for attacker_sq in their_diag {
        let attacker_role = match board.role_at(attacker_sq) {
            Some(r) => r,
            None => continue,
        };
        for target_sq in pin_targets {
            // Verify attacker and target are on the same diagonal
            if !attacks::bishop_attacks(attacker_sq, Bitboard::EMPTY).contains(target_sq) {
                continue;
            }
            let between = attacks::between(attacker_sq, target_sq);
            let blockers = between & occupied;
            if blockers.count() == 1 {
                let blocker_sq = blockers.into_iter().next().unwrap();
                if our_pieces.contains(blocker_sq) {
                    if let Some(blocker_role) = board.role_at(blocker_sq) {
                        let target_role = board.role_at(target_sq).unwrap_or(Role::Pawn);
                        let target_undefended = target_role != Role::King
                            && attackers_to(board, target_sq, us).is_empty();
                        // Report if: king (absolute pin), attacker can profitably/evenly
                        // capture the pinned piece, attacker profits from capturing the
                        // exposed target, or the exposed target is undefended.
                        if target_role == Role::King
                            || piece_value(attacker_role) <= piece_value(blocker_role)
                            || piece_value(attacker_role) < piece_value(target_role)
                            || target_undefended
                        {
                            let target_name = if target_role == Role::King {
                                "the king".to_string()
                            } else {
                                format!("{} on {}", piece_name(target_role), target_sq)
                            };
                            findings.push(Finding::new(CRITICAL, format!(
                                "{}'s {} on {} is pinned to {} by {} on {}",
                                pinned_side,
                                piece_name(blocker_role),
                                blocker_sq,
                                target_name,
                                piece_name(attacker_role),
                                attacker_sq
                            )));
                        }
                    }
                }
            }
        }
    }

    // Check straight pins
    for attacker_sq in their_straight {
        let attacker_role = match board.role_at(attacker_sq) {
            Some(r) => r,
            None => continue,
        };
        for target_sq in pin_targets {
            if !attacks::rook_attacks(attacker_sq, Bitboard::EMPTY).contains(target_sq) {
                continue;
            }
            let between = attacks::between(attacker_sq, target_sq);
            let blockers = between & occupied;
            if blockers.count() == 1 {
                let blocker_sq = blockers.into_iter().next().unwrap();
                if our_pieces.contains(blocker_sq) {
                    if let Some(blocker_role) = board.role_at(blocker_sq) {
                        let target_role = board.role_at(target_sq).unwrap_or(Role::Pawn);
                        let target_undefended = target_role != Role::King
                            && attackers_to(board, target_sq, us).is_empty();
                        if target_role == Role::King
                            || piece_value(attacker_role) <= piece_value(blocker_role)
                            || piece_value(attacker_role) < piece_value(target_role)
                            || target_undefended
                        {
                            let target_name = if target_role == Role::King {
                                "the king".to_string()
                            } else {
                                format!("{} on {}", piece_name(target_role), target_sq)
                            };
                            findings.push(Finding::new(CRITICAL, format!(
                                "{}'s {} on {} is pinned to {} by {} on {}",
                                pinned_side,
                                piece_name(blocker_role),
                                blocker_sq,
                                target_name,
                                piece_name(attacker_role),
                                attacker_sq
                            )));
                        }
                    }
                }
            }
        }
    }
}

/// Detects skewers: a sliding piece attacks a valuable piece, and behind it
/// on the same ray sits a less valuable piece that will be captured if the
/// front piece moves.
fn detect_skewers(board: &Board, color: Color, findings: &mut Vec<Finding>) {
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
    findings: &mut Vec<Finding>,
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
                    findings.push(Finding::new(CRITICAL, format!(
                        "{} on {} skewers {} on {} through to {} on {}",
                        piece_name(attacker_role),
                        attacker_sq,
                        piece_name(front_role),
                        front_sq,
                        piece_name(back_role),
                        back_sq
                    )));
                }
            }
        }
    }
}

/// Detects discovered attacks and discovered checks: a piece moves off a ray,
/// unblocking a friendly slider that now attacks a valuable enemy piece behind it.
/// Takes the board state before and after the move, plus the from-square of the
/// piece that moved. Called from the PV walk, not from detect_all_tactics.
pub fn detect_discovered_attack(
    board_before: &Board,
    board_after: &Board,
    mover_color: Color,
    from_sq: Square,
    findings: &mut Vec<Finding>,
) {
    let occupied_before = board_before.occupied();
    let occupied_after = board_after.occupied();
    let enemy_after = board_after.by_color(!mover_color);

    // Find friendly sliders that existed at the same square before the move
    // (i.e., they did NOT move — the discovered attack is by a DIFFERENT piece)
    let our_before = board_before.by_color(mover_color);
    let sliders_before = (board_before.by_role(Role::Bishop)
        | board_before.by_role(Role::Rook)
        | board_before.by_role(Role::Queen))
        & our_before;

    for slider_sq in sliders_before {
        // The sliding piece must not be the piece that just moved
        if slider_sq == from_sq {
            continue;
        }

        let slider_role = match board_before.role_at(slider_sq) {
            Some(r) => r,
            None => continue,
        };

        // Compute attack rays before and after — occupied squares changed because
        // the mover vacated from_sq (and possibly captured an enemy on to_sq)
        let attacks_before = match slider_role {
            Role::Bishop => attacks::bishop_attacks(slider_sq, occupied_before),
            Role::Rook => attacks::rook_attacks(slider_sq, occupied_before),
            Role::Queen => attacks::queen_attacks(slider_sq, occupied_before),
            _ => continue,
        };
        let attacks_after = match slider_role {
            Role::Bishop => attacks::bishop_attacks(slider_sq, occupied_after),
            Role::Rook => attacks::rook_attacks(slider_sq, occupied_after),
            Role::Queen => attacks::queen_attacks(slider_sq, occupied_after),
            _ => continue,
        };

        // Enemy pieces on squares newly reached after the mover stepped aside
        let newly_attacked_enemies = (attacks_after & !attacks_before) & enemy_after;

        for target_sq in newly_attacked_enemies {
            // Verify from_sq was the blocker: it must lie on the ray from slider to target
            if !attacks::between(slider_sq, target_sq).contains(from_sq) {
                continue;
            }

            let target_role = match board_after.role_at(target_sq) {
                Some(r) => r,
                None => continue,
            };

            // Only report attacks on pieces worth >= knight (or the king)
            if piece_value(target_role) < KNIGHT_VALUE && target_role != Role::King {
                continue;
            }

            let mover_name = board_before
                .role_at(from_sq)
                .map(piece_name)
                .unwrap_or("piece");
            let attack_type = if target_role == Role::King {
                "a discovered check"
            } else {
                "a discovered attack"
            };

            findings.push(Finding::new(CRITICAL, format!(
                "{}'s {} on {} reveals {} by {} on {} targeting {} on {}",
                color_name(mover_color),
                mover_name,
                from_sq,
                attack_type,
                piece_name(slider_role),
                slider_sq,
                piece_name(target_role),
                target_sq,
            )));
        }
    }
}

/// Detects double check: the side to move is in check from 2+ pieces at once.
/// In double check, the only legal response is a king move.
fn detect_double_check(pos: &Chess, findings: &mut Vec<Finding>) {
    let checkers = pos.checkers();
    if checkers.count() >= 2 {
        let board = pos.board();
        let checked_color = pos.turn();
        let checker_names: Vec<String> = checkers
            .into_iter()
            .filter_map(|sq| {
                board.role_at(sq).map(|role| format!("{} on {}", piece_name(role), sq))
            })
            .collect();
        findings.push(Finding::new(CRITICAL, format!(
            "{} is in double check from {}",
            color_name(checked_color),
            checker_names.join(" and ")
        )));
    }
}

/// Detects trapped pieces: minor pieces or above (value >= 3) that have no
/// safe square to move to. Every destination is either blocked by a friendly
/// piece or attacked by a cheaper enemy piece.
/// Returns true if the piece on `sq` (belonging to `color`) has no safe squares
/// to move to AND is currently being attacked — i.e. it is trapped.
fn is_piece_trapped(board: &Board, sq: Square, color: Color) -> bool {
    let occupied = board.occupied();
    let our_pieces = board.by_color(color);
    let their_pieces = board.by_color(!color);

    let role = match board.role_at(sq) {
        Some(r) => r,
        None => return false,
    };
    let value = piece_value(role);

    let move_squares = match role {
        Role::Knight => attacks::knight_attacks(sq),
        Role::Bishop => attacks::bishop_attacks(sq, occupied),
        Role::Rook => attacks::rook_attacks(sq, occupied),
        Role::Queen => attacks::queen_attacks(sq, occupied),
        _ => return false,
    };

    let destinations = move_squares & !our_pieces;
    if destinations.is_empty() {
        return false;
    }

    for dest in destinations {
        let enemy_attackers = attackers_to(board, dest, !color);
        if enemy_attackers.is_empty() {
            return false; // safe square exists
        }
        let min_attacker_value = enemy_attackers
            .into_iter()
            .filter_map(|a| board.role_at(a))
            .map(piece_value)
            .min()
            .unwrap_or(0);
        if min_attacker_value >= value {
            return false; // trade is acceptable
        }
        if their_pieces.contains(dest) {
            if let Some(cap_role) = board.role_at(dest) {
                if piece_value(cap_role) >= value {
                    return false; // capture offsets loss
                }
            }
        }
    }

    // No safe square — only report as trapped if it's also under attack
    !attackers_to(board, sq, !color).is_empty()
}

fn detect_trapped_piece(board: &Board, color: Color, findings: &mut Vec<Finding>) {
    let our_pieces = board.by_color(color);
    let candidates = our_pieces & !board.by_role(Role::King) & !board.by_role(Role::Pawn);

    for sq in candidates {
        let role = match board.role_at(sq) {
            Some(r) => r,
            None => continue,
        };
        if is_piece_trapped(board, sq, color) {
            findings.push(Finding::new(IMPORTANT, format!(
                "{}'s {} on {} is trapped — no safe squares",
                color_name(color),
                piece_name(role),
                sq
            )));
        }
    }
}

/// Detects x-ray attacks: a friendly slider attacks through one of our own
/// pieces to reach a valuable enemy piece behind it. If the friendly blocker
/// moves, the enemy piece becomes directly attacked.
fn detect_xray_attack(board: &Board, color: Color, findings: &mut Vec<Finding>) {
    let occupied = board.occupied();
    let our_pieces = board.by_color(color);
    let their_pieces = board.by_color(!color);
    let side = color_name(color);

    // Our diagonal sliders (bishop + queen)
    let our_diag = (board.by_role(Role::Bishop) | board.by_role(Role::Queen)) & our_pieces;
    // Our straight sliders (rook + queen)
    let our_straight = (board.by_role(Role::Rook) | board.by_role(Role::Queen)) & our_pieces;

    // Check diagonal x-rays through our own pieces
    for attacker_sq in our_diag {
        let attacker_role = match board.role_at(attacker_sq) {
            Some(r) => r,
            None => continue,
        };
        let attack_bb = attacks::bishop_attacks(attacker_sq, occupied);
        // Find our own pieces directly in the attack path
        let our_blockers = attack_bb & our_pieces;

        for blocker_sq in our_blockers {
            let blocker_role = match board.role_at(blocker_sq) {
                Some(r) => r,
                None => continue,
            };
            // Remove the blocker and see what's behind on the same ray
            let occ_without = occupied & !Bitboard::from(blocker_sq);
            let extended = attacks::bishop_attacks(attacker_sq, occ_without);
            // Only new squares (past the blocker)
            let behind = extended & their_pieces & !attack_bb;

            for target_sq in behind {
                // Must be on the same ray direction (attacker → blocker → target)
                if !attacks::between(attacker_sq, target_sq).contains(blocker_sq) {
                    continue;
                }
                if let Some(target_role) = board.role_at(target_sq) {
                    // Only report if target piece is valuable (>= bishop)
                    if piece_value(target_role) >= BISHOP_VALUE {
                        findings.push(Finding::new(IMPORTANT, format!(
                            "{}'s {} on {} x-rays through {} on {} to {} on {}",
                            side,
                            piece_name(attacker_role),
                            attacker_sq,
                            piece_name(blocker_role),
                            blocker_sq,
                            piece_name(target_role),
                            target_sq
                        )));
                    }
                }
            }
        }
    }

    // Check straight x-rays through our own pieces
    for attacker_sq in our_straight {
        let attacker_role = match board.role_at(attacker_sq) {
            Some(r) => r,
            None => continue,
        };
        let attack_bb = attacks::rook_attacks(attacker_sq, occupied);
        let our_blockers = attack_bb & our_pieces;

        for blocker_sq in our_blockers {
            let blocker_role = match board.role_at(blocker_sq) {
                Some(r) => r,
                None => continue,
            };
            let occ_without = occupied & !Bitboard::from(blocker_sq);
            let extended = attacks::rook_attacks(attacker_sq, occ_without);
            let behind = extended & their_pieces & !attack_bb;

            for target_sq in behind {
                if !attacks::between(attacker_sq, target_sq).contains(blocker_sq) {
                    continue;
                }
                if let Some(target_role) = board.role_at(target_sq) {
                    if piece_value(target_role) >= BISHOP_VALUE {
                        findings.push(Finding::new(IMPORTANT, format!(
                            "{}'s {} on {} x-rays through {} on {} to {} on {}",
                            side,
                            piece_name(attacker_role),
                            attacker_sq,
                            piece_name(blocker_role),
                            blocker_sq,
                            piece_name(target_role),
                            target_sq
                        )));
                    }
                }
            }
        }
    }
}

/// Detects "removing the defender" patterns: capturing an enemy defender
/// profitably would leave a more valuable enemy piece undefended.
fn detect_removing_the_defender(board: &Board, color: Color, findings: &mut Vec<Finding>) {
    let their_pieces = board.by_color(!color);

    // Look at each enemy piece worth >= knight that we attack
    let their_non_king = their_pieces & !board.by_role(Role::King);

    for target_sq in their_non_king {
        let target_role = match board.role_at(target_sq) {
            Some(r) => r,
            None => continue,
        };
        let target_value = piece_value(target_role);
        if target_value < KNIGHT_VALUE {
            continue;
        }

        // Must be currently defended (otherwise it's just hanging)
        let defenders = attackers_to(board, target_sq, !color);
        if defenders.is_empty() {
            continue;
        }

        // We must attack the target
        let our_attackers_on_target = attackers_to(board, target_sq, color);
        if our_attackers_on_target.is_empty() {
            continue;
        }

        // For each defender: can we profitably capture it?
        for def_sq in defenders {
            let def_role = match board.role_at(def_sq) {
                Some(r) => r,
                None => continue,
            };
            if def_role == Role::King {
                continue; // Can't capture the king
            }
            let def_value = piece_value(def_role);

            // Only interesting if target is worth more than the defender
            if target_value <= def_value {
                continue;
            }

            // We must attack the defender to be able to capture it
            let our_attacks_on_def = attackers_to(board, def_sq, color);
            if our_attacks_on_def.is_empty() {
                continue;
            }

            // Find our cheapest attacker on this defender
            let cheapest_attacker = our_attacks_on_def
                .into_iter()
                .filter_map(|a| board.role_at(a))
                .map(piece_value)
                .min()
                .unwrap_or(0);

            // Check if capturing the defender is profitable:
            // defender is undefended, or our cheapest attacker is cheaper
            let def_is_defended = !attackers_to(board, def_sq, !color).is_empty();
            if !def_is_defended || cheapest_attacker < def_value {
                // After removing this defender, would the target be vulnerable?
                let remaining = defenders.count() - 1;
                if remaining == 0 || remaining < our_attackers_on_target.count() {
                    findings.push(Finding::new(IMPORTANT, format!(
                        "Capturing {}'s {} on {} removes defense of the {} on {}",
                        color_name(!color),
                        piece_name(def_role),
                        def_sq,
                        piece_name(target_role),
                        target_sq
                    )));
                }
            }
        }
    }
}

/// Detects deflection/overloaded defender patterns: an enemy piece is the sole
/// defender of 2+ valuable pieces. Forcing it to move (deflection) would leave
/// at least one of those pieces undefended.
fn detect_deflection(board: &Board, color: Color, findings: &mut Vec<Finding>) {
    let occupied = board.occupied();
    let their_pieces = board.by_color(!color);
    let their_non_king = their_pieces & !board.by_role(Role::King);

    // For each enemy piece, check how many valuable pieces it solely defends
    for def_sq in their_non_king {
        let def_role = match board.role_at(def_sq) {
            Some(r) => r,
            None => continue,
        };

        // Compute what squares this defender attacks (= squares it defends)
        let def_attacks = match def_role {
            Role::Pawn => pawn_attacks_from(!color, def_sq),
            Role::Knight => attacks::knight_attacks(def_sq),
            Role::Bishop => attacks::bishop_attacks(def_sq, occupied),
            Role::Rook => attacks::rook_attacks(def_sq, occupied),
            Role::Queen => attacks::queen_attacks(def_sq, occupied),
            Role::King => continue,
        };

        // Find valuable friendly pieces it defends that are also attacked by us
        let defended_friendlies = def_attacks & their_non_king;
        let mut sole_duties: Vec<(Square, Role)> = Vec::new();

        for target_sq in defended_friendlies {
            let target_role = match board.role_at(target_sq) {
                Some(r) => r,
                None => continue,
            };
            if piece_value(target_role) < BISHOP_VALUE {
                continue;
            }

            // Target must be attacked by the opponent (us) for deflection to matter
            let opp_attackers = attackers_to(board, target_sq, color);
            if opp_attackers.is_empty() {
                continue;
            }

            // Is this piece the SOLE defender of the target?
            let all_defenders = attackers_to(board, target_sq, !color);
            if all_defenders.count() == 1 {
                sole_duties.push((target_sq, target_role));
            }
        }

        // Overloaded: sole defender of 2+ valuable pieces
        if sole_duties.len() >= 2 {
            let duty_names: Vec<String> = sole_duties
                .iter()
                .map(|(sq, role)| format!("{} on {}", piece_name(*role), sq))
                .collect();
            findings.push(Finding::new(IMPORTANT, format!(
                "{}'s {} on {} is the sole defender of {} — vulnerable to deflection",
                color_name(!color),
                piece_name(def_role),
                def_sq,
                duty_names.join(" and ")
            )));
        }
    }
}

/// Detects interference opportunities: a square exists on an enemy slider's
/// defensive ray where one of our pieces could move, blocking that defense
/// and leaving the defended piece unprotected.
fn detect_interference(board: &Board, color: Color, findings: &mut Vec<Finding>) {
    let occupied = board.occupied();
    let their_pieces = board.by_color(!color);

    // Enemy diagonal sliders (bishop + queen)
    let their_diag = (board.by_role(Role::Bishop) | board.by_role(Role::Queen)) & their_pieces;
    // Enemy straight sliders (rook + queen)
    let their_straight = (board.by_role(Role::Rook) | board.by_role(Role::Queen)) & their_pieces;

    // Check diagonal defensive rays
    for slider_sq in their_diag {
        let slider_role = match board.role_at(slider_sq) {
            Some(r) => r,
            None => continue,
        };
        let slider_attacks = attacks::bishop_attacks(slider_sq, occupied);
        let defended_allies = slider_attacks & their_pieces;

        for defended_sq in defended_allies {
            let defended_role = match board.role_at(defended_sq) {
                Some(r) => r,
                None => continue,
            };
            // Only care about valuable defended pieces
            if piece_value(defended_role) < BISHOP_VALUE {
                continue;
            }

            // Must be the sole defender (blocking removes all defense)
            let all_defenders = attackers_to(board, defended_sq, !color);
            if all_defenders.count() != 1 {
                continue;
            }

            // Target must be attacked by us for interference to matter
            let our_attackers = attackers_to(board, defended_sq, color);
            if our_attackers.is_empty() {
                continue;
            }

            // Find empty squares on the ray between slider and defended piece
            let between = attacks::between(slider_sq, defended_sq);
            for block_sq in between {
                if occupied.contains(block_sq) {
                    continue; // Square is occupied, can't place a piece there
                }
                // Can any of our pieces move to this blocking square?
                let our_pieces_reaching = attackers_to(board, block_sq, color);
                if !our_pieces_reaching.is_empty() {
                    findings.push(Finding::new(IMPORTANT, format!(
                        "A piece on {} would block {}'s {} from defending the {} on {}",
                        block_sq,
                        color_name(!color),
                        piece_name(slider_role),
                        piece_name(defended_role),
                        defended_sq
                    )));
                    break; // One finding per defensive ray
                }
            }
        }
    }

    // Check straight defensive rays
    for slider_sq in their_straight {
        let slider_role = match board.role_at(slider_sq) {
            Some(r) => r,
            None => continue,
        };
        let slider_attacks = attacks::rook_attacks(slider_sq, occupied);
        let defended_allies = slider_attacks & their_pieces;

        for defended_sq in defended_allies {
            let defended_role = match board.role_at(defended_sq) {
                Some(r) => r,
                None => continue,
            };
            if piece_value(defended_role) < BISHOP_VALUE {
                continue;
            }

            let all_defenders = attackers_to(board, defended_sq, !color);
            if all_defenders.count() != 1 {
                continue;
            }

            let our_attackers = attackers_to(board, defended_sq, color);
            if our_attackers.is_empty() {
                continue;
            }

            let between = attacks::between(slider_sq, defended_sq);
            for block_sq in between {
                if occupied.contains(block_sq) {
                    continue;
                }
                let our_pieces_reaching = attackers_to(board, block_sq, color);
                if !our_pieces_reaching.is_empty() {
                    findings.push(Finding::new(IMPORTANT, format!(
                        "A piece on {} would block {}'s {} from defending the {} on {}",
                        block_sq,
                        color_name(!color),
                        piece_name(slider_role),
                        piece_name(defended_role),
                        defended_sq
                    )));
                    break;
                }
            }
        }
    }
}

/// Detects weak back rank: a king on its back rank with no escape squares
/// (blocked by own pieces or attacked by enemy) while the opponent has
/// heavy pieces (rook/queen) that could exploit the weakness.
fn detect_weak_back_rank(board: &Board, color: Color, findings: &mut Vec<Finding>) {
    let king_sq = match (board.by_color(color) & board.by_role(Role::King))
        .into_iter()
        .next()
    {
        Some(sq) => sq,
        None => return,
    };

    // King must be on the back rank (rank 1 for White, rank 8 for Black)
    let back_rank: u32 = match color {
        Color::White => 0,
        Color::Black => 7,
    };
    if u32::from(king_sq) / 8 != back_rank {
        return;
    }

    // Determine which rank the king would escape to
    let second_rank: u32 = match color {
        Color::White => 1,
        Color::Black => 6,
    };

    // Check if the king has any available escape squares on the second rank
    let king_moves = attacks::king_attacks(king_sq);
    let our_pieces = board.by_color(color);
    let mut has_escape = false;
    let mut has_second_rank_square = false;

    for sq in king_moves {
        if u32::from(sq) / 8 != second_rank {
            continue;
        }
        has_second_rank_square = true;

        // Square is available if not blocked by own piece and not attacked by enemy
        if !our_pieces.contains(sq) && attackers_to(board, sq, !color).is_empty() {
            has_escape = true;
            break;
        }
    }

    // If no second-rank squares exist (corner edge case) or king has an escape, skip
    if !has_second_rank_square || has_escape {
        return;
    }

    // Only report if the opponent has heavy pieces that could deliver a back rank mate
    let enemy_heavy =
        (board.by_role(Role::Rook) | board.by_role(Role::Queen)) & board.by_color(!color);
    if enemy_heavy.is_empty() {
        return;
    }

    findings.push(Finding::new(CRITICAL, format!(
        "{}'s king has a weak back rank — no escape squares",
        color_name(color)
    )));
}

/// Detects batteries: two same-color sliders lined up on the same file, rank,
/// or diagonal with no pieces between them, creating combined firepower.
fn detect_battery(board: &Board, color: Color, findings: &mut Vec<Finding>) {
    let occupied = board.occupied();
    let our_pieces = board.by_color(color);
    let side = color_name(color);

    // Straight-line sliders (rook + queen) — batteries on files/ranks
    let straight_sliders: Vec<Square> =
        ((board.by_role(Role::Rook) | board.by_role(Role::Queen)) & our_pieces)
            .into_iter()
            .collect();

    for i in 0..straight_sliders.len() {
        for j in (i + 1)..straight_sliders.len() {
            let sq1 = straight_sliders[i];
            let sq2 = straight_sliders[j];

            // Must be on the same file or rank
            if !attacks::rook_attacks(sq1, Bitboard::EMPTY).contains(sq2) {
                continue;
            }

            // No pieces between them
            let between = attacks::between(sq1, sq2);
            if !(between & occupied).is_empty() {
                continue;
            }

            let role1 = board.role_at(sq1).unwrap();
            let role2 = board.role_at(sq2).unwrap();
            findings.push(Finding::new(IMPORTANT, format!(
                "{}'s {} on {} and {} on {} form a battery",
                side,
                piece_name(role1),
                sq1,
                piece_name(role2),
                sq2
            )));
        }
    }

    // Diagonal sliders (bishop + queen) — batteries on diagonals
    let diag_sliders: Vec<Square> =
        ((board.by_role(Role::Bishop) | board.by_role(Role::Queen)) & our_pieces)
            .into_iter()
            .collect();

    for i in 0..diag_sliders.len() {
        for j in (i + 1)..diag_sliders.len() {
            let sq1 = diag_sliders[i];
            let sq2 = diag_sliders[j];

            if !attacks::bishop_attacks(sq1, Bitboard::EMPTY).contains(sq2) {
                continue;
            }

            let between = attacks::between(sq1, sq2);
            if !(between & occupied).is_empty() {
                continue;
            }

            let role1 = board.role_at(sq1).unwrap();
            let role2 = board.role_at(sq2).unwrap();

            // Skip bishop-bishop on same diagonal (not a meaningful battery)
            if role1 == Role::Bishop && role2 == Role::Bishop {
                continue;
            }

            findings.push(Finding::new(IMPORTANT, format!(
                "{}'s {} on {} and {} on {} form a battery",
                side,
                piece_name(role1),
                sq1,
                piece_name(role2),
                sq2
            )));
        }
    }
}

/// Detects desperado situations: a piece that is doomed (hanging or profitably
/// attacked by a cheaper piece) but can capture an enemy piece before being
/// taken, recovering some material.
fn detect_desperado(board: &Board, color: Color, findings: &mut Vec<Finding>) {
    let occupied = board.occupied();
    // Only check pieces worth >= knight (skip pawns and kings)
    let candidates =
        board.by_color(color) & !board.by_role(Role::King) & !board.by_role(Role::Pawn);
    let their_pieces = board.by_color(!color);

    for sq in candidates {
        let role = match board.role_at(sq) {
            Some(r) => r,
            None => continue,
        };
        let value = piece_value(role);

        let attackers = attackers_to(board, sq, !color);
        if attackers.is_empty() {
            continue;
        }

        let defenders = attackers_to(board, sq, color);
        let is_undefended = defenders.is_empty();

        // Cheapest enemy attacker value
        let cheapest_attacker = attackers
            .into_iter()
            .filter_map(|a| board.role_at(a))
            .map(piece_value)
            .min()
            .unwrap_or(0);

        // Piece is "doomed" if undefended or profitably attacked by a cheaper piece
        let profitably_attacked = cheapest_attacker < value;
        if !is_undefended && !profitably_attacked {
            continue;
        }

        // Only report desperado if the piece is also trapped (no safe escape squares)
        if !is_piece_trapped(board, sq, color) {
            continue;
        }

        // What enemy pieces can this doomed piece capture?
        let piece_attacks = match role {
            Role::Knight => attacks::knight_attacks(sq),
            Role::Bishop => attacks::bishop_attacks(sq, occupied),
            Role::Rook => attacks::rook_attacks(sq, occupied),
            Role::Queen => attacks::queen_attacks(sq, occupied),
            _ => continue,
        };

        let capturable = piece_attacks & their_pieces & !board.by_role(Role::King);
        if capturable.is_empty() {
            continue;
        }

        // Report the most valuable capture available
        let best_capture = capturable
            .into_iter()
            .filter_map(|s| board.role_at(s).map(|r| (s, r)))
            .max_by_key(|(_, r)| piece_value(*r));

        if let Some((cap_sq, cap_role)) = best_capture {
            findings.push(Finding::new(IMPORTANT, format!(
                "{}'s {} on {} is under threat but can capture the {} on {} (desperado)",
                color_name(color),
                piece_name(role),
                sq,
                piece_name(cap_role),
                cap_sq
            )));
        }
    }
}

/// Returns all pieces of `color` that are currently attacked by `!color`.
/// Each finding describes the attacked piece and the cheapest (most threatening) attacker.
/// Used in the PV walk to surface newly attacked pieces at each checkpoint.
pub fn get_attacked_pieces(board: &Board, color: Color) -> Vec<Finding> {
    let pieces = board.by_color(color) & !board.by_role(Role::King);
    let mut result = Vec::new();
    for sq in pieces {
        let role = match board.role_at(sq) {
            Some(r) => r,
            None => continue,
        };
        let atk = attackers_to(board, sq, !color);
        if atk.is_empty() {
            continue;
        }
        // Show the cheapest attacker — it represents the most dangerous immediate capture
        if let Some((atk_sq, atk_role)) = atk
            .into_iter()
            .filter_map(|a| board.role_at(a).map(|r| (a, r)))
            .min_by_key(|(_, r)| piece_value(*r))
        {
            result.push(Finding::new(
                POSITIONAL,
                format!(
                    "{}'s {} on {} attacks {} on {}",
                    color_name(!color),
                    piece_name(atk_role),
                    atk_sq,
                    piece_name(role),
                    sq,
                ),
            ));
        }
    }
    result
}

/// Detects stalemate: the side to move has no legal moves and is not in check.
fn detect_stalemate(pos: &Chess, findings: &mut Vec<Finding>) {
    if !pos.is_check() && pos.legal_moves().is_empty() {
        findings.push(Finding::new(CRITICAL, format!(
            "{} is in stalemate — the game is a draw",
            color_name(pos.turn())
        )));
    }
}

/// Runs all tactical pattern detections on the current position.
/// Returns a list of findings with priority tiers.
pub fn detect_all_tactics(pos: &Chess) -> Vec<Finding> {
    let mut findings = Vec::new();
    let board = pos.board();

    // Position-level detections (not per-color)
    detect_double_check(pos, &mut findings);
    detect_stalemate(pos, &mut findings);

    // Run all detections for both sides so we never miss patterns
    for &color in &[Color::White, Color::Black] {
        detect_hanging(pos, color, &mut findings);
        detect_forks(board, color, &mut findings);
        detect_pins(board, color, !color, &mut findings);
        detect_skewers(board, color, &mut findings);
        detect_trapped_piece(board, color, &mut findings);
        detect_xray_attack(board, color, &mut findings);
        detect_removing_the_defender(board, color, &mut findings);
        detect_deflection(board, color, &mut findings);
        detect_interference(board, color, &mut findings);
        detect_weak_back_rank(board, color, &mut findings);
        detect_battery(board, color, &mut findings);
        detect_desperado(board, color, &mut findings);
    }

    findings
}
