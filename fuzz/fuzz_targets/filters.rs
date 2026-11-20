//! Decodes arbitrary data through the two filters and the parameters its first two bytes choose.

#![no_main]

extern crate alloc;

use alloc::sync::Arc;

use starlightpdf::document::{Document, OpenOptions};
use starlightpdf::input::{BytesSourceOptions, Source};
use starlightpdf::limits::{ResourcePolicy, ResourcePolicyOptions};
use starlightpdf::objects::StreamDataOptions;
use starlightpdf::output::{MemorySinkOptions, Sink};

/// The catalog, page tree and cross-reference table, up to the stream object at offset 233.
const BEFORE_STREAM: &[u8] = b"%PDF-2.0\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\nxref\n0 4\n0000000000 65535 f \n0000000009 00000 n \n0000000058 00000 n \n0000000233 00000 n \ntrailer\n<< /Size 4 /Root 1 0 R >>\n";
/// After the data; `startxref` is fixed as the table precedes the stream.
const AFTER_DATA: &[u8] = b"\nendstream\nendobj\nstartxref\n110\n%%EOF\n";
/// The filters a stage chooses from by three bits, none first and past the end.
const FILTERS: [&str; 6] = [
    "",
    "/ASCIIHexDecode",
    "/ASCII85Decode",
    "/LZWDecode",
    "/FlateDecode",
    "/RunLengthDecode",
];
/// The predictors.
const PREDICTORS: [u8; 8] = [1, 2, 10, 11, 12, 13, 14, 15];

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&[filters, parameters], stream)) = data.split_first_chunk() else {
        return;
    };
    let filter = |index: u8| {
        FILTERS
            .get(usize::from(index & 7))
            .copied()
            .unwrap_or_default()
    };
    let decode_parms = format!(
        "<< /Predictor {} /Colors {} /BitsPerComponent {} /Columns {} /EarlyChange {} >>",
        PREDICTORS
            .get(usize::from(parameters & 7))
            .copied()
            .unwrap_or_default(),
        (parameters >> 6).saturating_add(1),
        1_u8 << ((parameters >> 3) & 7),
        (filters >> 6).saturating_add(1),
        (filters >> 6) & 1,
    );
    let mut file = BEFORE_STREAM.to_vec();
    file.extend_from_slice(
        format!(
            "3 0 obj\n<< /Filter [{} {}] /DecodeParms [{decode_parms} {decode_parms}] /Length {} >>\nstream\n",
            filter(filters),
            filter(filters >> 3),
            stream.len(),
        )
        .as_bytes(),
    );
    file.extend_from_slice(stream);
    file.extend_from_slice(AFTER_DATA);
    let mut source = BytesSourceOptions::default();
    source.bytes = Arc::new(file);
    let mut limits = ResourcePolicyOptions::default();
    limits.decoded_limit = Some(1 << 20);
    let mut options = StreamDataOptions::default();
    options.decode = true;
    options.id.number = 3;
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
                document.stream_data(&Sink::in_memory(&MemorySinkOptions::default())?, &options)
            }),
    );
});
