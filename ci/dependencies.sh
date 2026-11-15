#!/bin/sh
# Fails when README.md's crates.io Dependencies rows differ from the external crates and CI tools, or give another license than declared.
set -eu
export LC_ALL=C
cd "$(dirname "$0")/.."

# name version, one per line
want=$(
  {
    awk '
      function flush() { if (name != "" && src) print name, version }
      /^\[\[package\]\]/ { flush(); name = ""; src = 0 }
      /^name = / { gsub(/"/, "", $3); name = $3 }
      /^version = / { gsub(/"/, "", $3); version = $3 }
      /^source = / { src = 1 }
      END { flush() }
    ' Cargo.lock
    sed -n 's/.*cargo install .*--locked \(.*\)$/\1/p' .github/workflows/ci.yml | tr ' ' '\n' | grep @ | tr @ ' '
  } | sort -u
)
have=$(
  sed -n '/^### Dependencies$/,$p' README.md | grep '^|.*(https://crates\.io/crates/' |
    sed 's|.*(https://crates\.io/crates/\([^)/]*\)).*|\1|' | sort -u
)
names=$(echo "$want" | cut -d' ' -f1 | sort -u)
if [ "$names" != "$have" ]; then
  echo 'README.md Dependencies: crates.io rows differ from the Cargo.lock packages and CI tools:' >&2
  { echo "$names"; echo "$have"; } | sort | uniq -u >&2
  exit 1
fi

while IFS=' ' read -r name version; do
  info=$(cargo info "$name@$version" </dev/null) || exit 1
  declared=$(echo "$info" | sed -n 's/^license: //p' | sed 's|/| OR |g')
  row=$(sed -n '/^### Dependencies$/,$p' README.md | grep "^|.*(https://crates\.io/crates/$name)" |
    awk -F' [|] ' '{ print $2 }' | sed 's/\[\([^]]*\)\]([^)]*)/\1/g')
  if [ -z "$declared" ] || [ "$row" != "$declared" ]; then
    echo "README.md Dependencies: $name: row has '$row', declared '$declared'" >&2
    exit 1
  fi
done <<END
$want
END
