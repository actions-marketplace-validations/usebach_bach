#!/bin/sh
# Checks scripts/split-args.sh: the words the action's `args` input becomes.
#   sh scripts/test-action-args.sh
set -eu
here=$(dirname "$0")
fail=0
check() {
  expected=$1; shift
  actual=$(sh "$here/split-args.sh" "$1" | tr '\n' '|')
  if [ "$actual" = "$expected" ]; then echo "ok    [$1]"; else echo "FAIL  [$1] gave [$actual], not [$expected]"; fail=1; fi
}
check ''                                   ''
check ''                                   '   '
check '--fast|'                            '--fast'
check '--fast|--online|'                   '  --fast   --online '
check '--out|My Folder/Setup.exe|'         '--out "My Folder/Setup.exe"'
check '--out|My Folder/Setup.exe|'         "--out 'My Folder/Setup.exe'"
check '--patch-from|dist\old.exe|'         '--patch-from dist\old.exe'
check '--notes|it'"'"'s new|'              '--notes "it'"'"'s new"'
check 'a*|?b|'                             'a* ?b'
check '--key=x y|--fast|'                  '--key="x y" --fast'
check '||'                                 '"" '"''"
if out=$(sh "$here/split-args.sh" '--fast --out "open' 2>/dev/null); then echo "FAIL  an open quote is accepted"; fail=1
elif [ -n "$out" ]; then echo "FAIL  an open quote still printed [$out]"; fail=1
else echo "ok    an open quote is refused, and nothing is printed"; fi
exit $fail
