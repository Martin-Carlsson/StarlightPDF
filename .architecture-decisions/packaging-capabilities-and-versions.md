# Packaging, capabilities and versions

One package per registry with everything in it; heavy parts (OCR, rendering, conversion, signing) are core features that Rust and WebAssembly users can switch off; `capabilities` reports what is built in, and calling a missing part fails with a code. A single install is what most users want, size matters where the switch exists, and a merge-only app loads no engine.

One version number everywhere; every binding reports it and its capabilities; the C ABI version is explicit. A bug report naming a version then points at the same code.
