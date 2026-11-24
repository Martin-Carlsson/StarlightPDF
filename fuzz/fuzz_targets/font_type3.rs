//! Shows text in a Type 3 font whose glyph is arbitrary bytes and may show itself.

#![no_main]

extern crate alloc;

mod page;

use page::{render, stream};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    drop(render(
        b"BT /F 40 Tf 5 30 Td (AA) Tj ET",
        "/Font << /F 5 0 R >>",
        vec![
            b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 10 10] /FontMatrix [0.1 0 0 0.1 0 0] /FirstChar 65 \
              /LastChar 65 /Widths [10] /Encoding << /Differences [65 /a] >> /CharProcs << /a 6 0 R >> \
              /Resources << /Font << /F 5 0 R >> >> >>"
                .to_vec(),
            stream("", data),
        ],
    ));
});
