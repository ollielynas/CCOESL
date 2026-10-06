use super::*;
use alloc::vec;

/// Counts the leaf positions `depth` moves ahead: the standard check that move generation is
/// exactly right, against numbers every chess programmer uses.
fn perft(pos: &Position, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    pos.legal_moves()
        .into_iter()
        .map(|m| perft(&pos.play(m), depth - 1))
        .sum()
}

fn fen(s: &str) -> Position {
    Position::from_fen(s).unwrap()
}

#[test]
fn perft_from_the_start() {
    let pos = Position::default();
    assert_eq!(perft(&pos, 1), 20);
    assert_eq!(perft(&pos, 2), 400);
    assert_eq!(perft(&pos, 3), 8_902);
}

/// "Kiwipete": castling both ways, en passant, promotions and pins all at once.
#[test]
fn perft_kiwipete() {
    let pos = fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1");
    assert_eq!(perft(&pos, 1), 48);
    assert_eq!(perft(&pos, 2), 2_039);
    assert_eq!(perft(&pos, 3), 97_862);
}

#[test]
fn perft_endgame_with_en_passant_pins() {
    let pos = fen("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1");
    assert_eq!(perft(&pos, 1), 14);
    assert_eq!(perft(&pos, 2), 191);
    assert_eq!(perft(&pos, 3), 2_812);
    assert_eq!(perft(&pos, 4), 43_238);
}

#[test]
fn perft_promotions_and_checks() {
    let pos = fen("r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1");
    assert_eq!(perft(&pos, 1), 6);
    assert_eq!(perft(&pos, 2), 264);
    assert_eq!(perft(&pos, 3), 9_467);
    let pos = fen("rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8");
    assert_eq!(perft(&pos, 1), 44);
    assert_eq!(perft(&pos, 2), 1_486);
    assert_eq!(perft(&pos, 3), 62_379);
}

#[test]
fn fen_reads_and_writes_the_same() {
    for s in [
        START_FEN,
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        "rnbqkbnr/pp1ppppp/8/2p5/4P3/8/PPPP1PPP/RNBQKBNR w KQkq c6 0 2",
        "8/8/8/8/8/8/8/K6k b - - 99 140",
    ] {
        assert_eq!(fen(s).fen(), s);
    }
    for bad in [
        "",
        "8/8/8/8/8/8/8 w - - 0 1",
        "9/8/8/8/8/8/8/K6k w - - 0 1",
        "8/8/8/8/8/8/8/K5xk w - - 0 1",
        "8/8/8/8/8/8/8/K7 w - - 0 1",
        "8/8/8/8/8/8/8/K6k x - - 0 1",
        "8/8/8/8/8/8/8/K6k w - z9 0 1",
        // Black to move would be giving check to a king that isn't to move.
        "k6R/8/8/8/8/8/8/K7 w - - 0 1",
    ] {
        assert!(Position::from_fen(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn coordinates_and_square_names() {
    assert_eq!(square_name(0), "a1");
    assert_eq!(square_name(63), "h8");
    assert_eq!(parse_square("e4"), Some(28));
    assert_eq!(parse_square("i1"), None);
    let m = Move::parse_coordinate("e7e8q").unwrap();
    assert_eq!(m.promotion, Some(Kind::Queen));
    assert_eq!(m.coordinate(), "e7e8q");
    for bad in ["e2", "e2e9", "e7e8k", "e7e8p", "e7e8x", "e2e4e5"] {
        assert_eq!(Move::parse_coordinate(bad), None, "{bad:?}");
    }
    let pos = Position::default();
    assert!(pos.legal_coordinate("e2e4").is_some());
    assert!(pos.legal_coordinate("e2e5").is_none());
}

fn play(game: &mut Game, sans: &[&str]) {
    for san in sans {
        let m = game
            .position()
            .parse_san(san)
            .unwrap_or_else(|| panic!("{san}"));
        assert!(game.play(m));
    }
}

#[test]
fn san_names_moves_as_people_write_them() {
    let mut g = Game::default();
    play(
        &mut g,
        &["e4", "d5", "exd5", "Nf6", "Nc3", "Nxd5", "Nxd5", "Qxd5"],
    );
    assert_eq!(
        g.san_moves(),
        ["e4", "d5", "exd5", "Nf6", "Nc3", "Nxd5", "Nxd5", "Qxd5"]
    );
    // Two knights can reach d2: the file says which.
    let pos = fen("4k3/8/8/8/8/8/8/1N2KN2 w - - 0 1");
    let m = pos.parse_san("Nbd2").unwrap();
    assert_eq!(m.from, parse_square("b1").unwrap());
    assert_eq!(pos.san(m), "Nbd2");
    // Two rooks on one file: the rank says which.
    let pos = fen("R7/8/8/8/8/8/8/R3K1k1 w - - 0 1");
    let m = pos.parse_san("R1a4").unwrap();
    assert_eq!(pos.san(m), "R1a4");
    // Three queens: sometimes only the whole square will do.
    let pos = fen("1k6/8/8/8/4Q2Q/8/8/K6Q w - - 0 1");
    let m = Move {
        from: parse_square("h4").unwrap(),
        to: parse_square("e1").unwrap(),
        promotion: None,
    };
    assert_eq!(pos.san(m), "Qh4e1");
}

#[test]
fn castling_en_passant_and_promotion() {
    // Castling both ways, and the rook moves with the king.
    let pos = fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1");
    let short = pos.parse_san("O-O").unwrap();
    let after = pos.play(short);
    assert_eq!(
        after.at(parse_square("f1").unwrap()).unwrap().kind,
        Kind::Rook
    );
    assert!(!after.castling.white_king && !after.castling.white_queen);
    assert!(pos.parse_san("0-0-0").is_some(), "zeros are read as O");
    // Not through an attacked square.
    let pos = fen("r3k2r/8/8/8/8/8/5r2/R3K2R w KQkq - 0 1");
    assert!(pos.parse_san("O-O").is_none());
    assert!(pos.parse_san("O-O-O").is_some());
    // Not out of check.
    let pos = fen("r3k2r/8/8/8/8/8/4r3/R3K2R w KQkq - 0 1");
    assert!(pos.parse_san("O-O").is_none() && pos.parse_san("O-O-O").is_none());

    // En passant takes the pawn beside, and only straight after its double step.
    let mut g = Game::default();
    play(&mut g, &["e4", "a6", "e5", "d5"]);
    assert_eq!(g.position().en_passant, parse_square("d6"));
    play(&mut g, &["exd6"]);
    assert!(g.position().at(parse_square("d5").unwrap()).is_none());
    assert_eq!(g.san_moves()[4], "exd6");

    // Promotion to any piece but a king, with a check.
    let pos = fen("4k3/1P6/8/8/8/8/8/4K3 w - - 0 1");
    let promotions: Vec<_> = pos
        .legal_moves()
        .into_iter()
        .filter_map(|m| m.promotion)
        .collect();
    assert_eq!(
        promotions,
        vec![Kind::Queen, Kind::Rook, Kind::Bishop, Kind::Knight]
    );
    let m = pos.parse_san("b8=Q+").unwrap();
    assert_eq!(pos.san(m), "b8=Q+");
}

#[test]
fn the_ways_a_game_ends() {
    let mut g = Game::default();
    play(&mut g, &["f3", "e5", "g4", "Qh4"]);
    assert_eq!(g.outcome(), Some(Outcome::Checkmate(Color::White)));
    assert_eq!(g.outcome().unwrap().result(), "0-1");
    assert_eq!(g.san_moves()[3], "Qh4#");
    // Nothing more can be played.
    assert!(!g.play(Move::parse_coordinate("a2a3").unwrap()));

    let stalemate = fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1");
    assert!(stalemate.legal_moves().is_empty() && !stalemate.in_check());
    assert!(fen("8/8/8/8/8/8/8/KB5k w - - 0 1").insufficient_material());
    assert!(!fen("7k/8/8/8/8/8/8/KR6 w - - 0 1").insufficient_material());

    let mut g = Game::default();
    play(
        &mut g,
        &["Nf3", "Nf6", "Ng1", "Ng8", "Nf3", "Nf6", "Ng1", "Ng8"],
    );
    assert_eq!(g.outcome(), Some(Outcome::Repetition));
    assert_eq!(Outcome::Repetition.result(), "1/2-1/2");
    assert_eq!(Outcome::Checkmate(Color::Black).result(), "1-0");
}

#[test]
fn undo_takes_back_the_last_move() {
    let mut g = Game::default();
    play(&mut g, &["e4", "e5"]);
    assert_eq!(g.undo().map(|m| m.coordinate()), Some("e7e5".into()));
    assert_eq!(g.moves().len(), 1);
    assert_eq!(g.position().side, Color::Black);
    g.undo();
    assert!(g.undo().is_none());
    assert_eq!(*g.position(), Position::default());
}

#[test]
fn pieces_have_glyphs_and_letters() {
    let p = Piece {
        color: Color::Black,
        kind: Kind::Knight,
    };
    assert_eq!(p.glyph(), '♞');
    assert_eq!(Kind::from_letter('q'), Some(Kind::Queen));
    assert_eq!(Kind::from_letter('x'), None);
    assert_eq!(Color::White.other(), Color::Black);
}

#[test]
fn pgn_round_trip() {
    let mut g = Game::default();
    play(&mut g, &["e4", "e5", "Qh5", "Nc6", "Bc4", "Nf6", "Qxf7"]);
    let tags = pgn::Tags {
        white: "Ann".into(),
        black: "GNU Chess".into(),
        date: "2026.10.07".into(),
    };
    let text = pgn::write(&g, &tags);
    assert!(text.contains("[Result \"1-0\"]"));
    assert!(text.contains("1. e4 e5 2. Qh5 Nc6 3. Bc4 Nf6 4. Qxf7# 1-0"));
    let (again, read_tags) = pgn::read(&text).unwrap();
    assert_eq!(again, g);
    assert_eq!(read_tags, tags);
}

#[test]
fn pgn_reading_skips_what_isnt_a_move() {
    let text = "[Event \"x\"]\n[White \"A\"]\n\n1. e4 {best by test} e5 2. Nf3 (2. f4 exf4) \
                2... Nc6 $1 ; a line comment\n3.Bb5 a6 *\n";
    let (g, tags) = pgn::read(text).unwrap();
    assert_eq!(g.san_moves(), ["e4", "e5", "Nf3", "Nc6", "Bb5", "a6"]);
    assert_eq!(tags.white, "A");
    // Castling written with zeros, and an unknown game's tags.
    let (g, tags) = pgn::read("1. e4 e5 2. Nf3 Nf6 3. Bc4 Bc5 4. 0-0 0-0 1/2-1/2").unwrap();
    assert_eq!(g.san_moves()[6], "O-O");
    assert_eq!(tags, pgn::Tags::default());
    let written = pgn::write(&Game::default(), &pgn::Tags::default());
    assert!(written.contains("[Date \"????.??.??\"]") && written.contains("[White \"?\"]"));
    assert!(written.ends_with("*\n"));
    let resigned = pgn::write_with_result(&Game::default(), &pgn::Tags::default(), "0-1");
    assert!(resigned.contains("[Result \"0-1\"]") && resigned.ends_with("0-1\n"));
}

#[test]
fn pgn_reading_refuses_what_it_cannot_follow() {
    assert!(pgn::read("1. e5").unwrap_err().contains("e5"));
    assert!(pgn::read("[FEN \"8/8/8/8/8/8/8/K6k w - - 0 1\"]\n1. Ka2").is_err());
    assert!(pgn::read("[Event \"never closed\"\n1. e4").is_err());
}

#[test]
fn long_games_wrap_their_moves() {
    let mut g = Game::default();
    play(
        &mut g,
        &[
            "Nf3", "Nf6", "Nc3", "Nc6", "Ng1", "Ng8", "Nb1", "Nb8", "e4", "e5", "Nf3", "Nf6",
            "Nc3", "Nc6", "Bb5", "Bb4", "O-O", "O-O", "d3", "d6",
        ],
    );
    let text = pgn::write(&g, &pgn::Tags::default());
    assert!(text.lines().all(|l| l.len() < 80));
    assert_eq!(pgn::read(&text).unwrap().0, g);
}
