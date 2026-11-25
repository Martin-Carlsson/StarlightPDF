//! Evaluates arbitrary bytes as a calculator or sampled function of one and of two inputs, or as a function's
//! object where they start as one.

#![no_main]

extern crate alloc;

mod page;

use page::{render, stream};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let first = data.first().copied().unwrap_or(0);
    let (one, two) = if first == b'{' {
        (
            String::from("/FunctionType 4"),
            String::from("/FunctionType 4"),
        )
    } else {
        let bits = [1_u8, 2, 4, 8, 12, 16, 24, 32]
            .get(usize::from(first & 7))
            .copied()
            .unwrap_or(8);
        let size = u16::from(first.wrapping_shr(3)).saturating_add(1);
        let sampled = format!("/FunctionType 0 /BitsPerSample {bits} /Size");
        (
            format!("{sampled} [{size}]"),
            format!("{sampled} [{size} 2]"),
        )
    };
    let range = "/Range [0 1 0 1 0 1]";
    // A dictionary or an array is the function itself, so types 2 and 3 and arrays are reached.
    let functions = if matches!(first, b'<' | b'[') {
        vec![data.to_vec(), data.to_vec()]
    } else {
        vec![
            stream(&format!("{one} /Domain [0 1] {range}"), data),
            stream(&format!("{two} /Domain [0 1 0 1] {range}"), data),
        ]
    };
    drop(render(
        b"/C cs 0.3 scn 0 0 50 100 re f 50 0 50 100 re W n /S sh",
        "/ColorSpace << /C [/Separation /Spot /DeviceRGB 5 0 R] >> /Shading << /S << /ShadingType 1 \
         /ColorSpace /DeviceRGB /Domain [0 1 0 1] /Matrix [100 0 0 100 0 0] /Function 6 0 R >> >>",
        functions,
    ));
});
