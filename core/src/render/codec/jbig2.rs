//! `JBIG2Decode` through hayro-jbig2, behind a guard (ISO 32000-2:2020, 7.4.7; ITU-T T.88).
//!
//! Nothing in the decoder's interface limits it, so its segments are walked first as it walks
//! them: the headers bound all but symbol dictionaries, which are scanned.

use alloc::collections::BTreeMap;
use core::mem;

use hayro_jbig2::{DecodeSettings, Decoder, Globals, Page};

use crate::render::codec::symbols::{self, Meter, SIDE, Standard, Symbols, Table};
use crate::render::codec::{self, Held, Info, Reader};
use crate::render::job::Job;
use crate::{PdfError, pull};

/// The decoder's contexts and state, whatever the data.
const BASE: u64 = 0x3_5000;

/// The fewest bytes of a segment's header: fewer at the end are padding.
const HEADER: usize = 11;

/// Work every region and pattern dictionary takes, to reset its contexts.
const REGION: u64 = 0x1_0000;

/// The bytes a segment takes besides its references.
const SEGMENT: u64 = 512;

/// The most segments, the globals' included.
const SEGMENTS: usize = 0x1_0000;

/// The most work of one image, of about 2.5 ns a unit: a 1200 dpi A4 page still fits.
const WORK: u64 = 0x2000_0000;

/// The code tables and symbol dictionaries of one part, by segment number.
#[derive(Default)]
struct Known {
    /// The dictionaries.
    dictionaries: BTreeMap<u32, Symbols>,
    /// The tables.
    tables: BTreeMap<u32, Table>,
}

/// The largest of what regions refer to.
struct Largest {
    /// Rows of a page.
    page: u32,
    /// Width and height of a pattern.
    pattern: [u32; 2],
    /// Patterns of one dictionary.
    patterns: u64,
}

/// The page, written as the decoder gives it.
struct Pixels<'row> {
    /// The pixel in the row.
    column: u32,
    /// The rows from this one on.
    rows: &'row mut [u8],
    /// The bytes of a row.
    stride: usize,
}

/// A segment (7.2).
struct Segment<'data> {
    /// Its data.
    body: &'data [u8],
    /// Its type.
    kind: u8,
    /// Its number.
    number: u32,
    /// The numbers it refers to, as stored.
    references: &'data [u8],
    /// The bytes of each of them.
    size: usize,
    /// Its length was not in the header.
    unsized_body: bool,
}

impl Decoder for Pixels<'_> {
    fn next_line(&mut self) {
        self.column = 0;
        self.rows = mem::take(&mut self.rows)
            .get_mut(self.stride..)
            .unwrap_or_default();
    }

    fn push_pixel(&mut self, black: bool) {
        let place = usize::try_from(self.column >> 3_u32).unwrap_or(usize::MAX);
        if black
            && place < self.stride
            && let Some(byte) = self.rows.get_mut(place)
        {
            *byte &= !0x80_u8.wrapping_shr(self.column & 7);
        }
        self.column = self.column.saturating_add(1);
    }

    fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
        let first = usize::try_from(self.column >> 3_u32).unwrap_or(usize::MAX);
        let count = usize::try_from(chunk_count).unwrap_or(usize::MAX);
        let end = first.saturating_add(count).min(self.stride);
        if black && let Some(span) = self.rows.get_mut(first..end) {
            span.fill(0);
        }
        self.column = self.column.saturating_add(chunk_count.saturating_mul(8));
    }
}

impl Segment<'_> {
    /// The numbers it refers to.
    fn referred(&self) -> impl Iterator<Item = u32> {
        self.references.chunks_exact(self.size).map(pull::number)
    }
}

/// The bytes of a region's body after its 17 of position and size (7.4.1), checked.
fn body<'data>(meter: &Meter<'_, '_>, segment: &Segment<'data>) -> Result<&'data [u8], PdfError> {
    let mut reader = Reader::new(segment.body);
    let operator = reader
        .take(16)
        .and_then(|_place| reader.byte())
        .ok_or_else(|| meter.malformed())?;
    if operator & 8 != 0 {
        // A coloured region, which the decoder would leave out without a word.
        return Err(meter.unsupported());
    }
    Ok(reader.rest())
}

/// The samples of `data`: 0 is black.
pub(crate) fn decode(job: &Job<'_>, data: &[u8], globals: Option<&[u8]>) -> Result<Held, PdfError> {
    let shared = globals.filter(|part| !part.is_empty());
    if guard(job, data, shared)? {
        job.warn("7.4.7", "JBIG2 data damaged or short")?;
    }
    let settings = DecodeSettings::default();
    let malformed = || job.objects.malformed();
    let decoded = shared
        .map(|part| Globals::new(part, &settings))
        .transpose()
        .map_err(|_error| malformed())?;
    let page =
        Page::new_embedded(data, decoded.as_ref(), &settings).map_err(|_error| malformed())?;
    let (width, height) = (page.width(), page.height());
    let stride = width.div_ceil(8);
    let size = u64::from(stride).saturating_mul(u64::from(height));
    let mut samples = codec::filled(job, size, 0xFF)?;
    let mut pixels = Pixels {
        column: 0,
        rows: samples.as_mut_slice(),
        stride: usize::try_from(stride).map_err(|_error| job.limit())?,
    };
    page.decode(&mut pixels).map_err(|_error| malformed())?;
    Ok(Held {
        alpha: None,
        data: samples,
        implied: None,
        info: Info {
            bits: 1,
            components: 1,
            height,
            width,
        },
        premultiplied: false,
    })
}

/// Whether the generic region's adaptive pixels are the default, which decode faster.
fn default_pixels(flags: u8, adaptive: &[u8]) -> bool {
    let wanted: &[u8] = match (flags >> 1_u32) & 3 {
        0 => &[3, 0xFF, 0xFD, 0xFF, 2, 0xFE, 0xFE, 0xFE],
        1 => &[3, 0xFF],
        _ => &[2, 0xFF],
    };
    adaptive.starts_with(wanted)
}

/// Fails unless the decoder is bounded on `data` with `globals`; whether the data is cut short.
fn guard(job: &Job<'_>, data: &[u8], globals: Option<&[u8]>) -> Result<bool, PdfError> {
    let mut meter = Meter::new(job, codec::most(job), WORK);
    meter.keep(BASE)?;
    let mut shared = Vec::new();
    let mut own = Vec::new();
    let mut cut = if let Some(part) = globals {
        codec::admitted(job, part)?;
        walk(&mut meter, part, &mut shared, SEGMENTS)?
    } else {
        false
    };
    codec::admitted(job, data)?;
    let left = SEGMENTS.saturating_sub(shared.len());
    cut |= walk(&mut meter, data, &mut own, left)?;
    let every = || shared.iter().chain(own.iter());
    let mut stripes = 0_u32;
    for segment in every().filter(|found| found.kind == 50 && found.body.len() == 4) {
        stripes = stripes.max(pull::number(segment.body).saturating_add(1));
    }
    let mut largest = Largest {
        page: 0,
        pattern: [0, 0],
        patterns: 1,
    };
    for segment in every() {
        match segment.kind {
            48 => paged(&mut meter, &mut largest, segment.body, stripes)?,
            16 => patterned(&mut meter, &mut largest, segment.body)?,
            _ => {}
        }
    }
    // Immediate regions and what is inside a region are freed before the next.
    let mut passing = 0_u64;
    for segment in every() {
        if matches!(
            segment.kind,
            4 | 6 | 7 | 20 | 22 | 23 | 36 | 38 | 39 | 40 | 42 | 43
        ) {
            passing = passing.max(region(&mut meter, &largest, segment)?);
        }
    }
    meter.keep(passing)?;
    if every().any(|found| matches!(found.kind, 0 | 4 | 6 | 7 | 53)) {
        let standard = Standard::new().ok_or_else(|| meter.malformed())?;
        let outer = scanned(&mut meter, &standard, &shared, None)?;
        let _inner: Known = scanned(&mut meter, &standard, &own, Some(&outer))?;
    }
    Ok(cut)
}

/// Meters a halftone's grid (6.6): the bytes it holds.
fn halftone(meter: &mut Meter<'_, '_>, largest: &Largest, body: &[u8]) -> Result<u64, PdfError> {
    let mut reader = Reader::new(body);
    let grid = reader
        .take(1)
        .and_then(|_flags| Some([reader.long()?, reader.long()?]))
        .ok_or_else(|| meter.malformed())?;
    let [across, down] = grid;
    if across > SIDE || down > SIDE {
        return Err(meter.refused());
    }
    let cells = u64::from(across).saturating_mul(u64::from(down));
    let [width, height] = largest.pattern;
    let blit = u64::from(height).saturating_mul(u64::from(width >> 5_u32).saturating_add(2));
    let above = largest.patterns.saturating_sub(1).leading_zeros();
    let planes = u64::from(u64::BITS.saturating_sub(above)).max(1);
    let each = planes
        .saturating_mul(6)
        .saturating_add(4)
        .saturating_add(blit.saturating_mul(2));
    let rows = u64::from(down).saturating_mul(16);
    meter.spend(cells.saturating_mul(each).saturating_add(rows))?;
    Ok(cells
        .saturating_mul(4)
        .saturating_add(symbols::words(across, down).saturating_mul(3))
        .saturating_add(u64::from(across).saturating_mul(24)))
}

/// Meters a page information segment (7.4.8), its bitmap and the samples made of it.
fn paged(
    meter: &mut Meter<'_, '_>,
    largest: &mut Largest,
    data: &[u8],
    stripes: u32,
) -> Result<(), PdfError> {
    let mut reader = Reader::new(data);
    let width = reader.long().ok_or_else(|| meter.malformed())?;
    let declared = reader.long().ok_or_else(|| meter.malformed())?;
    let height = if declared == u32::MAX {
        stripes
    } else {
        declared
    };
    if width > SIDE || height > SIDE {
        return Err(meter.refused());
    }
    largest.page = largest.page.max(height);
    let packed = u64::from(width.div_ceil(8)).saturating_mul(u64::from(height));
    meter.keep(symbols::words(width, height).saturating_add(packed))?;
    meter.spend(u64::from(width).saturating_mul(u64::from(height)))
}

/// Meters a pattern dictionary (7.4.4): its bitmap, patterns and shifted copies.
fn patterned(
    meter: &mut Meter<'_, '_>,
    largest: &mut Largest,
    data: &[u8],
) -> Result<(), PdfError> {
    let mut reader = Reader::new(data);
    let header = reader.take(3).zip(reader.long());
    let (shape, last) = header.ok_or_else(|| meter.malformed())?;
    let mut fields = Reader::new(shape);
    let (Some(flags), Some(across), Some(down)) = (fields.byte(), fields.byte(), fields.byte())
    else {
        return Err(meter.malformed());
    };
    if flags & 1 == 0 && across >= 128 {
        // The decoder negates the width in 8 bits: it overflows at 128 and is wrong above.
        return Err(meter.unsupported());
    }
    let count = u64::from(last).saturating_add(1);
    let total = u32::try_from(count.saturating_mul(u64::from(across)))
        .ok()
        .filter(|found| *found <= SIDE && across != 0 && down != 0)
        .ok_or_else(|| meter.malformed())?;
    let (width, height) = (u32::from(across), u32::from(down));
    let [widest, tallest] = largest.pattern;
    largest.pattern = [widest.max(width), tallest.max(height)];
    largest.patterns = largest.patterns.max(count);
    let mut each = symbols::words(width, height).saturating_add(96);
    if width <= 32 && height <= 32 {
        each = each.saturating_add(u64::from(height).saturating_mul(256));
    }
    let bytes = symbols::words(total, height)
        .saturating_add(u64::from(total).saturating_mul(24))
        .saturating_add(count.saturating_mul(each));
    meter.keep(bytes)?;
    let pixels = u64::from(total).saturating_mul(u64::from(height));
    let cut = u64::from(height).saturating_mul(32).saturating_add(256);
    let work = pixels
        .saturating_mul(6)
        .saturating_add(count.saturating_mul(cut))
        .saturating_add(REGION);
    meter.spend(work)
}

/// Meters a region: the bytes it holds while it is decoded.
fn region(
    meter: &mut Meter<'_, '_>,
    largest: &Largest,
    segment: &Segment<'_>,
) -> Result<u64, PdfError> {
    let mut reader = Reader::new(segment.body);
    let width = reader.long().ok_or_else(|| meter.malformed())?;
    let mut height = reader.long().ok_or_else(|| meter.malformed())?;
    let rest = body(meter, segment)?;
    if segment.unsized_body {
        // The rows follow the data (7.4.6.4).
        let start = segment.body.len().saturating_sub(4);
        let rows = pull::number(segment.body.get(start..).unwrap_or_default());
        if rows > height {
            return Err(meter.malformed());
        }
        height = rows;
    }
    if width > SIDE || height > SIDE {
        return Err(meter.refused());
    }
    let (kind, flags) = (segment.kind, rest.first().copied().unwrap_or(0));
    let mut held = if matches!(kind, 4 | 20 | 36 | 40) {
        // An intermediate region stays to the page's end.
        meter.keep(symbols::words(width, height))?;
        0
    } else {
        symbols::words(width, height)
    };
    let cost = match kind {
        36 | 38 | 39 if flags & 0x17 == 0x10 => 8, // The extended template reads 16 pixels.
        40 | 42 | 43 => 6,
        36 | 38 | 39 if flags & 1 == 0 => {
            if default_pixels(flags, rest.get(1..).unwrap_or_default()) {
                2
            } else {
                6
            }
        }
        _ => 2,
    };
    let pixels = u64::from(width.max(1)).saturating_mul(u64::from(height.max(1)));
    meter.spend(pixels.saturating_mul(cost).saturating_add(REGION))?;
    let more = match kind {
        // The lists of changes of MMR.
        36 | 38 | 39 => u64::from(width).saturating_mul(24),
        40 | 42 | 43 => {
            let words = u64::from(width >> 5_u32).saturating_add(1);
            meter.spend(u64::from(largest.page).saturating_mul(words))?;
            // A copy of the page under it.
            symbols::words(width, height)
        }
        4 | 6 | 7 => 0,
        _ => halftone(meter, largest, rest)?,
    };
    held = held.saturating_add(more);
    meter.hold(held)?;
    Ok(held)
}

/// What the dictionaries and tables of one part hold, in the decoder's order; with `outer`, the
/// globals', the part is a page, whose text regions are metered.
fn scanned(
    meter: &mut Meter<'_, '_>,
    standard: &Standard,
    segments: &[Segment<'_>],
    outer: Option<&Known>,
) -> Result<Known, PdfError> {
    let mut order: Vec<&Segment<'_>> = segments.iter().collect();
    order.sort_by_key(|segment| segment.number);
    let coded = order.iter().any(|segment| matches!(segment.kind, 0 | 53));
    let mut pairs = order.iter().zip(order.iter().skip(1));
    // The decoder searches by number, so of a number that repeats it would find either.
    if coded && pairs.any(|(left, right)| left.number == right.number) {
        return Err(meter.malformed());
    }
    let mut known = Known::default();
    for segment in order {
        match segment.kind {
            53 => {
                let table = Table::custom(meter, segment.body)?;
                let _none: Option<Table> = known.tables.insert(segment.number, table);
                continue;
            }
            0 => {}
            4 | 6 | 7 if outer.is_some() => {}
            _ => continue,
        }
        meter.spend(codec::wide(segment.references.len()).saturating_mul(64))?;
        let (mut inputs, mut tables) = (Vec::new(), Vec::new());
        for number in segment.referred() {
            let dictionary = known
                .dictionaries
                .get(&number)
                .or_else(|| outer?.dictionaries.get(&number));
            if let Some(found) = dictionary {
                inputs.push(found);
                continue;
            }
            let table = known
                .tables
                .get(&number)
                .or_else(|| outer?.tables.get(&number));
            // A number that is neither is passed over, as the decoder does.
            if let Some(found) = table {
                tables.push(found);
            }
        }
        if segment.kind == 0 {
            let found = symbols::dictionary(meter, standard, segment.body, &inputs, &tables)?;
            let _none: Option<Symbols> = known.dictionaries.insert(segment.number, found);
        } else {
            symbols::text(meter, segment.body, &inputs)?;
        }
    }
    Ok(known)
}

/// The length of a generic region that has none in its header: to its end marker and row count
/// (7.2.7).
fn unsized_length(rest: &[u8]) -> Option<usize> {
    let flags = rest.get(17).copied()?;
    let adaptive = if flags & 1 != 0 {
        0
    } else if (flags >> 1_u32) & 3 != 0 {
        2
    } else if flags & 0x10 != 0 {
        24
    } else {
        8
    };
    let marker: [u8; 2] = if flags & 1 == 0 { [0xFF, 0xAC] } else { [0, 0] };
    let start = 18_usize.saturating_add(adaptive);
    let searched = rest.get(start..rest.len().checked_sub(4)?)?;
    let found = searched.windows(2).position(|pair| pair == marker)?;
    start.checked_add(found)?.checked_add(6)
}

/// Reads the segments of `data` as the decoder does, at most `most`; whether it drops the last,
/// for being cut short.
fn walk<'data>(
    meter: &mut Meter<'_, '_>,
    data: &'data [u8],
    segments: &mut Vec<Segment<'data>>,
    most: usize,
) -> Result<bool, PdfError> {
    let mut reader = Reader::new(data);
    while reader.left() > 0 {
        let cut = reader.left() >= HEADER;
        let (Some(number), Some(flags), Some(count)) =
            (reader.long(), reader.byte(), reader.byte())
        else {
            return Ok(cut);
        };
        let referred = match count >> 5_u32 {
            5 | 6 => return Err(meter.malformed()),
            7 => {
                let Some(low) = reader.take(3).map(pull::number) else {
                    return Ok(cut);
                };
                let listed = (u32::from(count & 0x1F) << 24_u32) | low;
                let held = usize::try_from(listed).map_err(|_error| meter.refused())?;
                if reader.take(held.saturating_add(1).div_ceil(8)).is_none() {
                    return Ok(cut);
                }
                held
            }
            short => usize::from(short),
        };
        let size = match number {
            0..=256 => 1_usize,
            257..=0x1_0000 => 2,
            _ => 4,
        };
        let Some(references) = referred.checked_mul(size).and_then(|all| reader.take(all)) else {
            return Ok(cut);
        };
        if references
            .chunks_exact(size)
            .any(|stored| pull::number(stored) >= number)
        {
            return Err(meter.malformed());
        }
        let listed = codec::wide(referred).saturating_mul(24);
        // The decoder lists them before it finds what follows cut short.
        meter.keep(listed.saturating_add(SEGMENT))?;
        let association = if flags & 0x40 == 0 { 1 } else { 4 };
        let (Some(_), Some(length)) = (reader.take(association), reader.long()) else {
            return Ok(cut);
        };
        let kind = flags & 0x3F;
        let unsized_body = length == u32::MAX;
        let bytes = if unsized_body {
            matches!(kind, 38 | 39)
                .then(|| reader.rest())
                .and_then(unsized_length)
                .ok_or_else(|| meter.malformed())?
        } else {
            usize::try_from(length).map_err(|_error| meter.refused())?
        };
        let Some(found) = reader.take(bytes) else {
            return Ok(cut);
        };
        if segments.len() >= most {
            return Err(meter.refused());
        }
        segments.push(Segment {
            body: found,
            kind,
            number,
            references,
            size,
            unsized_body,
        });
        if kind == 51 {
            break;
        }
    }
    Ok(false)
}
