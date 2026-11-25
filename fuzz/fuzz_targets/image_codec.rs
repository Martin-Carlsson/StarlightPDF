//! Draws arbitrary bytes as CCITT, JBIG2 or JPX data, through the guards, as the first byte chooses.

#![no_main]

extern crate alloc;

mod page;

use page::{render, stream};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&chosen, rest)) = data.split_first() else {
        return;
    };
    let set = |bit: u8| chosen & bit != 0;
    let (mut coded, mut shared): (&[u8], &[u8]) = (rest, b"");
    let entries = match chosen & 3 {
        1 => {
            if set(4)
                && let Some((&length, after)) = rest.split_first()
                && let Some((globals, own)) = after.split_at_checked(usize::from(length))
            {
                (coded, shared) = (own, globals);
            }
            format!(
                "/ImageMask {} /Filter /JBIG2Decode /DecodeParms << /JBIG2Globals 6 0 R >> /Width 64 /Height 64 \
                 /ColorSpace /DeviceGray",
                set(8),
            )
        }
        2 => {
            let space = match (set(4), set(16)) {
                (true, _) => "",
                (false, true) => "/ColorSpace [/Indexed /DeviceRGB 1 <FF0000 0000FF>]",
                (false, false) => "/ColorSpace /DeviceGray",
            };
            format!(
                "/Filter /JPXDecode /Width 32 /Height 32 /SMaskInData {} {space}",
                u8::from(set(8)),
            )
        }
        _ => format!(
            "/Filter /CCITTFaxDecode /Width 64 /Height 64 /ColorSpace /DeviceGray /BitsPerComponent 1 /DecodeParms \
             << /K {} /EncodedByteAlign {} /EndOfLine {} /BlackIs1 {} /EndOfBlock {} /Columns 64 >>",
            [-1_i8, 0, 2, 4]
                .get(usize::from(chosen.wrapping_shr(2) & 3))
                .copied()
                .unwrap_or_default(),
            set(16),
            set(32),
            set(64),
            !set(128),
        ),
    };
    drop(render(
        b"100 0 0 100 0 0 cm /I Do",
        "/XObject << /I 5 0 R >>",
        vec![
            stream(&format!("/Subtype /Image {entries}"), coded),
            stream("", shared),
        ],
    ));
});
