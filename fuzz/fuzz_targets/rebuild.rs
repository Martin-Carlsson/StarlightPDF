//! Opens arbitrary bytes, rebuilding a damaged file, and reads its catalog, objects, first page
//! and warnings.

#![no_main]

extern crate alloc;

use alloc::sync::Arc;

use starlightpdf::document::{Document, OpenOptions, PageOptions};
use starlightpdf::input::{BytesSourceOptions, Source};
use starlightpdf::limits::{ResourcePolicy, ResourcePolicyOptions};
use starlightpdf::objects::{ObjectOptions, StreamDataOptions};
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf::page::{Boundary, BoundaryOptions};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let mut source = BytesSourceOptions::default();
    source.bytes = Arc::new(data.to_vec());
    let mut limits = ResourcePolicyOptions::default();
    limits.decoded_limit = Some(1 << 20);
    let Ok(document) = Source::from_bytes(&source).and_then(|made| {
        Document::open(
            &made,
            Some(&ResourcePolicy::new(&limits)?),
            &OpenOptions::default(),
        )
    }) else {
        return;
    };
    drop(document.page_count());
    drop(document.catalog());
    for number in 1..=16 {
        let mut object = ObjectOptions::default();
        object.id.number = number;
        drop(document.object(&object));
        let mut stream = StreamDataOptions::default();
        stream.decode = true;
        stream.id = object.id;
        drop(
            Sink::in_memory(&MemorySinkOptions::default())
                .and_then(|sink| document.stream_data(&sink, &stream)),
        );
    }
    if let Ok(page) = document.page(&PageOptions::default()) {
        for kind in [
            Boundary::Art,
            Boundary::Bleed,
            Boundary::Crop,
            Boundary::Media,
            Boundary::Trim,
        ] {
            let mut boundary = BoundaryOptions::default();
            boundary.kind = kind;
            drop(page.boundary(&boundary));
        }
        drop(page.rotation());
    }
    if let Ok(warnings) = document.warnings() {
        while let Ok(Some(_)) = warnings.next() {}
    }
});
