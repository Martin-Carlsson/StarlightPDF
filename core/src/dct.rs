//! `DCTDecode`: baseline, extended and progressive JPEG, a row at a time (ISO 32000-2:2020, 7.4.8;
//! ITU-T T.81).

mod entropy;
mod idct;

use alloc::rc::Rc;

use crate::pull::{Buffered, Pull, number};
use crate::render::room;
use crate::{ErrorCode, PdfError};
use entropy::{Coding, Part, Scan, Table};

/// Every coefficient of a block.
const ALL: u64 = u64::MAX;

/// The side of a block.
const BLOCK: usize = 8;

/// What one scan holds: its state, and the buffers and window of the data it reads.
const SCAN_HELD: u64 = 0xA000;

/// The most scans of one image.
const SCANS: usize = 64;

/// Bytes skipped at a time.
const STEP: usize = 4096;

/// The most symbols of one code (B.2.4.2).
const SYMBOLS: usize = 256;

/// The most code tables of one image: each costs far more than its bytes to set up.
const TABLES: u32 = 512;

/// The natural position of each coefficient in zig-zag order (Figure A.6).
pub(crate) const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// What a frame header declares.
#[derive(Clone, Copy)]
pub(crate) struct Frame {
    /// Components.
    pub components: u8,
    /// Rows.
    pub height: u32,
    /// Columns.
    pub width: u32,
}

/// What a decoder may take.
pub(crate) struct Bounds<'bounds> {
    /// The error past a cap of its own: of tables or scans.
    pub capped: ErrorCode,
    /// Fails unless buffers of so many bytes fit beside all that is held.
    pub fits: &'bounds dyn Fn(u64) -> Result<(), PdfError>,
    /// The most bytes of the image decoded.
    pub limit: u64,
    /// What the stages before this one hold for each scan's reader of its data.
    pub stages: u64,
}

/// Opens the data from its start.
pub(crate) type Open<'open, 'data> = dyn Fn() -> Result<Box<dyn Pull + 'data>, PdfError> + 'open;

/// A decoder that gives one row at a time.
pub(crate) struct Decoder<'data> {
    /// The components.
    components: Vec<Component>,
    /// Damage that cost detail only; it fails the last row.
    damage: Option<PdfError>,
    /// The frame.
    frame: Frame,
    /// The next row.
    given: u32,
    /// The bytes held.
    held: u64,
    /// The next row within the MCU row.
    line: usize,
    /// The MCU rows decoded.
    rows: usize,
    /// The largest sampling factors, across and down.
    sampling: [usize; 2],
    /// The scans, less those damaged.
    scans: Vec<Option<Scan<'data>>>,
    /// Whether the colour transform applies.
    transformed: bool,
    /// The MCUs across.
    units: usize,
}

/// One component's place in the frame and its blocks of the current MCU row.
struct Component {
    /// Blocks across that hold samples.
    across: usize,
    /// The coefficients, in natural order.
    blocks: Vec<[i16; 64]>,
    /// The coefficients the scans read so far have completed.
    complete: u64,
    /// The sampling factor across.
    horizontal: usize,
    /// The identifier.
    id: u8,
    /// Block rows that hold samples.
    lines: usize,
    /// The quantisers, in natural order, as they were at the first scan.
    quant: Option<[u16; 64]>,
    /// The block row `samples` holds.
    ready: Option<usize>,
    /// The samples of one block row.
    samples: Vec<u8>,
    /// The quantisation table's number.
    selector: usize,
    /// Blocks from one row of blocks to the next.
    stride: usize,
    /// The sampling factor down.
    vertical: usize,
}

/// The scans found and the damage that ended the search.
type Indexed<'data> = (Vec<Option<Scan<'data>>>, Option<PdfError>);

/// What the markers have set so far (B.2).
struct Header {
    /// The codes for AC, by number.
    ac: [Option<Rc<Table>>; 4],
    /// The Adobe marker's transform.
    adobe: Option<u8>,
    /// The error past a cap of its own.
    capped: ErrorCode,
    /// The frame's components.
    components: Vec<Component>,
    /// The codes for DC, by number.
    dc: [Option<Rc<Table>>; 4],
    /// The frame.
    frame: Option<Frame>,
    /// The restart interval.
    interval: u32,
    /// The operation errors name.
    operation: &'static str,
    /// Whether the frame is progressive.
    progressive: bool,
    /// The quantisers, by number.
    quant: [Option<[u16; 64]>; 4],
    /// The code tables set up.
    tables: u32,
}

/// The data of one reader and where it stands.
struct Input<'data> {
    /// The data.
    data: Buffered<Box<dyn Pull + 'data>>,
    /// The offset of the next byte.
    offset: u64,
    /// The operation errors name.
    operation: &'static str,
}

impl Component {
    /// The component of a frame's three bytes, or `None` where they are out of range.
    fn new([id, sampling, selector]: [u8; 3], alone: bool) -> Option<Self> {
        let horizontal = usize::from(sampling.wrapping_shr(4));
        let vertical = usize::from(sampling & 0xF);
        if !(1..=4).contains(&horizontal) || !(1..=4).contains(&vertical) || selector > 3 {
            return None;
        }
        Some(Self {
            across: 0,
            blocks: Vec::new(),
            complete: 0,
            // A lone component's factors are void (A.2.2).
            horizontal: if alone { 1 } else { horizontal },
            id,
            lines: 0,
            quant: None,
            ready: None,
            samples: Vec::new(),
            selector: usize::from(selector),
            stride: 0,
            vertical: if alone { 1 } else { vertical },
        })
    }

    /// Writes line `line` of the MCU row to every `step`th byte of `out`.
    fn write(
        &mut self,
        line: usize,
        [most_across, most_down]: [usize; 2],
        (slots, step): (&mut [u8], usize),
    ) {
        // The sample whose centre is nearest (A.1.1).
        let within = line
            .saturating_mul(2)
            .saturating_add(1)
            .saturating_mul(self.vertical)
            .checked_div(most_down.saturating_mul(2))
            .unwrap_or_default();
        let row = within.wrapping_shr(3);
        let width = self.across.saturating_mul(BLOCK);
        if width == 0 {
            return;
        }
        if self.ready != Some(row)
            && let Some(quant) = self.quant.as_ref()
        {
            let start = row.saturating_mul(self.stride);
            let blocks = self.blocks.iter().skip(start).take(self.across);
            for (index, block) in blocks.enumerate() {
                let place = (width, index.saturating_mul(BLOCK));
                idct::inverse(block, quant, &mut self.samples, place);
            }
            self.ready = Some(row);
        }
        let first = (within & 7).saturating_mul(width);
        let source = self
            .samples
            .get(first..first.saturating_add(width))
            .unwrap_or_default();
        idct::upsampled(source, slots, step, (self.horizontal, most_across));
    }
}

impl<'data> Decoder<'data> {
    /// Decodes the next MCU row's coefficients.
    fn advance(&mut self) -> Result<(), PdfError> {
        for component in &mut self.components {
            component.blocks.fill([0; 64]);
            component.ready = None;
        }
        for slot in &mut self.scans {
            let Some(scan) = slot.as_mut() else {
                continue;
            };
            match scan.row(&mut self.components, self.rows, self.units) {
                Ok(()) => {}
                Err(error) if error.code() == ErrorCode::Malformed && scan.is_detail() => {
                    *slot = None;
                    if self.damage.is_none() {
                        self.damage = Some(error);
                    }
                }
                Err(error) => return Err(error),
            }
        }
        self.rows = self.rows.saturating_add(1);
        self.line = 0;
        Ok(())
    }

    /// The frame.
    pub(crate) const fn frame(&self) -> Frame {
        self.frame
    }

    /// The bytes it holds.
    pub(crate) const fn held(&self) -> u64 {
        self.held
    }

    /// A decoder of the data `open` gives, `transform` being `ColorTransform`, within `bounds`.
    pub(crate) fn new(
        open: &Open<'_, 'data>,
        transform: Option<bool>,
        bounds: &Bounds<'_>,
        operation: &'static str,
    ) -> Result<Self, PdfError> {
        let mut input = Input::new(open()?, 0, operation);
        // Junk may come before SOI.
        while input.next_marker()?.ok_or_else(|| input.malformed())? != 0xD8 {}
        let mut header = Header::new(operation, bounds.capped);
        let body = header
            .until_scan(&mut input)?
            .ok_or_else(|| input.malformed())?;
        let mut frame = header.frame.ok_or_else(|| input.malformed())?;
        if frame.height == 0 {
            // DNL gives it after the first scan (B.2.5).
            let start = input.offset;
            frame.height = input.lines()?;
            input = Input::at(open, start, operation)?;
        }
        let decoded = u64::from(frame.width)
            .saturating_mul(u64::from(frame.height))
            .saturating_mul(u64::from(frame.components));
        if decoded > bounds.limit {
            return Err(PdfError::new(ErrorCode::LimitExceeded, operation));
        }
        let (sampling, units) = header.place(frame);
        let buffers = header.allocate(bounds.fits)?;
        let scan = SCAN_HELD.saturating_add(bounds.stages);
        // Each scan reads through a pipeline of its own, open beside the others.
        let held = |count: usize| {
            let scans = u64::try_from(count).unwrap_or(u64::MAX);
            buffers.saturating_add(scans.saturating_mul(scan))
        };
        let (scans, damage) =
            header.scans(open, (input, &body), &|count| (bounds.fits)(held(count)))?;
        if header.components.iter().any(|one| one.quant.is_none()) {
            return Err(damage.unwrap_or_else(|| PdfError::new(ErrorCode::Malformed, operation)));
        }
        Ok(Self {
            damage,
            frame,
            given: 0,
            held: held(scans.len()),
            line: 0,
            rows: 0,
            sampling,
            scans,
            transformed: header.transformed(transform, frame.components),
            units,
            components: header.components,
        })
    }

    /// The columns of samples each scan walks a row, padded to whole blocks, of every component.
    pub(crate) fn padded(&self) -> u64 {
        let [across, _] = self.sampling;
        let columns = self
            .units
            .saturating_mul(across)
            .saturating_mul(BLOCK)
            .saturating_mul(self.components.len());
        u64::try_from(columns).unwrap_or(u64::MAX)
    }

    /// The bytes its scans stand at, together; as they read, it grows by the bytes read.
    pub(crate) fn read(&self) -> u64 {
        self.scans
            .iter()
            .flatten()
            .fold(0, |total, scan| total.saturating_add(scan.offset()))
    }

    /// The next row's samples, interleaved, into `out`; unless `convert` the row is only passed
    /// over. `false` at the end.
    pub(crate) fn row(&mut self, out: &mut [u8], convert: bool) -> Result<bool, PdfError> {
        if self.given >= self.frame.height {
            return Ok(false);
        }
        let [_, down] = self.sampling;
        if (self.rows == 0 || self.line >= down.saturating_mul(BLOCK))
            && let Err(error) = self.advance()
        {
            self.given = self.frame.height;
            return Err(error);
        }
        if convert {
            let count = self.components.len();
            for (index, component) in self.components.iter_mut().enumerate() {
                let slots = out.get_mut(index..).unwrap_or_default();
                component.write(self.line, self.sampling, (slots, count));
            }
            if self.transformed {
                idct::coloured(out, count);
            }
        }
        self.line = self.line.saturating_add(1);
        self.given = self.given.saturating_add(1);
        if self.given == self.frame.height
            && let Some(error) = self.damage.take()
        {
            return Err(error);
        }
        Ok(true)
    }

    /// The scans every MCU row reads.
    pub(crate) fn scanned(&self) -> u64 {
        u64::try_from(self.scans.len()).unwrap_or(u64::MAX)
    }
}

impl Header {
    /// Allocates each component's buffers, once they `fits`; returns their bytes.
    fn allocate(&mut self, fits: &dyn Fn(u64) -> Result<(), PdfError>) -> Result<u64, PdfError> {
        let mut bytes = 0_u64;
        for component in &self.components {
            let blocks = component.stride.saturating_mul(component.vertical);
            let coefficients = blocks.saturating_mul(size_of::<[i16; 64]>());
            let samples = component.across.saturating_mul(BLOCK.saturating_mul(BLOCK));
            let sum = coefficients.saturating_add(samples);
            bytes = bytes.saturating_add(u64::try_from(sum).unwrap_or(u64::MAX));
        }
        fits(bytes)?;
        for component in &mut self.components {
            let blocks = component.stride.saturating_mul(component.vertical);
            component.blocks = filled(blocks, [0; 64])?;
            let samples = component.across.saturating_mul(BLOCK.saturating_mul(BLOCK));
            component.samples = filled(samples, 0)?;
        }
        Ok(bytes)
    }

    /// Takes a DHT segment's codes (B.2.4.2).
    fn codes(&mut self, body: &[u8]) -> Result<(), PdfError> {
        let mut rest = body;
        while let Some((kind, after)) = rest.split_first() {
            let (counts, values) = after
                .split_first_chunk::<16>()
                .ok_or_else(|| self.malformed())?;
            let total = counts.iter().map(|count| usize::from(*count)).sum();
            let (symbols, remainder) = values
                .split_at_checked(total)
                .filter(|_| total <= SYMBOLS)
                .ok_or_else(|| self.malformed())?;
            self.tables = self.tables.saturating_add(1);
            if self.tables > TABLES {
                return Err(self.error(self.capped));
            }
            let table = Table::new(counts, symbols).ok_or_else(|| self.malformed())?;
            let place = match kind.wrapping_shr(4) {
                0 => self.dc.get_mut(usize::from(*kind & 0xF)),
                1 => self.ac.get_mut(usize::from(*kind & 0xF)),
                _ => None,
            };
            match place {
                Some(slot) => *slot = Some(Rc::new(table)),
                None => return Err(self.malformed()),
            }
            rest = remainder;
        }
        Ok(())
    }

    /// An error of `code`.
    fn error(&self, code: ErrorCode) -> PdfError {
        PdfError::new(code, self.operation)
    }

    /// The scan after the one whose data starts at `start`, and where its own does.
    fn following<'data>(
        &mut self,
        open: &Open<'_, 'data>,
        start: u64,
    ) -> Result<Option<(Scan<'data>, u64)>, PdfError> {
        let mut input = Input::at(open, start, self.operation)?;
        let Some(body) = self.until_scan(&mut input)? else {
            return Ok(None);
        };
        let offset = input.offset;
        Ok(Some((self.scan(input, &body)?, offset)))
    }

    /// Takes a frame header (B.2.2).
    fn frame(&mut self, marker: u8, body: &[u8]) -> Result<(), PdfError> {
        let (head, specs) = body
            .split_first_chunk::<6>()
            .ok_or_else(|| self.malformed())?;
        let [precision, high, low, wide, narrow, count] = *head;
        if precision == 12 {
            return Err(self.error(ErrorCode::Unsupported));
        }
        let (triples, spare) = specs.as_chunks::<3>();
        let width = number(&[wide, narrow]);
        let expected = (1..=4).contains(&count) && triples.len() == usize::from(count);
        if self.frame.is_some() || precision != 8 || width == 0 || !expected || !spare.is_empty() {
            return Err(self.malformed());
        }
        for triple in triples {
            let component = Component::new(*triple, count == 1).ok_or_else(|| self.malformed())?;
            self.components.push(component);
        }
        self.progressive = marker == 0xC2;
        self.frame = Some(Frame {
            components: count,
            height: number(&[high, low]),
            width,
        });
        Ok(())
    }

    /// An [`ErrorCode::Malformed`] error.
    fn malformed(&self) -> PdfError {
        self.error(ErrorCode::Malformed)
    }

    /// Nothing set yet.
    const fn new(operation: &'static str, capped: ErrorCode) -> Self {
        Self {
            ac: [None, None, None, None],
            adobe: None,
            capped,
            components: Vec::new(),
            dc: [None, None, None, None],
            frame: None,
            interval: 0,
            operation,
            progressive: false,
            quant: [None; 4],
            tables: 0,
        }
    }

    /// Sizes each component for `frame` (A.1.1, A.2.4); returns the largest factors and the MCUs
    /// across.
    fn place(&mut self, frame: Frame) -> ([usize; 2], usize) {
        let mut across = 1_usize;
        let mut down = 1_usize;
        for component in &self.components {
            across = across.max(component.horizontal);
            down = down.max(component.vertical);
        }
        let width = usize::try_from(frame.width).unwrap_or(usize::MAX);
        let height = usize::try_from(frame.height).unwrap_or(usize::MAX);
        let units = width.div_ceil(across.saturating_mul(BLOCK));
        for component in &mut self.components {
            component.across = width
                .saturating_mul(component.horizontal)
                .div_ceil(across)
                .div_ceil(BLOCK);
            component.lines = height
                .saturating_mul(component.vertical)
                .div_ceil(down)
                .div_ceil(BLOCK);
            component.stride = units.saturating_mul(component.horizontal);
        }
        ([across, down], units)
    }

    /// Takes a DQT segment's tables (B.2.4.1).
    fn quantisers(&mut self, body: &[u8]) -> Result<(), PdfError> {
        let mut rest = body;
        while let Some((kind, after)) = rest.split_first() {
            let size = match kind.wrapping_shr(4) {
                0 => 1_usize,
                1 => 2_usize,
                _ => return Err(self.malformed()),
            };
            let (values, remainder) = after
                .split_at_checked(size.saturating_mul(ZIGZAG.len()))
                .ok_or_else(|| self.malformed())?;
            let mut table = [0_u16; 64];
            for (value, at) in values.chunks_exact(size).zip(ZIGZAG) {
                if let Some(slot) = table.get_mut(usize::from(at)) {
                    *slot = u16::try_from(number(value)).unwrap_or_default();
                }
            }
            match self.quant.get_mut(usize::from(*kind & 0xF)) {
                Some(slot) => *slot = Some(table),
                None => return Err(self.malformed()),
            }
            rest = remainder;
        }
        Ok(())
    }

    /// The scan a scan header begins (B.2.3); notes what it completes.
    fn scan<'data>(&mut self, input: Input<'data>, body: &[u8]) -> Result<Scan<'data>, PdfError> {
        let (count, rest) = body.split_first().ok_or_else(|| self.malformed())?;
        let size = usize::from(*count).saturating_mul(2);
        let (selected, tail) = rest
            .split_at_checked(size)
            .filter(|_| (1..=4).contains(count))
            .ok_or_else(|| self.malformed())?;
        let [start, end, approximation] =
            *tail.first_chunk::<3>().ok_or_else(|| self.malformed())?;
        let high = approximation.wrapping_shr(4);
        let low = approximation & 0xF;
        let band = [usize::from(start), usize::from(end)];
        let shift = u32::from(low);
        let coding = match (start, end, high) {
            _ if !self.progressive => Coding::Sequential,
            _ if low > 13 => return Err(self.malformed()),
            (0, 0, 0) => Coding::DcFirst(shift),
            (0, 0, _) => Coding::DcRefine(shift),
            (1..=63, 1..=63, _) if start <= end && *count == 1 => {
                if high == 0 {
                    Coding::AcFirst { band, low: shift }
                } else {
                    Coding::AcRefine { band, low: shift }
                }
            }
            _ => return Err(self.malformed()),
        };
        let covered = if !self.progressive {
            ALL
        } else if low == 0 {
            ALL.wrapping_shr(63_u32.saturating_sub(u32::from(end)))
                & ALL.wrapping_shl(u32::from(start))
        } else {
            0
        };
        let first = matches!(coding, Coding::DcFirst(_) | Coding::Sequential);
        let mut parts = Vec::new();
        for pair in selected.as_chunks::<2>().0 {
            let [id, codes] = *pair;
            let found = self
                .components
                .iter_mut()
                .enumerate()
                .find(|found| found.1.id == id);
            let Some((index, component)) = found else {
                return Err(self.malformed());
            };
            component.complete |= covered;
            if first && component.quant.is_none() {
                component.quant = self.quant.get(component.selector).copied().flatten();
                if component.quant.is_none() {
                    return Err(self.malformed());
                }
            }
            parts.push(Part::new(
                index,
                self.dc
                    .get(usize::from(codes.wrapping_shr(4)))
                    .cloned()
                    .flatten(),
                self.ac.get(usize::from(codes & 0xF)).cloned().flatten(),
            ));
        }
        Ok(Scan::new(input, parts, coding, self.interval))
    }

    /// A reader of each scan, from the one `body` heads at `input` on, and the damage that ended
    /// the search; each opens once `fits` passes for that many.
    fn scans<'data>(
        &mut self,
        open: &Open<'_, 'data>,
        (input, body): (Input<'data>, &[u8]),
        fits: &dyn Fn(usize) -> Result<(), PdfError>,
    ) -> Result<Indexed<'data>, PdfError> {
        let mut start = input.offset;
        fits(1)?;
        let mut scans = vec![Some(self.scan(input, body)?)];
        while self.components.iter().any(|one| one.complete != ALL) {
            fits(scans.len().saturating_add(1))?;
            match self.following(open, start) {
                Ok(Some(_)) if scans.len() >= SCANS => {
                    return Err(self.error(self.capped));
                }
                Ok(Some((scan, after))) => {
                    scans.push(Some(scan));
                    start = after;
                }
                Ok(None) => break,
                Err(error) if error.code() == ErrorCode::Malformed => {
                    return Ok((scans, Some(error)));
                }
                Err(error) => return Err(error),
            }
        }
        Ok((scans, None))
    }

    /// Whether the colour transform applies: the Adobe marker outranks `ColorTransform`.
    fn transformed(&self, parameter: Option<bool>, components: u8) -> bool {
        self.adobe
            .map(|value| value != 0)
            .or(parameter)
            .unwrap_or(components == 3)
    }

    /// Takes the segments up to a scan header and returns its body; `None` at EOI.
    fn until_scan(&mut self, input: &mut Input<'_>) -> Result<Option<Vec<u8>>, PdfError> {
        loop {
            let marker = input.next_marker()?.ok_or_else(|| input.malformed())?;
            match marker {
                0xC0..=0xC2 => self.frame(marker, &input.segment()?)?,
                // Lossless, hierarchical and arithmetic coding.
                0xC3 | 0xC5..=0xCB | 0xCD..=0xCF => return Err(self.error(ErrorCode::Unsupported)),
                0xC4 => self.codes(&input.segment()?)?,
                0xD8 => return Err(self.malformed()),
                0xD9 => return Ok(None),
                0xDA => return input.segment().map(Some),
                0xDB => self.quantisers(&input.segment()?)?,
                0xDD => self.interval = number(&input.segment()?),
                0xEE => {
                    let body = input.segment()?;
                    if let Some(marked) = body.strip_prefix(b"Adobe")
                        && self.adobe.is_none()
                    {
                        self.adobe = marked.get(6).copied();
                    }
                }
                _ => input.pass()?,
            }
        }
    }
}

impl<'data> Input<'data> {
    /// A reader at `start` of the data `open` gives.
    fn at(open: &Open<'_, 'data>, start: u64, operation: &'static str) -> Result<Self, PdfError> {
        let mut data = open()?;
        let mut buffer = [0; STEP];
        let mut left = start;
        while left > 0 {
            let wanted = usize::try_from(left).unwrap_or(STEP).min(STEP);
            let count = data.pull(buffer.get_mut(..wanted).unwrap_or_default())?;
            if count == 0 {
                return Err(PdfError::new(ErrorCode::Malformed, operation));
            }
            left = left.saturating_sub(u64::try_from(count).unwrap_or(u64::MAX));
        }
        Ok(Self::new(data, start, operation))
    }

    /// The next byte.
    fn byte(&mut self) -> Result<Option<u8>, PdfError> {
        let byte = self.data.byte()?;
        if byte.is_some() {
            self.offset = self.offset.wrapping_add(1);
        }
        Ok(byte)
    }

    /// The code after an FF and any fill bytes (B.1.1.2).
    fn code(&mut self) -> Result<Option<u8>, PdfError> {
        loop {
            match self.byte()? {
                Some(0xFF) => {}
                found => return Ok(found),
            }
        }
    }

    /// The rows the DNL segment after this scan declares.
    fn lines(&mut self) -> Result<u32, PdfError> {
        if self.next_marker()? != Some(0xDC) {
            return Err(self.malformed());
        }
        let lines = number(&self.segment()?);
        if (1..=0xFFFF).contains(&lines) {
            Ok(lines)
        } else {
            Err(self.malformed())
        }
    }

    /// An [`ErrorCode::Malformed`] error.
    fn malformed(&self) -> PdfError {
        PdfError::new(ErrorCode::Malformed, self.operation)
    }

    /// The next marker, whatever comes before it.
    fn marker(&mut self) -> Result<Option<u8>, PdfError> {
        loop {
            match self.byte()? {
                Some(0xFF) => match self.code()? {
                    Some(0) => {}
                    found => return Ok(found),
                },
                Some(_) => {}
                None => return Ok(None),
            }
        }
    }

    /// A reader of `data`, which stands at `offset`.
    fn new(data: Box<dyn Pull + 'data>, offset: u64, operation: &'static str) -> Self {
        Self {
            data: Buffered::new(data),
            offset,
            operation,
        }
    }

    /// The next marker that is neither TEM nor a restart.
    fn next_marker(&mut self) -> Result<Option<u8>, PdfError> {
        loop {
            match self.marker()? {
                Some(0x01 | 0xD0..=0xD7) => {}
                found => return Ok(found),
            }
        }
    }

    /// Passes over a segment.
    fn pass(&mut self) -> Result<(), PdfError> {
        for _ in 0..self.size()? {
            if self.byte()?.is_none() {
                return Err(self.malformed());
            }
        }
        Ok(())
    }

    /// A segment's body.
    fn segment(&mut self) -> Result<Vec<u8>, PdfError> {
        let size = self.size()?;
        let mut body = Vec::new();
        for _ in 0..size {
            body.push(self.byte()?.ok_or_else(|| self.malformed())?);
        }
        Ok(body)
    }

    /// A segment's length after its length field.
    fn size(&mut self) -> Result<u32, PdfError> {
        let high = self.byte()?.ok_or_else(|| self.malformed())?;
        let low = self.byte()?.ok_or_else(|| self.malformed())?;
        number(&[high, low])
            .checked_sub(2)
            .ok_or_else(|| self.malformed())
    }
}

/// `count` copies of `value`.
fn filled<T>(count: usize, value: T) -> Result<Vec<T>, PdfError>
where
    T: Clone,
{
    let mut values = room(count)?;
    values.resize(count, value);
    Ok(values)
}
