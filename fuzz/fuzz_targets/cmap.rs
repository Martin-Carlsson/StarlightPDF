//! Shows text through arbitrary bytes as an embedded `CMap`.

#![no_main]

extern crate alloc;

mod page;

use page::{render, stream};

/// An embedded TrueType program of one box.
const FONT: &[u8] = include_bytes!("../../tests/corpus/render/font-truetype.ttf");

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    drop(render(
        b"BT /F 20 Tf 5 30 Td <41424300018001FFFF0041> Tj ET",
        "/Font << /F 5 0 R >>",
        vec![
            b"<< /Type /Font /Subtype /Type0 /BaseFont /SLTest /Encoding 6 0 R /DescendantFonts [7 0 R] >>".to_vec(),
            stream("/Type /CMap /CMapName /SL-H", data),
            b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /SLTest /FontDescriptor 8 0 R /CIDToGIDMap /Identity \
              /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> >>"
                .to_vec(),
            b"<< /Type /FontDescriptor /FontName /SLTest /Flags 4 /FontBBox [0 0 1000 1000] /ItalicAngle 0 \
              /Ascent 800 /Descent 0 /CapHeight 800 /StemV 80 /FontFile2 9 0 R >>"
                .to_vec(),
            stream("", FONT),
        ],
    ));
});
