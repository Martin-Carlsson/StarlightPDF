#!/bin/sh
# Runs every check of an area.
set -eu

case "$*" in
  core | python | advisories | licenses | dependencies | workflows | hook | mods) ;;
  *)
    echo 'usage: sh ci/check.sh core|python|advisories|licenses|dependencies|workflows|hook|mods' >&2
    exit 2
    ;;
esac
cd "$(dirname "$0")/.."

case $1 in
  licenses | python)
    PY=
    for candidate in python3 python; do
      if "$candidate" -c 'import sys; sys.exit(sys.version_info < (3, 11))' >/dev/null 2>&1; then
        PY=$candidate
        break
      fi
    done
    if [ -z "$PY" ]; then
      echo 'Python 3.11 or newer not found' >&2
      exit 1
    fi
    export PY
    ;;
esac

case $1 in
  advisories)
    cargo deny --locked check -D warnings advisories
    exit 0
    ;;
  licenses)
    sh ci/licenses.sh
    exit 0
    ;;
  dependencies)
    sh ci/dependencies.sh
    exit 0
    ;;
  hook)
    sh .claude/skills/build/hook.test.sh
    exit 0
    ;;
  mods)
    # Prettier: the standard formatter; large to write here.
    npx -y prettier@3.10.1 --check .claude/mods
    exit 0
    ;;
  workflows)
    # actionlint (MIT): the standard workflow linter; large to write here. shellcheck (GPL) stays off.
    case "$(uname -s)-$(uname -m)" in
      Linux-x86_64) target=linux_amd64 sum=8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8 ;;
      Darwin-arm64) target=darwin_arm64 sum=aba9ced2dee8d27fecca3dc7feb1a7f9a52caefa1eb46f3271ea66b6e0e6953f ;;
      *)
        echo "no actionlint for $(uname -s) $(uname -m)" >&2
        exit 1
        ;;
    esac
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    version=1.7.12
    archive=actionlint_${version}_$target.tar.gz
    curl -fsSL -o "$tmp/$archive" "https://github.com/rhysd/actionlint/releases/download/v$version/$archive"
    printf '%s  %s\n' "$sum" "$tmp/$archive" | shasum -a 256 -c -
    tar -xzf "$tmp/$archive" -C "$tmp" actionlint
    files=$(find .github/workflows -type f \( -name '*.yml' -o -name '*.yaml' \) | sort)
    "$tmp/actionlint" -shellcheck= -pyflakes= $files
    for f in $files; do
      awk '{ if (a == "defaults:" && b == "  run:" && $0 == "    shell: bash") ok = 1; a = b; b = $0 } END { exit !ok }' "$f" || {
        echo "$f: set top-level defaults.run.shell to bash" >&2
        exit 1
      }
    done
    exit 0
    ;;
esac

if [ "$1" = core ]; then
  sh ci/lints.sh
  sh ci/deps.sh
  cargo fmt --check -p starlightpdf
  cargo clippy --all-targets --locked -p starlightpdf
  cargo test --locked -p starlightpdf
  RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --locked -p starlightpdf
  exit 0
fi

sh ci/deps.sh
cargo fmt --check -p starlightpdf-python
cargo clippy --all-targets --locked -p starlightpdf-python
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --locked -p starlightpdf-python
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
"$PY" -m venv "$tmp/venv"
if [ -x "$tmp/venv/bin/python" ]; then
  venv=$tmp/venv/bin/python
else
  venv=$tmp/venv/Scripts/python.exe
fi
"$venv" -m pip install --no-index ./bindings/python
"$venv" -W error -X dev -bb -m unittest discover -s bindings/python/tests
