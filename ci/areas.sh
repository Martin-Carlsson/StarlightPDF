#!/bin/sh
# Changed paths on stdin, one per line; prints the areas to run, sorted.
set -eu

advisories=0
core=0
python=0
while IFS= read -r path || [ -n "$path" ]; do
  case $path in
    *.md) ;;
    Cargo.lock | deny.toml | ci/check.sh | .github/workflows/ci.yml)
      advisories=1
      core=1
      python=1
      ;;
    bindings/python/*) python=1 ;;
    core/* | tests/* | ci/* | .cargo/* | supply-chain/*)
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
if [ "$advisories" = 1 ]; then echo advisories; fi
if [ "$core" = 1 ]; then echo core; fi
if [ "$python" = 1 ]; then echo python; fi
