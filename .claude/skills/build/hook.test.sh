#!/bin/sh
# Tests for hook.sh. Run: sh hook.test.sh
set -eu

hook="$(cd "$(dirname "$0")" && pwd)/hook.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export CLAUDE_PROJECT_DIR="$tmp"
state="$tmp/.claude/build/state"
failed=0
mode=stop

# run NAME WANT_EXIT WANT_STDERR_PATTERN
run() {
  code=0
  echo >"$tmp/out"
  sh "$hook" "$mode" </dev/null >>"$tmp/out" 2>"$tmp/err" || code=$?
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

# 24-hour limit. `started: 0` is always over it; a far-future stamp never is.
reset
printf 'status: working\ncheck: false\nstarted: 0\n' >"$state"
run "stop allowed after limit" 0 'without the user'
expect test -f "$state"
expect grep -q '^check: false$' "$state"
if grep -q '^started: ' "$state"; then echo "FAIL: clock not reset after limit"; failed=1; fi

reset
printf 'status: blocked\ncheck: false\nstarted: 0\n' >"$state"
run "blocked resets clock" 0 ''
if grep -q '^started: ' "$state"; then echo "FAIL: clock not reset when blocked"; failed=1; fi

reset
printf 'status: working\ncheck: false\n' >"$state"
run "stop stamps start" 2 'check failed'
expect grep -q '^started: [0-9][0-9]*$' "$state"

mode=tool
reset
run "tool: no state allows" 0 ''

reset
printf 'status: working\ncheck: false\nstarted: 0\n' >"$state"
run "tool: refused after limit" 2 'ask'
expect grep -q '^started: 0$' "$state"

reset
printf 'status: working\ncheck: false\nstarted: 9999999999\n' >"$state"
run "tool: allowed within limit" 0 ''

reset
printf 'status: working\ncheck: false\n' >"$state"
run "tool: stamps start" 0 ''
expect grep -q '^started: [0-9][0-9]*$' "$state"

reset
printf 'status: blocked\ncheck: false\nstarted: 0\n' >"$state"
run "tool: blocked allows" 0 ''

# Results: every check runs and is recorded, in order.
results="$tmp/.claude/build/results"
mode=stop
reset
printf 'status: working\ncheck: true\ncheck: false\ncheck: test -f marker\n' >"$state"
: >"$tmp/marker"
run "stop records every result" 2 'check failed: false'
printf 'pass true\nfail false\npass test -f marker\n' >"$tmp/want"
expect cmp -s "$tmp/want" "$results"

mode=check
reset
printf 'status: working\ncheck: true\ncheck: false\n' >"$state"
run "check: fails when a check fails" 1 'fail false'
expect grep -q '^pass true$' "$results"
expect test ! -e "$tmp/.claude/build/refusals"

reset
printf 'status: working\ncheck: true\n' >"$state"
run "check: passes without a pr" 0 'pass true'
expect test -f "$state"

reset
run "check: no state fails" 1 'no build'

mode=stop
reset
printf 'status: working\ncheck: true\npr: https://example.test/1\n' >"$state"
run "success removes results" 0 ''
expect test ! -e "$results"

[ "$failed" -eq 0 ] && echo PASS
exit "$failed"
