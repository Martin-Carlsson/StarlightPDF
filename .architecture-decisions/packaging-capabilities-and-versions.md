# Packaging, capabilities and versions

OCR, rendering, conversion and signing are heavy, and most users want a single install. One package per registry with everything in it; the heavy parts are core features that Rust users switch off; the npm package holds two prebuilt WebAssembly modules, with and without them, and the caller loads one; `capabilities` reports what is built in, and calling a missing part fails with a code. Size matters where the choice exists, a merge-only page downloads no engine, and npm users do not compile Rust, so their choice must ship prebuilt.

A binding and the core could each carry a version of their own. One version number everywhere; every binding reports it and its capabilities; the C ABI version is explicit. A bug report naming a version then points at the same code; the price is that a fix in one binding releases them all.
