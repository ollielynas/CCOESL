## Shows the figures a running job has changed since it last looked, in the CCOSEL app's
## Command Window, the way desktop windows would show them by now: at drawnow, pause, and
## whenever the job asks the user something. Each is printed to a PNG in the session's folder
## and reported as a FIGURE prompt, which the server reads in and passes to the app.
##
## Unless FORCE, it looks at most once a second, so a drawnow in an animation loop stays cheap.
##
## Loaded from the server's support folder: edit this file, not a copy.
function __ccosel_live__ (force = false)
  persistent last = 0;
  if (isequal (getappdata (0, "__ccosel_live_busy__"), true))
    return;
  endif
  if (! force && time () - last < 1)
    return;
  endif
  before = getappdata (0, "__ccosel_live_before__");
  dir = getappdata (0, "__ccosel_dir__");
  if (! ischar (dir))
    return;
  endif
  ## `print` draws, and drawing comes back here: not while this is printing.
  setappdata (0, "__ccosel_live_busy__", true);
  unwind_protect
    for f = sort (get (0, "children"))'
      if (! any (! ismember (findall (f), before)))
        continue;
      endif
      n = getappdata (0, "__ccosel_live_count__") + 1;
      setappdata (0, "__ccosel_live_count__", n);
      file = fullfile (dir, sprintf ("live-%d.png", n));
      try
        print (f, file, "-dpng", "-r80");
        __ccosel_prompt__ (false, "FIGURE", sprintf ("%d", f), file);
      catch
      end_try_catch
    endfor
    setappdata (0, "__ccosel_live_before__", findall (0));
    last = time ();
  unwind_protect_cleanup
    setappdata (0, "__ccosel_live_busy__", false);
  end_unwind_protect
endfunction
