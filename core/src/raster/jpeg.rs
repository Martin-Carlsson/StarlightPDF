//! JPEG files (ITU-T T.81, B), embedded unchanged under `DCTDecode` (ISO 32000-2:2020, 7.4.8).

use super::{Cursor, Image, Profile, READ, exif, malformed};
use crate::input::Source;
use crate::pull::number;
use crate::{ErrorCode, PdfError};

/// The size of an `ICC_PROFILE` segment's own header (ICC.1:2010, B.4).
const ICC_HEADER: u64 = 14;

/// The frame header (B.2.2).
struct Frame {
    /// Components.
    components: u8,
    /// Lines.
    height: u32,
    /// Samples per line.
    width: u32,
}

/// Reads the JPEG in `source` through its first scan header; its ICC profile may hold at most
/// `limit` bytes.
pub(super) fn read(source: &Source, limit: u64) -> Result<Image, PdfError> {
    let mut cursor = Cursor::new(source, 0);
    let mut start = [0; 2];
    if cursor.fill(&mut start)? != 2 || start != [0xFF, 0xD8] {
        return Err(malformed());
    }
    let mut adobe = false;
    let mut frame = None;
    let mut orientation = None;
    // Offsets and lengths of the ICC segments by sequence number, and their count.
    let mut icc: Vec<Option<(u64, u64)>> = Vec::new();
    loop {
        let marker = next_marker(&mut cursor)?;
        if matches!(marker, 0x01 | 0xD0..=0xD7) {
            continue;
        }
        if matches!(marker, 0xD8 | 0xD9) {
            return Err(malformed());
        }
        let mut length = [0; 2];
        if cursor.fill(&mut length)? != 2 {
            return Err(malformed());
        }
        let body = u64::from(number(&length))
            .checked_sub(2)
            .ok_or_else(malformed)?;
        let at = cursor.offset();
        // The bytes of the segment read; the rest are skipped.
        let wanted = match marker {
            0xC0..=0xC2 | 0xE1 => body,
            0xE2 => ICC_HEADER,
            0xEE => 5,
            _ => 0,
        };
        let mut head = vec![0; usize::try_from(wanted.min(body)).unwrap_or(0)];
        // A cut segment fails at the next marker.
        let _: usize = cursor.fill(&mut head)?;
        cursor.skip(body.saturating_sub(wanted));
        match marker {
            0xC0..=0xC2 => {
                if frame.replace(parse_frame(&head)?).is_some() {
                    return Err(malformed());
                }
            }
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Err(PdfError::new(ErrorCode::Unsupported, READ));
            }
            0xDA => break,
            0xE1 if orientation.is_none() => {
                if let Some(tiff) = head.strip_prefix(b"Exif\0\0") {
                    orientation = Some(exif::orientation(tiff));
                }
            }
            0xE2 => {
                if let Some(&[sequence, count]) = head.strip_prefix(b"ICC_PROFILE\0") {
                    add_part(
                        &mut icc,
                        sequence,
                        count,
                        at.saturating_add(ICC_HEADER),
                        body,
                    )?;
                }
            }
            0xEE => adobe |= head == b"Adobe",
            _ => {}
        }
    }
    let found = frame.ok_or_else(malformed)?;
    let embedded = !icc.is_empty();
    let profile = if embedded {
        let parts = icc
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(malformed)?;
        let profile = Profile {
            compressed: false,
            length: parts.iter().map(|&(_, length)| length).sum(),
            parts,
        };
        profile
            .check(source, found.components, limit)?
            .then_some(profile)
    } else {
        None
    };
    Ok(Image {
        bits: 8,
        components: found.components,
        dropped_profile: embedded && profile.is_none(),
        height: found.height,
        inverted: adobe && found.components == 4,
        length: source.size(READ)?,
        orientation: orientation.unwrap_or(1),
        palette: None,
        png: None,
        profile,
        source: source.clone(),
        width: found.width,
    })
}

/// Records ICC segment `sequence` of `count`, of `body` bytes with its data at `offset`.
fn add_part(
    icc: &mut Vec<Option<(u64, u64)>>,
    sequence: u8,
    count: u8,
    offset: u64,
    body: u64,
) -> Result<(), PdfError> {
    if icc.is_empty() {
        icc.resize(usize::from(count), None);
    }
    let length = body.saturating_sub(ICC_HEADER);
    let slot = usize::from(sequence)
        .checked_sub(1)
        .filter(|_| icc.len() == usize::from(count))
        .and_then(|index| icc.get_mut(index))
        .ok_or_else(malformed)?;
    if slot.replace((offset, length)).is_some() {
        return Err(malformed());
    }
    Ok(())
}

/// The marker after fill bytes (B.1.1.2); fails at the end or on other bytes.
fn next_marker(cursor: &mut Cursor<'_>) -> Result<u8, PdfError> {
    if cursor.byte()? != Some(0xFF) {
        return Err(malformed());
    }
    loop {
        match cursor.byte()? {
            Some(0xFF) => {}
            Some(0) | None => return Err(malformed()),
            Some(marker) => return Ok(marker),
        }
    }
}

/// Parses a frame header (B.2.2) of 8-bit samples and 1, 3 or 4 components.
fn parse_frame(head: &[u8]) -> Result<Frame, PdfError> {
    let Some((
        &[
            precision,
            height_high,
            height_low,
            width_high,
            width_low,
            components,
        ],
        specs,
    )) = head.split_first_chunk::<6>()
    else {
        return Err(malformed());
    };
    if specs.len() != usize::from(components).saturating_mul(3) || components == 0 {
        return Err(malformed());
    }
    let width = number(&[width_high, width_low]);
    if width == 0 {
        return Err(malformed());
    }
    let height = number(&[height_high, height_low]);
    if precision != 8 || height == 0 || !matches!(components, 1 | 3 | 4) {
        return Err(PdfError::new(ErrorCode::Unsupported, READ));
    }
    Ok(Frame {
        components,
        height,
        width,
    })
}
