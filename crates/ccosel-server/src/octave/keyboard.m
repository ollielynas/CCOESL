## Stands in for Octave's own `keyboard` in CCOSEL sessions. Octave's input there is the server's
## command channel, so nothing can answer it: the script would wait until its time limit.
## It stops with an error instead, at once.
##
## Loaded from the server's support folder, ahead of the built-in: edit this file, not a copy.
function varargout = keyboard (varargin)
  error ("Octave:ccosel-keyboard", ["keyboard can't be answered here, so the script stopped at that ", ...
         "point. Set the value in the script, or in the Command Window, instead."]);
endfunction
