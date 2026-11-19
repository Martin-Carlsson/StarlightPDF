#!/bin/sh
# Installs the CI tools; CI runs this too.
set -eu
cargo install --locked cargo-deny@0.20.2 cargo-vet@0.10.3
