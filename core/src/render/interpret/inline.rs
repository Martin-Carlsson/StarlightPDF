//! Inline images (ISO 32000-2:2020, 8.9.7).

use crate::overlay::Object;
use crate::pull::Pull;
use crate::render::data::{self, Category};
use crate::render::fixed::Fixed;
use crate::render::interpret::{Run, content};
use crate::render::job::Job;
use crate::render::{colour, image};
use crate::stream::Feeder;
use crate::syntax::{Reader, is_regular, is_white_space};
use crate::{ErrorCode, PdfError};

/// Work of an inline image besides its entries and samples.
const INLINE_WORK: u64 = 256;

/// Work of a pass over an inline image's data, beside its bytes.
const PASS_WORK: u64 = 16;

/// The bytes after `EI` that must look like content.
const PLAUSIBLE: usize = 64;

/// The bytes looked ahead.
const WINDOW: usize = 256;

/// An inline image's data as stored, pulled from the content.
struct Data<'reader, 'data> {
    /// Where it ends.
    end: End,
    /// The byte before the next.
    last: u8,
    /// The content.
    reader: &'reader mut Reader<Feeder<'data>>,
    /// Takes work.
    spend: &'reader dyn Fn(u64) -> Result<(), PdfError>,
}

/// Where the data ends.
#[derive(Clone, Copy)]
enum End {
    /// After so many more bytes.
    Count(u64),
    /// Here.
    Found,
    /// Before an `EI` that content follows.
    Scan,
}

impl Pull for Data<'_, '_> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let mut filled = 0_usize;
        while filled < buf.len() {
            (self.spend)(PASS_WORK)?;
            let most = match self.end {
                End::Count(0) | End::Found => break,
                End::Count(left) => usize::try_from(left).unwrap_or(usize::MAX),
                End::Scan => {
                    let ahead = self.reader.lookahead(WINDOW)?;
                    let Some(from) = ends(self.last, ahead) else {
                        self.end = End::Found;
                        break;
                    };
                    // Up to the next candidate: E after white space or >, then I.
                    let next = (from.max(1)..ahead.len()).find(|&at| {
                        let before = ahead.get(at.saturating_sub(1)).copied().unwrap_or(0);
                        ahead.get(at) == Some(&b'E')
                            && (is_white_space(before) || before == b'>')
                            && ahead
                                .get(at.saturating_add(1))
                                .is_none_or(|byte| *byte == b'I')
                    });
                    let most = next.unwrap_or(ahead.len());
                    // Each byte examined once, the plausible ones after a failed candidate twice.
                    (self.spend)(u64::try_from(most).unwrap_or(u64::MAX))?;
                    most
                }
            };
            let until = filled.saturating_add(most).min(buf.len());
            let out = buf.get_mut(filled..until).unwrap_or_default();
            let count = self.reader.take(out)?;
            if count == 0 {
                break;
            }
            self.last = out
                .get(count.saturating_sub(1))
                .copied()
                .unwrap_or_default();
            if let End::Count(left) = self.end {
                let taken = u64::try_from(count).unwrap_or(u64::MAX);
                self.end = End::Count(left.saturating_sub(taken));
            }
            filled = filled.saturating_add(count);
        }
        Ok(filled)
    }
}

impl Data<'_, '_> {
    /// Passes over what is left and `EI`.
    fn close(&mut self, job: &Job<'_>) -> Result<(), PdfError> {
        self.drain(job)?;
        if matches!(self.end, End::Count(_)) {
            loop {
                let ahead = self.reader.lookahead(WINDOW)?;
                let spaces = ahead
                    .iter()
                    .position(|byte| !is_white_space(*byte))
                    .unwrap_or(ahead.len());
                if spaces == 0 {
                    break;
                }
                job.spend(PASS_WORK.saturating_add(u64::try_from(spaces).unwrap_or(u64::MAX)))?;
                let mut skipped = [0; WINDOW];
                let _spaces: usize = self
                    .reader
                    .take(skipped.get_mut(..spaces).unwrap_or_default())?;
            }
            if !closes(self.reader.lookahead(3)?) {
                job.warn("8.9.7", "an inline image's data is not as long as declared")?;
                (self.end, self.last) = (End::Scan, b' ');
                self.drain(job)?;
            }
        }
        if closes(self.reader.lookahead(3)?) {
            // The look ahead left both in the window.
            let _end: usize = self.reader.take(&mut [0; 2])?;
            return Ok(());
        }
        job.warn("8.9.7", "an inline image has no EI")
    }

    /// Passes over the data left.
    fn drain(&mut self, job: &Job<'_>) -> Result<(), PdfError> {
        let mut scratch = [0; WINDOW];
        while self.pull(&mut scratch)? > 0 {
            job.spend(1)?;
        }
        Ok(())
    }
}

/// Whether `ahead` starts with `EI` as a word.
fn closes(ahead: &[u8]) -> bool {
    ahead.starts_with(b"EI") && ahead.get(2).is_none_or(|byte| !is_regular(*byte))
}

/// Draws the image after `BI` and leaves the content after its `EI`.
pub(crate) fn draw(
    run: &mut Run<'_, '_>,
    operations: &mut content::Operations<'_>,
) -> Result<(), PdfError> {
    read(run, operations, true)
}

/// Passes over the image after `BI`, drawing nothing, and leaves the content after its `EI`.
pub(crate) fn pass(
    run: &mut Run<'_, '_>,
    operations: &mut content::Operations<'_>,
) -> Result<(), PdfError> {
    read(run, operations, false)
}

/// Reads the image after `BI`, drawing it when `drawing`, and leaves the content after its `EI`.
fn read(
    run: &mut Run<'_, '_>,
    operations: &mut content::Operations<'_>,
    drawing: bool,
) -> Result<(), PdfError> {
    let job = run.job;
    // Setting up an image allocates as no operator does.
    job.spend(INLINE_WORK)?;
    let mut entries = Vec::new();
    for (short, value) in operations.image_entries(job)? {
        let key = full_key(short);
        let expanded = match key.as_slice() {
            b"ColorSpace" => spelled(value, space_name),
            b"Filter" => spelled(value, filter_name),
            _ => value,
        };
        entries.push((key, expanded));
    }
    // After ID one white space.
    let reader = operations.input();
    if reader
        .lookahead(1)?
        .first()
        .is_some_and(|byte| is_white_space(*byte))
    {
        let _space: usize = reader.take(&mut [0])?;
    }
    let mut dictionary = Object::Dictionary(entries);
    let mut known = true;
    if let Some(Object::Name(name)) = dictionary.get(b"ColorSpace")
        && !matches!(
            name.as_slice(),
            b"DeviceCMYK" | b"DeviceGray" | b"DeviceRGB"
        )
    {
        let found = data::named(job, &run.scope, Category::ColorSpace, &name)
            .and_then(|named| named.map(|space| space.small(job)).transpose());
        let message = "an inline image's colour space cannot be read";
        match job.tolerate(found, "8.9.7", message)? {
            Some(Some(space)) => dictionary.set(b"ColorSpace", Some(space)),
            Some(None) => {
                job.warn("7.8.3", "a colour space is missing")?;
                known = false;
            }
            None => known = false,
        }
    }
    if !known {
        dictionary.set(b"ColorSpace", None);
    }
    let declared = job.tolerate(
        length(run, &dictionary),
        "8.9.7",
        "an inline image's length cannot be told",
    )?;
    let spend = |units: u64| job.spend(units);
    let mut data = Data {
        end: declared.unwrap_or(End::Scan),
        last: b' ',
        reader,
        spend: &spend,
    };
    let drawn = if drawing && known && run.hidden == 0 {
        image::draw_inline(run, &dictionary, &mut data)
    } else {
        Ok(())
    };
    // Whatever became of the image, content starts after it.
    if let Err(error) = drawn.as_ref()
        && error.code() != ErrorCode::Malformed
    {
        return drawn;
    }
    data.close(job)?;
    drawn
}

/// `None` when the data ends before `ahead`, `last` being the byte before: an `EI` that content,
/// not more binary data, follows. Otherwise where in `ahead` the next end may be: past the byte
/// that failed the content, which fails every `EI` before it too.
fn ends(last: u8, ahead: &[u8]) -> Option<usize> {
    if !(is_white_space(last) || last == b'>') || !closes(ahead) {
        return Some(1);
    }
    let after = ahead.get(2..).unwrap_or_default();
    let mut before = b' ';
    for (at, &byte) in after.iter().take(PLAUSIBLE).enumerate() {
        // Strings, comments and the next image's data may hold any byte.
        if byte == b'(' || byte == b'%' || (before == b'B' && byte == b'I') {
            return None;
        }
        if !byte.is_ascii_graphic() && !is_white_space(byte) {
            return Some(at.saturating_add(3));
        }
        before = byte;
    }
    None
}

/// A filter's name in full.
const fn filter_name(name: &[u8]) -> &[u8] {
    match name {
        b"A85" => b"ASCII85Decode",
        b"AHx" => b"ASCIIHexDecode",
        b"CCF" => b"CCITTFaxDecode",
        b"DCT" => b"DCTDecode",
        b"Fl" => b"FlateDecode",
        b"LZW" => b"LZWDecode",
        b"RL" => b"RunLengthDecode",
        _ => name,
    }
}

/// The key `short` in full.
fn full_key(short: Vec<u8>) -> Vec<u8> {
    let full: &[u8] = match short.as_slice() {
        b"BPC" => b"BitsPerComponent",
        b"CS" => b"ColorSpace",
        b"D" => b"Decode",
        b"DP" => b"DecodeParms",
        b"F" => b"Filter",
        b"H" => b"Height",
        b"I" => b"Interpolate",
        b"IM" => b"ImageMask",
        b"L" => b"Length",
        b"W" => b"Width",
        _ => return short,
    };
    full.to_vec()
}

/// Where the data of `dictionary` ends: as declared, else as counted when unfiltered.
fn length(run: &mut Run<'_, '_>, dictionary: &Object) -> Result<End, PdfError> {
    let job = run.job;
    let value = |key: &[u8]| data::entry(job, dictionary, key);
    let count = |found: &Object| {
        let number = Fixed::from_object(found)?;
        u64::try_from(number.floor()).ok()
    };
    if let Some(declared) = count(value(b"Length")?.value()) {
        return Ok(End::Count(declared));
    }
    let filter = value(b"Filter")?;
    let filtered = match *filter.value() {
        Object::Null => false,
        Object::Array(_) => data::items(filter.value()).is_some_and(|items| !items.is_empty()),
        Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => true,
    };
    if filtered {
        return Ok(End::Scan);
    }
    let width = count(value(b"Width")?.value());
    let height = count(value(b"Height")?.value());
    let (components, bits) = if matches!(*value(b"ImageMask")?.value(), Object::Boolean(true)) {
        (1, Some(1))
    } else {
        let bits = count(value(b"BitsPerComponent")?.value());
        let family = value(b"ColorSpace")?;
        if matches!(*family.value(), Object::Null) {
            return Ok(End::Scan);
        }
        let space = colour::resolve(job, &mut run.caches.spaces, &run.scope, &family)?;
        (u64::try_from(space.components()).unwrap_or(u64::MAX), bits)
    };
    let total = bits
        .filter(|found| matches!(found, 1 | 2 | 4 | 8 | 16))
        .zip(width)
        .and_then(|(found, columns)| columns.checked_mul(components)?.checked_mul(found))
        .zip(height)
        .and_then(|(row, rows)| row.div_ceil(8).checked_mul(rows));
    Ok(total.map_or(End::Scan, End::Count))
}

/// A colour space's name in full.
const fn space_name(name: &[u8]) -> &[u8] {
    match name {
        b"CMYK" => b"DeviceCMYK",
        b"G" => b"DeviceGray",
        b"I" => b"Indexed",
        b"RGB" => b"DeviceRGB",
        _ => name,
    }
}

/// `value` with its names, its own or its items', as `full` spells them.
fn spelled(value: Object, full: fn(&[u8]) -> &[u8]) -> Object {
    let spell = |item: Object| match item {
        Object::Name(name) => Object::Name(full(&name).to_vec()),
        other @ (Object::Array(_)
        | Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Null
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_)) => other,
    };
    if let Object::Array(items) = value {
        Object::Array(items.into_iter().map(spell).collect())
    } else {
        spell(value)
    }
}
