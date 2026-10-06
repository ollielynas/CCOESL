# Chess

Play chess against [GNU Chess](https://www.gnu.org/software/chess/), which runs on the server.

## Starting a game

Press **♙ New game as White** to move first, or **♟ New game as Black** to let GNU Chess open.
Playing Black turns the board round so your pieces are at the bottom; **⇅ Flip board** turns it
either way whenever you like.

Choose how strong GNU Chess plays with **Level**, from **1** (easiest) to **5**. At every level
it answers within about two seconds. You can change the level during a game; it applies from its
next move.

## Making a move

1. Click one of your pieces. It shows in brackets, like `[♘]`, and every square it can move to is
   marked: `•` for an empty square, `×` before a piece it can take.
2. Click where it should go.

Only legal moves are allowed: if you click a square it can't reach, the app says so, and the
piece stays in your hand so you can choose again. Click the piece again to put it down, or click
another of your pieces to pick that one up instead. Hover over any square to see its name, such
as `e4`.

All the rules apply: castling (move the king two squares), en passant, and promotion. When a
pawn reaches the far side, choose what it becomes with **♕ Queen**, **♖ Rook**, **♗ Bishop** or
**♘ Knight**, or **Cancel** to choose a different move.

While GNU Chess is thinking, the app says so and the board waits for its reply.

## Taking moves back

**↶ Undo** takes back your last move and GNU Chess's reply to it, so it is your move again.
Press it again to go further back.

## How a game ends

The line above the board says whose move it is, and when you are in check. It also says when the
game is over:

- **Checkmate**, either way.
- **GNU Chess resigns**, when it judges its position hopeless.
- **A draw**: stalemate, the same position three times, fifty moves each without a capture or a
  pawn move, or too few pieces left for either side to checkmate.

## The moves so far

Under the board, every move of the game is listed in the usual notation, numbered in pairs:
`1. e4 e5  2. Nf3 Nc6`. `x` means a capture, `+` check, `#` checkmate, and `O-O` castling.

## Saving and opening games

Games are saved as `.pgn` files, the format every chess program reads.

- Type a path in **Game file**, such as `/home/your-name/game.pgn`, and press **💾 Save**.
- To carry on with a saved game, type its path and press **📂 Open**. You play the side that GNU
  Chess didn't.

You can open and save files only where you are allowed to. Read about
[who can see what](docs.md) in the Docs page.

## If GNU Chess doesn't answer

If GNU Chess crashes, takes too long, or answers with a move that isn't legal, the app says so
and doesn't play it. Press **Try again** to ask again.
