#!/bin/sh
# Runs every check of an area.
set -eu

case "$*" in
  core | python) ;;
  *)
    echo 'usage: sh ci/check.sh core|python' >&2
    exit 2
    ;;
esac
cd "$(dirname "$0")/.."

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
py=
for candidate in python3 python; do
  if "$candidate" -c 'import sys; sys.exit(sys.version_info < (3, 11))' >/dev/null 2>&1; then
    py=$candidate
    break
  fi
done
if [ -z "$py" ]; then
  echo 'Python 3.11 or newer not found' >&2
  exit 1
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
"$py" -m venv "$tmp/venv"
if [ -x "$tmp/venv/bin/python" ]; then
  venv=$tmp/venv/bin/python
else
  venv=$tmp/venv/Scripts/python.exe
fi
"$venv" -m pip install --no-index ./bindings/python
"$venv" -W error -X dev -bb -m unittest discover -s bindings/python/tests
