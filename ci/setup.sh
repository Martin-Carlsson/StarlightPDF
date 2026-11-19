#!/bin/sh
# Installs the tools of the given CI areas, or of every area; CI runs this too.
set -eu
if [ "$#" = 0 ]; then set -- core fuzz; fi
for area; do
  case ${area} in
    core | python | advisories)
      cargo install --locked cargo-deny@0.20.2 cargo-vet@0.10.3
      ;;
    fuzz)
      cargo install --locked cargo-fuzz@0.13.2
      ;;
    *) ;;
  esac
done
