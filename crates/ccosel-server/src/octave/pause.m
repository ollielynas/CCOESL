## Stands in for Octave's own `pause` in CCOSEL sessions. With a duration, it shows what the
## script has drawn (`__ccosel_live__`), as windows would be showing it, then waits as the
## built-in does. With none, the built-in waits for a key on the terminal, which here is the
## server's command channel: the app asks the user to press Enter instead.
##
## Loaded from the server's support folder, ahead of the built-in: edit this file, not a copy.
function pause (varargin)
  if (nargin == 0)
    __ccosel_prompt__ (true, "INPUT", "Paused: press Enter to carry on ");
    return;
  endif
  if (ischar (varargin{1}))
    ## pause ("on"), pause ("off"), pause ("query"): settings, not waits.
    builtin ("pause", varargin{:});
    return;
  endif
  __ccosel_live__ (true);
  builtin ("pause", varargin{:});
endfunction
