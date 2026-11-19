//! Counts the pages of a valid file whose cross-reference stream holds arbitrary Flate data.

#![no_main]

extern crate alloc;

use alloc::sync::Arc;

use starlightpdf::document::{Document, OpenOptions};
use starlightpdf::input::{BytesSourceOptions, Source};

/// Up to the `Length` value; the catalog and page tree lie in an object stream.
const BEFORE_LENGTH: &[u8] = b"%PDF-2.0\n3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n4 0 obj\n<< /Type /ObjStm /N 2 /First 9 /Length 85 >>\nstream\n1 0 2 34 << /Type /Catalog /Pages 2 0 R >>\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\n\nendstream\nendobj\n5 0 obj\n<< /Type /XRef /Size 6 /W [1 2 1] /Root 1 0 R /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 4 >> /Length ";
/// Between the `Length` value and the data.
const BEFORE_DATA: &[u8] = b" >>\nstream\n";
/// After the data; `startxref` is fixed as the cross-reference stream precedes the data.
const AFTER_DATA: &[u8] = b"\nendstream\nendobj\nstartxref\n243\n%%EOF\n";

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let mut file = BEFORE_LENGTH.to_vec();
    file.extend_from_slice(data.len().to_string().as_bytes());
    file.extend_from_slice(BEFORE_DATA);
    file.extend_from_slice(data);
    file.extend_from_slice(AFTER_DATA);
    let mut options = BytesSourceOptions::default();
    options.bytes = Arc::new(file);
    drop(
        Source::from_bytes(&options)
            .and_then(|source| Document::open(&source, None, &OpenOptions::default()))
            .and_then(|document| document.page_count()),
    );
});
