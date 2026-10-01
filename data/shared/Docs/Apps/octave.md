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

Commands that wait for something nobody here can do never finish on their own, so stop them, or
avoid them:

- waiting for you to type, such as `input` or `keyboard`;
- waiting for figure windows to be closed, such as
  `while ! isempty(get(0, "children")), pause(0.2); end`. Figures here have no windows to close.
  Look at them in the **Figures** tab instead.

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

The **Editor** tab is for scripts: several lines of code you want to keep.

- Type a path in **File**, such as `/home/your-name/fit.m`, and press **:folder_open: Open** to load it,
  or **:floppy_disk: Save** to save what you have written there.
- **:play: Run** saves the file and runs it. Without a file name, it runs what is in the editor as
  one script, from a temporary copy that isn't kept; the command window shows
  `run (unsaved script)`. Save it to a file to keep it.
- **New** starts an empty script.

Files you can read but not change (such as the ones in `/Docs`) open read-only: you can run
them, but not save over them.

## Figures

Plots, such as `plot(x, sin(x))`, appear in the **Figures** tab after the command that draws
them. Each line is drawn as its own graph, and the lines in one plot share the same scale, so
they still compare. The range of each axis is written above and below.

Only line plots are drawn in the window. For anything else, such as bar charts or surfaces,
press **:floppy_disk: Save PNG**: it saves the figure as `figureN.png` in the working folder, and you
can open it from the Files app.

`close all` removes every figure.

## Starting again

**:arrow_clockwise: Restart** starts a fresh Octave: every variable and figure is cleared. Use it if Octave
seems stuck, or to start from a clean slate. Octave also restarts by itself if it stops, for
example after `exit`, or after a command runs too long.

If the window says **Octave isn't installed on this server**, the server doesn't have Octave;
ask whoever runs it.
