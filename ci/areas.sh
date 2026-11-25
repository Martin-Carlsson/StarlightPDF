#!/bin/sh
# Changed paths on stdin, one per line; prints the areas to run, sorted.
set -eu

advisories=0
core=0
dependencies=0
fuzz=0
fuzz_files=0
hook=0
javascript=0
licenses=0
mods=0
python=0
ruff=0
shell=0
workflows=0
while IFS= read -r path || [ -n "${path}" ]; do
  licenses=1
  case ${path} in
    .github/workflows/*) workflows=1 ;;
    .claude/skills/build/*) hook=1 ;;
    .claude/mods/*) mods=1 ;;
    Cargo.toml | */Cargo.toml) dependencies=1 ;;
    *) ;;
  esac
  case ${path} in
    *.py | *.pyi | ci/licenses.sh | ruff.toml) ruff=1 ;;
    *) ;;
  esac
  case ${path} in
    *.sh) shell=1 ;;
    *) ;;
  esac
  case ${path} in
    README.md) dependencies=1 ;;
    *.md | REUSE.toml | ci/licenses.sh | ruff.toml) ;;
    ci/dependencies.sh) dependencies=1 ;;
    ci/workflows.sh) workflows=1 ;;
    ci/check.sh | .github/workflows/ci.yml)
      advisories=1 core=1 dependencies=1 fuzz=1 fuzz_files=1 hook=1 javascript=1 mods=1 python=1
      ruff=1 shell=1 workflows=1
      ;;
    Cargo.lock | ci/setup.sh)
      advisories=1
      core=1
      dependencies=1
      fuzz=1
      fuzz_files=1
      javascript=1
      python=1
      ;;
    deny.toml)
      advisories=1
      core=1
      javascript=1
      python=1
      ;;
    bindings/javascript/*) javascript=1 ;;
    bindings/python/*) python=1 ;;
    LICENSE)
      javascript=1
      python=1
      ;;
    tests/ci/*) ;;
    tests/corpus/render/*.png | tests/corpus/render/*.jpg)
      core=1
      fuzz=1
      fuzz_files=1
      javascript=1
      python=1
      ;;
    tests/corpus/render/*)
      core=1
      fuzz=1
      javascript=1
      python=1
      ;;
    core/* | tests/corpus/* | .cargo/*)
      core=1
      fuzz=1
      fuzz_files=1
      javascript=1
      python=1
      ;;
    tests/* | ci/* | supply-chain/*)
      core=1
      javascript=1
      python=1
      ;;
    fuzz/*)
      fuzz=1
      fuzz_files=1
      ;;
    */*) ;;
    *.toml)
      core=1
      fuzz=1
      fuzz_files=1
      javascript=1
      python=1
      ;;
    *) ;;
  esac
done
if [ "${advisories}" = 1 ]; then echo advisories; fi
if [ "${core}" = 1 ]; then echo core; fi
if [ "${dependencies}" = 1 ]; then echo dependencies; fi
if [ "${fuzz}" = 1 ]; then echo fuzz; fi
if [ "${fuzz_files}" = 1 ]; then echo fuzz-files; fi
if [ "${hook}" = 1 ]; then echo hook; fi
if [ "${javascript}" = 1 ]; then echo javascript; fi
if [ "${licenses}" = 1 ]; then echo licenses; fi
if [ "${mods}" = 1 ]; then echo mods; fi
if [ "${python}" = 1 ]; then echo python; fi
if [ "${ruff}" = 1 ]; then echo ruff; fi
if [ "${shell}" = 1 ]; then echo shell; fi
if [ "${workflows}" = 1 ]; then echo workflows; fi
