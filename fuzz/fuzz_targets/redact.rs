//! Opens arbitrary bytes, marks the middle of the first page for redaction, applies the marks
//! and saves the document.

#![no_main]

extern crate alloc;

use alloc::sync::Arc;

use starlightpdf::PdfError;
use starlightpdf::annotations::{AddAnnotationOptions, AnnotationKind};
use starlightpdf::document::{Document, OpenOptions, PageOptions, SaveOptions};
use starlightpdf::edit::ApplyRedactionsOptions;
use starlightpdf::input::{BytesSourceOptions, Source};
use starlightpdf::limits::{ResourcePolicy, ResourcePolicyOptions};
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf::page::BoundaryOptions;
use starlightpdf::security::Cipher;

/// Marks the middle half of the first page, applies every mark and saves decrypted to a sink in
/// memory.
fn redact(document: &Document) -> Result<(), PdfError> {
    let page = document.page(&PageOptions::default())?;
    let shown = page.boundary(&BoundaryOptions::default())?;
    let mut mark = AddAnnotationOptions::default();
    mark.kind = AnnotationKind::Redact;
    mark.x = shown.width.mul_add(0.25_f64, 0.0_f64);
    mark.y = shown.height.mul_add(0.25_f64, 0.0_f64);
    mark.width = shown.width.mul_add(0.5_f64, 0.0_f64);
    mark.height = shown.height.mul_add(0.5_f64, 0.0_f64);
    drop(page.add_annotation(None, &mark)?);
    document.apply_redactions(None, &ApplyRedactionsOptions::default())?;
    let mut options = SaveOptions::default();
    options.break_signatures = true;
    options.cipher = Some(Cipher::Unencrypted);
    let sink = Sink::in_memory(&MemorySinkOptions::default())?;
    document.save(&sink, None, &options).map(drop)
}

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let mut source = BytesSourceOptions::default();
    source.bytes = Arc::new(data.to_vec());
    let mut limits = ResourcePolicyOptions::default();
    limits.decoded_limit = Some(1 << 20);
    limits.object_limit = Some(1 << 16);
    let Ok(document) = Source::from_bytes(&source).and_then(|made| {
        Document::open(
            &made,
            Some(&ResourcePolicy::new(&limits)?),
            &OpenOptions::default(),
        )
    }) else {
        return;
    };
    drop(redact(&document));
});
