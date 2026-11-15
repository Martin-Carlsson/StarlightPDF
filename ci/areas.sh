#!/bin/sh
# Changed paths on stdin, one per line; prints the areas to run, sorted.
set -eu

advisories=0
core=0
dependencies=0
licenses=0
python=0
while IFS= read -r path || [ -n "$path" ]; do
  licenses=1
  case $path in
    README.md) dependencies=1 ;;
    *.md | REUSE.toml | ci/licenses.sh) ;;
    ci/dependencies.sh) dependencies=1 ;;
    Cargo.lock | ci/check.sh | .github/workflows/ci.yml)
      advisories=1
      core=1
      dependencies=1
      python=1
      ;;
    deny.toml)
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
if [ "$dependencies" = 1 ]; then echo dependencies; fi
if [ "$licenses" = 1 ]; then echo licenses; fi
if [ "$python" = 1 ]; then echo python; fi
