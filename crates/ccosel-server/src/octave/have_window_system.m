## Stands in for Octave's own `have_window_system` in CCOSEL sessions. Octave there runs with
## no display, so the built-in says false, and scripts that check it skip their plots. But the
## figures are shown: in the Command Window under the command that drew them, and in the
## Figures tab. So, as far as a script needs to know, there are windows.
##
## Loaded from the server's support folder, ahead of the built-in: edit this file, not a copy.
function tf = have_window_system ()
  tf = true;
endfunction
