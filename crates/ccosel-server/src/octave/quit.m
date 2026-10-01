## Stands in for Octave's own `quit` in CCOSEL sessions, which would end the user's Octave and
## lose every variable. The script ends there instead: `__ccosel_run__` recognises the error
## this raises, and reports a status of 0 (the default) as a normal end, not an error.
##
## Loaded from the server's support folder, ahead of the built-in: edit this file, not a copy.
function quit (status, varargin)
  ## `quit` and `quit ("force")` mean status 0.
  if (nargin < 1 || ! isnumeric (status))
    status = 0;
  endif
  setappdata (0, "__ccosel_exit__", status);
  error ("Octave:ccosel-exit", ["quit(%d): the script ends here. Octave keeps running, so your ", ...
         "variables are kept; press Restart to start again from empty."], status);
endfunction
