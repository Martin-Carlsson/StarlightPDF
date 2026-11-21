//! Opens arbitrary bytes with the password `user` and reads its objects and decoded streams.

#![no_main]

extern crate alloc;

use alloc::sync::Arc;

use starlightpdf::document::{Document, OpenOptions};
use starlightpdf::input::{BytesSourceOptions, Source};
use starlightpdf::limits::{ResourcePolicy, ResourcePolicyOptions};
use starlightpdf::objects::{ObjectOptions, StreamDataOptions};
use starlightpdf::output::{MemorySinkOptions, Sink};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let mut source = BytesSourceOptions::default();
    source.bytes = Arc::new(data.to_vec());
    let mut limits = ResourcePolicyOptions::default();
    limits.decoded_limit = Some(1 << 20);
    let mut options = OpenOptions::default();
    options.password = String::from("user");
    let Ok(document) = Source::from_bytes(&source)
        .and_then(|made| Document::open(&made, Some(&ResourcePolicy::new(&limits)?), &options))
    else {
        return;
    };
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
});
