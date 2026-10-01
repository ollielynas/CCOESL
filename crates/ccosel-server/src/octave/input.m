## Stands in for Octave's own `input` in CCOSEL sessions. Octave's input there is the server's
## command channel, so nothing can answer it: the script would wait until its time limit.
## It stops with an error instead, at once.
##
## Loaded from the server's support folder, ahead of the built-in: edit this file, not a copy.
function varargout = input (varargin)
  error ("Octave:ccosel-input", ["input can't be answered here, so the script stopped at that ", ...
         "point. Set the value in the script, or in the Command Window, instead."]);
endfunction
