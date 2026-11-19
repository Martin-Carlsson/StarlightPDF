#!/bin/sh
# Hooks for the build skill. Usage: hook.sh stop|tool|check|ready|next|start N|merge|unneeded CMD
# stop: refuse until every check passes on the current files, a PR is recorded, its CI is green, a comment is recorded and merge has run on the current head and, for the first three heads behind main, found it up to date.
#       Gives way once after 30 refusals with no check run in between.
#       Passes, uncounted, while its input lists a running or pending subagent.
# tool: refuse tool calls after 24 hours without the user, and Bash calls running gh pr merge.
# check: refuse while another check runs; else fetch origin main, run every check, and the mods type-check if mods changed; fail each file main also changed since the merge base; record the results. Only this mode runs the state file's commands.
# ready: print the lowest open build-ready issue of mine not building and blocked by no open issue; exit 1 if none.
# next: as ready, one at a time across worktrees; label it building.
# start N: exit 1 naming N's open blockers; else 3 if N is building and the state does not name it.
# merge: label and squash-merge the PR if every condition holds; else name each failed one and exit 1.
# unneeded CMD: print each code line the branch adds outside tests and Markdown that sh -c CMD passes without; exit 1 if any.
set -eu

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
dir=.claude/build
state=$dir/state
count=$dir/refusals
results=$dir/results
clock=$dir/started
done=$dir/done
lock=$dir/lock
merged=$dir/merged
behind=$dir/behind
cap=30
limit=86400
again='run: sh .claude/skills/build/hook.sh check'
merge='sh .claude/skills/build/hook.sh merge'

# A hash of the working tree: every file git does not ignore.
tree() {
  index=$(mktemp)
  cp "$(git rev-parse --git-path index 2>/dev/null)" "$index" 2>/dev/null || rm -f "$index"
  GIT_INDEX_FILE="$index" git add -A >/dev/null 2>&1 || true
  GIT_INDEX_FILE="$index" git write-tree 2>/dev/null || true
  rm -f "$index"
}

# True if .claude/mods differs from merge base $1's, or $1 is empty.
mods_changed() {
  now=$(git rev-parse --verify -q "$(tree):.claude/mods" 2>/dev/null || true)
  base=
  if [ -n "$1" ]; then base=$(git rev-parse --verify -q "$1:.claude/mods" 2>/dev/null) || base=; fi
  [ "$now" != "$base" ]
}

# Print the first check not passing in the last run, fail lines with no check: line included; false if none.
failing() {
  {
    sed -n 's/^check: //p' "$state" | while IFS= read -r cmd; do
      grep -qxF "pass $cmd" "$results" 2>/dev/null || printf '%s\n' "$cmd"
    done
    sed -n 's/^fail //p' "$results" 2>/dev/null || true
  } | head -n 1 | grep .
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

# Take $lock, a link to the owner's PID; a dead owner's is taken over. False if a live process holds it.
hold() {
  take() { ln -s "$$" "$lock" 2>/dev/null && [ "$(readlink "$lock")" = "$$" ]; }
  take && return
  p=$(readlink "$lock" 2>/dev/null) || p=
  case $p in
    0* | *[!0-9]*) ;;
    ?*) if kill -0 "$p" 2>/dev/null; then return 1; fi ;;
  esac
  if [ -n "$p" ] && [ "$(readlink "$lock" 2>/dev/null)" = "$p" ]; then rm -f "$lock"; fi
  take
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
  rm -f "$behind"
  exit 0
fi

if [ "$1" = ready ] || [ "$1" = next ]; then
  if [ "$1" = next ]; then
    lock=$(git rev-parse --git-common-dir)/build-next
    until hold; do sleep 1; done
    trap 'rm -f "$lock"' EXIT
  fi
  list=$(gh issue list --label build-ready --author @me --limit 1000 --json number,labels \
    --jq '.[] | "\(.number) \(any(.labels[]; .name == "building"))"')
  for n in $(printf '%s\n' "$list" | grep ' false$' | cut -d' ' -f1 | sort -n); do
    open=$(blockers "$n")
    if [ -z "$open" ]; then
      if [ "$1" = next ]; then
        b=$(gh issue view "$n" --json labels --jq 'any(.labels[]; .name == "building")')
        if [ "$b" = true ]; then continue; fi
        gh issue edit "$n" --add-label building >/dev/null
        rm -f "$behind"
      fi
      echo "$n"
      exit 0
    fi
  done
  echo 'build: no open build-ready issue is unblocked and not building' >&2
  exit 1
fi

if [ "$1" = merge ]; then
  pr=$(sed -n 's/^pr: //p' "$state" 2>/dev/null | head -n 1)
  if [ -z "$pr" ]; then
    echo "build: no pr: line in $state" >&2
    exit 1
  fi
  if ! git fetch -q origin main </dev/null; then
    echo 'build: git fetch origin main failed; nothing merged' >&2
    exit 1
  fi
  h=$(git rev-parse HEAD)
  echo "$h" >"$merged"
  found=
  failed=
  # ok|no CONDITION TEXT: record one line for the condition.
  ok() { found="$found$1: $(printf '%s' "$2" | tr '\n' ' ')
"; }
  no() { failed="$failed$1: $(printf '%s' "$2" | tr '\n' ' ')
"; }
  view=$(gh pr view "$pr" --json headRefOid,labels,body \
    --jq '.headRefOid, (.labels[] | "label: \(.name)"), "body:", .body') || view=
  head=$(printf '%s\n' "$view" | sed -n 1p)
  labels=$(printf '%s\n' "$view" | awk 'NR > 1 && /^body:$/ { exit } NR > 1 { sub(/^label: /, ""); print }')
  body=$(printf '%s\n' "$view" | awk 'b; /^body:$/ { b = 1 }' | tr -d '\r')
  short=$(git rev-parse --short=7 HEAD)
  base=origin/main...HEAD

  paths=$(git diff --name-only --no-renames "$base")
  bad=$(printf '%s\n' "$paths" | awk '/\.toml$/ || !/^(README\.md|GLOSSARY\.md|(core|bindings|tests)\/.*)?$/')
  if [ -n "$bad" ]; then no paths "not allowed: $bad"; else ok paths "$paths"; fi

  t=':(glob)**/tests/**'
  lost=$({
    git diff --name-only --no-renames --diff-filter=D "$base" -- "$t"
    git diff --numstat --no-renames --diff-filter=a "$base" -- "$t" | awk -F '\t' '$2 != 0 { print $3 }'
  } | sort -u)
  if [ -n "$lost" ]; then no tests "deleted, renamed or losing lines: $lost"; else ok tests 'no file under tests/ deleted, renamed or losing a line'; fi

  api='- **API:** none'
  deps='- **Dependencies:** none'
  if [ -z "$view" ]; then
    no 'api and dependencies' 'gh pr view failed'
  elif printf '%s\n' "$labels" | grep -Eqx 'api-additive|api-breaking|adds-dependency'; then
    no 'api and dependencies' "labels: $labels"
  elif ! printf '%s\n' "$body" | grep -qxF -- "$api" ||
    ! printf '%s\n' "$body" | grep -qxF -- "$deps"; then
    no 'api and dependencies' "body lacks '$api' or '$deps'"
  else
    ok 'api and dependencies' "labels: $labels; body: $api, $deps"
  fi

  ci=$(gh pr checks "$pr" --json name,bucket,link --jq '.[] | "\(.name) \(.bucket) \(.link)"' | grep '^All checks pass ') || ci=
  if [ -n "$ci" ] && ! printf '%s\n' "$ci" | grep -qv '^All checks pass pass '; then ok ci "$ci"; else no ci "${ci:-no check named All checks pass}"; fi

  if git merge-base --is-ancestor origin/main HEAD; then
    ok 'up to date' "head $short contains main $(git rev-parse --short=7 origin/main)"
  else
    no 'up to date' "head $short lacks main $(git rev-parse --short=7 origin/main)"
    grep -qxF "$h" "$behind" 2>/dev/null || echo "$h" >>"$behind"
  fi

  files=$(git rev-parse 'HEAD^{tree}')
  why=
  if f=$(failing); then
    why="check not passing: $f"
  elif ! grep -qxF "tree $files" "$results" 2>/dev/null; then
    why='last check run was not on the files of the head commit'
  elif [ "$(tree)" != "$files" ]; then
    why='uncommitted changes'
  elif [ "$(git rev-parse HEAD)" != "$head" ]; then
    why="local head $short is not the pull request's head ${head:-unknown}"
  fi
  if [ -n "$why" ]; then no head "$why"; else ok head "checks passed on the files of $short"; fi

  v=$(grep '^verdict: ' "$state" | tail -n 1)
  case $v in
    "verdict: safe $head "?*) ok verdict "${v#"verdict: safe $head "}" ;;
    *) no verdict "no safe verdict for ${head:-the head}${v:+; last: $v}" ;;
  esac

  if [ -n "$failed" ]; then
    printf 'build: not merged\n%s' "$failed" >&2
    exit 1
  fi
  gh pr edit "$pr" --add-label auto-merged >/dev/null
  if ! gh pr merge "$pr" --squash --match-head-commit "$head"; then
    gh pr edit "$pr" --remove-label auto-merged >/dev/null || true
    echo 'build: GitHub refused the merge; label removed' >&2
    exit 1
  fi
  printf 'merged %s\n%s' "$pr" "$found"
  exit 0
fi

if [ "$1" = unneeded ]; then
  if [ $# -ne 2 ]; then
    echo "build: usage: hook.sh unneeded 'CMD'" >&2
    exit 2
  fi
  tmp=$(mktemp -d)
  file=
  pid=
  # stop CODE: end the test command, restore the file under test, exit CODE.
  stop() {
    if [ -n "$pid" ]; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" || true
    fi
    if [ -n "$file" ]; then cat "$tmp/b" >"$file"; fi
    exit "$1"
  }
  trap 'rm -rf "$tmp"' EXIT
  trap 'stop 129' HUP
  trap 'stop 130' INT
  trap 'stop 143' TERM
  trap 'stop 141' PIPE
  git diff -U0 --no-color --no-ext-diff --no-textconv --src-prefix=a/ --dst-prefix=b/ --merge-base origin/main >"$tmp/diff"
  awk '
    /^diff --git / { h = 1; f = ""; next }
    h && /^\+\+\+ b\// { f = substr($0, 7); sub(/\t$/, "", f); next }
    /^@@ / { h = 0; split($3, a, ","); n = substr(a[1], 2); next }
    !h && /^\+/ {
      t = substr($0, 2)
      sub(/^[[:space:]]+/, "", t)
      if (t != "" && t !~ /^\/[\/*]/ && (t !~ /^#/ || t ~ /^#!?\[/) && f !~ /(^|\/)(tests\/|test_[^\/]*$)|\.test\.[^\/]*$|\.md$/) print f "\t" n
      n++
    }' "$tmp/diff" >"$tmp/lines"
  found=
  while IFS="$(printf '\t')" read -r f n; do
    if [ ! -f "$f" ] || [ -L "$f" ]; then continue; fi
    cp "$f" "$tmp/b"
    file=$f
    sed "${n}d" "$tmp/b" >"$f"
    sh -c "$2" </dev/null >/dev/null 2>&1 &
    pid=$!
    if wait "$pid"; then
      printf '%s:%s: %s\n' "$f" "$n" "$(sed -n "${n}p" "$tmp/b")"
      found=1
    fi
    pid=
    cat "$tmp/b" >"$f"
    file=
  done <"$tmp/lines"
  exit "${found:-0}"
fi

if [ "$1" = check ]; then
  if ! grep -q '^check: .' "$state" 2>/dev/null; then
    echo 'build: no build in progress, or no check: lines' >&2
    exit 1
  fi
  if ! hold; then
    echo 'build: a check is in progress; no checks run' >&2
    exit 1
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
  m=$(git merge-base HEAD origin/main 2>/dev/null) || m=
  if mods_changed "$m"; then
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
      printf 'pass %s\n' "$cmd" >>"$results.tmp"
    else
      printf 'fail %s\n' "$cmd" >>"$results.tmp"
      failures=1
    fi
  done <"$tmp/checks"
  b=${m:-$(git hash-object -t tree /dev/null)}
  w=$(tree)
  git diff --name-only --no-renames "$b" origin/main | LC_ALL=C sort >"$tmp/main"
  git diff --name-only --no-renames "$b" "$w" | LC_ALL=C sort >"$tmp/build"
  LC_ALL=C comm -12 "$tmp/main" "$tmp/build" | sed 's/^/fail merge origin\/main, which also changed /' >"$tmp/both"
  if grep . "$tmp/both" >>"$results.tmp"; then failures=1; fi
  cat "$results.tmp"
  echo "tree $w" >>"$results.tmp"
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

input=
[ -t 0 ] || input=$(cat | LC_ALL=C tr '\n' ' ')

if [ "$1" = tool ]; then
  case $input in
    *'gh pr merge'*)
      if printf '%s\n' "$input" | grep -Eq '"tool_name"[[:space:]]*:[[:space:]]*"Bash"'; then
        echo "build: only the merge command merges during a build: $merge" >&2
        exit 2
      fi
      ;;
    *) ;;
  esac
  exit 0
fi
[ "$1" = stop ] || exit 0
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

if f=$(failing); then refuse "check not passing: $f; delegate the fix, then $again"; fi

grep -qxF "tree $(tree)" "$results" || refuse "files changed since the checks ran; $again"

h=$(git rev-parse HEAD 2>/dev/null || true)
if [ "$(cat "$merged" 2>/dev/null)" = "$h" ] && grep -qxF "$h" "$behind" 2>/dev/null && [ "$(wc -l <"$behind")" -le 3 ]; then
  refuse "head is behind main; merge origin/main, run ${again#run: }, push, record a new verdict, then run $merge"
fi

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
  *pending*) refuse "CI pending on $pr; wait with: gh pr checks $pr --watch >/dev/null; gh pr checks $pr" ;;
esac

if ! grep -q '^comment: .' "$state"; then
  refuse "CI is green; post the ideas and retro comment on $pr and add 'comment: <url>' to $state"
fi

if [ ! -f "$merged" ] || [ "$(cat "$merged")" != "$h" ]; then
  refuse "comment recorded; run the merge command on the current head: $merge"
fi

cp "$state" "$done"
rm -f "$count" "$clock" "$merged"
