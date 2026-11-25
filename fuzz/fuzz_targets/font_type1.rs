//! Shows text in arbitrary bytes as a Type 1 program.

#![no_main]

extern crate alloc;

mod page;

use page::{render, stream};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    drop(render(
        b"1 0 0 rg 0 0 1 RG BT /F 20 Tf 5 60 Td (AB C ab01\\001\\060\\341) Tj 5 Tr 0 -30 Td (A) Tj ET",
        "/Font << /F 5 0 R >>",
        vec![
            b"<< /Type /Font /Subtype /Type1 /BaseFont /SLTest /FirstChar 65 /LastChar 66 /Widths [500 500] \
              /FontDescriptor 6 0 R >>"
                .to_vec(),
            b"<< /Type /FontDescriptor /FontName /SLTest /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 \
              /Ascent 800 /Descent 0 /CapHeight 800 /StemV 80 /FontFile 7 0 R >>"
                .to_vec(),
            stream("", data),
        ],
    ));
});
