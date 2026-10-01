## Stands in for Octave's own `waitfor` in CCOSEL sessions. It waits for a figure window to be
## closed, but figures there have no window, so it would wait until the job's time limit.
## It carries on at once instead, with a warning saying why.
##
## Loaded from the server's support folder, ahead of the built-in: edit this file, not a copy.
function waitfor (varargin)
  warning ("Octave:ccosel-waitfor", ["waitfor doesn't wait here: figures have no windows to close. ", ...
           "They are shown in the Command Window and the Figures tab."]);
endfunction
