#!/bin/sh
# Tests for hook.sh. Run: sh hook.test.sh
set -eu

hook="$(cd "$(dirname "$0")" && pwd)/hook.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export TMPDIR="$tmp"
proj="$tmp/proj"
export CLAUDE_PROJECT_DIR="$proj"

# Stub gh: logs its args to gh.log. Answers issue N's blocked_by
# query with gh.blocked.N (none if absent); the pull request's head,
# labels and body with gh.head, gh.labels and gh.body; its checks with
# gh.checks ("NAME BUCKET LINK" lines); a merge with gh.merge.err and
# exit gh.merge.code (default 0); a pr edit with nothing. issue edit N
# --add-label building claims N. With gh.issues ("NUMBER BUILDING" lines),
# issue list prints it after a delay (search lags); issue view N, whether N is claimed.
# Else prints gh.out (default pass), exits gh.code (default 0), writes gh.err to stderr.
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
  'pr view '*' --json headRefOid,labels,body '*)
    cat "$t/gh.head"
    sed 's/^/label: /' "$t/gh.labels"
    echo body:
    exec cat "$t/gh.body"
    ;;
  'pr checks '*' --json name,bucket,link '*)
    if [ ! -s "$t/gh.checks" ]; then
      echo "no checks reported on the '81-x' branch" >&2
      exit 1
    fi
    cat "$t/gh.checks"
    if grep -Eq ' (fail|cancel) ' "$t/gh.checks"; then exit 1; fi
    if grep -q ' pending ' "$t/gh.checks"; then exit 8; fi
    exit 0
    ;;
  'issue list '*)
    if [ -f "$t/gh.issues" ]; then
      sleep 0.5
      exec cat "$t/gh.issues"
    fi
    ;;
  'issue view '*)
    if [ -f "$t/gh.issues" ]; then
      n=$*
      n=${n#issue view }
      if grep -qx "${n%% *}" "$t/gh.claimed" 2>/dev/null; then echo true; else echo false; fi
      exit 0
    fi
    ;;
  'issue edit '*' --add-label building')
    n=$*
    n=${n#issue edit }
    echo "${n%% *}" >>"$t/gh.claimed"
    exit 0
    ;;
  'pr edit '*) exit 0 ;;
  'pr merge '*)
    if [ -f "$t/gh.merge.err" ]; then cat "$t/gh.merge.err" >&2; fi
    exit "$(cat "$t/gh.merge.code" 2>/dev/null || echo 0)"
    ;;
  *) ;;
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
run "skipping CI check counts as green" 2 'hook.sh merge'
expect test ! -e "$dir/done"

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
run "no CI checks counts as green" 2 'hook.sh merge'
expect test ! -e "$dir/done"

reset "$pr_state"
check "check: passes" 0 'pass true'
gh_says '' 4 'gh: authentication required'
run "other gh failure refuses" 2 '.'
expect test ! -e "$dir/done"

# A gh warning on stderr is never read as a bucket.
reset "$commented"
check "check: passes" 0 'pass true'
gh_says 'pass\n' 0 'warning: 1 check failed to load\n'
run "gh warning on stderr counts as green" 2 'hook.sh merge'
expect test ! -e "$dir/done"

# Stop: the ideas and retro comment must be recorded once CI is green.
reset "$pr_state"
check "check: passes" 0 'pass true'
gh_says 'pending\n' 8 ''
run "pending CI without a comment refuses for CI first" 2 'pending'
gh_says 'pass\n' 0 ''
run "green CI without a comment refuses" 2 'comment:'
expect test ! -e "$dir/done"
echo 'comment: https://example.test/1#c' >>"$state"
run "green CI with a comment asks for the merge" 2 'hook.sh merge'
expect test ! -e "$dir/done"

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

# A check's backslash is recorded as written.
reset
c="printf 'a\\n' | grep -qx a"
printf '%s\n' 'status: working' "check: $c" >"$state"
good=1
sh "$hook" check >"$tmp/out" 2>&1 || good=0
grep -qxF "pass $c" "$tmp/out" || good=0
grep -qxF "pass $c" "$results" || good=0
code=0
sh "$hook" stop </dev/null >"$tmp/out" 2>&1 || code=$?
[ "$code" -eq 2 ] && grep -q 'pr:' "$tmp/out" || good=0
expect -n "check: records a backslash intact" test "$good" -eq 1

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

# Overlapping checks. block.sh logs its PID to b/runs, then waits for b/go.N, N its run number.
cat >"$tmp/block.sh" <<'END'
b=$(dirname "$0")/b
echo "$$" >>"$b/runs"
n=$(($(wc -l <"$b/runs")))
w=0
while [ ! -e "$b/go.$n" ] && [ "$w" -lt 100 ]; do
  sleep 0.1
  w=$((w + 1))
done
END
# await COMMAND...: true once COMMAND is, polling for at most 10 seconds.
await() {
  w=0
  until "$@"; do
    [ "$w" -lt 100 ] || return 1
    sleep 0.1
    w=$((w + 1))
  done
}
runs() { [ "$(wc -l <"$tmp/b/runs")" -ge "$1" ]; }
# fresh: a blocking build with no runs yet.
fresh() {
  reset "status: working\ncheck: true\ncheck: sh $tmp/block.sh\n"
  rm -rf "$tmp/b"
  mkdir "$tmp/b"
  : >"$tmp/b/runs"
}
# bg NAME [MODE DIR]: MODE (default check) from DIR in the background; output in b/NAME.out, exit code in b/NAME.code once it ends.
bg() {
  (
    c=0
    CLAUDE_PROJECT_DIR=${3:-$proj} sh "$hook" "${2:-check}" >"$tmp/b/$1.out" 2>&1 || c=$?
    echo "$c" >"$tmp/b/$1.code"
  ) &
}

fresh
good=1
bg a
a=$!
await runs 1 || good=0
bg b
b=$!
await eval 'test -e "$tmp/b/b.code" || runs 2' || good=0
sh "$hook" check >/dev/null 2>&1 && good=0
: >"$tmp/b/go.1"
wait "$a"
: >"$tmp/b/go.2"
wait "$b"
[ "$(cat "$tmp/b/b.code")" -ne 0 ] || good=0
grep -qi 'progress' "$tmp/b/b.out" || good=0
[ "$(wc -l <"$tmp/b/runs")" -eq 1 ] || good=0
expect -n "overlapping check refuses" test "$good" -eq 1
printf 'pass true\npass sh %s/block.sh\n' "$tmp" >"$tmp/want"
good=1
[ "$(cat "$tmp/b/a.code")" -eq 0 ] || good=0
cmp -s "$tmp/want" "$tmp/b/a.out" || good=0
grep -v '^tree ' "$results" | cmp -s "$tmp/want" - || good=0
[ "$(grep -c '^tree [0-9a-f]' "$results")" -eq 1 ] || good=0
expect -n "overlapped check ends whole" test "$good" -eq 1

fresh
good=1
sh "$hook" check >/dev/null 2>&1 &
k=$!
await runs 1 || good=0
read -r self <"$tmp/b/runs" || true
kill -9 "$k" "$self" 2>/dev/null || true
wait "$k" 2>/dev/null || true
: >"$tmp/b/go.2"
code=0
sh "$hook" check >/dev/null 2>&1 || code=$?
[ "$code" -eq 0 ] || good=0
expect -n "check after killed run goes ahead" test "$good" -eq 1

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
# blocked N 'NUMBER STATE'...: issue N's blockers.
blocked() {
  n=$1
  shift
  printf '%s\n' "$@" >"$tmp/gh.blocked.$n"
}
mode=next
reset
gh_says '10 false\n2 true\n7 false\n3 false\n5 false\n' 0 ''
blocked 3 '20 closed' '21 open'
echo a >"$dir/behind"
run 'next: lowest eligible' 0 '^5$'
expect -n 'next: only my issues' grep -q '^issue list .*--author @me' "$tmp/gh.log"
expect -n 'next: claims it' grep -qx 'issue edit 5 --add-label building' "$tmp/gh.log"
expect -n 'next: claim clears heads behind main' test ! -e "$dir/behind"

reset
gh_says '8 false\n4 false\n' 0 ''
blocked 4 '20 closed' '21 closed'
run 'next: closed blockers' 0 '^4$'

reset
gh_says '2 false\n6 true\n' 0 ''
blocked 2 '20 closed' '21 open'
run 'next: none' 1 '.'
expect -n 'next: none claims nothing' sh -c '! grep -q "^issue edit" "$1"' _ "$tmp/gh.log"

mode=ready
reset
gh_says '9 false
' 0 ''
run 'ready: picks' 0 '^9$'
expect -n 'ready: claims nothing' sh -c '! grep -q "^issue edit" "$1"' _ "$tmp/gh.log"
mode=stop

# nexts ISSUE...: those issues eligible; next runs a from proj and b from a second worktree, together.
nexts() {
  reset
  printf '%s false\n' "$@" >"$tmp/gh.issues"
  git -C "$proj" -c user.name=t -c user.email=t@t commit -q --allow-empty -m base
  rm -rf "$tmp/wt" "$tmp/b"
  mkdir "$tmp/b"
  git -C "$proj" worktree add -q -b wt "$tmp/wt"
  bg a next "$proj"
  a=$!
  bg b next "$tmp/wt"
  b=$!
  wait "$a" "$b"
}
nexts 3 5
good=1
[ "$(cat "$tmp/b/a.code") $(cat "$tmp/b/b.code")" = '0 0' ] || good=0
[ "$(cat "$tmp/b/a.out" "$tmp/b/b.out" | sort | tr '\n' ' ')" = '3 5 ' ] || good=0
expect -n "next: concurrent runs pick different issues" test "$good" -eq 1

nexts 3
good=1
[ "$(sort "$tmp/b/a.code" "$tmp/b/b.code" | tr '\n' ' ')" = '0 1 ' ] || good=0
[ "$(cat "$tmp/b/a.out" "$tmp/b/b.out" | grep -cx 3)" -eq 1 ] || good=0
[ "$(cat "$tmp/b/a.out" "$tmp/b/b.out" | grep -c '^build: ')" -eq 1 ] || good=0
expect -n "next: second run finds none eligible" test "$good" -eq 1

# Start N: refuse an issue blocked by an open issue (exit 1), ask about
# one labelled building by another build (exit 3).
# The label query prints true or false.
snapshot() {
  git -C "$proj" for-each-ref refs/heads
  git -C "$proj" symbolic-ref HEAD
  ls -A "$dir"
  cat "$dir"/* 2>/dev/null || true
}
# start_is NAME WANT N [TEST...]: start N exits WANT, changes no build file,
# branch or HEAD, makes only gh reads, and each TEST (shell code) holds.
start_is() {
  name=$1
  want=$2
  snapshot >"$tmp/before"
  code=0
  sh "$hook" start "$3" </dev/null >"$tmp/out" 2>"$tmp/err" || code=$?
  shift 3
  good=1
  [ "$code" -eq "$want" ] || good=0
  snapshot | cmp -s "$tmp/before" - || good=0
  if grep -Ev '^(api|issue view) ' "$tmp/gh.log" 2>/dev/null |
    grep -q . ||
    grep -Eq -- ' (-X|--method|-f|-F|--field|--raw-field|--input)( |=|$)' "$tmp/gh.log" 2>/dev/null; then
    good=0
  fi
  for t in "$@"; do eval "$t" || good=0; done
  expect -n "$name" test "$good" -eq 1
}

reset
gh_says 'false\n' 0 ''
blocked 40 '11 closed' '12 open' '13 open'
start_is "start: open blocker names it and stops" 1 40 \
  'grep -qw 12 "$tmp/err"' 'grep -qw 13 "$tmp/err"' '! grep -qw 11 "$tmp/err"'

# Another build's issue 77 is not issue 7.
reset 'status: working\nissue: 77 t\n'
gh_says 'true\n' 0 ''
start_is "start: building elsewhere asks" 3 7

reset 'status: working\nissue: 7 t\n'
gh_says 'true\n' 0 ''
start_is "start: building here builds" 0 7

reset 'status: working\nissue: 9 t\n'
gh_says 'true\n' 0 ''
blocked 40 '12 open'
start_is "start: blocked and building refuses" 1 40

reset
gh_says 'false\n' 0 ''
blocked 40 '11 closed' '12 closed'
start_is "start: closed blockers builds" 0 40 \
  'grep -q "issues/40/dependencies/blocked_by" "$tmp/gh.log"'

# Merge: the build's pull request, on an origin of its own.
url=https://example.test/1
commit() {
  git -C "$proj" add -A
  git -C "$proj" -c user.name=t -c user.email=t@t commit -qm "$1"
}
# pr_base: origin main holds README.md, core/tests/a.rs, tests/b.txt and an empty tests/e; branch 81-x starts from it.
pr_base() {
  reset "status: working\ncheck: true\npr: $url\ncomment: $url#c\n"
  mkdir -p "$proj/core/tests" "$proj/tests"
  printf 'a\nb\n' >"$proj/core/tests/a.rs"
  printf 'a\nb\n' >"$proj/tests/b.txt"
  : >"$proj/tests/e"
  echo base >"$proj/README.md"
  commit base
  rm -rf "$tmp/pr.git" "$tmp/other"
  git init -q --bare -b main "$tmp/pr.git"
  git -C "$proj" remote set-url origin "$tmp/pr.git"
  git -C "$proj" push -q origin HEAD:main
  git -C "$proj" fetch -q origin main
  git -C "$proj" checkout -q -b 81-x
}
# pr_ready: commit and push the branch, run the checks, and give its head green ci, a clean body and a safe verdict.
pr_ready() {
  commit change-subject
  git -C "$proj" push -q -u origin 81-x
  sh "$hook" check >/dev/null 2>&1 || true
  head=$(git -C "$proj" rev-parse HEAD)
  echo "$head" >"$tmp/gh.head"
  echo 'ci pass https://example.test/runs/7' >"$tmp/gh.checks"
  echo feature >"$tmp/gh.labels"
  printf '%s\r\n' '## Merge impact' '' '- **API:** none' '- **Dependencies:** none' '- **Affects:** docs' >"$tmp/gh.body"
  echo "verdict: safe $head docs-only-reason" >>"$state"
}
# pr_docs: a ready pull request that changes README.md only.
pr_docs() {
  pr_base
  echo more >>"$proj/README.md"
  pr_ready
}
# merge_runs: run the merge command on a fresh gh log; sets code.
merge_runs() {
  rm -f "$tmp/gh.log"
  code=0
  sh "$hook" merge </dev/null >"$tmp/out" 2>&1 || code=$?
}
# calls: the label and merge calls in the gh log, in order.
calls() {
  awk '/--add-label auto-merged/ { printf "add " } /^pr merge / { printf "merge " } /--remove-label auto-merged/ { printf "remove " }' "$tmp/gh.log" 2>/dev/null || true
}
# merges: good=0 unless the merge command exits 0, labelling and then squash-merging exactly the head.
merges() {
  merge_runs
  if [ "$code" -ne 0 ] || [ "$(calls)" != 'add merge ' ] ||
    ! grep -q "^pr merge .*--squash" "$tmp/gh.log" ||
    ! grep -q "^pr merge .*--match-head-commit $head" "$tmp/gh.log"; then
    good=0
  fi
}
# stops CONDITION: good=0 unless the merge command exits non-zero, naming CONDITION, with no label or merge call.
stops() {
  merge_runs
  if [ "$code" -eq 0 ] || [ -n "$(calls)" ] || ! grep -q "^$1: " "$tmp/out"; then good=0; fi
}
# unset_verdict: drop the state's verdict lines.
unset_verdict() {
  grep -v '^verdict:' "$state" >"$tmp/state" || true
  mv "$tmp/state" "$state"
}

pr_docs
good=1
merges
for want in "$url" README.md feature https://example.test/runs/7 "$(git -C "$proj" rev-parse --short=7 HEAD)" docs-only-reason; do
  grep -qF -- "$want" "$tmp/out" || good=0
done
expect -n "merge merges when every condition holds" test "$good" -eq 1

pr_base
mkdir "$proj/ci"
echo x >"$proj/ci/x"
pr_ready
unset_verdict
good=1
stops paths
grep -q '^verdict: ' "$tmp/out" || good=0
expect -n "merge refuses when a condition fails" test "$good" -eq 1

pr_docs
echo 1 >"$tmp/gh.merge.code"
echo 'GraphQL: Head branch was modified. Review and try the merge again.' >"$tmp/gh.merge.err"
good=1
merge_runs
[ "$code" -ne 0 ] || good=0
[ "$(calls)" = 'add merge remove ' ] || good=0
grep -qF 'Head branch was modified' "$tmp/out" || good=0
expect -n "merge removes the label when GitHub refuses" test "$good" -eq 1

# Paths: README.md, GLOSSARY.md, core/, bindings/ and tests/ only, no .toml.
good=1
for p in .claude/x ci/x core/Cargo.toml; do
  pr_base
  mkdir -p "$proj/$(dirname "$p")"
  echo x >"$proj/$p"
  pr_ready
  stops paths
done
pr_base
mkdir -p "$proj/core/src" "$proj/bindings/py"
for p in README.md GLOSSARY.md core/src/x.rs bindings/py/x.py tests/new.txt; do echo x >>"$proj/$p"; done
pr_ready
merges
expect -n "merge condition paths" test "$good" -eq 1

# Tests: no file main has under a tests/ directory is deleted, renamed or loses a line.
good=1
for change in 'rm tests/e' 'mv tests/b.txt tests/c.txt' 'echo a >core/tests/a.rs'; do
  pr_base
  (cd "$proj" && eval "$change")
  pr_ready
  stops tests
done
pr_base
echo c >>"$proj/core/tests/a.rs"
echo c >>"$proj/tests/b.txt"
pr_ready
merges
expect -n "merge condition tests" test "$good" -eq 1

# API and dependencies: no such label, and the body's exact none lines.
pr_docs
good=1
for l in api-additive api-breaking adds-dependency; do
  printf 'feature\n%s\n' "$l" >"$tmp/gh.labels"
  stops "api and dependencies"
done
echo feature >"$tmp/gh.labels"
cp "$tmp/gh.body" "$tmp/body"
sed 's/^- \*\*API:\*\* none/- **API:** additive/' "$tmp/body" >"$tmp/gh.body"
stops "api and dependencies"
sed 's/^- \*\*Dependencies:\*\* none/- **Dependencies:** none yet/' "$tmp/body" >"$tmp/gh.body"
stops "api and dependencies"
expect -n "merge condition api and dependencies" test "$good" -eq 1

# CI: the ci check passed on the head commit.
pr_docs
good=1
for checks in 'ci pending https://example.test/runs/7' 'ci pass https://example.test/runs/7
ci fail https://example.test/runs/7' \
  'ci cancel https://example.test/runs/7' 'mods pass https://example.test/runs/8' ''; do
  printf '%s' "$checks" >"$tmp/gh.checks"
  stops ci
done
expect -n "merge condition ci" test "$good" -eq 1

# behind: main moves on origin after the checks; the merge command fetches it and fails up to date.
behind() {
  more=$(git -C "$tmp/pr.git" -c user.name=t -c user.email=t@t commit-tree -m more -p main 'main^{tree}')
  git -C "$tmp/pr.git" update-ref refs/heads/main "$more"
  stops "up to date"
}
pr_docs
good=1
behind
expect -n "merge condition up to date" test "$good" -eq 1

# Head: the last check run passed on exactly the head commit's files.
good=1
pr_docs
echo 'check: false' >>"$state"
sh "$hook" check >/dev/null 2>&1 || true
stops head
# Checked with a file the head lacks, then the file removed.
pr_docs
echo x >"$proj/core/extra.rs"
sh "$hook" check >/dev/null 2>&1 || true
rm "$proj/core/extra.rs"
stops head
# Uncommitted.
pr_docs
echo dirty >>"$proj/README.md"
stops head
# Unpushed: checked and committed locally, the pull request still on the old head.
pr_docs
echo local >>"$proj/README.md"
commit local
sh "$hook" check >/dev/null 2>&1 || true
stops head
# Foreign: someone else pushed to the branch.
pr_docs
git clone -q -b 81-x "$tmp/pr.git" "$tmp/other"
echo foreign >>"$tmp/other/README.md"
git -C "$tmp/other" -c user.name=t -c user.email=t@t commit -qam foreign
git -C "$tmp/other" push -q origin 81-x
foreign=$(git -C "$tmp/other" rev-parse HEAD)
echo "$foreign" >"$tmp/gh.head"
echo "verdict: safe $foreign foreign-reason" >>"$state"
stops head
expect -n "merge condition head" test "$good" -eq 1

# Verdict: safe, for the head commit.
pr_docs
good=1
unset_verdict
stops verdict
echo "verdict: unsafe $head doubt" >>"$state"
stops verdict
unset_verdict
echo "verdict: safe $(git -C "$proj" rev-parse origin/main) other-commit" >>"$state"
stops verdict
expect -n "merge condition verdict" test "$good" -eq 1

# Stop: once CI is green and the comment recorded, the merge command must have run on the current head.
# stop_says WANT PATTERN: good=0 unless stop exits WANT with output matching PATTERN.
stop_says() {
  code=0
  echo >"$tmp/out"
  sh "$hook" stop </dev/null >>"$tmp/out" 2>&1 || code=$?
  if [ "$code" -ne "$1" ] || ! grep -q "$2" "$tmp/out"; then good=0; fi
}
good=1
pr_docs
git -C "$proj" remote set-url origin "$tmp/none.git"
merge_runs
stop_says 2 'hook.sh merge'
pr_docs
unset_verdict
echo "verdict: unsafe $head doubt" >>"$state"
stop_says 2 'hook.sh merge'
merge_runs
[ "$code" -ne 0 ] || good=0
echo again >>"$proj/README.md"
commit again
git -C "$proj" push -q origin 81-x
sh "$hook" check >/dev/null 2>&1 || true
git -C "$proj" rev-parse HEAD >"$tmp/gh.head"
stop_says 2 'hook.sh merge'
merge_runs
[ "$code" -ne 0 ] || good=0
stop_says 0 ''
pr_docs
merges
echo 0 >"$dir/refusals"
stop_says 0 ''
expect -n "stop waits for merge on head" test "$good" -eq 1
expect test ! -e "$clock"
expect test ! -e "$dir/refusals"

# A finished build stays on disk for the pane, and no longer holds anything back.
expect cmp -s "$state" "$dir/done"
expect grep -qx 'pass true' "$results"
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

# Stop: a head behind main sends the build back to merge main, for a build's first three heads.
# Heads an earlier build left behind do not count; once up to date, the stop passes unmerged.
good=1
pr_docs
printf '%s\n' a b c >"$dir/behind"
sh "$hook" start 81 >/dev/null 2>&1 || good=0
unset_verdict
echo "verdict: unsafe $head doubt" >>"$state"
behind
stop_says 2 'origin/main'
# CI failing too: merging main comes first.
echo 'ci fail https://example.test/runs/7' >"$tmp/gh.checks"
echo fail >"$tmp/gh.out"
echo 1 >"$tmp/gh.code"
stops ci
stop_says 2 'origin/main'
rm "$tmp/gh.out" "$tmp/gh.code"
echo 'ci pass https://example.test/runs/7' >"$tmp/gh.checks"
git -C "$proj" -c user.name=t -c user.email=t@t merge -q -m main origin/main
git -C "$proj" push -q origin 81-x
sh "$hook" check >/dev/null 2>&1 || true
head=$(git -C "$proj" rev-parse HEAD)
echo "$head" >"$tmp/gh.head"
echo "verdict: unsafe $head doubt" >>"$state"
merge_runs
[ "$code" -ne 0 ] || good=0
stop_says 0 ''
expect -n "stop asks to merge main when the branch is behind" test "$good" -eq 1

good=1
pr_base
for want in 2 2 2 0; do
  echo "$want" >>"$proj/README.md"
  pr_ready
  behind
  merge_runs
  if [ "$want" -eq 2 ]; then stop_says 2 'origin/main'; else stop_says 0 ''; fi
done
expect -n "stop gives way on the fourth head behind main" test "$good" -eq 1

# Tool: during a build, only the merge command merges.
# tool_says WANT PATTERN TOOL INPUT_FIELDS: good=0 unless the tool hook exits WANT with output matching PATTERN.
tool_says() {
  code=0
  echo >"$tmp/out"
  printf '{"session_id":"s","hook_event_name":"PreToolUse","tool_name":"%s","tool_input":{%s}}' "$3" "$4" |
    sh "$hook" tool >>"$tmp/out" 2>&1 || code=$?
  if [ "$code" -ne "$1" ] || ! grep -q "$2" "$tmp/out"; then good=0; fi
}
good=1
reset 'status: working\ncheck: true\n'
tool_says 2 'hook.sh merge' Bash "\"command\":\"gh pr merge $url --squash\""
tool_says 0 '' Bash '"command":"sh .claude/skills/build/hook.sh merge"'
tool_says 0 '' Write '"file_path":"x.md","content":"never gh pr merge"'
reset
tool_says 0 '' Bash "\"command\":\"gh pr merge $url --squash\""
expect -n "tool refuses gh pr merge" test "$good" -eq 1

# Check: files both the build and main changed since their merge base fail it.
# main_changes FILE...: on origin, main appends a line to each FILE.
main_changes() {
  rm -rf "$tmp/other"
  git clone -q -b main "$tmp/pr.git" "$tmp/other"
  for f in "$@"; do echo main >>"$tmp/other/$f"; done
  git -C "$tmp/other" -c user.name=t -c user.email=t@t commit -qam main
  git -C "$tmp/other" push -q origin main
}
pr_base
echo branch >>"$proj/README.md"
commit branch
echo branch >>"$proj/tests/b.txt"
main_changes README.md tests/b.txt core/tests/a.rs
good=1
check "check: names files changed on both sides" 1 README.md
grep -qF tests/b.txt "$tmp/out" || good=0
! grep -qF core/tests/a.rs "$tmp/out" || good=0
code=0
sh "$hook" stop </dev/null >"$tmp/out" 2>&1 || code=$?
[ "$code" -eq 2 ] && ! grep -q 'hook.sh merge' "$tmp/out" || good=0
commit y
git -C "$proj" fetch -q origin main
git -C "$proj" -c user.name=t -c user.email=t@t merge -q -X ours --no-edit origin/main >/dev/null
check "check: names files changed on both sides" 0 'pass true'
stop_says 2 'hook.sh merge'
echo more >>"$proj/README.md"
check "check: names files changed on both sides" 0 'pass true'
expect -n "check: names files changed on both sides" test "$good" -eq 1

pr_base
echo branch >>"$proj/README.md"
commit branch
main_changes tests/b.txt
good=1
check "check: passes when main shares no file or is merged" 0 'pass true'
expect -n "check: passes when main shares no file or is merged" test "$good" -eq 1

[ "$failed" -eq 0 ] && echo PASS
exit "$failed"
