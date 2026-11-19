//! Reads the header version of arbitrary bytes.

#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    drop(starlightpdf::header_version(data));
});
