#!/bin/sh
# Tests for the skills' decision and documentation rules. Run: sh skills.test.sh [case]
set -eu

root="$(cd "$(dirname "$0")/../.." && pwd)"
issue="$root/.claude/skills/issue/SKILL.md"
build="$root/.claude/skills/build/SKILL.md"
tpl="$root/.github/ISSUE_TEMPLATE"
failed=0

# units FILE: one line per bullet, numbered item, heading or paragraph, lowercased.
units() {
  awk '
    function out() { if (u != "") print u; u = "" }
    /^[[:space:]]*$/ { out(); next }
    /^[[:space:]]*(- |[0-9]+\. |#)/ { out() }
    { sub(/^[[:space:]]+/, ""); u = u " " $0 }
    END { out() }
  ' "$1" | tr '[:upper:]' '[:lower:]' | tr -s ' '
}

# section FILE HEADING: text under a markdown heading, lowercased, on one line.
section() {
  awk -v h="$2" '/^#/ { on = ($0 == h); next } on' "$1" |
    tr '\n' ' ' | tr '[:upper:]' '[:lower:]' | tr -s ' '
}

# has TEXT PHRASE...: TEXT contains every phrase.
has() {
  t=$1
  shift
  for p in "$@"; do
    case $t in *"$p"*) ;; *) return 1 ;; esac
  done
}

bar() { has "$1" 'hard to reverse' 'surprising without context' 'real trade-off'; }

fail() {
  echo "FAIL $1: $2"
  failed=1
}

issue_decisions() {
  b=$(units "$issue" | grep '^ - \*\*decisions:\*\*' || true)
  { bar "$b" && has "$b" only; } ||
    fail issue-decisions "issue/SKILL.md Decisions rule lacks the bar or 'only'"
  for f in bug feature; do
    bar "$(section "$tpl/$f.md" '## Decisions')" ||
      fail issue-decisions "$f.md Decisions comment lacks the bar"
  done
}

build_records() {
  ok=1
  units "$build" | grep -F '.architecture-decisions/' >"$tmp" || true
  while IFS= read -r u; do
    if bar "$u" && has "$u" only; then ok=0; fi
  done <"$tmp"
  [ "$ok" -eq 0 ] ||
    fail build-records "no .architecture-decisions/ rule with the bar and 'only'"
  ok=1
  while IFS= read -r u; do
    if has "$u" follow; then
      case $u in *'without record'* | *'not record'* | *"don't record"* | *unrecorded*) ok=0 ;; esac
    fi
  done <"$tmp"
  [ "$ok" -eq 0 ] ||
    fail build-records "does not say other decisions are followed without recording"
  ! grep -qF 'Record each in `.architecture-decisions/`' "$build" ||
    fail build-records "old 'Record each' text remains"
}

review() { units "$build" | grep '^ - \*\*review:\*\*' || true; }

review_docs() {
  has "$(review)" documentation comment merge 'same topic' delete shorten ||
    fail review-docs "Review step lacks the documentation check"
}

review_no_repeat() {
  r=$(review)
  case $r in
    *'repeats another'*) ;;
    *) fail review-no-repeat "Review step does not forbid a file repeating another" ;;
  esac
}

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT

case ${1:-all} in
  issue-decisions) issue_decisions ;;
  build-records) build_records ;;
  review-docs) review_docs ;;
  review-no-repeat) review_no_repeat ;;
  all) issue_decisions; build_records; review_docs; review_no_repeat ;;
  *) echo "unknown case: $1" >&2; exit 2 ;;
esac

[ "$failed" -eq 0 ] && echo PASS
exit "$failed"
