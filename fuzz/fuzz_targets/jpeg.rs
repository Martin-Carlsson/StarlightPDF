//! Draws arbitrary bytes as a JPEG on a new page and saves the document.

#![no_main]

extern crate alloc;

use alloc::sync::Arc;

use starlightpdf::arrange::InsertBlankPageOptions;
use starlightpdf::document::{CreateOptions, Document, SaveOptions};
use starlightpdf::drawing::DrawImageOptions;
use starlightpdf::images::ImageFormat;
use starlightpdf::input::{BytesSourceOptions, Source};
use starlightpdf::limits::{ResourcePolicy, ResourcePolicyOptions};
use starlightpdf::output::{MemorySinkOptions, Sink};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let mut source = BytesSourceOptions::default();
    source.bytes = Arc::new(data.to_vec());
    let mut limits = ResourcePolicyOptions::default();
    limits.decoded_limit = Some(1 << 20);
    let mut draw = DrawImageOptions::default();
    draw.format = Some(ImageFormat::Jpeg);
    drop(ResourcePolicy::new(&limits).and_then(|policy| {
        let document = Document::create(Some(&policy), &CreateOptions::default())?;
        document
            .insert_blank_page(&InsertBlankPageOptions::default())?
            .draw_image(&Source::from_bytes(&source)?, None, None, &draw)?;
        document.save(
            &Sink::in_memory(&MemorySinkOptions::default())?,
            None,
            &SaveOptions::default(),
        )
    }));
});
