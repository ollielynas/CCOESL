# Octave

Run [GNU Octave](https://octave.org) code on the server: calculate, keep your variables from one
command to the next, write scripts, and plot. Octave is a language for numbers and matrices, and
most MATLAB code runs in it too.

Your code runs on the server, not on this computer, so heavy calculations don't slow it down.
Each person gets their own Octave: nobody else sees your variables.

## The command window

The prompt, **>>**, is always at the bottom of the command window. Type code there and press
**Enter** (or **:play: Run**). What Octave prints appears above it, the same as at Octave's own
prompt: `x = 3` shows `x = 3`, and ending a line with `;` keeps it quiet. Errors are shown in
*italics*. The prompt stays ready for the next command, so you can type one after another.

A command that draws a figure, such as `plot(x, sin(x))` or `surf(peaks)`, shows it right under
the command, where Octave on a desktop would pop up a window. A script that is still running
shows what it has drawn so far at the moments a window would update: at `drawnow` (at most
once a second, so a drawing loop stays fast), at `pause`, and before it asks you anything. So a
script that draws a figure and then asks whether you can see it shows the figure first. When
a figure is drawn again with nothing printed since, its picture is updated where it is, as a
window would be, so an animation plays in one place instead of filling the window. Click **Figure N · open in the
Figures tab** below it to see it larger, with every other figure.

A command that prints a great deal, such as a loop printing thousands of lines, isn't sent
to you line by line: the window shows its first lines, then how many were left out, then its
latest lines, like this:

```
line 1
.
. 4950 lines hidden
.
line 4961
```

While it goes on printing, the count goes up and the lines under it are the latest. To keep
all of it, have the script write to a file instead (`fprintf` to a file, or `diary`). Very long
lines are cut short with "…".

The output scrolls on its own and keeps the newest line in view. Scroll up to read back; it stops
following new output until you scroll to the bottom again.

- **:caret_up:** and **:caret_down:** bring back earlier commands, so you can run one again or change it.
- **:broom: Clear** empties the command window. Your variables are kept.
- While something runs, **:hourglass: Running…** shows at the top. One command runs at a time:
  until it finishes, the buttons that would run something else (Run, Restart, **cd** and the
  others) are greyed out. You can still type the next command; press Enter once it's done.

## Long-running commands

While a command runs, the top of the window shows how long it has run and its time limit, such
as `1:05 of 10:00`. A command gets **10 minutes**. Then the server stops it and starts Octave
again, which clears your variables.

- **:timer: +10 min** and **+1 h** give the running command more time, up to 8 hours in all.
  Use them when you expect a script to take a while.
- **:stop: Stop** interrupts it, like Ctrl-C at Octave's own prompt. Your variables are kept,
  including any the command had set before it stopped.

Figures here have no windows of their own. `waitfor` and `uiwait`, which wait for a window to
be closed, carry on at once with a warning. A loop that waits for every window to close, such as
`while ! isempty(get(0, "children")), pause(0.2); end`, never finishes on its own: stop it.

When a script asks a question with `input`, the question takes the place of **>>** at the
prompt, and the top of the window says **:chat_text: Waiting for your answer**. Type the answer
and press Enter. With `input(question, "s")` the script gets exactly what you typed; otherwise
it is worked out as Octave, so `3 * 2` gives 6, and it can use the script's variables. An empty
answer gives `[]`. The question and your answer stay in the command window, as in a terminal.

`pause` on its own, which waits for a key on a desktop, asks you to press Enter instead.

`keyboard` stops the script at a **K>>** prompt: each line you type there runs with the
script's variables. Type `return` (or `dbcont`) to carry on with the script, or `dbquit` to stop
it there.

A few commands work differently here, because there is no terminal behind the window:

- `exit` and `quit` don't end Octave. The script ends there and your variables are kept; it
  counts as an error only if it gave a status other than 0, such as `exit(1)`. Use
  **:arrow_clockwise: Restart** to start again from empty.
- `have_window_system()` says true, and new figures are visible, since figures are shown here
  even without windows of their own.

## Dialogs

Scripts can still talk to you through dialogs:

- `msgbox`, `errordlg`, `warndlg` and `helpdlg` show their message in a window over the app.
  The script carries on meanwhile, as in MATLAB; press **OK** to close it.
- `uigetfile` opens a file picker over the shared files, starting in Octave's working folder
  (or the folder the script names). It lists folders, and the files that match the script's
  filter, such as `*.csv`. Open folders to look inside, **:arrow_up:** to go up, pick a file
  and press **:folder_open: Open**. The script waits meanwhile, and gets the file's name and
  folder, ready for `load` or `fopen`. **Cancel** gives it `0` for both, as in MATLAB.

You can only pick a file you're allowed to read. Other dialogs, such as `uiputfile`,
`inputdlg` or `questdlg`, don't work here yet.

## The working folder

The folder at the top, next to :folder:, is where Octave reads and writes files: `load`, `save`,
`print` and scripts you run by name all use it. It starts as your own folder, `/home/your-name`.
Change it with `cd` at the prompt, for example `cd /Docs`, or from **Current Folder** below.

## Current Folder

At the top of the left side are the files in the working folder, folders first.

- Click a folder to open or close it, and see what is inside.
- **cd** next to a folder makes it the working folder. **:arrow_up:** goes up to the folder above.
- Click a script (a file ending in `.m`, marked :file_code:) to open it in the **Editor**.
- Other files are listed but can't be opened here; use the Files app for them.

The list updates after each command, so files you save, or that Octave writes, show up.

## Workspace

Below the current folder, the left side lists your variables: their name, size (such as `3x3`) and class (such as
`double` or `char`). Small values are shown underneath. Hover over a variable to see whether it
is `global` or `persistent`.

Click a variable to see all of it in the **Variable** tab.

## Command History

Under the workspace is everything you have run, newest first. Type in the box to search it.

- Click a command to put it back in the prompt.
- **:play:** runs it again straight away.
- **:note_pencil: Create script** opens the commands listed (all of them, or just the ones your search
  matched) in the editor, oldest first, as a new script.

The history belongs to this window: closing it forgets the history, not your variables.

## Editor

The **Editor** tab is for scripts: several lines of code you want to keep. Code is coloured as
you type: keywords such as `if` and `for`, strings, numbers, and comments in italics.

- Type a path in **File**, such as `/home/your-name/fit.m`, and press **:folder_open: Open** to load it,
  or **:floppy_disk: Save** to save what you have written there.
- **:play: Run** saves the file and runs it. Without a file name, it runs what is in the editor as
  one script, from a temporary copy that isn't kept; the command window shows
  `run (unsaved script)`. Save it to a file to keep it.
- **New** starts an empty script.

Files you can read but not change (such as the ones in `/Docs`) open read-only: you can run
them, but not save over them.


### Errors

When a script you run from the editor stops with an error, the app goes back to the editor and
shows where: the line is shaded red and underlined from the point Octave gave, and above the
editor it says, for example, **:x_circle: Line 3, column 12: syntax error …**. The error is in
the command window too.

That works for the file you have open however it was run, even by name at the prompt, and for
unsaved text run from the editor. An error inside another file, such as a function your script
calls, is marked when that file is the one open. The mark goes as soon as you change the text,
or run again.
## Figures

Plots appear in the **Figures** tab, drawn by Octave itself, exactly as `print` would save
them: line plots, bar charts, histograms, surfaces, colour bars, legends and all.

Octave draws them when you open the tab, so it can take a moment, with **:hourglass: Drawing
figures…** at the top meanwhile. Until then, and if a figure can't be drawn, its lines are shown
as simple graphs instead, with each axis's range written above and below. After another command,
the figures are drawn again the next time you look at them, in case it changed them.

**:floppy_disk: Save PNG** saves the figure as `figureN.png` in the working folder, to keep or
open from the Files app.

`close all` removes every figure.

## Starting again

**:arrow_clockwise: Restart** starts a fresh Octave: every variable and figure is cleared. Use it if Octave
seems stuck, or to start from a clean slate. Octave also restarts by itself if it stops, for
example if it crashes, or after a command runs past its time limit.

If the window says **Octave isn't installed on this server**, the server doesn't have Octave;
ask whoever runs it.
