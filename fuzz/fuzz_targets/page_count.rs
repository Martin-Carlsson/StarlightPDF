//! Opens arbitrary bytes as a document and counts its pages.

#![no_main]

extern crate alloc;

use alloc::sync::Arc;

use starlightpdf::document::{Document, OpenOptions};
use starlightpdf::input::{BytesSourceOptions, Source};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let mut options = BytesSourceOptions::default();
    options.bytes = Arc::new(data.to_vec());
    drop(
        Source::from_bytes(&options)
            .and_then(|source| Document::open(&source, None, &OpenOptions::default()))
            .and_then(|document| document.page_count()),
    );
});
