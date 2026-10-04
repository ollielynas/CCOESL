# Go

Play the board game Go against **GNU Go**, a computer opponent that runs on the server. You
play one colour and GNU Go plays the other.

## Playing

Click an empty point to put your stone there. GNU Go then thinks for a moment and plays its
stone; while it does, the line above the board says **GNU Go is thinking…**.

- A stone or group with no empty points next to it is captured and taken off the board. The
  line under the board counts how many stones each side has captured.
- The last stone played is highlighted, and named under the board (for example **E5**).
- If a point isn't allowed, the app says why and nothing is played:
  - **there is already a stone there**;
  - **that would leave your stones with no liberties**: a stone can't be placed where it would
    be captured straight away, unless it captures something;
  - **ko**: you can't take back a single stone straight after it was taken from you. Play
    somewhere else first, then you may.

The board is labelled like a real one: columns **A** to **T** (there is no **I**) and rows
counted from the bottom.

## The buttons

- **Pass** gives up your turn. When both players pass one after the other, the game ends and is
  counted.
- **Resign** gives the game to GNU Go.
- **Undo** takes back your last move and GNU Go's reply to it.
- **New game** opens the settings for a new game.

## Ending and counting

After two passes in a row, GNU Go counts the game: your territory and stones against its own,
plus komi. Stones it judges dead are marked **×** and counted as captured. The result is shown
above the board, for example **White wins by 7.5 points (GNU Go)**. GNU Go may also resign if
it is clearly losing.

## A new game

**New game** lets you choose:

- **Board**: 9×9 (a short game, good to learn on), 13×13, or 19×19 (the full game).
- **You play**: Black moves first, unless there are handicap stones.
- **Handicap**: extra Black stones placed on the board at the start, to make the game fairer
  between players of different strength. With a handicap, White moves first. 9×9 takes up to 5;
  the bigger boards up to 9.
- **Komi**: points given to White to make up for Black moving first, in half points. 6.5 is
  usual; with a handicap, 0.5 is common.
- **GNU Go's strength**: from 1 (weakest and quickest) to 10 (strongest). On a 19×19 board the
  strongest levels can take several seconds a move.

Press **Start** to begin.

## Saving and opening games

Type a file's path in **File** (for example `/home/you/game.sgf`), then:

- **Save** writes the game as an `.sgf` file, the format Go programs share, so you can look at
  it in other Go software too.
- **Open** reads an `.sgf` file and carries on from where it left off. You take whichever
  colour is to move.

Only the main line of a game is read; variations are left out. Games whose handicap stones
aren't on the usual points, or whose moves break the rules, can't be opened.

You can save only where you are allowed to change files. Read about
[who can see what](docs.md) in the Docs page.
