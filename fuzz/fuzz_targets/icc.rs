//! Fills and draws an image in arbitrary bytes as an ICC profile.

#![no_main]

extern crate alloc;

mod page;

use page::{render, stream};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    // The header's data colour space (ICC.1:2022, 7.2.6) picks the components.
    let (components, operands, width) = match data.get(16) {
        Some(&b'G') => (1_u8, "0.5", 12_u8),
        Some(&b'C') => (4, "0.1 0.2 0.3 0.4", 3),
        Some(_) | None => (3, "0.2 0.5 0.8", 4),
    };
    drop(render(
        format!("/C cs {operands} scn 0 0 50 100 re f 50 0 0 100 50 0 cm /I Do").as_bytes(),
        "/ColorSpace << /C [/ICCBased 5 0 R] >> /XObject << /I 6 0 R >>",
        vec![
            stream(&format!("/N {components}"), data),
            stream(
                &format!(
                    "/Subtype /Image /Width {width} /Height 1 /ColorSpace [/ICCBased 5 0 R] /BitsPerComponent 8"
                ),
                &[0, 32, 64, 96, 128, 160, 192, 224, 255, 1, 2, 3],
            ),
        ],
    ));
});
