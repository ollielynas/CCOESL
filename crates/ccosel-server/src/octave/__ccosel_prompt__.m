## Asks the CCOSEL app something while a job runs: a message to show, or a file to pick. Used
## by the stand-ins for msgbox, uigetfile and the like.
##
## Prints a report line under the job's tag, which the server passes to the app, and returns
## the prompt's number. KIND and each of FIELDS go on that line, escaped so a field can't
## carry a tab or a newline. With WAIT, it then waits for the app's answer, which the server
## writes as a file in the session's folder, and returns that answer's text instead.
##
## Loaded from the server's support folder: edit this file, not a copy.
function out = __ccosel_prompt__ (wait, kind, varargin)
  ## Before asking, show what the script has drawn, as a desktop's windows would be by now.
  if (! strcmp (kind, "FIGURE"))
    __ccosel_live__ (true);
  endif
  n = getappdata (0, "__ccosel_prompts__");
  if (isempty (n))
    n = 0;
  endif
  n += 1;
  setappdata (0, "__ccosel_prompts__", n);
  fields = cellfun (@__ccosel_escape__, varargin, "uniformoutput", false);
  fflush (stdout);
  printf ("%s PROMPT\t%d\t%s\t%s\n", getappdata (0, "__ccosel_tag__"), n, kind,
          strjoin (fields, "\t"));
  fflush (stdout);
  if (! wait)
    out = n;
    return;
  endif
  answer = fullfile (getappdata (0, "__ccosel_dir__"), sprintf ("answer-%d", n));
  ## Waiting in `pause` keeps it interruptible: Stop and the time limit still work.
  while (! exist (answer, "file"))
    pause (0.1);
  endwhile
  out = fileread (answer);
  delete (answer);
endfunction

function s = __ccosel_escape__ (s)
  if (iscellstr (s))
    s = strjoin (s(:)', "\n");
  elseif (! ischar (s))
    s = disp (s);
  endif
  s = strrep (strrep (strrep (s(:)', "\\", "\\\\"), "\t", "\\t"), "\n", "\\n");
  s = strrep (s, "\r", "");
endfunction
