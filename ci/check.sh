#!/bin/sh
# Runs every check of an area.
set -eu

areas='core python advisories licenses dependencies workflows hook mods ruff'
ok=
for a in $areas; do
  case $a in "$*") ok=1 ;; esac
done
if [ -z "$ok" ]; then
  echo "usage: sh ci/check.sh $(echo $areas | tr ' ' '|')" >&2
  exit 2
fi
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
    # covered FILE...: CHANGED (changed paths, one per line) is unset or names a FILE.
    covered() {
      [ -z "${CHANGED+x}" ] && return
      for f; do printf '%s\n' "$CHANGED" | grep -qxF -- "$f" && return; done
      return 1
    }
    if covered tests/ci/test_licenses.py ci/licenses.sh ci/check.sh REUSE.toml || printf '%s\n' "$CHANGED" | grep -q '^LICENSES/'; then
      "$PY" -W error -X dev -bb -m unittest discover -s tests/ci -p test_licenses.py
    fi
    if covered tests/ci/test_dependencies.py ci/dependencies.sh ci/check.sh README.md Cargo.lock .github/workflows/ci.yml; then
      "$PY" -W error -X dev -bb -m unittest discover -s tests/ci -p test_dependencies.py
    fi
    if covered tests/ci/test_areas.py ci/areas.sh ci/check.sh; then
      "$PY" -W error -X dev -bb -m unittest discover -s tests/ci -p test_areas.py
    fi
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
  ruff)
    # ruff (MIT): the standard Python linter and formatter; large to write here.
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    py=$tmp/licenses.py
    awk '/<<.PY.$/ { f = 1; next } /^PY$/ { f = 0 } f' ci/licenses.sh >"$py"
    files=$(git ls-files -co --exclude-standard '*.py')
    if ! grep -qxF 'select = ["ALL"]' ruff.toml || grep -vxE -e 'target-version = "py311"' -e '\[lint\]' -e 'select = \["ALL"\]' \
      -e 'ignore = \[' -e '  "[A-Z]+[0-9]+", # .*[[:alnum:]].*' -e '\]' -e '' ruff.toml; then
      echo 'ruff.toml: only select = ["ALL"] and ignore entries "CODE", # reason' >&2
      exit 1
    fi
    if grep -En '#[[:space:]]*((ruff|flake8):[[:space:]]*)?[Nn][Oo][Qq][Aa][[:space:]]*:?[[:space:]]*([A-Z]+[0-9]+[[:space:],]*)*[^[:alnum:]]*$' "$py" $files; then
      echo 'give each noqa a reason after its codes' >&2
      exit 1
    fi
    case "$(uname -s)-$(uname -m)" in
      Linux-x86_64) target=x86_64-unknown-linux-gnu sum=7d261fc981a784668e19e9fa6ad437bfd95b67f2dc263fc1eab87d54d72fcc90 ;;
      Darwin-arm64) target=aarch64-apple-darwin sum=4ffadabec5c5fee1ddb2eecac0787ffe6fa18c665b73da9258b84211f40c5829 ;;
      *)
        echo "no ruff for $(uname -s) $(uname -m)" >&2
        exit 1
        ;;
    esac
    archive=ruff-$target.tar.gz
    curl -fsSL -o "$tmp/$archive" "https://github.com/astral-sh/ruff/releases/download/0.17.5/$archive"
    printf '%s  %s\n' "$sum" "$tmp/$archive" | shasum -a 256 -c -
    tar -xzf "$tmp/$archive" -C "$tmp"
    ruff=$tmp/ruff-$target/ruff
    "$ruff" check --no-cache --config ruff.toml $files
    "$ruff" format --no-cache --config ruff.toml --check $files
    "$ruff" check --no-cache --config ruff.toml --stdin-filename ci/licenses.py - <"$py"
    "$ruff" format --no-cache --config ruff.toml --diff --stdin-filename ci/licenses.py - <"$py"
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
