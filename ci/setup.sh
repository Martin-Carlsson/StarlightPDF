#!/bin/sh
# Installs the tools of the given CI areas, or of every area; CI runs this too.
set -eu
if [ "$#" = 0 ]; then set -- fuzz javascript; fi
os=$(uname -s)
machine=$(uname -m)
case "${os}-${machine}" in
  Linux-x86_64) platform=linux ;;
  Darwin-arm64) platform=mac ;;
  MINGW*-x86_64 | MSYS*-x86_64) platform=windows ;;
  *) platform= ;;
esac
bin=${CARGO_HOME:-${HOME}/.cargo}/bin
if command -v cygpath >/dev/null 2>&1; then bin=$(cygpath -u "${bin}"); fi
tmp=$(mktemp -d)
trap 'rm -rf "${tmp}"' EXIT

# Installs crate@version $1: on Linux, macOS or Windows from its release archive, $2 to $4 as "path-under-github.com
# SHA-256", in seconds where building takes CI minutes; elsewhere built.
fetch() {
  case ${platform} in
    linux) release=$2 ;;
    mac) release=$3 ;;
    windows) release=$4 ;;
    *)
      cargo install --locked "$1"
      return
      ;;
  esac
  path=${release%% *}
  archive=${tmp}/${path##*/}
  curl -fsSL -o "${archive}" "https://github.com/${path}"
  if command -v sha256sum >/dev/null 2>&1; then
    printf '%s  %s\n' "${release#* }" "${archive}" | sha256sum -c -
  else
    printf '%s  %s\n' "${release#* }" "${archive}" | shasum -a 256 -c -
  fi
  mkdir "${archive}.d"
  case ${archive} in
    *.zip) unzip -q "${archive}" -d "${archive}.d" ;;
    *) tar -xf "${archive}" -C "${archive}.d" ;;
  esac
  mkdir -p "${bin}"
  find "${archive}.d" -type f ! -name '*.md' ! -name 'LICENSE*' -exec mv {} "${bin}" ';'
}

for area; do
  case ${area} in
    core | python | advisories)
      fetch cargo-deny@0.20.2 \
        'EmbarkStudios/cargo-deny/releases/download/0.20.2/cargo-deny-0.20.2-x86_64-unknown-linux-musl.tar.gz 9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f' \
        'EmbarkStudios/cargo-deny/releases/download/0.20.2/cargo-deny-0.20.2-aarch64-apple-darwin.tar.gz fe67d82a10d8597a3549364cb733a3f9cc1bfff9031b7ae46384a9f2a72090c3' \
        'EmbarkStudios/cargo-deny/releases/download/0.20.2/cargo-deny-0.20.2-x86_64-pc-windows-msvc.tar.gz 975a22143262fd27476d19ee00c7af67978426e40e1dee94eed6bbade1cf87dc'
      fetch cargo-vet@0.10.3 \
        'mozilla/cargo-vet/releases/download/v0.10.3/cargo-vet-x86_64-unknown-linux-gnu.tar.xz e83878b80913fbe43629c28768e14ab51f4e86a4ba8f9dd08cc25b8dbc503c19' \
        'mozilla/cargo-vet/releases/download/v0.10.3/cargo-vet-aarch64-apple-darwin.tar.xz a83f7d6f48a069890743ba61f7637660c8577f263efd64fee47e673632f0e423' \
        'mozilla/cargo-vet/releases/download/v0.10.3/cargo-vet-x86_64-pc-windows-msvc.zip 855fda4724dada8ba235aec7c0da7d3b28d1697c946627733143fe37444dd510'
      ;;
    fuzz)
      cargo install --locked cargo-fuzz@0.13.2
      ;;
    javascript)
      sh "$0" core
      rustup target add wasm32-unknown-unknown
      fetch wasm-bindgen-cli@0.2.130 \
        'wasm-bindgen/wasm-bindgen/releases/download/0.2.130/wasm-bindgen-0.2.130-x86_64-unknown-linux-musl.tar.gz 9687f3bb551b9f66f37ed32a40d7913a00f7962db653054e56857bc306c3227f' \
        'wasm-bindgen/wasm-bindgen/releases/download/0.2.130/wasm-bindgen-0.2.130-aarch64-apple-darwin.tar.gz b90ddb9357f584b1af897d58ad77e4cce2b48fc17e2a967bc249a87f72809444' \
        'wasm-bindgen/wasm-bindgen/releases/download/0.2.130/wasm-bindgen-0.2.130-x86_64-pc-windows-msvc.tar.gz 844d23f1b56097549b936dd2e0d561fadebc1e8c9e985d5068b6b919654104ad'
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
