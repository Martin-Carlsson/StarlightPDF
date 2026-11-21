//! Exif orientation (CIPA DC-008-2019; TIFF 6.0, 2).

/// The Orientation tag (4.6.4 A).
const ORIENTATION: u16 = 274;

/// The SHORT field type (TIFF 6.0, 2).
const SHORT: u16 = 3;

/// The orientation, 1 to 8, in the TIFF data `tiff`; 1 when absent or unreadable.
pub(super) fn orientation(tiff: &[u8]) -> u8 {
    let little = match tiff.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        Some(_) | None => return 1,
    };
    let read = |at: usize, count: usize| -> Option<u32> {
        let bytes = tiff.get(at..at.checked_add(count)?)?;
        Some(if little {
            bytes.iter().rev().fold(0, |value: u32, byte| {
                value.wrapping_shl(8) | u32::from(*byte)
            })
        } else {
            super::number(bytes)
        })
    };
    let short = |at: usize| u16::try_from(read(at, 2)?).ok();
    let found = || -> Option<u8> {
        if short(2)? != 42 {
            return None;
        }
        let directory = usize::try_from(read(4, 4)?).ok()?;
        let entries = directory.checked_add(2)?;
        for index in 0..usize::from(short(directory)?) {
            let entry = entries.checked_add(index.checked_mul(12)?)?;
            if short(entry)? == ORIENTATION {
                return (short(entry.checked_add(2)?)? == SHORT
                    && read(entry.checked_add(4)?, 4)? == 1)
                    .then(|| short(entry.checked_add(8)?))
                    .flatten()
                    .and_then(|value| u8::try_from(value).ok())
                    .filter(|value| (1..=8).contains(value));
            }
        }
        None
    };
    found().unwrap_or(1)
}
