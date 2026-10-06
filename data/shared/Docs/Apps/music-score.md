# Music Score

Write music as text and see it as sheet music. You type the notes in
[LilyPond](https://lilypond.org) notation, and the server engraves them into pages you can read
here, download as a PDF to print, or download as MIDI to play in another program.

## Writing a score

A new score starts with a short tune you can engrave straight away. Change it in the editor, or
replace it with your own. This is the starter score:

```
\version "2.24.0"
\header { title = "Untitled" }

\score {
  \relative c' {
    \clef treble \time 4/4
    c4 d e f | g2 g | a4 a a a | g1 \bar "|."
  }
  \layout { }
  \midi { }
}
```

- Notes are letters, and the number after one is its length: `c4` is a quarter-note C, `g2` a
  half note, `g1` a whole note. A note with no number is as long as the one before it.
- In `\relative c'`, each note is the one nearest the note before it. Add `'` to go an octave up
  and `,` to go an octave down.
- `|` marks a bar line. LilyPond warns you if a bar doesn't add up.
- `\layout { }` asks for sheet music, and `\midi { }` for a MIDI file. Leave out `\midi { }` if
  you don't want one.

The [LilyPond learning manual](https://lilypond.org/doc/v2.24/Documentation/learning/index) covers
chords, lyrics, several staves, and everything else.

## Engraving

Press **🎼 Engrave**. It takes a few seconds, and the app shows how long it has been going. When
it's done:

- The first page appears under the editor. For a score with more than one page, **◀ Previous**
  and **Next ▶** turn the pages.
- **Download PDF** downloads the score, ready to print.
- **Download MIDI** downloads the music as a MIDI file, when the score has `\midi { }`.

Engraving the same text again shows the same pages at once, without waiting.

### When something is wrong

If LilyPond can't make sense of the score, it lists what is wrong, with the line and column of
each problem, such as `❌ Line 4, column 9: syntax error, unexpected '}'`. Errors (❌) stop the
score from being engraved; warnings (⚠) don't, but are worth a look. Lines are counted from 1 at
the top of the editor.

## Opening and saving

Type a file's path in the **File** box, such as `/home/your-name/song.ly`, then:

- **📂 Open** reads the file into the editor. If you have changes you haven't saved, it asks you
  to press **Open** again first, so you don't lose them by accident.
- **💾 Save** writes the editor to that file, creating it if it isn't there. "(unsaved)" next to
  the buttons means there are changes you haven't saved.

You can open and save files only where you are allowed to. Read about
[who can see what](docs.md) in the Docs page. Your own folder, `/home/your-name`, always works.

## Be careful with scores from other people

A LilyPond file can contain small programs, and engraving it runs them on the server, as the
server. Only engrave scores you wrote yourself or got from someone you trust, the same as you
would only run a program from someone you trust.
