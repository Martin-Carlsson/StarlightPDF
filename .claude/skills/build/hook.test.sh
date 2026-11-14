#!/bin/sh
# Tests for hook.sh. Run: sh hook.test.sh
set -eu

hook="$(cd "$(dirname "$0")" && pwd)/hook.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
proj="$tmp/proj"
export CLAUDE_PROJECT_DIR="$proj"
dir="$proj/.claude/build"
state="$dir/state"
results="$dir/results"
clock="$dir/started"
failed=0
mode=stop

# run NAME WANT_EXIT WANT_OUTPUT_PATTERN
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

# reset STATE_TEXT: a fresh repository, with that state file if given.
reset() {
  rm -rf "$proj"
  mkdir -p "$dir"
  git init -q "$proj"
  echo '.claude/build/' >"$proj/.gitignore"
  if [ $# -gt 0 ]; then printf "$1" >"$state"; fi
}

check() {
  mode=check
  run "$@"
  mode=stop
}

# Stop: inactive and blocked.
reset
run "no state allows stop" 0 ''

reset 'status: blocked\ncheck: false\n'
echo 0 >"$clock"
run "blocked allows stop" 0 ''
expect test -f "$state"
expect test ! -e "$clock"

# Stop: checks must have passed on the files as they are now.
reset 'status: working\npr: https://example.test/1\n'
run "no checks refuses" 2 'no check'

reset 'status: working\ncheck: true\npr: https://example.test/1\n'
run "checks never run refuses" 2 'hook.sh check'

reset 'status: working\ncheck: true\ncheck: echo boom; false\npr: https://example.test/1\n'
check "check: fails when a check fails" 1 'fail echo boom; false'
expect grep -q boom "$tmp/err"
run "failing check refuses" 2 'not passing: echo boom; false'
expect test -f "$state"

reset 'status: working\ncheck: true\n'
check "check: passes without a pr" 0 'pass true'
run "missing pr refuses" 2 'pr:'

reset 'status: working\ncheck: test -f marker\npr: https://example.test/1\n'
: >"$proj/marker"
check "check: runs from the root" 0 'pass test -f marker'
echo 0 >"$dir/refusals"
run "all pass allows stop" 0 ''
expect test ! -e "$clock"
expect test ! -e "$dir/refusals"

# A finished build stays on disk for the pane, and no longer holds anything back.
expect cmp -s "$state" "$dir/done"
expect grep -qx 'pass test -f marker' "$results"
echo change >"$proj/file"
run "finished build allows stop" 0 ''
echo 0 >"$clock"
mode=tool
run "tool: finished build allows" 0 ''
mode=stop

# A new build's state differs from the finished one, so it is enforced again.
rm -f "$clock"
printf 'status: working\ncheck: true\n' >"$state"
run "new build after a finished one refuses" 2 'hook.sh check'

reset 'status: working\ncheck: true\npr: https://example.test/1\n'
check "check: passes" 0 'pass true'
echo change >"$proj/file"
run "files changed since the checks refuses" 2 'changed'

reset 'status: working\ncheck: true\npr: https://example.test/1\n'
check "check: passes" 0 'pass true'
echo 'check: false' >>"$state"
run "check added since the run refuses" 2 'not passing: false'

# Check mode.
reset 'status: working\ncheck: true\ncheck: false\ncheck: test -f marker\n'
: >"$proj/marker"
check "check: records every result" 1 'fail false'
printf 'pass true\nfail false\npass test -f marker\n' >"$tmp/want"
grep -v '^tree ' "$results" >"$tmp/got"
expect cmp -s "$tmp/want" "$tmp/got"
expect test ! -e "$dir/refusals"

reset 'status: working\ncheck: read x\n'
check "check: commands get no stdin" 1 'fail read x'

reset
check "check: no state fails" 1 'no build'

# The cap counts refusals since the checks last ran.
reset 'status: working\ncheck: false\n'
echo 29 >"$dir/refusals"
check "check: resets the refusal count" 1 'fail false'
expect test ! -e "$dir/refusals"

# Check finds the repository root from a subdirectory by itself.
reset 'status: working\ncheck: test -f marker\n'
: >"$proj/marker"
mkdir "$proj/sub"
code=0
(cd "$proj/sub" && unset CLAUDE_PROJECT_DIR && sh "$hook" check >"$tmp/out" 2>"$tmp/err") || code=$?
expect test "$code" -eq 0
expect grep -qx 'pass test -f marker' "$tmp/out"

# The hooks themselves never run a command from the state file.
reset 'status: working\ncheck: touch pwned\npr: https://example.test/1\n'
run "stop does not run checks" 2 'hook.sh check'
mode=tool
run "tool does not run checks" 0 ''
mode=stop
expect test ! -e "$proj/pwned"

# Cap: the 31st refusal in a row lets one stop through, then enforcement resumes.
reset 'status: working\ncheck: false\n'
i=0
while [ "$i" -lt 30 ]; do
  run "refusal $i" 2 'hook.sh check'
  i=$((i + 1))
done
run "cap allows stop" 0 'gave up'
expect grep -q '^status: working$' "$state"
run "refuses again after the cap" 2 'hook.sh check'

# 24-hour limit. A clock of 0 is always over it; a far-future one never is.
reset 'status: working\ncheck: false\n'
echo 0 >"$clock"
run "stop allowed after limit" 0 'without the user'
expect test -f "$state"
expect test ! -e "$clock"

mode=tool
reset
run "tool: no state allows" 0 ''

reset 'status: working\ncheck: false\n'
echo 0 >"$clock"
run "tool: refused after limit" 2 'ask'

reset 'status: working\ncheck: false\n'
echo 9999999999 >"$clock"
run "tool: allowed within limit" 0 ''

reset 'status: blocked\ncheck: false\n'
echo 0 >"$clock"
run "tool: blocked allows" 0 ''

# The clock is the hook's own file: the state file is never written.
reset 'status: working\ncheck: sh tests/run.sh'
cp "$state" "$tmp/before"
run "tool: stamps the clock" 0 ''
expect grep -qx '[0-9][0-9]*' "$clock"
expect cmp -s "$tmp/before" "$state"

# A clock two racing hooks or anything else left unreadable is restarted.
reset 'status: working\ncheck: false\n'
printf '1\n2\n' >"$clock"
run "tool: bad clock restarts" 0 ''
expect grep -qx '[0-9][0-9]*' "$clock"
expect test "$(wc -l <"$clock")" -eq 1
mode=stop

[ "$failed" -eq 0 ] && echo PASS
exit "$failed"
