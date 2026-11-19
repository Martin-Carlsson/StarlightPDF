#!/bin/sh
# Installs the tools of the given CI areas, or of every area; CI runs this too.
set -eu
if [ "$#" = 0 ]; then set -- fuzz javascript; fi
for area; do
  case ${area} in
    core | python | advisories)
      cargo install --locked cargo-deny@0.20.2 cargo-vet@0.10.3
      ;;
    fuzz)
      cargo install --locked cargo-fuzz@0.13.2
      ;;
    javascript)
      sh "$0" core
      rustup target add wasm32-unknown-unknown
      cargo install --locked wasm-bindgen-cli@0.2.130
      # GitHub's Linux and Windows runners default to an older Node.js; their tool cache holds 24.
      if [ -n "${GITHUB_PATH-}" ] && ! node -e 'if (parseInt(process.versions.node, 10) < 24) process.exit(1)'; then
        cache=${RUNNER_TOOL_CACHE:?}
        if command -v cygpath >/dev/null 2>&1; then cache=$(cygpath -u "${cache}"); fi
        # The highest 24.x of any architecture, as "24.minor.patch dir" sorted by number.
        found=$(for dir in "${cache}"/node/24.*/*/bin "${cache}"/node/24.*/*; do
          if [ -f "${dir}/node" ] || [ -f "${dir}/node.exe" ]; then
            version=${dir#"${cache}/node/"}
            echo "${version%%/*} ${dir}"
          fi
        done | sort -t. -k2,2n -k3,3n | tail -n 1 | cut -d' ' -f2-)
        if [ -z "${found}" ]; then
          echo "no Node.js 24 in ${cache}/node" >&2
          exit 1
        fi
        if command -v cygpath >/dev/null 2>&1; then found=$(cygpath -w "${found}"); fi
        echo "${found}" >>"${GITHUB_PATH}"
      fi
      ;;
    *) ;;
  esac
done
