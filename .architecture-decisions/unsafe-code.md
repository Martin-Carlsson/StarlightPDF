# Unsafe code

The core forbids `unsafe` for good; the C API crate alone may use it, each block with a SAFETY comment. `extern "C"` functions taking raw pointers cannot be written without it, and a parser of untrusted files with no `unsafe` is a selling point.
