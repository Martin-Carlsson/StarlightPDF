#!/bin/sh
# Runs every check of an area.
set -eu

areas='core python javascript advisories licenses dependencies fuzz workflows hook mods ruff shell'
ok=
for a in ${areas}; do
  case ${a} in
    "$*") ok=1 ;;
    *) ;;
  esac
done
if [ -z "${ok}" ]; then
  usage=$(printf '%s' "${areas}" | tr ' ' '|')
  echo "usage: sh ci/check.sh ${usage}" >&2
  exit 2
fi
cd "$(dirname "$0")/.."
os=$(uname -s)
machine=$(uname -m)

# Sets shellcheck to the pinned one, in tmp. The standard shell linter (GPL-3.0); CI only runs it.
get_shellcheck() {
  case "${os}-${machine}" in
    Linux-x86_64) sc_target=linux.x86_64 sc_sum=8c3be12b05d5c177a04c29e3c78ce89ac86f1595681cab149b65b97c4e227198 ;;
    Darwin-arm64) sc_target=darwin.aarch64 sc_sum=56affdd8de5527894dca6dc3d7e0a99a873b0f004d7aabc30ae407d3f48b0a79 ;;
    *)
      echo "no shellcheck for ${os} ${machine}" >&2
      exit 1
      ;;
  esac
  sc_dir='shellcheck-v0.11.0'
  sc_archive=${sc_dir}.${sc_target}.tar.xz
  curl -fsSL -o "${tmp}/${sc_archive}" "https://github.com/koalaman/shellcheck/releases/download/v0.11.0/${sc_archive}"
  printf '%s  %s\n' "${sc_sum}" "${tmp}/${sc_archive}" | shasum -a 256 -c -
  tar -xJf "${tmp}/${sc_archive}" -C "${tmp}" "${sc_dir}/shellcheck"
  shellcheck=${tmp}/${sc_dir}/shellcheck
  # Every check on, and no rc file to turn one off.
  export SHELLCHECK_OPTS='--norc --enable=all --severity=style'
}

case $1 in
  core | python) tools='cargo-deny cargo-vet' ;;
  javascript) tools='cargo-deny cargo-vet wasm-bindgen' ;;
  advisories) tools=cargo-deny ;;
  fuzz) tools=cargo-fuzz ;;
  *) tools= ;;
esac
for t in ${tools}; do
  command -v "${t}" >/dev/null 2>&1 || [ -x "${CARGO_HOME:-${HOME}/.cargo}/bin/${t}" ] || {
    echo "${t} is missing: run sh ci/setup.sh" >&2
    exit 1
  }
done

case $1 in
  licenses | python)
    PY=
    for candidate in python3 python; do
      if "${candidate}" -c 'import sys; sys.exit(sys.version_info < (3, 11))' >/dev/null 2>&1; then
        PY=${candidate}
        break
      fi
    done
    if [ -z "${PY}" ]; then
      echo 'Python 3.11 or newer not found' >&2
      exit 1
    fi
    export PY
    ;;
  *) ;;
esac

case $1 in
  advisories)
    cargo deny --locked check -D warnings advisories
    exit 0
    ;;
  licenses)
    sh ci/licenses.sh
    # Changed paths; unset stands for ci/check.sh, which every test below covers.
    changed=${CHANGED-ci/check.sh}
    if printf '%s\n' "${changed}" | grep -qxF -e tests/ci/test_licenses.py -e ci/licenses.sh -e ci/check.sh -e REUSE.toml ||
      printf '%s\n' "${changed}" | grep -q '^LICENSES/'; then
      "${PY}" -W error -X dev -bb -m unittest discover -s tests/ci -p test_licenses.py
    fi
    if printf '%s\n' "${changed}" | grep -qxF -e tests/ci/test_dependencies.py -e ci/dependencies.sh -e ci/check.sh -e README.md -e Cargo.lock -e ci/setup.sh; then
      "${PY}" -W error -X dev -bb -m unittest discover -s tests/ci -p test_dependencies.py
    fi
    if printf '%s\n' "${changed}" | grep -qxF -e tests/ci/test_workflows.py -e ci/workflows.sh -e ci/check.sh; then
      "${PY}" -W error -X dev -bb -m unittest discover -s tests/ci -p test_workflows.py
    fi
    if printf '%s\n' "${changed}" | grep -qxF -e tests/ci/test_areas.py -e ci/areas.sh -e ci/check.sh; then
      "${PY}" -W error -X dev -bb -m unittest discover -s tests/ci -p test_areas.py
    fi
    exit 0
    ;;
  dependencies)
    sh ci/dependencies.sh
    exit 0
    ;;
  fuzz)
    # Nightly for cargo-fuzz only, pinned by date; fetch locks deps as cargo fuzz has no --locked.
    nightly='nightly-2026-11-12'
    rustup toolchain install --no-self-update --profile minimal "${nightly}"
    cargo fetch --locked
    tmp=$(mktemp -d)
    trap 'rm -rf "${tmp}"' EXIT
    # Corpus, then seeded runs, before lints so a planted fault is a finding; new inputs go to the first dir; cmp and reload off: nondeterministic.
    for target in encrypted-open filters header-version jpeg page-count png rebuild xref-stream-flate; do
      mkdir "${tmp}/${target}"
      cargo "+${nightly}" fuzz run "${target}" "${tmp}/${target}" tests/corpus -- \
        -seed=1 -runs=1000000 -use_cmp=0 -reload=0 -timeout=10 -malloc_limit_mb=256
    done
    cargo fmt --check -p starlightpdf-fuzz
    cargo clippy --all-targets --locked -p starlightpdf-fuzz
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
    mkdir -p target
    tmp=$(mktemp -d target/ruff.XXXXXX)
    trap 'rm -rf "${tmp}"' EXIT
    py=${tmp}/licenses.py
    awk '/<<.PY.$/ { f = 1; next } /^PY$/ { f = 0 } f' ci/licenses.sh >"${py}"
    files=$(git ls-files -co --exclude-standard '*.py' '*.pyi')
    set --
    for f in ${files}; do set -- "$@" "${f}"; done
    if ! grep -qxF 'select = ["ALL"]' ruff.toml || grep -vxE -e 'target-version = "py311"' -e '\[lint\]' -e 'select = \["ALL"\]' \
      -e 'ignore = \[' -e '  "[A-Z]+[0-9]+", # .*[[:alnum:]].*' -e '\]' -e '' ruff.toml; then
      echo 'ruff.toml: only select = ["ALL"] and ignore entries "CODE", # reason' >&2
      exit 1
    fi
    if grep -En '#[[:space:]]*((ruff|flake8):[[:space:]]*)?[Nn][Oo][Qq][Aa][[:space:]]*:?[[:space:]]*([A-Z]+[0-9]+[[:space:],]*)*[^[:alnum:]]*$' "${py}" "$@"; then
      echo 'give each noqa a reason after its codes' >&2
      exit 1
    fi
    case "${os}-${machine}" in
      Linux-x86_64) target=x86_64-unknown-linux-gnu sum=7d261fc981a784668e19e9fa6ad437bfd95b67f2dc263fc1eab87d54d72fcc90 ;;
      Darwin-arm64) target=aarch64-apple-darwin sum=4ffadabec5c5fee1ddb2eecac0787ffe6fa18c665b73da9258b84211f40c5829 ;;
      *)
        echo "no ruff for ${os} ${machine}" >&2
        exit 1
        ;;
    esac
    version=0.17.5
    ruff=target/ruff-${version}-${target}/ruff
    if [ ! -x "${ruff}" ]; then
      archive=ruff-${target}.tar.gz
      curl -fsSL -o "${tmp}/${archive}" "https://github.com/astral-sh/ruff/releases/download/${version}/${archive}"
      printf '%s  %s\n' "${sum}" "${tmp}/${archive}" | shasum -a 256 -c -
      tar -xzf "${tmp}/${archive}" -C "${tmp}" --strip-components=1
      mkdir -p "$(dirname "${ruff}")"
      mv "${tmp}/ruff" "${ruff}"
    fi
    "${ruff}" check --no-cache --config ruff.toml "$@"
    "${ruff}" format --no-cache --config ruff.toml --check "$@"
    "${ruff}" check --no-cache --config ruff.toml --stdin-filename ci/licenses.py - <"${py}"
    "${ruff}" format --no-cache --config ruff.toml --diff --stdin-filename ci/licenses.py - <"${py}"
    exit 0
    ;;
  shell)
    tmp=$(mktemp -d)
    trap 'rm -rf "${tmp}"' EXIT
    files=$(git ls-files -co --exclude-standard '*.sh')
    set --
    for f in ${files}; do set -- "$@" "${f}"; done
    if grep -En '#[[:space:]]*shellcheck[[:space:]][^#]*disable=[^#]*(#[^[:alnum:]]*)?$' "$@"; then
      echo 'give each shellcheck disable a reason after it' >&2
      exit 1
    fi
    get_shellcheck
    "${shellcheck}" "$@"
    exit 0
    ;;
  workflows)
    # actionlint (MIT): the standard workflow linter; large to write here. It runs shellcheck on run: scripts,
    # with 7 checks off: missing sourced files, and false positives from its ${{ }} placeholders and env:.
    tmp=$(mktemp -d)
    trap 'rm -rf "${tmp}"' EXIT
    get_shellcheck
    case "${os}-${machine}" in
      Linux-x86_64) target=linux_amd64 sum=8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8 ;;
      Darwin-arm64) target=darwin_arm64 sum=aba9ced2dee8d27fecca3dc7feb1a7f9a52caefa1eb46f3271ea66b6e0e6953f ;;
      *)
        echo "no actionlint for ${os} ${machine}" >&2
        exit 1
        ;;
    esac
    version=1.7.12
    archive=actionlint_${version}_${target}.tar.gz
    curl -fsSL -o "${tmp}/${archive}" "https://github.com/rhysd/actionlint/releases/download/v${version}/${archive}"
    printf '%s  %s\n' "${sum}" "${tmp}/${archive}" | shasum -a 256 -c -
    tar -xzf "${tmp}/${archive}" -C "${tmp}" actionlint
    files=$(find .github/workflows -type f \( -name '*.yml' -o -name '*.yaml' \) | sort)
    set --
    for f in ${files}; do set -- "$@" "${f}"; done
    "${tmp}/actionlint" -shellcheck="${shellcheck}" -pyflakes= "$@"
    sh ci/workflows.sh "$@"
    for f; do
      awk '{ if (a == "defaults:" && b == "  run:" && $0 == "    shell: bash") ok = 1; a = b; b = $0 } END { exit !ok }' "${f}" || {
        echo "${f}: set top-level defaults.run.shell to bash" >&2
        exit 1
      }
    done
    exit 0
    ;;
  *) ;;
esac

if [ "$1" = core ]; then
  sh ci/lints.sh
  sh ci/deps.sh
  cargo fmt --check -p starlightpdf -p starlightpdf-memory
  cargo clippy --all-targets --locked -p starlightpdf -p starlightpdf-memory
  cargo test --locked -p starlightpdf -p starlightpdf-memory
  RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --locked -p starlightpdf -p starlightpdf-memory
  exit 0
fi

if [ "$1" = javascript ]; then
  node -e 'if (parseInt(process.versions.node, 10) < 24) process.exit(1)' || {
    echo 'Node.js 24 or newer not found' >&2
    exit 1
  }
  sh ci/lints.sh
  sh ci/deps.sh
  # Prettier: the standard formatter; large to write here.
  npx -y prettier@3.10.1 --check bindings/javascript
  cargo fmt --check -p starlightpdf-javascript
  cargo clippy --all-targets --locked --target wasm32-unknown-unknown -p starlightpdf-javascript
  RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --locked --target wasm32-unknown-unknown -p starlightpdf-javascript
  wasm=target/wasm32-unknown-unknown/release/starlightpdf_javascript.wasm
  # So a build that writes no .wasm fails rather than ship a stale one.
  rm -f "${wasm}"
  cargo build --release --locked --target wasm32-unknown-unknown -p starlightpdf-javascript
  out=target/javascript
  rm -rf "${out}"
  wasm-bindgen --target web --out-dir "${out}/package/wasm" "${wasm}"
  cp bindings/javascript/package.json LICENSE "${out}/package"
  # TypeScript (Apache-2.0): the standard type checker and compiler.
  npx -y -p typescript@7.0.2 tsc -p bindings/javascript
  npx -y -p typescript@7.0.2 tsc -p bindings/javascript/tests
  version=$(cargo pkgid -p starlightpdf-javascript)
  (cd "${out}/package" && npm pkg set "version=${version##*[#@]}" && npm pack --pack-destination ..)
  node --test --test-reporter=tap bindings/javascript/tests/*.test.mts
  exit 0
fi

sh ci/lints.sh
sh ci/deps.sh
cargo fmt --check -p starlightpdf-python
cargo clippy --all-targets --locked -p starlightpdf-python
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --locked -p starlightpdf-python
tmp=$(mktemp -d)
trap 'rm -rf "${tmp}"' EXIT
"${PY}" -m venv "${tmp}/venv"
if [ -x "${tmp}/venv/bin/python" ]; then
  venv=${tmp}/venv/bin/python
else
  venv=${tmp}/venv/Scripts/python.exe
fi
"${venv}" -m pip install --no-index ./bindings/python
"${venv}" -W error -X dev -bb -m unittest discover -s bindings/python/tests
"${venv}" -m pip install --require-hashes --only-binary :all: -r bindings/python/requirements-dev.txt
# stubtest caches in the working directory.
(cd "${tmp}" && "${venv}" -m mypy.stubtest --strict-type-check-only starlightpdf)
# Every check; the stub as a source so its own errors show.
set --
for c in deprecated exhaustive-match explicit-override ignore-without-code mutable-override possibly-undefined \
  redundant-expr redundant-self truthy-bool truthy-iterable unimported-reveal unused-awaitable unused-ignore; do
  set -- "$@" --enable-error-code "${c}"
done
"${venv}" -m mypy --config-file= --cache-dir "${tmp}/mypy" --strict --warn-unreachable --strict-equality-for-none \
  --disallow-any-expr --disallow-any-decorated --disallow-any-explicit --disallow-any-unimported --warn-incomplete-stub "$@" \
  bindings/python/tests bindings/python/starlightpdf/__init__.pyi
