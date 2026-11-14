# Glossary

## Code

**Core crate**:
The Rust crate `starlightpdf` in `core/`, with no dependencies.
_Avoid_: core library

**Binding crate**:
A crate that exposes the core crate to one language.
_Avoid_: wrapper

**Conformance suite**:
The cases in `tests/conformance/` that the core crate and every binding run.
_Avoid_: shared tests

## CI

**Area**:
A set of checks run together by `sh ci/check.sh <area>`. CI runs the areas that a pull request's changes need.
_Avoid_: job

**Nightly**:
The 22:00 UTC CI run of every area on Ubuntu, macOS and Windows; when all pass, it tags `main` as `nightly-YYYY-MM-DD`.
_Avoid_: nightly build
