#!/bin/sh
# Stop hook: refuse to stop until every check in the state file passes and a PR is recorded.
set -eu

cd "${CLAUDE_PROJECT_DIR:-.}"
state=.claude/build/state
count=.claude/build/refusals
cap=30

[ -f "$state" ] || exit 0
if grep -q '^status: blocked$' "$state"; then exit 0; fi

refuse() {
  n=0
  if [ -f "$count" ]; then n=$(cat "$count"); fi
  if [ "$n" -ge "$cap" ]; then
    rm -f "$count"
    { echo 'status: blocked'; grep -v '^status: ' "$state" || true; } >"$state.tmp"
    mv "$state.tmp" "$state"
    printf '{"systemMessage":"build: gave up after %s refused stops"}\n' "$cap"
    exit 0
  fi
  echo $((n + 1)) >"$count"
  printf 'build: %s\n' "$1" >&2
  exit 2
}

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
