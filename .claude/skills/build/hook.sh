#!/bin/sh
# Hooks for the build skill. Usage: hook.sh stop|tool|check|next|start N
# stop: refuse until every check passes on the current files, a PR is recorded, its CI is green and a comment is recorded.
#       Gives way once after 30 refusals with no check run in between.
#       Passes, uncounted, while its input lists a running or pending subagent.
# tool: refuse tool calls after 24 hours without the user.
# check: refuse while another check runs; else fetch origin main, run every check, and the mods type-check if mods changed; record the results. Only this mode runs the state file's commands.
# next: print the lowest open build-ready issue not building and blocked by no open issue; exit 1 if none.
# start N: exit 1 naming N's open blockers; else 3 if N is building and the state does not name it.
set -eu

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
dir=.claude/build
state=$dir/state
count=$dir/refusals
results=$dir/results
clock=$dir/started
done=$dir/done
lock=$dir/lock
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

# True if .claude/mods differs from its merge base with origin/main, or if there is no base.
mods_changed() {
  now=$(git rev-parse --verify -q "$(tree):.claude/mods" 2>/dev/null || true)
  base=$(git merge-base HEAD origin/main 2>/dev/null) &&
    base=$(git rev-parse --verify -q "$base:.claude/mods" 2>/dev/null) || base=
  [ "$now" != "$base" ]
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

# The open issues blocking issue $1, as #N lines.
blockers() {
  b=$(gh api --paginate "repos/{owner}/{repo}/issues/$1/dependencies/blocked_by" \
    --jq '.[] | "\(.number) \(.state)"') || return
  printf '%s\n' "$b" | sed -n 's/^\([0-9]*\) open$/#\1/p'
}

if [ "$1" = start ]; then
  open=$(blockers "$2")
  if [ -n "$open" ]; then
    echo "build: #$2 is blocked by open $(echo "$open" | paste -sd' ' -)" >&2
    exit 1
  fi
  building=$(gh issue view "$2" --json labels --jq 'any(.labels[]; .name == "building")')
  if [ "$building" = true ] && ! grep -q "^issue: $2 " "$state" 2>/dev/null; then
    echo "build: #$2 is labelled building; another build may be working on it" >&2
    exit 3
  fi
  exit 0
fi

if [ "$1" = next ]; then
  list=$(gh issue list --label build-ready --limit 1000 --json number,labels \
    --jq '.[] | "\(.number) \(any(.labels[]; .name == "building"))"')
  for n in $(printf '%s\n' "$list" | grep ' false$' | cut -d' ' -f1 | sort -n); do
    open=$(blockers "$n")
    if [ -z "$open" ]; then
      echo "$n"
      exit 0
    fi
  done
  echo 'build: no open build-ready issue is unblocked and not building' >&2
  exit 1
fi

if [ "$1" = check ]; then
  if ! grep -q '^check: .' "$state" 2>/dev/null; then
    echo 'build: no build in progress, or no check: lines' >&2
    exit 1
  fi
  # A link to the owner's PID; a dead owner's is taken over.
  take() { ln -s "$$" "$lock" 2>/dev/null && [ "$(readlink "$lock")" = "$$" ]; }
  busy() {
    echo 'build: a check is in progress; no checks run' >&2
    exit 1
  }
  if ! take; then
    p=$(readlink "$lock" 2>/dev/null) || p=
    case $p in
      0* | *[!0-9]*) ;;
      ?*) if kill -0 "$p" 2>/dev/null; then busy; fi ;;
    esac
    if [ -n "$p" ] && [ "$(readlink "$lock" 2>/dev/null)" = "$p" ]; then rm -f "$lock"; fi
    take || busy
  fi
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"; rm -f "$lock"' EXIT
  if ! git fetch -q origin main </dev/null; then
    echo 'build: git fetch origin main failed; no checks run' >&2
    exit 1
  fi
  rm -f "$count"
  failures=
  : >"$results.tmp"
  # A plain command shared by several checks before their first " | " runs once.
  shared() {
    case $1 in
      *' | '*)
        s=${1%% | *}
        t=$(printf '%s\n' "$s" | LC_ALL=C sed 's/2>&1//g; s|[[:alnum:] ._/=:,+-]||g')
        case $t in
          '') printf '%s\n' "$s" ;;
          *) ;;
        esac
        ;;
      *) ;;
    esac
  }
  sed -n 's/^check: //p' "$state" >"$tmp/checks"
  if mods_changed; then
    echo 'sh .claude/mods/typecheck.sh' >>"$tmp/checks"
  fi
  while IFS= read -r cmd; do
    shared "$cmd"
  done <"$tmp/checks" >"$tmp/suites"
  while IFS= read -r cmd; do
    suite=$(shared "$cmd")
    same=0
    if [ -n "$suite" ]; then same=$(grep -cxF -- "$suite" "$tmp/suites" || true); fi
    if [ "$same" -gt 1 ]; then
      n=$(grep -nxF -- "$suite" "$tmp/suites" | head -n 1 | cut -d: -f1)
      [ -f "$tmp/$n" ] || sh -c "$suite" </dev/null >"$tmp/$n" 2>/dev/null || true
      run="${cmd#"$suite" | }"
      in=$tmp/$n
    else
      run=$cmd
      in=/dev/null
    fi
    if sh -c "$run" <"$in" >/dev/null 2>&1; then
      echo "pass $cmd" >>"$results.tmp"
    else
      echo "fail $cmd" >>"$results.tmp"
      failures=1
    fi
  done <"$tmp/checks"
  cat "$results.tmp"
  echo "tree $(tree)" >>"$results.tmp"
  mv "$results.tmp" "$results"
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

input=
[ -t 0 ] || input=$(cat | LC_ALL=C tr '\n' ' ')
case $input in
  *'"background_tasks"'*)
    # Strings are paired first (control characters never occur raw in JSON), then
    # keywords are kept and the rest blanked, so no text can split an entry.
    o=$(printf '\001')
    c=$(printf '\002')
    b=$(printf '%s\n' "$input" | LC_ALL=C sed -E "
      s/\"(([^\"\\\\]|\\\\.)*)\"/${o}\\1${c}/g
      s/${o}(background_tasks|type|subagent|status|running|pending)${c}/@\\1@/g
      s/${o}[^${c}]*${c}/\"\"/g") || b=
    case $b in
      *@background_tasks@*)
        b=${b#*@background_tasks@}
        if printf '%s\n' "${b%%]*}" | tr '}' '\n' |
          grep '@type@[[:space:]]*:[[:space:]]*@subagent@' |
          grep -Eq '@status@[[:space:]]*:[[:space:]]*@(running|pending)@'; then
          exit 0
        fi
        ;;
      *) ;;
    esac
    ;;
  *) ;;
esac

grep -q '^check: .' "$state" || refuse "no check: lines in $state"
[ -f "$results" ] || refuse "the checks have not run; $again"

while IFS= read -r cmd; do
  grep -qxF "pass $cmd" "$results" || refuse "check not passing: $cmd; delegate the fix, then $again"
done <<END
$(sed -n 's/^check: //p' "$state")
END

# The type-check has no check: line.
fails=$(sed -n 's/^fail //p' "$results")
if [ -n "$fails" ]; then
  refuse "check not passing: $(printf '%s' "$fails" | head -n 1); delegate the fix, then $again"
fi

grep -qxF "tree $(tree)" "$results" || refuse "files changed since the checks ran; $again"

if ! grep -q '^pr: .' "$state"; then
  refuse "checks pass; open the pull request and add 'pr: <url>' to $state"
fi

pr=$(sed -n 's/^pr: //p' "$state" | head -n 1)
err=$(mktemp "$dir/gh.XXXXXX")
code=0
buckets=$(gh pr checks "$pr" --json bucket --jq '.[].bucket' 2>"$err") || code=$?
msg=$(cat "$err")
rm -f "$err"
if [ "$code" -ne 0 ] && [ -z "$buckets" ]; then
  case $msg in
    *'no checks reported'*) ;;
    *) refuse "gh failed: $msg" ;;
  esac
fi
case $buckets in
  *fail* | *cancel*) refuse "CI failing on $pr; delegate the fix, then follow the new run" ;;
  *pending*) refuse "CI pending on $pr; wait with: gh pr checks $pr --watch" ;;
esac

if ! grep -q '^comment: .' "$state"; then
  refuse "CI is green; post the ideas and retro comment on $pr and add 'comment: <url>' to $state"
fi

cp "$state" "$done"
rm -f "$count" "$clock"
