#!/bin/sh
set -eu

script="$(cd "$(dirname "$0")" && pwd)/version.sh"
tmp=$(mktemp -d)
trap 'rm -rf "${tmp}"' EXIT
repo="${tmp}/repo"
mod="${repo}/.claude/mods/orbit"
failed=0

g() { git -C "${repo}" -c commit.gpgsign=false "$@"; }

commit() {
  g add -A
  g commit -q -m change
}

# manifest DIR NAME VERSION
manifest() {
  mkdir -p "$1/.claude-plugin"
  printf '{\n  "name": "%s",\n  "version": "%s"\n}\n' "$2" "$3" >"$1/.claude-plugin/plugin.json"
}

# marketplace NAME...
marketplace() {
  mkdir -p "${repo}/.claude-plugin"
  {
    printf '{\n  "name": "test",\n  "plugins": ['
    sep=''
    for n in "$@"; do
      printf '%s\n    {\n      "name": "%s",\n      "source": "./.claude/mods/%s"\n    }' "${sep}" "${n}" "${n}"
      sep=','
    done
    printf '\n  ]\n}\n'
  } >"${repo}/.claude-plugin/marketplace.json"
}

# A repository whose main has plugin orbit 1.0.0, checked out on a work branch.
reset() {
  rm -rf "${repo}"
  git init -q -b main "${repo}"
  g config user.name test
  g config user.email test@example.test
  echo '*.log' >"${repo}/.gitignore"
  echo readme >"${repo}/README.md"
  marketplace orbit
  manifest "${mod}" orbit 1.0.0
  echo one >"${mod}/index.js"
  commit
  g checkout -q -b work
}

# run NAME WANT_EXIT [BASE]
run() {
  code=0
  (cd "${repo}" && sh "${script}" "${3:-main}") >/dev/null 2>"${tmp}/err" || code=$?
  if [ "${code}" -ne "$2" ]; then
    echo "FAIL $1: exit ${code}, stderr:"
    cat "${tmp}/err"
    failed=1
  fi
}

reset
run "unchanged plugin passes" 0

reset
echo two >"${mod}/index.js"
commit
run "changed file with same version fails" 1

reset
echo two >"${mod}/index.js"
manifest "${mod}" orbit 1.1.0
commit
run "changed file with raised version passes" 0

reset
echo two >"${mod}/index.js"
run "uncommitted change with same version fails" 1

reset
echo new >"${mod}/new.js"
run "untracked file with same version fails" 1

reset
echo noise >"${mod}/debug.log"
run "ignored file does not count" 0

reset
marketplace orbit pane
manifest "${repo}/.claude/mods/pane" pane 0.1.0
echo pane >"${repo}/.claude/mods/pane/index.js"
commit
run "plugin new since base passes" 0

reset
echo changed >"${repo}/README.md"
commit
run "change outside the plugin passes" 0

reset
run "missing base fails" 1 nosuchref

[ "${failed}" -eq 0 ] && echo PASS
exit "${failed}"
