## Stands in for Octave's own `uigetfile` in CCOSEL sessions: the app shows a file picker over
## the shared files, and the script waits for the choice. FNAME and FPATH are the file's name
## and real folder, ready for fopen or load; both 0 if the picker was cancelled.
##
## Takes the usual FLT (a pattern such as "*.m", patterns joined by ";", or a cell array whose
## first column holds them), TITLE and DEFAULT (a folder or file to start in). One file only.
##
## Loaded from the server's support folder, ahead of the built-in: edit this file, not a copy.
function [fname, fpath, fltidx] = uigetfile (flt = "", title = "Select a file", default = "", varargin)
  if (iscell (flt))
    flt = strjoin (flt(:, 1)', ";");
  endif
  if (isempty (flt))
    flt = "*";
  endif
  start = "";
  if (ischar (default) && ! isempty (default))
    start = default;
    if (! isfolder (start))
      start = fileparts (start);
    endif
    start = make_absolute_filename (start);
  endif
  answer = __ccosel_prompt__ (true, "OPENFILE", title, flt, start);
  if (isempty (answer))
    fname = 0;
    fpath = 0;
    fltidx = 0;
    return;
  endif
  [dir, name, ext] = fileparts (answer);
  fname = [name, ext];
  fpath = [dir, filesep];
  fltidx = 1;
endfunction
