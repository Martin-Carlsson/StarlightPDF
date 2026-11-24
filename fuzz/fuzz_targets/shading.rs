//! Paints arbitrary bytes as a shading of the type and bits its first two bytes choose, by `sh` or as a pattern.

#![no_main]

extern crate alloc;

mod page;

use page::{render, stream};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&[chosen, packing], body)) = data.split_first_chunk() else {
        return;
    };
    let kind = (chosen & 7).max(1);
    let boxed = if chosen & 32 == 0 {
        ""
    } else {
        "/Background [0 1 0] /BBox [10 10 90 90]"
    };
    let shading = if kind < 4 {
        let coords: Vec<String> = body
            .iter()
            .take(if kind == 2 { 4 } else { 6 })
            .map(|&byte| byte.cast_signed().to_string())
            .collect();
        let function = if kind == 1 {
            "6 0 R"
        } else {
            "<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>"
        };
        format!(
            "<< {boxed} /ShadingType {kind} /ColorSpace /DeviceRGB /Coords [{}] /Matrix [{}] /Extend [{} {}] \
             /Function {function} >>",
            coords.join(" "),
            coords.join(" "),
            chosen & 8 != 0,
            chosen & 16 != 0,
        )
        .into_bytes()
    } else {
        let coordinate = [1_u8, 2, 4, 8, 12, 16, 24, 32]
            .get(usize::from(packing & 7))
            .copied()
            .unwrap_or(8);
        let component = [1_u8, 2, 4, 8, 12, 16]
            .get(usize::from(packing.wrapping_shr(3) & 7))
            .copied()
            .unwrap_or(8);
        let flag = [2_u8, 4, 8, 8]
            .get(usize::from(packing.wrapping_shr(6)))
            .copied()
            .unwrap_or(8);
        stream(
            &format!(
                "{boxed} /ShadingType {kind} /ColorSpace /DeviceRGB /BitsPerFlag {flag} /BitsPerCoordinate {coordinate} \
                 /BitsPerComponent {component} /VerticesPerRow {} /Decode [0 100 0 100 0 1 0 1 0 1]",
                u16::from(chosen.wrapping_shr(3)).saturating_add(2),
            ),
            body,
        )
    };
    let ramp = stream(
        "/FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1]",
        b"{ 0 }",
    );
    let content: &[u8] = if chosen & 32 == 0 {
        b"/S sh"
    } else {
        b"/Pattern cs /P scn 5 5 90 90 re f"
    };
    drop(render(
        content,
        "/Shading << /S 5 0 R >> /Pattern << /P << /PatternType 2 /Shading 5 0 R >> >>",
        vec![shading, ramp],
    ));
});
