#!/bin/sh
# Changed paths on stdin, one per line; prints the areas to run, sorted.
set -eu

core=0
python=0
while IFS= read -r path || [ -n "$path" ]; do
  case $path in
    *.md) ;;
    bindings/python/*) python=1 ;;
    core/* | tests/* | ci/* | .cargo/* | .github/workflows/ci.yml | Cargo.lock)
      core=1
      python=1
      ;;
    */*) ;;
    *.toml)
      core=1
      python=1
      ;;
  esac
done
if [ "$core" = 1 ]; then echo core; fi
if [ "$python" = 1 ]; then echo python; fi
