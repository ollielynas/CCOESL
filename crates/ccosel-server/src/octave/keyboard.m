## Stands in for Octave's own `keyboard` in CCOSEL sessions, whose stdin is the server's
## command channel. A `K>>` prompt in the Command Window instead: each line typed there runs
## in the caller's workspace, until `return` or `dbcont` carries on with the script, or
## `dbquit` stops it.
##
## Loaded from the server's support folder, ahead of the built-in: edit this file, not a copy.
function keyboard (prompt = "K>> ")
  while (true)
    line = strtrim (__ccosel_prompt__ (true, "INPUT", prompt));
    if (any (strcmp (line, {"return", "dbcont", "dbstep", "dbnext"})))
      return;
    elseif (strcmp (line, "dbquit"))
      error ("Octave:ccosel-dbquit", "dbquit: the script stopped here.");
    elseif (! isempty (line))
      try
        evalin ("caller", line);
      catch err
        printf ("error: %s\n", err.message);
      end_try_catch
      fflush (stdout);
    endif
  endwhile
endfunction
