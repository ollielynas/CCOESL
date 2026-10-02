## Stands in for Octave's own `drawnow` in CCOSEL sessions: draws as the built-in does, then
## shows what changed in the app's Command Window (`__ccosel_live__`), as a window would.
##
## Loaded from the server's support folder, ahead of the built-in: edit this file, not a copy.
function drawnow (varargin)
  builtin ("drawnow", varargin{:});
  __ccosel_live__ (false);
endfunction
