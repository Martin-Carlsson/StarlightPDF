//! Peak allocation of decoding a stream whose predictor declares rows far longer than its data.

extern crate alloc;

use alloc::sync::Arc;

use starlightpdf::PdfError;
use starlightpdf::document::{Document, OpenOptions};
use starlightpdf::input::{BytesSourceOptions, Source};
use starlightpdf::objects::StreamDataOptions;
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf_memory::peak;

/// Most bytes opening and decoding the stream may hold allocated at once.
const BOUND: usize = 45_056;

/// An empty zlib stream: header, a final empty stored block, Adler-32 1 (RFC 1950; RFC 1951,
/// 3.2.4).
const EMPTY: &[u8] = b"\x78\x01\x01\x00\x00\xff\xff\x00\x00\x00\x01";

/// The objects before the stream's data: rows of 65,536 pixels of 32 16-bit components, 4 MiB.
const HEAD: &str = "%PDF-2.0\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n\
3 0 obj\n<< /Filter /FlateDecode /DecodeParms << /Predictor 12 /Colors 32 \
/BitsPerComponent 16 /Columns 65536 >> /Length 11 >>\nstream\n";

/// The PDF: [`HEAD`], [`EMPTY`], then the table (ISO 32000-2:2020, 7.5.4).
fn pdf() -> Option<Vec<u8>> {
    let mut bytes = HEAD.as_bytes().to_vec();
    bytes.extend_from_slice(EMPTY);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
    let pages = HEAD.find("2 0 obj")?;
    let stream = HEAD.find("3 0 obj")?;
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n0 4\n0000000000 65535 f\r\n0000000009 00000 n\r\n{pages:010} 00000 n\r\n\
             {stream:010} 00000 n\r\ntrailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    Some(bytes)
}

/// A predictor holds rows as their data arrives: no data, no row (ISO 32000-2:2020, 7.4.4.4).
#[test]
fn predictor_rows_grow_with_data() {
    let mut options = StreamDataOptions::default();
    options.decode = true;
    options.id.number = 3;
    let measured = pdf().map(|bytes| {
        let mut source = BytesSourceOptions::default();
        source.bytes = Arc::new(bytes);
        let opened = Source::from_bytes(&source);
        peak(|| -> Result<Vec<u8>, PdfError> {
            let sink = Sink::in_memory(&MemorySinkOptions::default())?;
            Document::open(&opened?, None, &OpenOptions::default())?
                .stream_data(&sink, &options)?;
            sink.take_bytes()
        })
    });
    assert_eq!(
        measured
            .as_ref()
            .map(|found| found.0.as_ref().map_err(PdfError::code)),
        Some(Ok(&Vec::new())),
        "data"
    );
    let top = measured.map_or(0, |(_, bytes)| bytes);
    assert!((1..=BOUND).contains(&top), "peak {top} bytes");
}
