#!/bin/sh
# Fails on a license outside the allow list, an unreasoned direct dependency, an exemption or an unaudited package.
set -eu
cd "$(dirname "$0")/.."
cargo deny --locked check -D warnings licenses
if grep -Eq '^\[\[?exemptions' supply-chain/config.toml; then
  echo 'supply-chain/config.toml: exemptions are not allowed' >&2
  exit 1
fi
cargo vet --locked

# A non-workspace direct dependency needs a `#` reason on its manifest line.
c='#[[:space:]]*[^[:space:]]'
ms=$(find . -name Cargo.toml -not -path './target/*' | sort)
ws=$(echo "$ms" | while IFS= read -r m; do sed -n 's/^name = "\(.*\)"$/\1/p' "$m" | head -n 1; done)
bad=$(echo "$ms" | while IFS= read -r manifest; do
  crate=$(sed -n 's/^name = "\(.*\)"$/\1/p' "$manifest" | head -n 1)
  [ -n "$crate" ] || continue
  cargo tree -p "$crate" --locked --all-features --target all -e normal,build,dev --depth 1 --prefix none |
    sed 1d | cut -d' ' -f1 | grep -vxF "$ws" | sort -u | while IFS= read -r dep; do
      n=$(printf '%s' "$dep" | sed 's/[-_]/[-_]/g')
      grep -Eq "^[[:space:]]*\"?$n\"?[[:space:]]*[.=].*$c|^\[.*dependencies\.\"?$n\"?\].*$c|package[[:space:]]*=[[:space:]]*\"$dep\".*$c" "$manifest" ||
        echo "$crate: $dep"
    done
done)
if [ -n "$bad" ]; then
  printf 'no reason (add a `#` comment on the line):\n%s\n' "$bad" >&2
  exit 1
fi
