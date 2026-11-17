# Unsafe code

Two things cannot be written without `unsafe`: `extern "C"` functions taking raw pointers, and the allocator that counts allocations in the memory test. The core forbids `unsafe` for good; only the C API crate and that test-only allocator may use it, each block with a SAFETY comment; no other crate writes any, so the browser source holds an id into a JavaScript table, not a JavaScript value, which is not thread-safe. A parser of untrusted files with no `unsafe` is a selling point, and every exception is a place to audit.
