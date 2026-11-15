#!/bin/sh
# Fails on a license outside the allow list, an exemption or an unaudited package.
set -eu
cd "$(dirname "$0")/.."
cargo deny --locked check -D warnings licenses
if grep -Eq '^\[\[?exemptions' supply-chain/config.toml; then
  echo 'supply-chain/config.toml: exemptions are not allowed' >&2
  exit 1
fi
cargo vet --locked
