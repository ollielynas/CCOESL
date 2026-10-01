#!/bin/sh
# Stands in for octave-cli in the server's tests, so CI needs no Octave.
#
# It answers the two things the server ever sends: the start-up line that prints a tag, which
# it prints, and `__ccosel_run__("code"|"file", "<script>", "<tag>")`, which it answers with a
# transcript real Octave produced, `<fixture>.out`, retagged. Which transcript comes from the
# script's first line, `% fixture: <name>`; an empty script (a restart) is `empty`. `@CWD` in a
# transcript is the folder it was recorded in, replaced by the one this runs in. Record the
# transcripts again with `CCOSEL_RECORD_OCTAVE=1 cargo test -p ccosel-server --test octave`.
#
# A render (`__ccosel_run__("render", "<folder>", ...)`) copies `figure.png` into the folder
# as figure 1, and reports the plot transcript's figures.
#
# Two fixtures do instead what real Octave would: `exit` ends the process, `hang` never answers.
# Like real Octave, an interrupt (SIGINT) abandons the command running and reads the next one.
here=$(dirname "$0")
trap ':' INT
while IFS= read -r line; do
  case "$line" in
    printf*)
      echo "$line" | sed 's/.*"\(@ccosel-[0-9a-f]*\)").*/\1/'
      ;;
    __ccosel_run__\(\"render\"*)
      # Print every figure: here, one, as `figure.png`, and the plot transcript's report.
      dir=$(echo "$line" | sed 's/^__ccosel_run__("render", "\(.*\)", "@ccosel-[0-9a-f]*")$/\1/')
      tag=$(echo "$line" | sed 's/.*"\(@ccosel-[0-9a-f]*\)")$/\1/')
      cp "$here/figure.png" "$dir/1.png"
      sed -e "s/@TAG/$tag/g" -e "s|@CWD|$PWD|g" "$here/plot.out" | grep "$tag"
      ;;
    __ccosel_run__*)
      script=$(echo "$line" | sed 's/^__ccosel_run__("[a-z]*", "\(.*\)", "@ccosel-[0-9a-f]*")$/\1/')
      tag=$(echo "$line" | sed 's/.*"\(@ccosel-[0-9a-f]*\)")$/\1/')
      name=$(sed -n '1s/^% fixture: //p' "$script")
      [ -n "$name" ] || name=empty
      case "$name" in
        exit) exit 0 ;;
        hang)
          # In the background so the trap can run: `wait` returns early on an interrupt.
          sleep 60 &
          wait $!
          kill $! 2>/dev/null
          continue
          ;;
      esac
      sed -e "s/@TAG/$tag/g" -e "s|@CWD|$PWD|g" "$here/$name.out"
      ;;
  esac
done
