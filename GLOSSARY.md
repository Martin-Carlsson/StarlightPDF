# Glossary

## Code

**Core crate**:
The Rust crate `starlightpdf` in `core/`.
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
The 22:00 UTC CI run of every area; when all pass, it tags `main` as `nightly-YYYY-MM-DD`.
_Avoid_: nightly build

## Public API

**Source**:
Input with two calls: its size and "read these bytes at this offset".
_Avoid_: input stream, reader

**Sink**:
Output that only appends.
_Avoid_: output stream, writer

**Overlay**:
A document's changed and new objects, kept apart from its source.
_Avoid_: change set, diff

**Handle**:
What `Document`, `Page` and the rest are: a reference-counted, thread-safe reference. It never holds decoded data.
_Avoid_: pointer

**Option record**:
The one struct of options an operation takes. Flat: no field names a struct other than a handle or a struct of only numbers.
_Avoid_: config, parameters

**Cursor**:
A handle that returns a large result one item at a time through `next`.
_Avoid_: iterator

**Error code**:
The stable `ErrorCode` of the one `PdfError`.
_Avoid_: error kind, error type

**Capability**:
A heavy part a build can leave out, such as OCR or rendering.
_Avoid_: plugin, extension

**Target**:
Where a link, bookmark or view goes.
_Avoid_: URL

**Object layer**:
The public API that reads and replaces raw objects and streams.
_Avoid_: low-level API

**Document layer**:
The public API for pages and operations such as rotate and merge.
_Avoid_: high-level API

**Resource policy**:
The limits passed at open: cache, decoded data, nesting depth, object count, warning count, concurrency and temporary storage. Documents opened with the same policy share its budget.
_Avoid_: configuration
