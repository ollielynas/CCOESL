## Stands in for Octave's own `quit` in CCOSEL sessions, which would end the user's Octave and
## lose every variable. A script that calls it stops there instead, as it would on an error.
##
## Loaded from the server's support folder, ahead of the built-in: edit this file, not a copy.
function quit (varargin)
  error ("Octave:ccosel-exit", ["quit doesn't end Octave here: the script stopped at that ", ...
         "point and your variables are kept. To start again from empty, press Restart."]);
endfunction
