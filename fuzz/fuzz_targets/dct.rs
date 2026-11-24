//! Renders arbitrary bytes as a `DCTDecode` image.

#![no_main]

extern crate alloc;

mod page;

use page::{render, stream};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let components = data.windows(10).find_map(|window| match *window {
        [0xFF, 0xC0..=0xC2, .., count] => Some(count),
        _ => None,
    });
    let space = match components {
        Some(1) => "Gray",
        Some(4) => "CMYK",
        Some(_) | None => "RGB",
    };
    drop(render(
        b"100 0 0 100 0 0 cm /I Do",
        "/XObject << /I 5 0 R >>",
        vec![stream(
            &format!(
                "/Subtype /Image /Width 8 /Height 8 /ColorSpace /Device{space} /BitsPerComponent 8 /Filter /DCTDecode"
            ),
            data,
        )],
    ));
});
