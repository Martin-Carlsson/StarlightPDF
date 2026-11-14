#!/bin/sh
# Tests for hook.sh branch naming (issue 10). Run: sh branch.test.sh
set -eu

hook="$(cd "$(dirname "$0")" && pwd)/hook.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# No user or system git configuration, no prompts.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 GIT_TERMINAL_PROMPT=0
proj="$tmp/proj"
export CLAUDE_PROJECT_DIR="$proj"
dir="$proj/.claude/build"
state="$dir/state"
origin="$tmp/origin.git"
pre=claude/nice-hamilton-6ww6sb
names="$tmp/names"
: >"$names"
mkdir "$tmp/cwd"
: >"$tmp/cwd/globbed"
failed=0

fail() {
  echo "FAIL $*"
  failed=1
}

expect() {
  if ! "$@"; then fail ": $*"; fi
}

ident() {
  git -C "$1" config user.name Test
  git -C "$1" config user.email test@example.test
  git -C "$1" config commit.gpgsign false
}

# commit DIR FILE: a new commit adding FILE.
commit() {
  echo "$2" >"$1/$2"
  git -C "$1" add -A
  git -C "$1" commit -q -m "$2"
}

# repo DIR: a fresh repository on main with one commit.
repo() {
  rm -rf "$1"
  git init -q "$1"
  git -C "$1" symbolic-ref HEAD refs/heads/main
  ident "$1"
  echo '.claude/build/' >"$1/.gitignore"
  : >"$1/globbed"
  commit "$1" first
}

# reset [STATE_LINE...]: a fresh project without a remote, with that state if given.
reset() {
  repo "$proj"
  mkdir -p "$dir"
  if [ $# -gt 0 ]; then printf '%s\n' "$@" >"$state"; fi
}

# seed: a bare origin with main and the pre-assigned branch on one commit.
seed() {
  rm -rf "$origin"
  git init -q --bare "$origin"
  git -C "$origin" symbolic-ref HEAD refs/heads/main
  repo "$tmp/seed"
  git -C "$tmp/seed" remote add origin "$origin"
  git -C "$tmp/seed" push -q origin main "main:refs/heads/$pre" 2>/dev/null
}

# enter [STATE_LINE...]: identity and state for a new clone.
enter() {
  ident "$proj"
  mkdir -p "$dir"
  if [ $# -gt 0 ]; then printf '%s\n' "$@" >"$state"; fi
}

# clone [STATE_LINE...]: the project as a session starts it, on the pre-assigned branch.
clone() {
  rm -rf "$proj"
  git clone -q -b "$pre" "$origin" "$proj" 2>/dev/null
  enter "$@"
}

# shallow [STATE_LINE...]: the same, with one commit of the pre-assigned branch alone.
shallow() {
  rm -rf "$proj"
  git clone -q --depth 1 --single-branch -b "$pre" "file://$origin" "$proj" 2>/dev/null
  enter "$@"
}

# extra: the named branch on origin, one commit past main.
extra() {
  git -C "$tmp/seed" checkout -q -b "$want"
  commit "$tmp/seed" extra
  extra_sha=$(sha "$tmp/seed" HEAD)
  git -C "$tmp/seed" push -q origin "$want" 2>/dev/null
}

# quiet NAME: the last run printed nothing and gave a build: message.
quiet() {
  if ! grep -q '^build: ' "$tmp/err"; then fail "$1: no message"; fi
  if [ -s "$tmp/out" ]; then fail "$1: printed '$(cat "$tmp/out")'"; fi
}

# sha DIR REF: the commit, or nothing.
sha() {
  git -C "$1" rev-parse -q --verify "$2^{commit}" 2>/dev/null || true
}

current() {
  ref=$(git -C "$proj" symbolic-ref -q HEAD 2>/dev/null || true)
  printf '%s\n' "${ref#refs/heads/}"
}

upstream() {
  git -C "$proj" for-each-ref --format='%(upstream:short)' "refs/heads/$1"
}

# br NAME WANT_EXIT: run branch mode.
br() {
  code=0
  if [ -f "$state" ]; then cp "$state" "$tmp/before"; fi
  (cd "$tmp/cwd" && sh "$hook" branch </dev/null >"$tmp/out" 2>"$tmp/err") || code=$?
  if [ "$code" -ne "$2" ]; then fail "$1: exit $code, stderr: $(cat "$tmp/err")"; fi
  # Branch mode never writes the state file.
  if [ -f "$tmp/before" ] && ! cmp -s "$tmp/before" "$state"; then fail "$1: state changed"; fi
  rm -f "$tmp/before"
}

# on NAME WANT: the last run printed exactly WANT and checked it out.
on() {
  printf '%s\n' "$2" >"$tmp/want"
  if ! cmp -s "$tmp/want" "$tmp/out"; then fail "$1: printed '$(cat "$tmp/out")', want '$2'"; fi
  if [ "$(current)" != "$2" ]; then fail "$1: on '$(current)', want '$2'"; fi
  if [ "${#2}" -gt 60 ]; then fail "$1: expectation longer than 60"; fi
  # 5: every name is a valid branch name.
  if ! git check-ref-format --branch "$2" >/dev/null 2>&1; then fail "$1: invalid ref '$2'"; fi
  cat "$tmp/out" >>"$names"
}

# named NAME ISSUE_LINE WANT: that issue line gives that branch.
named() {
  reset "$2"
  br "$1" 0
  on "$1" "$3"
}

# bad NAME [STATE_LINE...]: exit 1 with a message, and no switch.
bad() {
  name=$1
  shift
  reset "$@"
  br "$name" 1
  if [ ! -s "$tmp/err" ]; then fail "$name: no message"; fi
  if [ -s "$tmp/out" ]; then fail "$name: printed '$(cat "$tmp/out")'"; fi
  if [ "$(current)" != main ]; then fail "$name: on '$(current)', want 'main'"; fi
}

# stop NAME WANT_EXIT WANT_STDERR_PATTERN [MODE]
stop() {
  code=0
  sh "$hook" "${4:-stop}" </dev/null >"$tmp/out" 2>"$tmp/err" || code=$?
  if [ "$code" -ne "$2" ] || { [ -n "$3" ] && ! grep -q "$3" "$tmp/err"; }; then
    fail "$1: exit $code, stderr: $(cat "$tmp/err")"
  fi
}

tab=$(printf '\t')

# 1: the issue's own example.
want=12-name-build-branches-after-the-issue
issue='issue: 12 Name build branches after the issue'
named "1 example" "$issue" "$want"

# 2: lowercase; each run of other characters becomes one '-'; none at the ends.
named "2 mixed case" 'issue: 2 ADD CamelCase And UPPER' 2-add-camelcase-and-upper
named "2 punctuation runs" 'issue: 3 Fix PDF/A-1b: XMP -- metadata!!' 3-fix-pdf-a-1b-xmp-metadata
named "2 punctuation at the ends" 'issue: 4 ...[WIP] Hello, World?!...' 4-wip-hello-world
named "2 digits" 'issue: 5 Support ISO 32000-2:2020' 5-support-iso-32000-2-2020
named "2 tabs and spaces" "issue: 6 ${tab}Tabs$tab${tab}and   spaces $tab" 6-tabs-and-spaces
# Non-ASCII bytes are not a-z or 0-9, in either case.
named "2 non-ASCII" 'issue: 8 Größe é überprüfen' 8-gr-e-berpr-fen
named "2 non-ASCII capital" 'issue: 9 École Über' 9-cole-ber
# The title is data: nothing runs, globs or is taken as a format.
IFS= read -r hostile <<'END'
issue: 11 $(touch pwned) `touch pwned2` "q" 'r' * ; touch pwned3 %s x\ny
END
named "2 shell characters" "$hostile" 11-touch-pwned-touch-pwned2-q-r-touch-pwned3-s-x-ny
expect test -z "$(find "$tmp" -name 'pwned*')"

# 3: at most 60 characters in all, cut from the end, never ending in '-'.
w='aaaaaaaaa bbbbbbbbb ccccccccc ddddddddd eeeeeeeee'
p=aaaaaaaaa-bbbbbbbbb-ccccccccc-ddddddddd-eeeeeeeee
expect test "${#p}" -eq 49
# 4 + 49 + 7 = 60: kept whole.
named "3 exactly 60" "issue: 100 $w ffffff" "100-$p-ffffff"
# 61: the last character goes.
named "3 one over" "issue: 100 $w fffffff" "100-$p-ffffff"
# Character 60 is a '-': it goes too, leaving 59.
named "3 cut after a dash" "issue: 100 $w fffff ggg" "100-$p-fffff"
named "3 cut after a run" "issue: 100 $w fffff -- ggg" "100-$p-fffff"
# Character 61 is the '-'.
named "3 cut before a dash" "issue: 100 $w ffffff ggg" "100-$p-ffffff"
# The number counts: 6 + 49 + 5 = 60.
named "3 longer number" "issue: 12345 $w ffffff" "12345-$p-ffff"
named "3 sentence" \
  'issue: 10 Shorten very long issue titles so that the branch name stays within sixty characters' \
  10-shorten-very-long-issue-titles-so-that-the-branch-name-st
expect test "$(tail -n 1 "$names" | tr -d '\n' | wc -c)" -eq 60

# 4: no letters or digits gives the number alone.
named "4 punctuation only" 'issue: 7 !!! ??? ---' 7
named "4 non-ASCII only" 'issue: 7 äöü' 7
named "4 no title" 'issue: 7' 7
named "4 empty title" 'issue: 7 ' 7

# Errors: exit 1, no switch.
bad "no state"
bad "no issue line" 'status: working' 'check: true'
bad "number not digits" 'issue: abc Title'
bad "number partly digits" 'issue: 12a Title'
bad "no number" 'issue: '

# The other lines of the state do not matter.
reset 'status: working' "$issue" 'check: true'
br "issue among other lines" 0
on "issue among other lines" "$want"

# 7: an existing local branch is checked out, not recreated.
reset "$issue"
git -C "$proj" checkout -q -b "$want"
commit "$proj" local
local_sha=$(sha "$proj" HEAD)
git -C "$proj" checkout -q main
br "7 local branch" 0
on "7 local branch" "$want"
expect test "$(sha "$proj" HEAD)" = "$local_sha"
br "7 already on it" 0
on "7 already on it" "$want"
expect test "$(sha "$proj" HEAD)" = "$local_sha"

# 6: only the named branch is pushed; the pre-assigned branch gets no commits.
seed
clone "$issue"
expect test "$(current)" = "$pre"
expect test "$(upstream "$pre")" = "origin/$pre"
pre_sha=$(sha "$origin" "refs/heads/$pre")
br "6 pre-assigned branch" 0
on "6 pre-assigned branch" "$want"
expect test "$(upstream "$want")" != "origin/$pre"
commit "$proj" work
work_sha=$(sha "$proj" HEAD)
git -C "$proj" push -q origin HEAD 2>/dev/null || fail "6 push"
expect test "$(sha "$origin" "refs/heads/$pre")" = "$pre_sha"
expect test "$(sha "$origin" "refs/heads/$want")" = "$work_sha"
expect test "$work_sha" != "$pre_sha"
printf '%s\n' "refs/heads/$want" "refs/heads/$pre" refs/heads/main | sort >"$tmp/want"
git -C "$origin" for-each-ref --format='%(refname)' refs/heads | sort >"$tmp/got"
expect cmp -s "$tmp/want" "$tmp/got"

# 6: the named branch does not take the pre-assigned branch's upstream, whatever the configuration.
for setup in inherit always; do
  seed
  clone "$issue"
  git -C "$proj" config branch.autoSetupMerge "$setup"
  br "6 autoSetupMerge $setup" 0
  on "6 autoSetupMerge $setup" "$want"
  expect test -z "$(upstream "$want")"
  expect test -z "$(git -C "$proj" config --get-regexp "^branch\.$want\." || true)"
done

# 7: a branch already on the remote is continued, in a clone made after it was pushed.
seed
extra
clone "$issue"
br "7 remote branch" 0
on "7 remote branch" "$want"
expect git -C "$proj" merge-base --is-ancestor "$extra_sha" HEAD 2>/dev/null
expect test "$(upstream "$want")" = "origin/$want"
commit "$proj" more
git -C "$proj" push -q origin HEAD 2>/dev/null || fail "7 push"
expect test "$(sha "$origin" "refs/heads/$want")" = "$(sha "$proj" HEAD)"
expect git -C "$origin" merge-base --is-ancestor "$extra_sha" "refs/heads/$want" 2>/dev/null
expect test "$(sha "$origin" "refs/heads/$pre")" = "$(sha "$tmp/seed" main)"

# 7: and in a clone made before it was pushed, so it must be fetched.
seed
clone "$issue"
extra
br "7 remote branch not yet fetched" 0
on "7 remote branch not yet fetched" "$want"
expect git -C "$proj" merge-base --is-ancestor "$extra_sha" HEAD 2>/dev/null
expect test "$(upstream "$want")" = "origin/$want"

# 7: and in a shallow clone of the pre-assigned branch alone.
seed
extra
pre_sha=$(sha "$origin" "refs/heads/$pre")
shallow "$issue"
expect test "$(sha "$origin" "refs/heads/$want")" = "$extra_sha"
expect test -z "$(sha "$proj" "refs/remotes/origin/$want")"
br "7 shallow clone" 0
on "7 shallow clone" "$want"
expect git -C "$proj" merge-base --is-ancestor "$extra_sha" HEAD 2>/dev/null
expect test "$(upstream "$want")" = "origin/$want"
expect test "$(sha "$proj" "$want@{upstream}")" = "$extra_sha"
commit "$proj" more
git -C "$proj" push -q origin HEAD 2>/dev/null || fail "7 shallow push"
expect test "$(sha "$origin" "refs/heads/$want")" = "$(sha "$proj" HEAD)"
expect test "$(sha "$origin" "refs/heads/$pre")" = "$pre_sha"

# 7: a local branch behind the remote one is fast-forwarded to it.
seed
clone "$issue"
git -C "$proj" branch --no-track "$want"
extra
br "7 local behind" 0
on "7 local behind" "$want"
expect test "$(sha "$proj" HEAD)" = "$extra_sha"
expect test "$(upstream "$want")" = "origin/$want"
commit "$proj" more
git -C "$proj" push -q origin HEAD 2>/dev/null || fail "7 local behind push"
expect test "$(sha "$origin" "refs/heads/$want")" = "$(sha "$proj" HEAD)"

# 7: a local branch level with the remote one, or ahead of it, is left as it is.
seed
extra
clone "$issue"
git -C "$proj" branch --no-track "$want" "$extra_sha"
br "7 local level" 0
on "7 local level" "$want"
expect test "$(sha "$proj" HEAD)" = "$extra_sha"
expect test "$(upstream "$want")" = "origin/$want"

seed
extra
clone "$issue"
git -C "$proj" checkout -q --no-track -b "$want" "$extra_sha"
commit "$proj" local
local_sha=$(sha "$proj" HEAD)
git -C "$proj" checkout -q "$pre"
br "7 local ahead" 0
on "7 local ahead" "$want"
expect test "$(sha "$proj" HEAD)" = "$local_sha"
expect test "$(upstream "$want")" = "origin/$want"
git -C "$proj" push -q origin HEAD 2>/dev/null || fail "7 local ahead push"
expect test "$(sha "$origin" "refs/heads/$want")" = "$local_sha"

# 7: a local branch that has diverged from the remote one is an error.
seed
clone "$issue"
git -C "$proj" checkout -q --no-track -b "$want"
commit "$proj" local
local_sha=$(sha "$proj" HEAD)
git -C "$proj" checkout -q "$pre"
extra
br "7 diverged" 1
quiet "7 diverged"
expect test "$(current)" = "$pre"
expect test "$(sha "$proj" "refs/heads/$want")" = "$local_sha"
expect test "$(sha "$origin" "refs/heads/$want")" = "$extra_sha"

# 7: a fast-forward that uncommitted changes block is an error: no switch, nothing lost.
seed
clone "$issue"
git -C "$proj" branch --no-track "$want"
local_sha=$(sha "$proj" "refs/heads/$want")
git -C "$tmp/seed" checkout -q -b "$want"
echo theirs >"$tmp/seed/first"
git -C "$tmp/seed" commit -q -a -m theirs
git -C "$tmp/seed" push -q origin "$want" 2>/dev/null
echo mine >"$proj/first"
br "7 dirty tree" 1
quiet "7 dirty tree"
expect test "$(current)" = "$pre"
expect test "$(cat "$proj/first")" = mine
expect test "$(sha "$proj" "refs/heads/$want")" = "$local_sha"

# An origin that cannot be asked is an error: no switch, no new branch.
seed
clone "$issue"
git -C "$proj" remote set-url origin file:///nonexistent/x.git
br "unreachable origin" 1
quiet "unreachable origin"
expect test "$(current)" = "$pre"
expect test -z "$(sha "$proj" "refs/heads/$want")"
git -C "$proj" branch --no-track "$want"
br "unreachable origin, local branch" 1
quiet "unreachable origin, local branch"
expect test "$(current)" = "$pre"

# 5: every name produced above is a valid branch name.
expect test -s "$names"
while IFS= read -r name; do
  if ! git check-ref-format --branch "$name" >/dev/null 2>&1; then fail "5: invalid ref '$name'"; fi
done <"$names"

# 8: the state records the branch; stop is refused away from it.
pr='pr: https://example.test/1'
reset 'status: working' "$issue" 'check: true' "$pr"
git -C "$proj" checkout -q -b "$want"
stop "8 no branch line" 2 'branch:'

reset 'status: working' "$issue" 'branch: 12-something-else' 'check: true' "$pr"
git -C "$proj" checkout -q -b 12-something-else
stop "8 branch line is not the name" 2 'branch:'

reset 'status: working' "$issue" "branch: $want" 'check: true' "$pr"
git -C "$proj" branch "$want"
stop "8 on another branch" 2 "$want"
expect test "$(current)" = main

# The branch refusals come before the check refusals.
reset 'status: working' "$issue" "$pr"
git -C "$proj" checkout -q -b "$want"
stop "8 no branch line, no checks" 2 'branch:'

reset 'status: working' "$issue" "branch: $want" "$pr"
stop "8 on another branch, no checks" 2 "$want"

# On the recorded branch the earlier rules apply unchanged.
reset 'status: working' "$issue" "branch: $want" 'check: true' "$pr"
git -C "$proj" checkout -q -b "$want"
stop "8 on the branch, checks not run" 2 'hook.sh check'
expect test "$(grep -c branch "$tmp/err")" -eq 0
stop "8 check passes" 0 '' check
stop "8 on the branch, all pass" 0 ''
expect cmp -s "$state" "$dir/done"

# A tag of the same name does not hide the branch.
reset 'status: working' "$issue" "branch: $want" 'check: true' "$pr"
git -C "$proj" tag "$want"
br "8 tag, from main" 0
on "8 tag, from main" "$want"
br "8 tag, already on it" 0
on "8 tag, already on it" "$want"
stop "8 tag, checks not run" 2 'hook.sh check'
expect test "$(grep -c branch "$tmp/err")" -eq 0
stop "8 tag, check passes" 0 '' check
stop "8 tag, all pass" 0 ''

# A blocked build may stop on any branch; tool calls are not held back by the branch.
reset 'status: blocked' "$issue" 'check: true'
stop "8 blocked allows stop" 0 ''
reset 'status: working' "$issue" 'check: true'
stop "8 tool allowed" 0 '' tool

[ "$failed" -eq 0 ] && echo PASS
exit "$failed"
