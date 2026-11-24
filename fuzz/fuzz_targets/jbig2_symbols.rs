//! Differential: what passes the scanner of JBIG2 symbol dictionaries, hayro-jbig2 decodes.
//!
//! The decoder runs behind the guard alone. The page's data ends in a cut segment, which is warned of
//! (ISO 32000-2:2020, 7.4.7) only once the guard has passed, so a second warning in one render is the decoder refusing
//! what the scanner read.

#![no_main]

extern crate alloc;

mod page;

use page::{render, stream};

/// A page information segment of 8 x 8 (ITU-T T.88, 7.4.8), then the header of one whose data is missing.
const PAGE: &[u8] = &[
    0, 0, 1, 0, 48, 0, 1, 0, 0, 0, 19, 0, 0, 0, 8, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 1, 1, 62, 0, 1, 0, 0, 0, 1,
];

/// The most segments.
const SEGMENTS: u8 = 64;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    // Each record is a symbol dictionary or a code table: kind and references, length, references, body.
    let mut globals = Vec::new();
    let mut rest = data;
    for number in 1..=SEGMENTS {
        let Some((&[head, length], after)) = rest.split_first_chunk() else {
            break;
        };
        let count = (head.wrapping_shr(1) & 7).min(4);
        let Some((referred, coded)) = after.split_at_checked(usize::from(count)) else {
            break;
        };
        let (body, next) = coded.split_at(usize::from(length).min(coded.len()));
        rest = next;
        let kind = if head & 1 == 0 { 0 } else { 53 };
        globals.extend_from_slice(&[0, 0, 0, number, kind, count.wrapping_shl(5)]);
        globals.extend(referred.iter().filter_map(|&to| to.checked_rem(number)));
        globals.extend_from_slice(&[0, 0, 0, 0]);
        globals.push(u8::try_from(body.len()).unwrap_or_default());
        globals.extend_from_slice(body);
    }
    let rendered = render(
        b"100 0 0 100 0 0 cm /I Do",
        "/XObject << /I 5 0 R >>",
        vec![
            stream(
                "/Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceGray /Filter /JBIG2Decode \
                 /DecodeParms << /JBIG2Globals 6 0 R >>",
                PAGE,
            ),
            stream("", &globals),
        ],
    );
    for clauses in rendered {
        assert!(
            clauses.len() <= 1 || !clauses.iter().any(|clause| clause.ends_with(" 7.4.7")),
            "hayro-jbig2 refused what the scanner passed: {clauses:?}"
        );
    }
});
