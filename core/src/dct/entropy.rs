//! Entropy-coded data: Huffman codes, restarts, sequential and progressive blocks (ITU-T T.81,
//! Annexes C, F and G).

use alloc::rc::Rc;

use super::{Component, Input, ZIGZAG};
use crate::PdfError;

/// Bits that index the table of short codes.
const FAST: u32 = 9;

/// The longest code (C.2).
const LONGEST: u32 = 16;

/// The last coefficient of a block.
const LAST: usize = 63;

/// How a scan codes its blocks.
#[derive(Clone, Copy)]
pub(super) enum Coding {
    /// Bands of AC coefficients, high bits first (G.1.2.2).
    AcFirst {
        /// The first and the last coefficient.
        band: [usize; 2],
        /// The point transform.
        low: u32,
    },
    /// The next bit of a band (G.1.2.3).
    AcRefine {
        /// The first and the last coefficient.
        band: [usize; 2],
        /// The point transform.
        low: u32,
    },
    /// DC coefficients, high bits first (G.1.2.1).
    DcFirst(u32),
    /// The next bit of the DC coefficients.
    DcRefine(u32),
    /// Whole blocks (F.2.2).
    Sequential,
}

/// A component of a scan.
pub(super) struct Part {
    /// The AC code.
    ac: Option<Rc<Table>>,
    /// The frame's component.
    component: usize,
    /// The DC code.
    dc: Option<Rc<Table>>,
    /// The DC prediction.
    predictor: i32,
}

/// One scan, read an MCU row at a time.
pub(super) struct Scan<'data> {
    /// The data.
    bits: Bits<'data>,
    /// The coding.
    coding: Coding,
    /// The restart interval, 0 for none.
    interval: u32,
    /// MCUs left before the next restart.
    left: u32,
    /// The components.
    parts: Vec<Part>,
    /// The blocks left of an end-of-band run.
    run: u32,
}

/// A Huffman code (Annex C).
pub(super) struct Table {
    /// Symbol and length of each short code by the bits that begin it, 0 for none.
    fast: [u16; 512],
    /// Per length, the end of its codes aligned to 16 bits.
    limits: [u32; 16],
    /// Per length, what turns a code into the index of its symbol.
    offsets: [u32; 16],
    /// The symbols by code.
    symbols: Vec<u8>,
}

/// The bits of entropy-coded data, up to a marker.
struct Bits<'data> {
    /// Unread bits in `held`.
    count: u32,
    /// The bits, newest lowest.
    held: u32,
    /// The data.
    input: Input<'data>,
    /// The marker that ended the bits.
    marker: Option<u8>,
    /// The bits have ended.
    stopped: bool,
}

impl Bits<'_> {
    /// Reads ahead until 25 bits are held or the bits end.
    fn fill(&mut self) -> Result<(), PdfError> {
        while self.count <= 24 && !self.stopped {
            let byte = match self.input.byte()? {
                Some(0xFF) => match self.input.code()? {
                    Some(0) => 0xFF,
                    found => {
                        self.marker = found;
                        self.stopped = true;
                        break;
                    }
                },
                Some(byte) => byte,
                None => {
                    self.stopped = true;
                    break;
                }
            };
            self.held = self.held.wrapping_shl(8) | u32::from(byte);
            self.count = self.count.wrapping_add(8);
        }
        Ok(())
    }

    /// The next `count` bits, at most 16.
    fn receive(&mut self, count: u32) -> Result<u32, PdfError> {
        if self.count < count {
            self.fill()?;
        }
        self.count = self
            .count
            .checked_sub(count)
            .ok_or_else(|| self.input.malformed())?;
        let mask = !u32::MAX.wrapping_shl(count);
        Ok(self.held.wrapping_shr(self.count) & mask)
    }

    /// Passes the restart marker that is due (E.2.4).
    fn restart(&mut self) -> Result<(), PdfError> {
        let found = match self.marker.take() {
            Some(marker) => Some(marker),
            None => self.input.marker()?,
        };
        if !matches!(found, Some(0xD0..=0xD7)) {
            return Err(self.input.malformed());
        }
        self.count = 0;
        self.stopped = false;
        Ok(())
    }

    /// The next symbol of `table` (F.2.2.3).
    fn symbol(&mut self, table: &Table) -> Result<u32, PdfError> {
        if self.count < LONGEST {
            self.fill()?;
        }
        let ahead = if self.count >= LONGEST {
            self.held.wrapping_shr(self.count.wrapping_sub(LONGEST))
        } else {
            self.held.wrapping_shl(LONGEST.wrapping_sub(self.count))
        } & 0xFFFF;
        let (length, symbol) = table.lookup(ahead).ok_or_else(|| self.input.malformed())?;
        self.count = self
            .count
            .checked_sub(length)
            .ok_or_else(|| self.input.malformed())?;
        Ok(symbol)
    }

    /// The value of `size` further bits (F.2.2.1).
    fn value(&mut self, size: u32) -> Result<i32, PdfError> {
        if size == 0 {
            return Ok(0);
        }
        if size > LONGEST {
            return Err(self.input.malformed());
        }
        let bits = i32::try_from(self.receive(size)?).unwrap_or_default();
        let full = 1_i32.wrapping_shl(size);
        Ok(if bits < full.wrapping_shr(1) {
            bits.wrapping_sub(full).wrapping_add(1)
        } else {
            bits
        })
    }
}

impl Part {
    /// The part of `component`, with its codes.
    pub(super) const fn new(
        component: usize,
        dc: Option<Rc<Table>>,
        ac: Option<Rc<Table>>,
    ) -> Self {
        Self {
            ac,
            component,
            dc,
            predictor: 0,
        }
    }
}

impl<'data> Scan<'data> {
    /// Whether the image stays whole, only coarser, without this scan.
    pub(super) const fn is_detail(&self) -> bool {
        !matches!(self.coding, Coding::DcFirst(_) | Coding::Sequential)
    }

    /// A scan of `parts` whose data `input` gives next.
    pub(super) const fn new(
        input: Input<'data>,
        parts: Vec<Part>,
        coding: Coding,
        interval: u32,
    ) -> Self {
        Self {
            bits: Bits {
                count: 0,
                held: 0,
                input,
                marker: None,
                stopped: false,
            },
            coding,
            interval,
            left: interval,
            parts,
            run: 0,
        }
    }

    /// Counts an MCU, after the restart due before it.
    fn next_unit(&mut self) -> Result<(), PdfError> {
        if self.interval == 0 {
            return Ok(());
        }
        if self.left == 0 {
            self.bits.restart()?;
            for part in &mut self.parts {
                part.predictor = 0_i32;
            }
            self.run = 0;
            self.left = self.interval;
        }
        self.left = self.left.saturating_sub(1);
        Ok(())
    }

    /// Where its data stands.
    pub(super) const fn offset(&self) -> u64 {
        self.bits.input.offset
    }

    /// Adds MCU row `row`, `units` across, to the components' coefficients (A.2).
    pub(super) fn row(
        &mut self,
        components: &mut [Component],
        row: usize,
        units: usize,
    ) -> Result<(), PdfError> {
        if self.parts.len() > 1 {
            for unit in 0..units {
                self.next_unit()?;
                for part in &mut self.parts {
                    let Some(component) = components.get_mut(part.component) else {
                        continue;
                    };
                    let left = unit.saturating_mul(component.horizontal);
                    for down in 0..component.vertical {
                        let start = down.saturating_mul(component.stride).saturating_add(left);
                        for across in 0..component.horizontal {
                            if let Some(block) =
                                component.blocks.get_mut(start.saturating_add(across))
                            {
                                decode(&mut self.bits, part, self.coding, &mut self.run, block)?;
                            }
                        }
                    }
                }
            }
            return Ok(());
        }
        // One component: an MCU is one block, and only the component's own blocks are coded.
        let only = self.parts.first().map(|part| part.component);
        let Some(component) = only.and_then(|index| components.get_mut(index)) else {
            return Ok(());
        };
        let top = row.saturating_mul(component.vertical);
        let count = component.lines.saturating_sub(top).min(component.vertical);
        for down in 0..count {
            let start = down.saturating_mul(component.stride);
            for across in 0..component.across {
                self.next_unit()?;
                if let Some(part) = self.parts.first_mut()
                    && let Some(block) = component.blocks.get_mut(start.saturating_add(across))
                {
                    decode(&mut self.bits, part, self.coding, &mut self.run, block)?;
                }
            }
        }
        Ok(())
    }
}

impl Table {
    /// The length and symbol of the code that begins `ahead`, 16 bits.
    fn lookup(&self, ahead: u32) -> Option<(u32, u32)> {
        let short = usize::try_from(ahead.wrapping_shr(LONGEST.wrapping_sub(FAST))).ok()?;
        let entry = u32::from(*self.fast.get(short)?);
        if entry != 0 {
            return Some((entry.wrapping_shr(8), entry & 0xFF));
        }
        let found = self.limits.iter().position(|limit| ahead < *limit)?;
        let length = u32::try_from(found).ok()?.wrapping_add(1);
        let index = ahead
            .wrapping_shr(LONGEST.wrapping_sub(length))
            .wrapping_add(*self.offsets.get(found)?);
        let symbol = *self.symbols.get(usize::try_from(index).ok()?)?;
        Some((length, u32::from(symbol)))
    }

    /// The code with `counts` codes of each length for `symbols` (C.2), or `None` where it holds
    /// more codes than fit.
    pub(super) fn new(counts: &[u8; 16], symbols: &[u8]) -> Option<Self> {
        let mut table = Self {
            fast: [0; 512],
            limits: [0; 16],
            offsets: [0; 16],
            symbols: symbols.to_vec(),
        };
        let mut code = 0_u32;
        let mut rest = symbols.iter();
        let mut index = 0_u32;
        let ends = table.limits.iter_mut().zip(&mut table.offsets);
        for ((length, count), (limit, offset)) in (1..=LONGEST).zip(counts).zip(ends) {
            *offset = index.wrapping_sub(code);
            for symbol in rest.by_ref().take(usize::from(*count)) {
                if code >= 1_u32.wrapping_shl(length) {
                    return None;
                }
                if let Some(spare) = FAST.checked_sub(length) {
                    let start = usize::try_from(code.wrapping_shl(spare)).ok()?;
                    let end = start.saturating_add(1_usize.wrapping_shl(spare));
                    let entry = u16::try_from(length.wrapping_shl(8) | u32::from(*symbol)).ok()?;
                    table.fast.get_mut(start..end)?.fill(entry);
                }
                code = code.wrapping_add(1);
                index = index.wrapping_add(1);
            }
            *limit = code.wrapping_shl(LONGEST.wrapping_sub(length));
            code = code.wrapping_shl(1);
        }
        Some(table)
    }
}

/// AC coefficients of a band, high bits first, and runs of bands without any.
fn ac_first(
    bits: &mut Bits<'_>,
    table: &Table,
    ([first, last], low): ([usize; 2], u32),
    run: &mut u32,
    block: &mut [i16; 64],
) -> Result<(), PdfError> {
    if *run > 0 {
        *run = run.saturating_sub(1);
        return Ok(());
    }
    let mut at = first;
    while at <= last {
        let (zeros, size) = split(bits.symbol(table)?);
        if size == 0 && zeros < 15 {
            *run = run_length(bits, zeros)?;
            break;
        }
        at = at.saturating_add(usize::try_from(zeros).unwrap_or_default());
        if size > 0 {
            if at > last {
                return Err(bits.input.malformed());
            }
            let value = bits.value(size)?;
            store(block, at, value.wrapping_shl(low));
        }
        at = at.saturating_add(1);
    }
    Ok(())
}

/// One more bit of a band's coefficients.
fn ac_refine(
    bits: &mut Bits<'_>,
    table: &Table,
    ([first, last], low): ([usize; 2], u32),
    run: &mut u32,
    block: &mut [i16; 64],
) -> Result<(), PdfError> {
    let step = 1_i32.wrapping_shl(low);
    let mut at = first;
    if *run == 0 {
        while at <= last {
            let (mut zeros, size) = split(bits.symbol(table)?);
            let added = match size {
                0 if zeros < 15 => {
                    // This block is the first of the run: its rest is corrected below.
                    *run = run_length(bits, zeros)?.saturating_add(1);
                    break;
                }
                0 => 0_i32,
                1 if bits.receive(1)? == 1 => step,
                1 => step.wrapping_neg(),
                _ => return Err(bits.input.malformed()),
            };
            while at <= last {
                let Some(slot) = coefficient(block, at) else {
                    break;
                };
                if *slot != 0 {
                    correct(bits, slot, step)?;
                } else if zeros == 0 {
                    break;
                } else {
                    zeros = zeros.saturating_sub(1);
                }
                at = at.saturating_add(1);
            }
            if added != 0_i32 {
                if at > last {
                    return Err(bits.input.malformed());
                }
                store(block, at, added);
            }
            at = at.saturating_add(1);
        }
    }
    if *run > 0 {
        for position in at..=last {
            if let Some(slot) = coefficient(block, position)
                && *slot != 0
            {
                correct(bits, slot, step)?;
            }
        }
        *run = run.saturating_sub(1);
    }
    Ok(())
}

/// The slot of the coefficient at `at` in zig-zag order.
fn coefficient(block: &mut [i16; 64], at: usize) -> Option<&mut i16> {
    block.get_mut(usize::from(*ZIGZAG.get(at)?))
}

/// Adds the bit `step` to a coefficient that is not zero, away from zero.
fn correct(bits: &mut Bits<'_>, slot: &mut i16, step: i32) -> Result<(), PdfError> {
    if bits.receive(1)? == 1 {
        let value = i32::from(*slot);
        if value & step == 0_i32 {
            *slot = narrow(value.wrapping_add(if value > 0_i32 {
                step
            } else {
                step.wrapping_neg()
            }));
        }
    }
    Ok(())
}

/// The DC coefficient, predicted from the one before it.
fn dc_first(
    bits: &mut Bits<'_>,
    part: &mut Part,
    low: u32,
    block: &mut [i16; 64],
) -> Result<(), PdfError> {
    let table = part.dc.as_deref().ok_or_else(|| bits.input.malformed())?;
    let size = bits.symbol(table)?;
    part.predictor = part.predictor.wrapping_add(bits.value(size)?);
    store(block, 0, part.predictor.wrapping_shl(low));
    Ok(())
}

/// Decodes one block as `coding` says.
fn decode(
    bits: &mut Bits<'_>,
    part: &mut Part,
    coding: Coding,
    run: &mut u32,
    block: &mut [i16; 64],
) -> Result<(), PdfError> {
    match coding {
        Coding::AcFirst { band, low } => {
            let table = part.ac.as_deref().ok_or_else(|| bits.input.malformed())?;
            ac_first(bits, table, (band, low), run, block)
        }
        Coding::AcRefine { band, low } => {
            let table = part.ac.as_deref().ok_or_else(|| bits.input.malformed())?;
            ac_refine(bits, table, (band, low), run, block)
        }
        Coding::DcFirst(low) => dc_first(bits, part, low, block),
        Coding::DcRefine(low) => {
            if bits.receive(1)? == 1
                && let Some(slot) = block.first_mut()
            {
                *slot |= narrow(1_i32.wrapping_shl(low));
            }
            Ok(())
        }
        Coding::Sequential => sequential(bits, part, block),
    }
}

/// A coefficient held in 16 bits.
fn narrow(value: i32) -> i16 {
    i16::try_from(value.clamp(i32::from(i16::MIN), i32::from(i16::MAX))).unwrap_or_default()
}

/// The blocks after this one that an end-of-band run covers.
fn run_length(bits: &mut Bits<'_>, size: u32) -> Result<u32, PdfError> {
    let more = bits.receive(size)?;
    Ok(1_u32
        .wrapping_shl(size)
        .wrapping_add(more)
        .saturating_sub(1))
}

/// A whole block.
fn sequential(bits: &mut Bits<'_>, part: &mut Part, block: &mut [i16; 64]) -> Result<(), PdfError> {
    dc_first(bits, part, 0, block)?;
    let table = part.ac.as_deref().ok_or_else(|| bits.input.malformed())?;
    let mut at = 1_usize;
    while at <= LAST {
        let (zeros, size) = split(bits.symbol(table)?);
        if size == 0 && zeros < 15 {
            break;
        }
        at = at.saturating_add(usize::try_from(zeros).unwrap_or_default());
        if size > 0 {
            if at > LAST {
                return Err(bits.input.malformed());
            }
            let value = bits.value(size)?;
            store(block, at, value);
        }
        at = at.saturating_add(1);
    }
    Ok(())
}

/// A symbol's run and size.
const fn split(symbol: u32) -> (u32, u32) {
    (symbol.wrapping_shr(4), symbol & 0xF)
}

/// Writes the coefficient at `at` in zig-zag order.
fn store(block: &mut [i16; 64], at: usize, value: i32) {
    if let Some(slot) = coefficient(block, at) {
        *slot = narrow(value);
    }
}
