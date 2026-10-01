## Runs one job for the CCOSEL server, then reports the session's state.
##
## KIND is "code" (FILE is a script holding what the user typed), "file" (FILE is a script
## in the jail, run with `run` so it sees its own folder), or "render" (FILE is a folder to
## print every figure into, as N.png for figure N). Everything runs in the base
## workspace, as at a real prompt. Output is whatever the code prints; the report after it is
## lines starting with TAG, which the server strips out. TAG is new for every job, so nothing
## the code prints can pass for the report.
##
## Loaded from the server's support folder: edit this file, not a copy of it.
function __ccosel_run__ (kind, file, tag)
  failed = false;
  ## Every graphics object there is before the job, so the report can say which figures it
  ## created or drew into: those would have popped up a window on a desktop.
  before = findall (0);
  try
    if (strcmp (kind, "file"))
      evalin ("base", sprintf ("run ('%s');", strrep (file, "'", "''")));
    elseif (strcmp (kind, "render"))
      __ccosel_render__ (file);
    else
      evalin ("base", sprintf ("source ('%s');", strrep (file, "'", "''")));
    endif
  catch err
    if (strcmp (err.identifier, "Octave:ccosel-exit"))
      ## The script called `exit` or `quit` (our stand-ins): it ends there, an error only if
      ## it said so with a status other than 0.
      status = getappdata (0, "__ccosel_exit__");
      rmappdata (0, "__ccosel_exit__");
      failed = ! isequal (status, 0);
      if (failed)
        printf ("error: %s\n", err.message);
      else
        printf ("%s\n", err.message);
      endif
    else
      failed = true;
      printf ("error: %s\n", err.message);
    endif
  end_try_catch
  fflush (stdout);
  fflush (stderr);
  printf ("%s STATUS\t%d\t%s\n", tag, failed, __ccosel_clean__ (pwd ()));
  __ccosel_vars__ (tag);
  __ccosel_figs__ (tag, before);
  printf ("%s DONE\n", tag);
  fflush (stdout);
endfunction

function __ccosel_render__ (dir)
  ## One figure that won't print shouldn't cost the others theirs: the server sends what it
  ## finds, and the app draws the line data for the rest.
  figs = sort (get (0, "children"));
  for f = figs(:)'
    try
      print (f, fullfile (dir, sprintf ("%d.png", f)), "-dpng", "-r80");
    catch
    end_try_catch
  endfor
endfunction

function s = __ccosel_clean__ (s)
  ## One report field: no tabs or newlines, and never very long.
  if (! ischar (s))
    s = "";
  endif
  s = strrep (strrep (strrep (s(:)', "\t", " "), "\n", " "), "\r", " ");
  if (numel (s) > 120)
    s = [s(1:117) "..."];
  endif
endfunction

function __ccosel_vars__ (tag)
  vars = evalin ("base", "whos ()");
  for i = 1:numel (vars)
    v = vars(i);
    if (strncmp (v.name, "__ccosel", 8))
      continue;
    endif
    dims = sprintf ("%dx", v.size);
    attrs = {};
    if (v.global) attrs{end+1} = "global"; endif
    if (v.persistent) attrs{end+1} = "persistent"; endif
    if (v.complex) attrs{end+1} = "complex"; endif
    if (v.sparse) attrs{end+1} = "sparse"; endif
    value = "";
    try
      value = __ccosel_value__ (evalin ("base", v.name));
    catch
    end_try_catch
    printf ("%s VAR\t%s\t%s\t%s\t%s\t%s\n", tag, v.name, v.class, dims(1:end-1),
            __ccosel_clean__ (value), strjoin (attrs, ","));
  endfor
endfunction

function s = __ccosel_value__ (v)
  s = "";
  if (ischar (v) && rows (v) <= 1)
    s = ["\"" v "\""];
  elseif ((isnumeric (v) || islogical (v)) && ndims (v) == 2 && numel (v) <= 12 && ! issparse (v))
    s = mat2str (v, 5);
  elseif (isa (v, "function_handle"))
    s = func2str (v);
  endif
endfunction

function t = __ccosel_text__ (h)
  ## A text object's string, or "" — a title can be a cell array of lines.
  t = "";
  try
    t = get (h, "string");
    if (iscell (t))
      t = strjoin (t, " ");
    endif
    t = __ccosel_clean__ (t);
  catch
    t = "";
  end_try_catch
endfunction

function __ccosel_figs__ (tag, before)
  figs = sort (get (0, "children"));
  for f = figs(:)'
    changed = any (! ismember (findall (f), before));
    printf ("%s FIG\t%d\t%s\t%d\n", tag, f, __ccosel_clean__ (get (f, "name")), changed);
    axs = flipud (findobj (f, "type", "axes"));
    for a = axs(:)'
      ## A legend is an axes too, in Octave; it has no lines of its own worth drawing.
      if (strcmp (get (a, "tag"), "legend"))
        continue;
      endif
      printf ("%s AXES\t%s\n", tag, __ccosel_text__ (get (a, "title")));
      lines = flipud (findobj (a, "type", "line"));
      for l = lines(:)'
        x = double (get (l, "xdata"));
        y = double (get (l, "ydata"));
        n = min (numel (x), numel (y));
        ## At most about 400 points each: the server resamples to far fewer anyway.
        idx = 1:max (1, floor (n / 400)):n;
        printf ("%s LINE\t%s\t%s\t%s\n", tag, __ccosel_clean__ (get (l, "displayname")),
                sprintf ("%.7g ", x(idx)), sprintf ("%.7g ", y(idx)));
      endfor
    endfor
  endfor
endfunction
