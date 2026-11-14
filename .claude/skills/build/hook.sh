#!/bin/sh
# Hooks for the build skill. Usage: hook.sh stop|tool|check|branch
# stop: refuse to stop until the build is on its branch, every check has passed on the
#       current files and a PR is recorded.
#       Gives way once after 30 refusals with no check run in between.
# tool: refuse tool calls after 24 hours without the user.
# check: run every check and record the results. Only this mode runs commands.
# branch: switch to the branch named after the issue, level with origin's, and print its name.
set -eu

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
dir=.claude/build
state=$dir/state
count=$dir/refusals
results=$dir/results
clock=$dir/started
done=$dir/done
cap=30
limit=86400
again='run: sh .claude/skills/build/hook.sh check'
switch='run: sh .claude/skills/build/hook.sh branch'

# A hash of the working tree: every file git does not ignore.
tree() {
  index=$(mktemp)
  cp "$(git rev-parse --git-path index 2>/dev/null)" "$index" 2>/dev/null || rm -f "$index"
  GIT_INDEX_FILE="$index" git add -A >/dev/null 2>&1 || true
  GIT_INDEX_FILE="$index" git write-tree 2>/dev/null || true
  rm -f "$index"
}

# True if the limit has passed. Starts the clock if it is not running.
expired() {
  now=$(date +%s)
  started=$(cat "$clock" 2>/dev/null || true)
  case $started in
    '' | *[!0-9]*)
      echo "$now" >"$clock.$$"
      mv "$clock.$$" "$clock"
      return 1
      ;;
  esac
  [ $((now - started)) -gt "$limit" ]
}

# Branch name from the issue: line: <number>-<slug of the title>. The title is untrusted.
branch() {
  issue=$(LC_ALL=C sed -n '/^issue: /{s///p;q;}' "$state" 2>/dev/null) || return 1
  number=${issue%% *}
  case $number in '' | *[!0-9]*) return 1 ;; esac
  title=
  case $issue in *' '*) title=${issue#* } ;; esac
  slug=$(printf '%s' "$title" | LC_ALL=C tr 'A-Z' 'a-z' | LC_ALL=C tr -cs 'a-z0-9' '-')
  slug=${slug#-}
  name=$(printf '%s' "$number-${slug%-}" | cut -c1-60)
  printf '%s\n' "${name%-}"
}

# Track origin's branch and fast-forward to it.
follow() {
  [ "$found" -eq 0 ] || return 0
  git config "branch.$name.remote" origin || return 1
  git config "branch.$name.merge" "$mine" || return 1
  # A clone of one branch does not map this one yet.
  if ! git rev-parse -q --verify "$mine@{upstream}" >/dev/null 2>&1; then
    git remote set-branches --add origin "$name" || return 1
  fi
  [ "$behind" -eq 0 ] || git merge -q --ff-only "$theirs"
}

# Return to where branch mode started.
back() {
  case $from in
    "$mine") return 0 ;;
    refs/heads/*)
      if git show-ref --verify --quiet "$from"; then
        git checkout -q "${from#refs/heads/}" --
      else
        git symbolic-ref HEAD "$from"
      fi
      ;;
    *) git checkout -q --detach "$from" -- ;;
  esac
  [ "$had" -eq 1 ] || git branch -q -D "$name"
}

refuse() {
  n=0
  if [ -f "$count" ]; then n=$(cat "$count"); fi
  if [ "$n" -ge "$cap" ]; then
    rm -f "$count"
    printf '{"systemMessage":"build: gave up after %s refused stops"}\n' "$cap"
    exit 0
  fi
  echo $((n + 1)) >"$count"
  printf 'build: %s\n' "$1" >&2
  exit 2
}

if [ "$1" = check ]; then
  if ! grep -q '^check: .' "$state" 2>/dev/null; then
    echo 'build: no build in progress, or no check: lines' >&2
    exit 1
  fi
  rm -f "$count"
  failures=
  : >"$results.tmp"
  while IFS= read -r cmd; do
    if out=$(sh -c "$cmd" </dev/null 2>&1); then
      echo "pass $cmd" >>"$results.tmp"
    else
      echo "fail $cmd" >>"$results.tmp"
      failures="${failures}check failed: $cmd
$out
"
    fi
  done <<END
$(sed -n 's/^check: //p' "$state")
END
  cat "$results.tmp"
  echo "tree $(tree)" >>"$results.tmp"
  mv "$results.tmp" "$results"
  printf '%s' "$failures" >&2
  [ -z "$failures" ]
  exit
fi

if [ "$1" = branch ]; then
  if ! name=$(branch); then
    echo "build: no 'issue: <number> <title>' line in $state" >&2
    exit 1
  fi
  found=2
  if git remote 2>/dev/null | grep -qx origin; then
    found=0
    git ls-remote --exit-code --heads origin "refs/heads/$name" >/dev/null || found=$?
  fi
  if [ "$found" -ne 0 ] && [ "$found" -ne 2 ]; then
    echo "build: cannot ask origin for branch $name" >&2
    exit 1
  fi
  if [ "$found" -eq 0 ]; then
    git fetch -q origin "+refs/heads/$name:refs/remotes/origin/$name" >&2 || exit 1
  fi
  mine=refs/heads/$name
  theirs=refs/remotes/origin/$name
  had=0
  behind=0
  if git show-ref --verify --quiet "$mine"; then
    had=1
    if [ "$found" -ne 0 ] || git merge-base --is-ancestor "$theirs" "$mine" 2>/dev/null; then
      :
    elif git merge-base --is-ancestor "$mine" "$theirs" 2>/dev/null; then
      behind=1
    else
      echo "build: cannot fast-forward $name to origin/$name" >&2
      exit 1
    fi
  fi
  from=$(git symbolic-ref -q HEAD || git rev-parse -q --verify HEAD || true)
  if [ "$had" -eq 1 ]; then
    git checkout -q "$name" -- >&2 || exit 1
  elif [ "$found" -eq 0 ]; then
    git checkout -q --no-track -b "$name" "$theirs" -- >&2 || exit 1
  else
    git checkout -q --no-track -b "$name" >&2 || exit 1
  fi
  if ! follow >&2; then
    echo "build: cannot fast-forward $name to origin/$name" >&2
    back >&2 || true
    exit 1
  fi
  echo "$name"
  exit 0
fi

[ -f "$state" ] || exit 0
# A finished build is kept for the pane; a new build's state differs from it.
if cmp -s "$state" "$done"; then exit 0; fi

if grep -q '^status: blocked$' "$state"; then
  if [ "$1" = stop ]; then rm -f "$clock"; fi
  exit 0
fi

if expired; then
  if [ "$1" = tool ]; then
    echo 'build: 24 hours without the user. Stop and ask whether to continue.' >&2
    exit 2
  fi
  rm -f "$clock"
  echo '{"systemMessage":"build: paused after 24 hours without the user"}'
  exit 0
fi

[ "$1" = stop ] || exit 0

if ! want=$(branch) || ! grep -qxF "branch: $want" "$state"; then
  refuse "no branch: line for the issue in $state; $switch, then add 'branch: <name>' to $state"
fi
if [ "$(git symbolic-ref -q HEAD || true)" != "refs/heads/$want" ]; then
  refuse "not on branch $want; $switch"
fi

grep -q '^check: .' "$state" || refuse "no check: lines in $state"
[ -f "$results" ] || refuse "the checks have not run; $again"

while IFS= read -r cmd; do
  grep -qxF "pass $cmd" "$results" || refuse "check not passing: $cmd; fix it, then $again"
done <<END
$(sed -n 's/^check: //p' "$state")
END

grep -qxF "tree $(tree)" "$results" || refuse "files changed since the checks ran; $again"

if ! grep -q '^pr: .' "$state"; then
  refuse "checks pass; open the pull request and add 'pr: <url>' to $state"
fi

cp "$state" "$done"
rm -f "$count" "$clock"
