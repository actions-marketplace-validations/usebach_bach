#!/bin/sh
# Splits the action's `args` input into words, one per line, the way a person
# writes a command line: spaces separate words, and single or double quotes
# keep spaces (and anything else) inside one word, so `--out "My Folder/x.exe"`
# is two words. A backslash is a backslash (Windows paths), nothing is expanded
# and no pattern like * is matched against files. A word can't hold a line
# break. Exits with 2 and a sentence on an unclosed quote.
#
#   sh scripts/split-args.sh '--fast --out "My Folder/Setup.exe"'
set -eu
printf '%s' "${1:-}" | awk '
  function flush() { if (has) { out = out word "\n" } word = ""; has = 0 }
  { text = text (NR > 1 ? " " : "") $0 }
  END {
    gsub(/\r/, "", text)
    quote = ""; word = ""; has = 0
    for (i = 1; i <= length(text); i++) {
      c = substr(text, i, 1)
      if (quote != "") {
        if (c == quote) { quote = "" } else { word = word c }
      } else if (c == "\"" || c == "\x27") {
        quote = c; has = 1
      } else if (c == " " || c == "\t") {
        flush()
      } else {
        word = word c; has = 1
      }
    }
    if (quote != "") { print "args has a " quote " without its closing " quote > "/dev/stderr"; exit 2 }
    flush()
    printf "%s", out
  }
'
