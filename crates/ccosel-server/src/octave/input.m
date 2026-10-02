## Stands in for Octave's own `input` in CCOSEL sessions, whose stdin is the server's command
## channel, which nobody can type into. The app asks instead: the question shows at the
## Command Window's prompt, and what is typed there comes back as the answer.
##
## As Octave's: with FMT "s" the answer is returned as text; otherwise it is evaluated in the
## caller's workspace, and an empty answer gives []. Unlike Octave's, an answer that fails to
## evaluate is an error rather than a second asking.
##
## Loaded from the server's support folder, ahead of the built-in: edit this file, not a copy.
function out = input (prompt = "", fmt = "")
  answer = __ccosel_prompt__ (true, "INPUT", prompt);
  if (strcmp (fmt, "s"))
    out = answer;
  elseif (isempty (strtrim (answer)))
    out = [];
  else
    out = evalin ("caller", answer);
  endif
endfunction
