#!/bin/sh
# Hooks for the build skill. Usage: hook.sh stop|tool|check
# stop: refuse to stop until every check has passed on the current files and a PR is recorded.
#       Gives way once after 30 refusals with no check run in between.
# tool: refuse tool calls after 24 hours without the user.
# check: run every check and record the results. Only this mode runs commands.
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
