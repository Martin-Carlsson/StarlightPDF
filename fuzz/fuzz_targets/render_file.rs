//! Opens arbitrary bytes as a document and renders its first page, small: the page tree, its boxes,
//! annotations, optional content and every resource dictionary come from the input.

#![no_main]

extern crate alloc;

use alloc::sync::Arc;

use starlightpdf::document::{Document, OpenOptions, PageOptions};
use starlightpdf::input::{BytesSourceOptions, Source};
use starlightpdf::limits::{ResourcePolicy, ResourcePolicyOptions};
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf::rendering::RenderOptions;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let mut source = BytesSourceOptions::default();
    source.bytes = Arc::new(data.to_vec());
    let mut limits = ResourcePolicyOptions::default();
    limits.decoded_limit = Some(1_u64 << 20_u32);
    let mut options = RenderOptions::default();
    options.fit = Some(32);
    drop(
        Source::from_bytes(&source)
            .and_then(|made| {
                Document::open(
                    &made,
                    Some(&ResourcePolicy::new(&limits)?),
                    &OpenOptions::default(),
                )
            })
            .and_then(|document| {
                document.page(&PageOptions::default())?.render(
                    &Sink::in_memory(&MemorySinkOptions::default())?,
                    None,
                    &options,
                )
            }),
    );
});
