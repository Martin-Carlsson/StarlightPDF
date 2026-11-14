#!/bin/sh
# Hooks for the build skill. Usage: hook.sh stop|tool
# stop: refuse to stop until every check passes and a PR is recorded.
# tool: refuse tool calls after 24 hours without the user.
set -eu

cd "${CLAUDE_PROJECT_DIR:-.}"
state=.claude/build/state
count=.claude/build/refusals
cap=30
limit=86400

rewrite() {
  { "$@" || true; } >"$state.tmp"
  mv "$state.tmp" "$state"
}

# True if the limit has passed. Starts the clock if it is not running.
expired() {
  now=$(date +%s)
  started=$(sed -n 's/^started: //p' "$state")
  if [ -z "$started" ]; then
    echo "started: $now" >>"$state"
    return 1
  fi
  [ $((now - started)) -gt "$limit" ]
}

refuse() {
  n=0
  if [ -f "$count" ]; then n=$(cat "$count"); fi
  if [ "$n" -ge "$cap" ]; then
    rm -f "$count"
    rewrite grep -v '^status: ' "$state"
    echo 'status: blocked' >>"$state"
    printf '{"systemMessage":"build: gave up after %s refused stops"}\n' "$cap"
    exit 0
  fi
  echo $((n + 1)) >"$count"
  printf 'build: %s\n' "$1" >&2
  exit 2
}

[ -f "$state" ] || exit 0

if grep -q '^status: blocked$' "$state"; then
  if [ "$1" = stop ]; then rewrite grep -v '^started: ' "$state"; fi
  exit 0
fi

if expired; then
  if [ "$1" = tool ]; then
    echo 'build: 24 hours without the user. Stop and ask whether to continue.' >&2
    exit 2
  fi
  rewrite grep -v '^started: ' "$state"
  echo '{"systemMessage":"build: paused after 24 hours without the user"}'
  exit 0
fi

[ "$1" = stop ] || exit 0

checks=$(sed -n 's/^check: //p' "$state")
[ -n "$checks" ] || refuse "no check: lines in $state"

while IFS= read -r cmd; do
  out=$(sh -c "$cmd" </dev/null 2>&1) || refuse "check failed: $cmd
$out"
done <<END
$checks
END

if ! grep -q '^pr: .' "$state"; then
  refuse "checks pass; open the pull request and add 'pr: <url>' to $state"
fi

rm -f "$state" "$count"
