#!/bin/sh
# Tests for hook.sh. Run: sh hook.test.sh
set -eu

hook="$(cd "$(dirname "$0")" && pwd)/hook.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
proj="$tmp/proj"
export CLAUDE_PROJECT_DIR="$proj"

# Stub gh: logs its args to gh.log. Answers issue N's blocked_by
# query with gh.blocked.N (none if absent); else prints gh.out
# (default pass), exits gh.code (default 0), writes gh.err to stderr.
mkdir "$tmp/bin"
cat >"$tmp/bin/gh" <<'END'
#!/bin/sh
t=$(dirname "$0")/..
echo "$*" >>"$t/gh.log"
case "$*" in
  *issues/*/dependencies/blocked_by*)
    n=$*
    n=${n#*issues/}
    cat "$t/gh.blocked.${n%%/*}" 2>/dev/null
    exit 0
    ;;
esac
if [ -f "$t/gh.err" ]; then cat "$t/gh.err" >&2; fi
if [ -f "$t/gh.out" ]; then cat "$t/gh.out"; else echo pass; fi
if [ -f "$t/gh.code" ]; then exit "$(cat "$t/gh.code")"; fi
END
chmod +x "$tmp/bin/gh"
PATH="$tmp/bin:$PATH"
export PATH

# gh_says OUTPUT_PRINTF CODE STDERR: what the stub gh does from now on.
gh_says() {
  printf %b "$1" >"$tmp/gh.out"
  echo "$2" >"$tmp/gh.code"
  printf %b "$3" >"$tmp/gh.err"
}
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

# expect [-n NAME] COMMAND...: with a name, the case reports "ok NAME".
expect() {
  label=
  if [ "$1" = -n ]; then
    label=$2
    shift 2
  fi
  if "$@"; then
    if [ -n "$label" ]; then echo "ok $label"; fi
  else
    echo "FAIL ${label:-$*}"
    failed=1
  fi
}

# One local origin with one commit, shared by every case.
origin="$tmp/origin.git"
git init -q --bare -b main "$origin"
root=$(git -C "$origin" mktree </dev/null)
first=$(git -C "$origin" -c user.name=t -c user.email=t@example.test commit-tree -m init "$root")
git -C "$origin" update-ref refs/heads/main "$first"

# reset STATE_TEXT: a fresh repository on that origin, and that state file if given.
reset() {
  rm -rf "$proj" "$tmp"/gh.*
  mkdir -p "$dir"
  git init -q -b main "$proj"
  echo '.claude/build/' >"$proj/.gitignore"
  git -C "$proj" remote add origin "$origin"
  if [ $# -gt 0 ]; then printf %b "$1" >"$state"; fi
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
run "failing check refuses" 2 'not passing: echo boom; false'
expect test -f "$state"

reset 'status: working\ncheck: true\n'
check "check: passes without a pr" 0 'pass true'
run "missing pr refuses" 2 'pr:'

reset 'status: working\ncheck: test -f marker\npr: https://example.test/1\ncomment: https://example.test/1#c\n'
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

# Stop: CI on the pull request must be green (gh pr checks buckets).
pr_state='status: working\ncheck: true\npr: https://example.test/1\n'
commented="${pr_state}comment: https://example.test/1#c\n"

reset "$commented"
check "check: passes" 0 'pass true'
gh_says 'pass\npending\n' 8 ''
run "pending CI check refuses" 2 'pending'
expect test ! -e "$dir/done"
expect grep -qx 'pr checks https://example.test/1 --json bucket --jq .\[\].bucket' "$tmp/gh.log"
gh_says 'pass\nskipping\n' 0 ''
run "skipping CI check counts as green" 0 ''
expect cmp -s "$state" "$dir/done"

reset "$pr_state"
check "check: passes" 0 'pass true'
gh_says 'pass\nfail\npending\n' 1 ''
run "failing CI check refuses" 2 'failing'
expect test ! -e "$dir/done"
gh_says 'pass\ncancel\n' 1 ''
run "cancelled CI check refuses" 2 'failing'
expect test ! -e "$dir/done"

reset "$commented"
check "check: passes" 0 'pass true'
gh_says '' 1 "no checks reported on the 'main' branch\n"
run "no CI checks allows stop" 0 ''
expect cmp -s "$state" "$dir/done"

reset "$pr_state"
check "check: passes" 0 'pass true'
gh_says '' 4 'gh: authentication required'
run "other gh failure refuses" 2 '.'
expect test ! -e "$dir/done"

# A gh warning on stderr is never read as a bucket.
reset "$commented"
check "check: passes" 0 'pass true'
gh_says 'pass\n' 0 'warning: 1 check failed to load\n'
run "gh warning on stderr allows stop" 0 ''
expect cmp -s "$state" "$dir/done"

# Stop: the ideas and retro comment must be recorded once CI is green.
reset "$pr_state"
check "check: passes" 0 'pass true'
gh_says 'pending\n' 8 ''
run "pending CI without a comment refuses for CI first" 2 'pending'
gh_says 'pass\n' 0 ''
run "green CI without a comment refuses" 2 'comment:'
expect test ! -e "$dir/done"
echo 'comment: https://example.test/1#c' >>"$state"
run "green CI with a comment allows stop" 0 ''
expect cmp -s "$state" "$dir/done"

# gh is not asked before a pr is recorded.
reset 'status: working\ncheck: true\n'
check "check: passes without a pr" 0 'pass true'
run "missing pr refuses without asking gh" 2 'pr:'
expect test ! -e "$tmp/gh.log"

# Stop while subagents run: the Stop hook input lists background_tasks.
shell='{"id":"b","type":"shell","status":"running"}'
# stop_is WANT INPUT: a refusable build; stop with that input exits WANT, else the case is bad.
good=1
stop_is() {
  code=0
  printf %s "$2" | sh "$hook" stop >/dev/null 2>&1 || code=$?
  [ "$code" -eq "$1" ] || good=0
}
# stop_with WANT SESSION TASK...: stop_is on Stop hook input with those background
# tasks; a TASK is a subagent status, or a whole entry when it starts with "{".
stop_with() {
  want=$1
  sid=$2
  shift 2
  list=
  for e in "$@"; do
    case $e in
      '{'*) item=$e ;;
      *) item="{\"id\":\"a\",\"type\":\"subagent\",\"status\":\"$e\"}" ;;
    esac
    list="${list:+$list,}$item"
  done
  stop_is "$want" "{\"hook_event_name\":\"Stop\",\"session_id\":\"$sid\",\"stop_hook_active\":false,\"background_tasks\":[$list]}"
}
failing='status: working\ncheck: false\n'

reset "$failing"
echo 3 >"$dir/refusals"
good=1
# Field order and whitespace vary.
stop_is 0 '{ "background_tasks": [ { "status": "running", "type": "subagent", "id": "x" } ], "session_id": "s" }'
expect grep -qx 3 "$dir/refusals"
stop_with 2 s "$shell"
# A brace inside a string is not the end of the entry.
stop_with 0 s '{"id":"c","type":"subagent","description":"fix {x}","status":"running"}'
# A string ending in a keyword after an escaped quote does not mispair quotes.
stop_is 0 '{"last_assistant_message":"he said \"type","background_tasks":[{"type":"subagent","status":"running"}],"session_id":"s"}'
expect -n "stop passes while a subagent runs" test "$good" -eq 1

reset "$failing"
good=1
stop_with 0 s completed running
stop_with 0 s completed pending
stop_with 2 s completed failed killed
expect -n "stop refused only once every subagent ends" test "$good" -eq 1

reset "$failing"
good=1
# The hook keeps nothing between stops.
stop_with 0 s1 running
stop_with 2 s2
expect -n "subagent from an ended session does not count" test "$good" -eq 1

# Check mode.
reset 'status: working\ncheck: true\ncheck: false\ncheck: test -f marker\n'
: >"$proj/marker"
check "check: records every result" 1 'fail false'
printf 'pass true\nfail false\npass test -f marker\n' >"$tmp/want"
grep -v '^tree ' "$results" >"$tmp/got"
expect cmp -s "$tmp/want" "$tmp/got"
expect test ! -e "$dir/refusals"

# The main session learns which check failed, never its output.
reset 'status: working\ncheck: cat secret; false\n'
echo leaked-output >"$proj/secret"
check "check: hides a failing check's output" 1 'fail cat secret; false'
expect sh -c '! grep -q leaked-output "$1" "$2"' - "$tmp/out" "$tmp/err"

reset 'status: working\ncheck: read x\n'
check "check: commands get no stdin" 1 'fail read x'

reset
check "check: no state fails" 1 'no build'

# Checks on named tests of one suite run the suite once; each check is its own.
cat >"$tmp/suite.sh" <<'END'
echo run >>"$(dirname "$0")/suite.count"
for t in p01 p02 p03 p04 p05 p06 p07 p08 p09; do echo "test $t ... ok"; done
for t in f01 f02 f03 f04; do echo "test $t ... FAILED"; done
exit 1
END
# add_check WANT COMMAND: a check and the result it must record.
add_check() {
  echo "check: $2" >>"$state"
  echo "$1 $2" >>"$tmp/want"
}
reset
rm -f "$tmp/suite.count" "$tmp/want"
echo 'status: working' >"$state"
for t in p01 f01 p02 p03 f02 p04 p05 p06 f03 p07 p08 f04 p09; do
  case $t in
    p*) add_check pass "sh $tmp/suite.sh 2>&1 | grep -q '$t ... ok'" ;;
    *) add_check fail "sh $tmp/suite.sh 2>&1 | grep -q '$t ... ok'" ;;
  esac
done
add_check fail "false && sh $tmp/suite.sh | wc -l"
add_check fail "false && sh $tmp/suite.sh | wc -c"
code=0
sh "$hook" check >"$tmp/out" 2>&1 || code=$?
grep -v '^tree ' "$results" >"$tmp/got" || true
good=1
runs=$(wc -l <"$tmp/suite.count")
[ "$runs" -eq 1 ] || good=0
[ "$code" -eq 1 ] || good=0
expect -n "one suite run serves many checks" test "$good" -eq 1
expect -n "results list every check" cmp -s "$tmp/want" "$tmp/got"

# Check fetches main from origin first; this case moves its own copy of origin.
reset 'status: working\ncheck: true\n'
moved="$tmp/moved.git"
cp -R "$origin" "$moved"
git -C "$proj" remote set-url origin "$moved"
more=$(git -C "$moved" -c user.name=t -c user.email=t@example.test commit-tree -m more -p main 'main^{tree}')
git -C "$moved" update-ref refs/heads/main "$more"
sh "$hook" check >/dev/null 2>&1 || true
fetched=$(git -C "$proj" rev-parse origin/main 2>/dev/null) || fetched=
expect -n "check fetches main from origin" test "$fetched" = "$more"

reset 'status: working\ncheck: true\n'
git -C "$proj" remote set-url origin "$tmp/missing.git"
code=0
sh "$hook" check >"$tmp/out" 2>&1 || code=$?
good=1
[ "$code" -ne 0 ] || good=0
grep -qi fetch "$tmp/out" || good=0
expect -n "check fails when the fetch fails" test "$good" -eq 1

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

# mods_base: commit a failing type-check stub and make it origin/main, on an origin of its own that check can fetch.
mods_base() {
  mkdir -p "$proj/.claude/mods"
  echo 'exit 1' >"$proj/.claude/mods/typecheck.sh"
  git -C "$proj" add -A
  git -C "$proj" -c user.name=t -c user.email=t@t commit -qm base
  rm -rf "$tmp/mods.git"
  git init -q --bare -b main "$tmp/mods.git"
  git -C "$proj" push -q "$tmp/mods.git" HEAD:main
  git -C "$proj" remote set-url origin "$tmp/mods.git"
  git -C "$proj" fetch -q origin main
}

# Check: a build that changes .claude/mods fails while the type-check fails.
reset 'status: working\ncheck: true\npr: https://example.test/1\ncomment: https://example.test/1#c\n'
mods_base
echo 'x' >"$proj/.claude/mods/new.ts"
check "check: failing type-check fails a mods change" 1 ''
run "stop refuses while the type-check fails" 2 'not passing: sh .claude/mods/typecheck.sh'

reset 'status: working\ncheck: true\ncheck: sh .claude/mods/typecheck.sh\n'
mods_base
echo 'x' >"$proj/.claude/mods/new.ts"
check "check: failing type-check fails with its own check line" 1 ''

reset 'status: working\ncheck: true\n'
mods_base
echo 'x' >"$proj/other"
check "check: type-check skipped when mods are unchanged" 0 'pass true'

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

# Next: the lowest build-ready issue blocked by no open issue.
# Candidates are "NUMBER BUILDING" lines in any order.
# blocked N STATE...: the states of issue N's blockers.
blocked() {
  n=$1
  shift
  printf '%s\n' "$@" >"$tmp/gh.blocked.$n"
}
mode=next
reset
gh_says '10 false\n2 true\n7 false\n3 false\n5 false\n' 0 ''
blocked 3 closed open
run 'next: lowest eligible' 0 '^5$'

reset
gh_says '8 false\n4 false\n' 0 ''
blocked 4 closed closed
run 'next: closed blockers' 0 '^4$'

reset
gh_says '2 false\n6 true\n' 0 ''
blocked 2 closed open
run 'next: none' 1 '.'
mode=stop

[ "$failed" -eq 0 ] && echo PASS
exit "$failed"
