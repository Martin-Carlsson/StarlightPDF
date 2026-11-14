#!/bin/sh
# Tests for stop-check.sh. Run: sh stop-check.test.sh
set -eu

hook="$(cd "$(dirname "$0")" && pwd)/stop-check.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export CLAUDE_PROJECT_DIR="$tmp"
state="$tmp/.claude/build/state"
failed=0

# run NAME WANT_EXIT WANT_STDERR_PATTERN
run() {
  code=0
  echo >"$tmp/out"
  sh "$hook" </dev/null >>"$tmp/out" 2>"$tmp/err" || code=$?
  if [ "$code" -ne "$2" ] || ! grep -q "$3" "$tmp/err" "$tmp/out"; then
    echo "FAIL $1: exit $code, stderr: $(cat "$tmp/err")"
    failed=1
  fi
}

expect() {
  if ! "$@"; then
    echo "FAIL: $*"
    failed=1
  fi
}

reset() {
  rm -rf "$tmp/.claude"
  mkdir -p "$tmp/.claude/build"
}

reset
run "no state allows stop" 0 ''

reset
printf 'status: blocked\ncheck: false\n' >"$state"
run "blocked allows stop" 0 ''
expect test -f "$state"

reset
printf 'status: working\ncheck: true\ncheck: echo boom; false\npr: https://example.test/1\n' >"$state"
run "failing check refuses" 2 'check failed: echo boom; false'
expect grep -q boom "$tmp/err"
expect test -f "$state"

reset
printf 'status: working\npr: https://example.test/1\n' >"$state"
run "no checks refuses" 2 'no check'

reset
printf 'status: working\ncheck: true\n' >"$state"
run "missing pr refuses" 2 'pr:'

reset
printf 'status: working\ncheck: test -f marker\npr: https://example.test/1\n' >"$state"
: >"$tmp/marker"
run "all pass allows stop, checks run from root" 0 ''
expect test ! -e "$state"

reset
printf 'status: working\ncheck: read x\npr: https://example.test/1\n' >"$state"
run "checks get no stdin" 2 'check failed'

reset
printf 'status: working\ncheck: false\n' >"$state"
i=0
while [ "$i" -lt 30 ]; do
  run "refusal $i" 2 'check failed'
  i=$((i + 1))
done
run "cap allows stop" 0 ''
expect grep -q 'gave up' "$tmp/out"
expect grep -q '^status: blocked$' "$state"
run "stays blocked after cap" 0 ''

reset
printf 'status: working\ncheck: test -f marker\npr: https://example.test/1\n' >"$state"
rm -f "$tmp/marker"
run "refusal before success" 2 'check failed'
: >"$tmp/marker"
run "success" 0 ''
expect test ! -e "$tmp/.claude/build/refusals"

[ "$failed" -eq 0 ] && echo PASS
exit "$failed"
