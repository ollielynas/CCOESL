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
# Two fixtures do instead what real Octave would: `exit` ends the process, `hang` never answers.
here=$(dirname "$0")
while IFS= read -r line; do
  case "$line" in
    printf*)
      echo "$line" | sed 's/.*"\(@ccosel-[0-9a-f]*\)").*/\1/'
      ;;
    __ccosel_run__*)
      script=$(echo "$line" | sed 's/^__ccosel_run__("[a-z]*", "\(.*\)", "@ccosel-[0-9a-f]*")$/\1/')
      tag=$(echo "$line" | sed 's/.*"\(@ccosel-[0-9a-f]*\)")$/\1/')
      name=$(sed -n '1s/^% fixture: //p' "$script")
      [ -n "$name" ] || name=empty
      case "$name" in
        exit) exit 0 ;;
        hang) sleep 60; exit 0 ;;
      esac
      sed -e "s/@TAG/$tag/g" -e "s|@CWD|$PWD|g" "$here/$name.out"
      ;;
  esac
done
