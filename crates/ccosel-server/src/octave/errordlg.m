## Stands in for Octave's own `errordlg` in CCOSEL sessions, which has no window to open: the
## app shows the message as a dialog instead. Like MATLAB's, it doesn't wait for OK; wrap it
## in uiwait for that, which carries on at once here.
##
## Loaded from the server's support folder, ahead of the built-in: edit this file, not a copy.
function h = errordlg (msg = "", title = "", varargin)
  __ccosel_prompt__ (false, "MESSAGE", "error", title, msg);
  ## MATLAB returns the dialog's figure; there is none, and [] is safe to pass to uiwait,
  ## close or delete.
  h = [];
endfunction
