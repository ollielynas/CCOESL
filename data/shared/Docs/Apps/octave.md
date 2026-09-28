# Octave

Run [GNU Octave](https://octave.org) code on the server: calculate, keep your variables from one
command to the next, write scripts, and plot. Octave is a language for numbers and matrices, and
most MATLAB code runs in it too.

Your code runs on the server, not on this computer, so heavy calculations don't slow it down.
Each person gets their own Octave: nobody else sees your variables.

## The command window

Type code next to **>>** and press **⏎ Run**. What Octave prints appears above, the same as
at Octave's own prompt: `x = 3` shows `x = 3`, and ending a line with `;` keeps it quiet.
Errors are shown in *italics*.

- **▲** and **▼** bring back earlier commands, so you can run one again or change it.
- **Clear** empties the command window. Your variables are kept.
- While something runs, **⏳ Running…** shows at the top. One command runs at a time; wait for
  it to finish before running the next.

A command that runs for more than ten minutes is stopped, and the session starts again from
empty. Commands that wait for you to type something, such as `input` or `keyboard`, can't be
answered here, so avoid them.

## The working folder

The folder at the top, next to 📁, is where Octave reads and writes files: `load`, `save`,
`print` and scripts you run by name all use it. It starts as your own folder, `/home/your-name`.
Change it with `cd` at the prompt, for example `cd /Docs`.

## Workspace

The left side lists your variables: their name, size (such as `3x3`) and class (such as
`double` or `char`). Small values are shown underneath. Hover over a variable to see whether it
is `global` or `persistent`.

Click a variable to see all of it in the **Variable** tab.

## Command History

Under the workspace is everything you have run, newest first. Type in the box to search it.

- Click a command to put it back in the prompt.
- **▶** runs it again straight away.
- **📝 Create script** opens the commands listed (all of them, or just the ones your search
  matched) in the editor, oldest first, as a new script.

The history belongs to this window: closing it forgets the history, not your variables.

## Editor

The **Editor** tab is for scripts: several lines of code you want to keep.

- Type a path in **File**, such as `/home/your-name/fit.m`, and press **📂 Open** to load it,
  or **💾 Save** to save what you have written there.
- **▶ Run** saves the file and runs it. Without a file name, it just runs the code.
- **New** starts an empty script.

Files you can read but not change (such as the ones in `/Docs`) open read-only: you can run
them, but not save over them.

## Figures

Plots, such as `plot(x, sin(x))`, appear in the **Figures** tab after the command that draws
them. Each line is drawn as its own graph, and the lines in one plot share the same scale, so
they still compare. The range of each axis is written above and below.

Only line plots are drawn in the window. For anything else, such as bar charts or surfaces,
press **💾 Save PNG**: it saves the figure as `figureN.png` in the working folder, and you
can open it from the Files app.

`close all` removes every figure.

## Starting again

**⟳ Restart** starts a fresh Octave: every variable and figure is cleared. Use it if Octave
seems stuck, or to start from a clean slate. Octave also restarts by itself if it stops, for
example after `exit`, or after a command runs too long.

If the window says **Octave isn't installed on this server**, the server doesn't have Octave;
ask whoever runs it.
