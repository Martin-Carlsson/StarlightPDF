#!/bin/sh
# Fails when core has any dependency, or the binding one besides starlightpdf and pyo3.
set -eu
cd "$(dirname "$0")/.."
tree() { cargo tree -p "$1" --locked --all-features --target all -e normal,build,dev --depth 1 --prefix none; }
core=$(tree starlightpdf)
binding=$(tree starlightpdf-python)
bad=$({
  printf '%s\n' "$core" | sed 1d
  printf '%s\n' "$binding" | sed 1d | grep -Ev '^(pyo3|starlightpdf) ' || true
})
if [ -n "$bad" ]; then
  printf 'not allowed:\n%s\n' "$bad" >&2
  exit 1
fi
