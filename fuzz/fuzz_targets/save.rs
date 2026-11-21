//! Opens arbitrary bytes, saves the document, then splits it into a part per page and saves
//! each part.

#![no_main]

extern crate alloc;

use alloc::sync::Arc;

use starlightpdf::PdfError;
use starlightpdf::arrange::{SplitBy, SplitOptions};
use starlightpdf::document::{Document, OpenOptions, SaveOptions};
use starlightpdf::input::{BytesSourceOptions, Source};
use starlightpdf::limits::{ResourcePolicy, ResourcePolicyOptions};
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf::security::Cipher;

/// Saves `document` decrypted to a sink in memory.
fn save(document: &Document) -> Result<(), PdfError> {
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
    drop(save(&document));
    let mut split = SplitOptions::default();
    split.by = SplitBy::Pages;
    split.count = 1;
    split.allow_loss = true;
    if let Ok(parts) = document.split(&split) {
        while let Ok(Some(part)) = parts.next() {
            drop(save(&part));
        }
    }
});
