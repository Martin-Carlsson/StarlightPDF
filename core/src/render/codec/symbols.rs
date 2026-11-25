//! The guard that scans symbol dictionaries (ITU-T T.88, 6.5).
//!
//! Their sizes and counts are entropy coded, so this decodes what hayro-jbig2 0.4.0 would, with
//! its quirks, metering each step before taking it. Where the two disagree the guard has a hole.

use crate::PdfError;
use crate::render::codec::{self, Reader};
use crate::render::job::Job;

/// Table B.1.
const B1: &[Line] = &[
    line(1, 4, 0),
    line(2, 8, 16),
    line(3, 16, 272),
    line(3, 32, 65_808),
];

/// Table B.2.
const B2: &[Line] = &[
    line(1, 0, 0),
    line(2, 0, 1),
    line(3, 0, 2),
    line(4, 3, 3),
    line(5, 6, 11),
    line(6, 32, 75),
    out_of_band(6),
];

/// Table B.3.
const B3: &[Line] = &[
    line(8, 8, -256),
    line(1, 0, 0),
    line(2, 0, 1),
    line(3, 0, 2),
    line(4, 3, 3),
    line(5, 6, 11),
    lower(8, -257),
    line(7, 32, 75),
    out_of_band(6),
];

/// Table B.4.
const B4: &[Line] = &[
    line(1, 0, 1),
    line(2, 0, 2),
    line(3, 0, 3),
    line(4, 3, 4),
    line(5, 6, 12),
    line(5, 32, 76),
];

/// Table B.5.
const B5: &[Line] = &[
    line(7, 8, -255),
    line(1, 0, 1),
    line(2, 0, 2),
    line(3, 0, 3),
    line(4, 3, 4),
    line(5, 6, 12),
    lower(7, -256),
    line(6, 32, 76),
];

/// Context bits of each template.
const CONTEXT_BITS: [u32; 4] = [16, 13, 10, 10];

/// Bytes of the contexts and fixed state one dictionary decodes with.
const DICTIONARY: u64 = 0x1_6000;

/// The groups of Figure A.1: their lowest value and bits.
const GROUPS: [(u32, u32); 6] = [(0, 2), (4, 4), (20, 6), (84, 8), (340, 12), (4436, 32)];

/// The most empty height classes or export runs in a row: the decoder repeats them for ever.
const IDLE: u32 = 64;

/// The contexts of an integer decoder (A.2).
const INTEGER: usize = 512;

/// The most lines of a code table.
const LINES: usize = 4096;

/// Table E.1: Qe, the state after an MPS, after an LPS, and whether the MPS switches.
const STATES: [(u32, u8, u8, bool); 47] = [
    (0x5601, 1, 1, true),
    (0x3401, 2, 6, false),
    (0x1801, 3, 9, false),
    (0x0AC1, 4, 12, false),
    (0x0521, 5, 29, false),
    (0x0221, 38, 33, false),
    (0x5601, 7, 6, true),
    (0x5401, 8, 14, false),
    (0x4801, 9, 14, false),
    (0x3801, 10, 14, false),
    (0x3001, 11, 17, false),
    (0x2401, 12, 18, false),
    (0x1C01, 13, 20, false),
    (0x1601, 29, 21, false),
    (0x5601, 15, 14, true),
    (0x5401, 16, 14, false),
    (0x5101, 17, 15, false),
    (0x4801, 18, 16, false),
    (0x3801, 19, 17, false),
    (0x3401, 20, 18, false),
    (0x3001, 21, 19, false),
    (0x2801, 22, 19, false),
    (0x2401, 23, 20, false),
    (0x2201, 24, 21, false),
    (0x1C01, 25, 22, false),
    (0x1801, 26, 23, false),
    (0x1601, 27, 24, false),
    (0x1401, 28, 25, false),
    (0x1201, 29, 26, false),
    (0x1101, 30, 27, false),
    (0x0AC1, 31, 28, false),
    (0x09C1, 32, 29, false),
    (0x08A1, 33, 30, false),
    (0x0521, 34, 31, false),
    (0x0441, 35, 32, false),
    (0x02A1, 36, 33, false),
    (0x0221, 37, 34, false),
    (0x0141, 38, 35, false),
    (0x0111, 39, 36, false),
    (0x0085, 40, 37, false),
    (0x0049, 41, 38, false),
    (0x0025, 42, 39, false),
    (0x0015, 43, 40, false),
    (0x0009, 44, 41, false),
    (0x0005, 45, 42, false),
    (0x0001, 45, 43, false),
    (0x5601, 46, 46, false),
];

/// Work of a step of the scan.
const STEP: u64 = 256;

/// The most symbols the decoder takes.
const SYMBOLS: u32 = 0x10_0000;

/// The runs of pixels each template reads in a row (6.2.5.3): rows up, pixels, the rightmost.
const TEMPLATES: [&[Run]; 4] = [
    &[run(2, 3, 1), run(1, 5, 2), run(0, 4, -1)],
    &[run(2, 4, 2), run(1, 5, 2), run(0, 3, -1)],
    &[run(2, 3, 1), run(1, 4, 1), run(0, 2, -1)],
    &[run(1, 5, 1), run(0, 4, -1)],
];

/// The largest side the decoder takes.
pub(super) const SIDE: u32 = 0xFFFF;

/// An adaptive template pixel.
#[derive(Clone, Copy, Default)]
struct Adaptive {
    /// Columns right of the pixel.
    across: isize,
    /// Rows above it.
    up: usize,
}

/// The MQ decoder (Annex E).
struct Arithmetic<'data> {
    /// The A register.
    a_register: u32,
    /// The byte read last.
    at: usize,
    /// The C register.
    c_register: u32,
    /// The data, followed by 0xFF without end.
    data: &'data [u8],
    /// The bits of C not shifted out.
    left: u32,
}

/// A reader of bits.
struct Bits<'data> {
    /// The bits read.
    at: usize,
    /// The data.
    data: &'data [u8],
}

/// The header of a dictionary (7.4.2.1).
struct Header<'data> {
    /// The adaptive pixels.
    adaptive: [Adaptive; 4],
    /// The coded data.
    body: &'data [u8],
    /// SDNUMEXSYMS.
    exported: u32,
    /// The flags.
    flags: u16,
    /// SDNUMNEWSYMS.
    fresh: u32,
    /// SDTEMPLATE.
    template: usize,
}

/// An integer decoder (A.2).
struct Integer {
    /// Its contexts.
    contexts: [u8; INTEGER],
}

/// A line of a code table.
#[derive(Clone, Copy)]
struct Line {
    /// How its offset counts.
    kind: LineKind,
    /// RANGELOW.
    low: i64,
    /// PREFLEN.
    prefix: u8,
    /// RANGELEN.
    range: u8,
}

/// How a line's offset counts.
#[derive(Clone, Copy)]
enum LineKind {
    /// Down from the low.
    Lower,
    /// Not at all: the line is out of band.
    OutOfBand,
    /// Up from the low.
    Upper,
}

/// Bytes and work of one image's JBIG2, checked as they grow.
pub(super) struct Meter<'job, 'doc> {
    /// The render.
    job: &'job Job<'doc>,
    /// The bytes that stay.
    kept: u64,
    /// The most bytes.
    most: u64,
    /// The work so far.
    work: u64,
    /// The most work: the decoder cannot be interrupted.
    work_most: u64,
}

/// A node of a code's tree.
#[derive(Clone, Copy, Default)]
struct Node {
    /// The nodes after a 0 and after a 1; 0 is none, as none lead to the root.
    children: [usize; 2],
    /// The line of a code that ends here.
    line: Option<Line>,
}

/// The contexts a dictionary retains.
struct Retained {
    /// The contexts.
    contexts: Vec<u8>,
    /// SDRTEMPLATE.
    refining: bool,
    /// SDTEMPLATE.
    template: usize,
}

/// A run of template pixels in one row.
#[derive(Clone, Copy)]
struct Run {
    /// Its pixels.
    bits: u32,
    /// The rightmost, from the pixel decoded.
    to: isize,
    /// Rows above it.
    up: usize,
}

/// The decoding of one dictionary, as far as the guard must follow it.
struct Scan<'scan, 'data, 'job, 'doc> {
    /// The generic contexts.
    contexts: Vec<u8>,
    /// IAEX.
    exports: Integer,
    /// The header.
    header: Header<'data>,
    /// IADH.
    heights: Integer,
    /// The bytes the decoder holds for it.
    held: u64,
    /// The meter.
    meter: &'scan mut Meter<'job, 'doc>,
    /// The new symbols.
    sizes: Vec<Size>,
    /// The tables of heights, widths and bitmap sizes.
    tables: [&'scan Table; 3],
    /// The values' source.
    values: Values<'data>,
    /// IADW.
    widths: Integer,
    /// The rows of a bitmap that later pixels read.
    window: Vec<u8>,
}

/// A bitmap's size.
#[derive(Clone, Copy)]
struct Size {
    /// Rows.
    height: u32,
    /// Columns.
    width: u32,
}

/// The standard tables a dictionary chooses from.
pub(super) struct Standard {
    /// B.1.
    b1: Table,
    /// B.2.
    b2: Table,
    /// B.3.
    b3: Table,
    /// B.4.
    b4: Table,
    /// B.5.
    b5: Table,
}

/// What a dictionary exports.
pub(super) struct Symbols {
    /// The contexts it retains.
    contexts: Option<Retained>,
    /// The most words that drawing one of them touches.
    draw: u64,
    /// Their sizes.
    sizes: Vec<Size>,
}

/// A code table (Annex B).
pub(super) struct Table {
    /// Its tree, the root first.
    nodes: Vec<Node>,
}

/// A value decoded.
#[derive(Clone, Copy)]
enum Value {
    /// A number.
    Number(i32),
    /// OOB.
    OutOfBand,
}

/// Where a dictionary's values come from.
struct Values<'data> {
    /// Of the MQ decoder.
    arithmetic: Arithmetic<'data>,
    /// Of Huffman codes.
    bits: Bits<'data>,
    /// SDHUFF.
    huffman: bool,
}

impl<'data> Arithmetic<'data> {
    /// The byte `ahead` of the one read last.
    fn byte(&self, ahead: usize) -> u32 {
        let found = self.data.get(self.at.saturating_add(ahead)).copied();
        u32::from(found.unwrap_or(0xFF))
    }

    /// The next bit (E.3.2).
    fn decode(&mut self, context: &mut u8) -> bool {
        let (state, most_probable) = (*context & 0x7F, *context >> 7_u32 != 0);
        let (estimate, after_most, after_least, switches) =
            STATES.get(usize::from(state)).copied().unwrap_or_default();
        self.a_register = self.a_register.wrapping_sub(estimate);
        let below = (self.c_register >> 16_u32) < self.a_register;
        if below && self.a_register & 0x8000 != 0 {
            return most_probable;
        }
        let small = self.a_register < estimate;
        if !below {
            self.c_register = self.c_register.wrapping_sub(self.a_register << 16_u32);
            self.a_register = estimate;
        }
        let least = below == small;
        let next = if least { after_least } else { after_most };
        let flipped = most_probable != (least && switches);
        *context = next | if flipped { 0x80 } else { 0 };
        // The interval is never 0, so this ends within 16 turns.
        while self.a_register & 0x8000 == 0 && self.a_register != 0 {
            if self.left == 0 {
                self.refill();
            }
            self.a_register <<= 1_u32;
            self.c_register <<= 1_u32;
            self.left = self.left.saturating_sub(1);
        }
        most_probable != least
    }

    /// INITDEC (Annex G).
    fn new(data: &'data [u8]) -> Self {
        let mut found = Self {
            a_register: 0,
            at: 0,
            c_register: 0,
            data,
            left: 0,
        };
        found.c_register = (found.byte(0) ^ 0xFF) << 16_u32;
        found.refill();
        found.c_register <<= 7_u32;
        found.left = found.left.saturating_sub(7);
        found.a_register = 0x8000;
        found
    }

    /// BYTEIN (Annex G).
    fn refill(&mut self) {
        self.left = 8;
        let (base, shift) = if self.byte(0) == 0xFF {
            if self.byte(1) > 0x8F {
                return;
            }
            self.left = 7;
            (0xFE00, 9_u32)
        } else {
            (0xFF00, 8_u32)
        };
        self.at = self.at.saturating_add(1);
        self.c_register = self
            .c_register
            .wrapping_add(base)
            .wrapping_sub(self.byte(0).wrapping_shl(shift));
    }
}

impl<'data> Bits<'data> {
    /// Moves to the next byte.
    const fn align(&mut self) {
        self.at = self.at.saturating_add(7) & !7;
    }

    /// The next bit.
    fn bit(&mut self) -> Option<bool> {
        let byte = self.data.get(self.at >> 3_u32).copied()?;
        let mask = [0x80_u8, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01]
            .get(self.at & 7)
            .copied()?;
        self.at = self.at.saturating_add(1);
        Some(byte & mask != 0)
    }

    /// The next `count` bits, at most 32, as a number.
    fn read(&mut self, count: u8) -> Option<u32> {
        let mut value = 0_u32;
        for _ in 0..count {
            value = value.wrapping_shl(1) | u32::from(self.bit()?);
        }
        Some(value)
    }

    /// Skips `count` bytes, which must be there.
    fn skip(&mut self, count: u64) -> Option<&'data [u8]> {
        let start = self.at >> 3_u32;
        let length = usize::try_from(count).ok()?;
        let found = self.data.get(start..start.checked_add(length)?)?;
        self.at = self.at.checked_add(length.checked_mul(8)?)?;
        Some(found)
    }
}

impl<'data> Header<'data> {
    /// The contexts' count.
    fn contexts(&self) -> usize {
        let bits = CONTEXT_BITS.get(self.template).copied().unwrap_or_default();
        1_usize.wrapping_shl(bits)
    }

    /// Whether flag `mask` is set.
    const fn has(&self, mask: u16) -> bool {
        self.flags & mask != 0
    }

    /// The header in `data`, whose `flags` are read.
    fn read(flags: u16, reader: &mut Reader<'data>) -> Option<Self> {
        let template = usize::from((flags >> 10_u32) & 3);
        let mut adaptive = [Adaptive::default(); 4];
        if flags & 1 == 0 {
            let count = if template == 0 { 4 } else { 1 };
            for pixel in adaptive.iter_mut().take(count) {
                let (across, down) = (reader.byte()?.cast_signed(), reader.byte()?.cast_signed());
                if down > 0 || (down == 0 && across >= 0) {
                    return None;
                }
                *pixel = Adaptive {
                    across: isize::from(across),
                    up: usize::from(down.unsigned_abs()),
                };
            }
        }
        let (exported, fresh) = (reader.long()?, reader.long()?);
        (exported <= SYMBOLS && fresh <= SYMBOLS).then(|| Self {
            adaptive,
            body: reader.rest(),
            exported,
            flags,
            fresh,
            template,
        })
    }
}

impl Integer {
    /// One bit, and the bits so far in `seen`.
    fn bit(&mut self, decoder: &mut Arithmetic<'_>, seen: &mut usize) -> bool {
        let Some(context) = self.contexts.get_mut(*seen & 0x1FF) else {
            return false;
        };
        let found = decoder.decode(context);
        let next = seen.wrapping_shl(1) | usize::from(found);
        *seen = if *seen < 256 {
            next
        } else {
            (next & 0x1FF) | 256
        };
        found
    }

    /// A decoder that has seen nothing.
    const fn new() -> Self {
        Self {
            contexts: [0; INTEGER],
        }
    }

    /// The next value.
    fn read(&mut self, decoder: &mut Arithmetic<'_>) -> Value {
        let mut seen = 1_usize;
        let negative = self.bit(decoder, &mut seen);
        let mut group = 0_usize;
        while group < 5 && self.bit(decoder, &mut seen) {
            group = group.saturating_add(1);
        }
        let (low, width) = GROUPS.get(group).copied().unwrap_or_default();
        let mut found = 0_u32;
        for _ in 0..width {
            found = found.wrapping_shl(1) | u32::from(self.bit(decoder, &mut seen));
        }
        let value = found.wrapping_add(low);
        if !negative {
            Value::Number(value.cast_signed())
        } else if value == 0 {
            Value::OutOfBand
        } else {
            Value::Number(value.cast_signed().wrapping_neg())
        }
    }
}

impl<'job, 'doc> Meter<'job, 'doc> {
    /// Fails unless `bytes` fit on top of those that stay, beside all that the render holds.
    pub(super) fn hold(&self, bytes: u64) -> Result<(), PdfError> {
        let total = self.kept.saturating_add(bytes);
        if total > self.most {
            return Err(self.job.limit());
        }
        self.job.fits(total)
    }

    /// Takes `bytes` that stay.
    pub(super) fn keep(&mut self, bytes: u64) -> Result<(), PdfError> {
        self.kept = self.kept.saturating_add(bytes);
        self.hold(0)
    }

    /// An [`crate::ErrorCode::Malformed`] error.
    pub(super) fn malformed(&self) -> PdfError {
        self.job.objects.malformed()
    }

    /// A meter of `most` bytes and `work_most` work.
    pub(super) const fn new(job: &'job Job<'doc>, most: u64, work_most: u64) -> Self {
        Self {
            job,
            kept: 0,
            most,
            work: 0,
            work_most,
        }
    }

    /// An [`crate::ErrorCode::LimitExceeded`] error.
    pub(super) fn refused(&self) -> PdfError {
        self.job.limit()
    }

    /// Takes `units` of work.
    pub(super) fn spend(&mut self, units: u64) -> Result<(), PdfError> {
        self.work = self.work.saturating_add(units);
        if self.work > self.work_most {
            return Err(self.job.limit());
        }
        self.job.spend(units)
    }

    /// An [`crate::ErrorCode::Unsupported`] error.
    pub(super) fn unsupported(&self) -> PdfError {
        self.job.unsupported()
    }
}

impl Scan<'_, '_, '_, '_> {
    /// The height classes (6.5.5), until every new symbol has its size.
    fn classes(&mut self) -> Result<(), PdfError> {
        let [height_table, width_table, _] = self.tables;
        let fresh = usize::try_from(self.header.fresh).map_err(|_error| self.meter.refused())?;
        let (mut height, mut idle) = (0_u32, 0_u32);
        while self.sizes.len() < fresh {
            self.meter.spend(STEP)?;
            let grown = self.values.next(height_table, &mut self.heights);
            height = stepped(height, grown).ok_or_else(|| self.meter.malformed())?;
            let (mut width, mut total) = (0_u32, 0_u32);
            let first = self.sizes.len();
            loop {
                self.meter.spend(STEP)?;
                let wider = self.values.next(width_table, &mut self.widths);
                if matches!(wider, Some(Value::OutOfBand)) {
                    break;
                }
                if self.sizes.len() >= fresh {
                    return Err(self.meter.malformed());
                }
                width = stepped(width, wider).ok_or_else(|| self.meter.malformed())?;
                total = total
                    .checked_add(width)
                    .ok_or_else(|| self.meter.malformed())?;
                let size = Size { height, width };
                if !self.values.huffman {
                    self.symbol(size)?;
                }
                self.sizes.push(size);
            }
            idle = if self.sizes.len() == first {
                idle.saturating_add(1)
            } else {
                0
            };
            if idle > IDLE {
                return Err(self.meter.refused());
            }
            if self.values.huffman {
                self.collective(first, height, total)?;
            }
        }
        Ok(())
    }

    /// A height class's collective bitmap (6.5.9), which the decoder allocates before it reads.
    fn collective(&mut self, first: usize, height: u32, total: u32) -> Result<(), PdfError> {
        let [_, _, size_table] = self.tables;
        let bits = &mut self.values.bits;
        let length = match size_table.read(bits) {
            Some(Value::Number(number)) => u32::try_from(number).ok(),
            Some(Value::OutOfBand) | None => None,
        }
        .ok_or_else(|| self.meter.malformed())?;
        bits.align();
        if length != 0 && bits.skip(u64::from(length)).is_none() {
            return Err(self.meter.malformed());
        }
        let area = u64::from(total).saturating_mul(u64::from(height));
        if area > u64::from(SIDE).saturating_mul(u64::from(SIDE)) {
            return Err(self.meter.malformed());
        }
        let changes = u64::from(total).saturating_mul(24);
        let collective = words(total, height).saturating_add(changes);
        self.meter.hold(self.held.saturating_add(collective))?;
        let rows = u64::from(height).saturating_mul(8);
        self.meter
            .spend(area.saturating_mul(2).saturating_add(rows))?;
        let stored = u64::from(total.div_ceil(8)).saturating_mul(u64::from(height));
        if length == 0 && bits.skip(stored).is_none() {
            return Err(self.meter.malformed());
        }
        for size in self.sizes.iter().skip(first) {
            if size.width > SIDE || size.height > SIDE {
                return Err(self.meter.malformed());
            }
            self.held = self.held.saturating_add(words(size.width, size.height));
            let cut = u64::from(size.width >> 5_u32).saturating_add(1);
            self.meter
                .spend(cut.saturating_mul(u64::from(size.height)))?;
        }
        self.meter.hold(self.held.saturating_add(collective))
    }

    /// The symbols exported (6.5.10): clones of `inputs` and of the new ones.
    fn exported(mut self, table: &Table, inputs: &[&Symbols]) -> Result<Symbols, PdfError> {
        let before: usize = inputs.iter().map(|input| input.sizes.len()).sum();
        let count = before.saturating_add(self.sizes.len());
        let mut every = inputs
            .iter()
            .flat_map(|input| input.sizes.iter())
            .chain(self.sizes.iter())
            .copied();
        let mut found = Vec::new();
        let (mut index, mut idle) = (0_usize, 0_u32);
        let (mut export, mut copies, mut draw) = (false, 0_u64, 0_u64);
        while index < count {
            self.meter.spend(STEP)?;
            let length = match self.values.next(table, &mut self.exports) {
                Some(Value::Number(number)) => usize::try_from(number).ok(),
                Some(Value::OutOfBand) | None => None,
            }
            .filter(|found_run| *found_run <= count.saturating_sub(index))
            .ok_or_else(|| self.meter.malformed())?;
            idle = if length == 0 {
                idle.saturating_add(1)
            } else {
                0
            };
            if idle > IDLE {
                return Err(self.meter.refused());
            }
            self.meter.spend(codec::wide(length))?;
            for size in every.by_ref().take(length) {
                if export {
                    let bytes = words(size.width, size.height);
                    copies = copies.saturating_add(bytes).saturating_add(48);
                    // The decoder clones before it compares with the count declared.
                    let moving = copies.saturating_mul(2);
                    self.meter.hold(self.held.saturating_add(moving))?;
                    let across = u64::from(size.width >> 5_u32).saturating_add(2);
                    draw = draw.max(across.saturating_mul(u64::from(size.height)));
                    found.push(size);
                }
            }
            index = index.saturating_add(length);
            export = !export;
        }
        if codec::wide(found.len()) != u64::from(self.header.exported) {
            return Err(self.meter.malformed());
        }
        self.meter.spend(copies >> 2_u32)?;
        let retains = self.header.has(0x200);
        let state = if retains { DICTIONARY } else { STEP };
        self.meter.keep(copies.saturating_add(state))?;
        Ok(Symbols {
            contexts: retains.then(|| Retained {
                contexts: self.contexts,
                refining: self.header.has(0x1000),
                template: self.header.template,
            }),
            draw,
            sizes: found,
        })
    }

    /// One arithmetically coded bitmap (6.2.5): what matters is where it leaves the decoder.
    fn symbol(&mut self, size: Size) -> Result<(), PdfError> {
        let Size { height, width } = size;
        if width > SIDE || height > SIDE {
            return Err(self.meter.malformed());
        }
        self.held = self.held.saturating_add(words(width, height));
        self.meter.hold(self.held)?;
        let pixels = u64::from(width).saturating_mul(u64::from(height));
        // The decoder's turn and this one's.
        let units = pixels.saturating_mul(16).saturating_add(u64::from(height));
        self.meter.spend(units)?;
        if pixels == 0 {
            return Ok(());
        }
        let deepest = self.header.adaptive.iter().map(|found| found.up).max();
        let depth = deepest.unwrap_or_default().max(2).saturating_add(1);
        let counted = |count: u32| usize::try_from(count).map_err(|_error| self.meter.refused());
        let (columns, rows) = (counted(width)?, counted(height)?);
        let kept = depth.next_power_of_two().min(rows.next_power_of_two());
        let length = columns.saturating_mul(kept);
        let window = codec::wide(length.max(self.window.len()));
        self.meter.hold(self.held.saturating_add(window))?;
        if self.window.len() < length {
            self.window = codec::filled(self.meter.job, codec::wide(length), 0)?;
        }
        generic(self, [columns, rows], kept.saturating_sub(1));
        Ok(())
    }
}

impl Standard {
    /// The tables; none only if they were mistyped.
    pub(super) fn new() -> Option<Self> {
        Some(Self {
            b1: Table::build(B1)?,
            b2: Table::build(B2)?,
            b3: Table::build(B3)?,
            b4: Table::build(B4)?,
            b5: Table::build(B5)?,
        })
    }
}

impl Table {
    /// The tree of the codes B.3 gives `lines`.
    fn build(lines: &[Line]) -> Option<Self> {
        let longest = lines.iter().map(|entry| entry.prefix).max().unwrap_or(0);
        let mut nodes = vec![Node::default()];
        let mut code = Vec::with_capacity(usize::from(longest));
        let mut spent = false;
        for length in 1..=longest {
            if !spent {
                code.push(false);
            }
            for entry in lines.iter().filter(|entry| entry.prefix == length) {
                if spent {
                    return None;
                }
                insert(&mut nodes, &code, *entry)?;
                match code.iter().rposition(|bit| !*bit) {
                    Some(last) => {
                        for bit in code.iter_mut().skip(last) {
                            *bit = !*bit;
                        }
                    }
                    None => spent = true,
                }
            }
        }
        Some(Self { nodes })
    }

    /// The table of a code table segment (B.2).
    pub(super) fn custom(meter: &mut Meter<'_, '_>, data: &[u8]) -> Result<Self, PdfError> {
        let mut bits = Bits { at: 0, data };
        let lines = lines(&mut bits).ok_or_else(|| meter.malformed())?;
        if lines.len() > LINES {
            return Err(meter.refused());
        }
        let count = codec::wide(lines.len());
        meter.spend(count.saturating_mul(STEP))?;
        let table = Self::build(&lines).ok_or_else(|| meter.malformed())?;
        // The decoder's lines and nodes, in lists that double.
        let nodes = codec::wide(table.nodes.len()).saturating_mul(72);
        meter.keep(
            count
                .saturating_mul(48)
                .saturating_add(nodes)
                .saturating_add(2048),
        )?;
        Ok(table)
    }

    /// The next value (B.4).
    fn read(&self, bits: &mut Bits<'_>) -> Option<Value> {
        let mut node = self.nodes.first()?;
        loop {
            if let Some(found) = node.line {
                let sign = match found.kind {
                    LineKind::OutOfBand => return Some(Value::OutOfBand),
                    LineKind::Lower => -1_i64,
                    LineKind::Upper => 1_i64,
                };
                let offset = i64::from(bits.read(found.range)?).saturating_mul(sign);
                let value = found.low.saturating_add(offset);
                return i32::try_from(value).ok().map(Value::Number);
            }
            let child = node.children.get(usize::from(bits.bit()?)).copied()?;
            if child == 0 {
                return None;
            }
            node = self.nodes.get(child)?;
        }
    }
}

impl<'data> Values<'data> {
    /// The values of `data`.
    fn new(data: &'data [u8], huffman: bool) -> Self {
        Self {
            arithmetic: Arithmetic::new(data),
            bits: Bits { at: 0, data },
            huffman,
        }
    }

    /// The next value, by `table` or by `integer`.
    fn next(&mut self, table: &Table, integer: &mut Integer) -> Option<Value> {
        if self.huffman {
            table.read(&mut self.bits)
        } else {
            Some(integer.read(&mut self.arithmetic))
        }
    }
}

/// Scans the dictionary in `data`; `inputs` and `tables` are in the order of its references
/// (7.4.2.2).
pub(super) fn dictionary(
    meter: &mut Meter<'_, '_>,
    standard: &Standard,
    data: &[u8],
    inputs: &[&Symbols],
    tables: &[&Table],
) -> Result<Symbols, PdfError> {
    let mut reader = Reader::new(data);
    let flags = reader.short().ok_or_else(|| meter.malformed())?;
    if flags & 2 != 0 {
        // SDREFAGG: following it needs the rest of a decoder.
        return Err(meter.unsupported());
    }
    let header = Header::read(flags, &mut reader).ok_or_else(|| meter.malformed())?;
    let chosen = selected(flags, standard, tables).ok_or_else(|| meter.malformed())?;
    let before: u64 = inputs
        .iter()
        .map(|input| codec::wide(input.sizes.len()))
        .sum();
    if before > u64::from(SYMBOLS) {
        return Err(meter.malformed());
    }
    let (fresh, exported) = (u64::from(header.fresh), u64::from(header.exported));
    // A bitmap is 48 bytes, a width 4, an input 8 in a list that doubles.
    let held = DICTIONARY
        .saturating_add(fresh.saturating_mul(52))
        .saturating_add(exported.saturating_mul(48))
        .saturating_add(before.saturating_mul(24))
        .saturating_add(codec::wide(tables.len()).saturating_mul(24));
    meter.hold(held)?;
    let counts = fresh.saturating_add(exported).saturating_add(before);
    meter.spend(counts.saturating_add(0x1_0000))?;
    let count = codec::wide(header.contexts());
    let mut contexts = codec::filled(meter.job, count, 0)?;
    if header.has(0x100) {
        // Those of the last dictionary referred to, whose templates must be the same.
        let retained = inputs
            .last()
            .and_then(|input| input.contexts.as_ref())
            .filter(|kept| kept.template == header.template && kept.refining == header.has(0x1000))
            .ok_or_else(|| meter.malformed())?;
        contexts.clone_from(&retained.contexts);
    }
    let mut scan = Scan {
        contexts,
        exports: Integer::new(),
        values: Values::new(header.body, header.has(1)),
        header,
        held,
        heights: Integer::new(),
        meter,
        sizes: Vec::new(),
        tables: chosen,
        widths: Integer::new(),
        window: Vec::new(),
    };
    scan.classes()?;
    scan.exported(&standard.b1, inputs)
}

/// The generic procedure without typical prediction over `size`, keeping the rows `mask` selects.
fn generic(scan: &mut Scan<'_, '_, '_, '_>, size: [usize; 2], mask: usize) {
    let [width, height] = size;
    let template = scan.header.template;
    let runs = TEMPLATES.get(template).copied().unwrap_or_default();
    let count = if template == 0 { 4 } else { 1 };
    let adaptive = scan.header.adaptive.get(..count).unwrap_or_default();
    let window = scan.window.as_mut_slice();
    for row in 0..height {
        let start = |up: usize| {
            row.checked_sub(up)
                .map(|above| (above & mask).wrapping_mul(width))
        };
        let mut states = [0_u32; 3];
        for (run, state) in runs.iter().zip(states.iter_mut()) {
            // The pixels right of the first that are its template's.
            for column in 0..usize::try_from(run.to).unwrap_or(0) {
                let seen = pixel(window, start(run.up), Some(column), width);
                *state = state.wrapping_shl(1) | seen;
            }
        }
        let current = (row & mask).wrapping_mul(width);
        let mut above = [None; 3];
        for (slot, run) in above.iter_mut().zip(runs) {
            *slot = start(run.up);
        }
        let mut above_adaptive = [None; 4];
        for (slot, found) in above_adaptive.iter_mut().zip(adaptive) {
            *slot = start(found.up);
        }
        for column in 0..width {
            let mut index = 0_u32;
            for ((run, state), base) in runs.iter().zip(states.iter_mut()).zip(above) {
                let place = column.checked_add_signed(run.to);
                let seen = pixel(window, base, place, width);
                let kept = 1_u32.wrapping_shl(run.bits).wrapping_sub(1);
                *state = (state.wrapping_shl(1) | seen) & kept;
                index = index.wrapping_shl(run.bits) | *state;
            }
            for (found, base) in adaptive.iter().zip(above_adaptive) {
                let place = column.checked_add_signed(found.across);
                index = index.wrapping_shl(1) | pixel(window, base, place, width);
            }
            let context = usize::try_from(index).unwrap_or(usize::MAX);
            let bit = scan
                .contexts
                .get_mut(context)
                .is_some_and(|state| scan.values.arithmetic.decode(state));
            if let Some(out) = window.get_mut(current.wrapping_add(column)) {
                *out = u8::from(bit);
            }
        }
    }
}

/// Puts `line` at the end of `code`; none where it meets another code.
fn insert(nodes: &mut Vec<Node>, code: &[bool], found: Line) -> Option<()> {
    let mut at = 0_usize;
    for bit in code {
        let node = *nodes.get(at)?;
        if node.line.is_some() {
            return None;
        }
        let side = usize::from(*bit);
        let mut child = node.children.get(side).copied()?;
        if child == 0 {
            child = nodes.len();
            nodes.push(Node::default());
            *nodes.get_mut(at)?.children.get_mut(side)? = child;
        }
        at = child;
    }
    nodes.get_mut(at)?.line = Some(found);
    Some(())
}

/// A line counting up from `low`.
const fn line(prefix: u8, range: u8, low: i64) -> Line {
    Line {
        kind: LineKind::Upper,
        low,
        prefix,
        range,
    }
}

/// The lines of a code table (B.2), at most one past those allowed.
fn lines(bits: &mut Bits<'_>) -> Option<Vec<Line>> {
    let flags = bits.read(8)?;
    let width = |shift: u32| u8::try_from((flags.wrapping_shr(shift) & 7).saturating_add(1)).ok();
    let (prefixes, ranges) = (width(1)?, width(4)?);
    let lowest = bits.read(32)?.cast_signed();
    let highest = bits.read(32)?.cast_signed();
    let mut found = Vec::new();
    let mut next = lowest;
    while next < highest {
        let prefix = u8::try_from(bits.read(prefixes)?).ok()?;
        let range = u8::try_from(bits.read(ranges)?).ok()?;
        found.push(line(prefix, range, i64::from(next)));
        if found.len() > LINES {
            return Some(found);
        }
        let covered = 1_i64.checked_shl(u32::from(range))?;
        next = i32::try_from(i64::from(next).checked_add(covered)?).ok()?;
    }
    let below = u8::try_from(bits.read(prefixes)?).ok()?;
    found.push(lower(below, i64::from(lowest).saturating_sub(1)));
    let above = u8::try_from(bits.read(prefixes)?).ok()?;
    found.push(line(above, 32, i64::from(highest)));
    if flags & 1 != 0 {
        found.push(out_of_band(u8::try_from(bits.read(prefixes)?).ok()?));
    }
    Some(found)
}

/// The line counting down from `low`.
const fn lower(prefix: u8, low: i64) -> Line {
    Line {
        kind: LineKind::Lower,
        low,
        prefix,
        range: 32,
    }
}

/// The line that is out of band.
const fn out_of_band(prefix: u8) -> Line {
    Line {
        kind: LineKind::OutOfBand,
        low: 0,
        prefix,
        range: 0,
    }
}

/// The pixel in the row at `start`; 0 outside the bitmap.
fn pixel(window: &[u8], start: Option<usize>, column: Option<usize>, width: usize) -> u32 {
    let (Some(base), Some(place)) = (start, column) else {
        return 0;
    };
    if place >= width {
        return 0;
    }
    u32::from(window.get(base.wrapping_add(place)).copied().unwrap_or(0))
}

/// A run of `bits` pixels in the row `up` above, the rightmost at `to`.
const fn run(up: usize, bits: u32, to: isize) -> Run {
    Run { bits, to, up }
}

/// The tables of heights, widths and bitmap sizes (7.4.2.1.6). The decoder wants those chosen even
/// where nothing is Huffman coded.
fn selected<'scan>(
    flags: u16,
    standard: &'scan Standard,
    tables: &[&'scan Table],
) -> Option<[&'scan Table; 3]> {
    let mut custom = tables.iter().copied();
    let height = match (flags >> 2_u32) & 3 {
        0 => &standard.b4,
        1 => &standard.b5,
        3 => custom.next()?,
        _ => return None,
    };
    let width = match (flags >> 4_u32) & 3 {
        0 => &standard.b2,
        1 => &standard.b3,
        3 => custom.next()?,
        _ => return None,
    };
    let size = if flags & 0x40 == 0 {
        &standard.b1
    } else {
        custom.next()?
    };
    if flags & 0x80 != 0 {
        // SDHUFFAGGINST: unused without aggregation, but wanted.
        let _unused: &Table = custom.next()?;
    }
    Some([height, width, size])
}

/// `value` moved by a decoded delta.
fn stepped(value: u32, delta: Option<Value>) -> Option<u32> {
    match delta? {
        Value::Number(number) => value.checked_add_signed(number),
        Value::OutOfBand => None,
    }
}

/// Meters a text region (7.4.3) that draws the symbols of `inputs`.
pub(super) fn text(
    meter: &mut Meter<'_, '_>,
    data: &[u8],
    inputs: &[&Symbols],
) -> Result<(), PdfError> {
    let mut reader = Reader::new(data);
    let flags = reader
        .take(17)
        .and_then(|_region| reader.short())
        .ok_or_else(|| meter.malformed())?;
    if flags & 2 != 0 {
        // SBREFINE: refined sizes are coded in the region.
        return Err(meter.unsupported());
    }
    let huffman = flags & 1 != 0;
    if huffman && reader.short().is_none() {
        return Err(meter.malformed());
    }
    let instances = u64::from(reader.long().ok_or_else(|| meter.malformed())?);
    let count: u64 = inputs
        .iter()
        .map(|input| codec::wide(input.sizes.len()))
        .sum();
    let most = codec::wide(data.len()).saturating_mul(32);
    if instances > most || count > u64::from(SYMBOLS) {
        return Err(meter.malformed());
    }
    let draw = inputs.iter().map(|input| input.draw).max().unwrap_or(0);
    let each = draw.saturating_mul(4).saturating_add(512);
    let mut work = instances.saturating_mul(each).saturating_add(count);
    let mut bytes = count.saturating_mul(24).saturating_add(0x1_0000);
    if huffman {
        // The identifiers' lines and nodes.
        bytes = bytes
            .saturating_add(count.saturating_mul(192))
            .saturating_add(0x8000);
        work = work.saturating_add(count.saturating_mul(64));
    } else {
        let contexts = count.max(1).next_power_of_two();
        bytes = bytes.saturating_add(contexts.saturating_mul(2));
        work = work.saturating_add(contexts);
    }
    meter.hold(bytes)?;
    meter.spend(work)
}

/// The bytes of a bitmap in the decoder's words.
pub(super) fn words(width: u32, height: u32) -> u64 {
    u64::from(width.div_ceil(32))
        .saturating_mul(u64::from(height))
        .saturating_mul(4)
}
