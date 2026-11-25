//! Samples, what they decode to, and the colour or share they give (ISO 32000-2:2020, 8.9.3,
//! 8.9.5.2, 8.9.6, 11.6.5.3).

use alloc::rc::Rc;

use crate::PdfError;
use crate::render::colour::{Converter, Intent, Space};
use crate::render::fixed::{Fixed, quotient, saturated};
use crate::render::function::room;
use crate::render::job::Job;

/// Work of a converter's tables.
const CONVERTER: usize = 4096;

/// A share of one.
const FULL: u16 = u16::MAX;

/// The most bytes of a row, which is held whole: 1.4 million pixels of RGB.
const ROW: u64 = 0x40_0000;

/// A texel that changes nothing.
pub(super) const WHOLE: Cell = Cell {
    color: [0; 3],
    opacity: FULL,
    shape: FULL,
};

/// One texel.
#[derive(Clone, Copy)]
pub(super) struct Cell {
    /// The colour.
    pub color: [u16; 3],
    /// The opacity.
    pub opacity: u16,
    /// The shape.
    pub shape: u16,
}

/// How samples become colours.
pub(super) struct Colouring {
    /// The channel that is no colour.
    pub alpha: Option<usize>,
    /// The intent.
    pub intent: Intent,
    /// The ranges of the colour key; none: no key.
    pub key: Vec<[i64; 2]>,
    /// What the colours are preblended with; none: nothing.
    pub matte: Vec<Fixed>,
    /// Whether that channel is the opacity.
    pub opacity: bool,
    /// The space.
    pub space: Rc<Space>,
    /// Whether the samples are YCC of sRGB.
    pub ycc: bool,
}

/// Converts the texels of one image.
struct Colours {
    /// The converter.
    converter: Converter,
    /// How.
    how: Colouring,
    /// The last colour.
    known: Option<[u16; 3]>,
    /// The samples of that.
    last: Vec<u16>,
    /// The samples of the texel.
    raw: Vec<u16>,
    /// The colours of one component's samples, as they are met; empty: not kept.
    table: Vec<Option<[u16; 3]>>,
    /// The values of the texel.
    values: Vec<Fixed>,
}

/// The map from samples to values (8.9.5.2).
pub(super) struct Decode {
    /// The largest sample.
    most: u16,
    /// What 0 and the largest go to, per component.
    ranges: Vec<[Fixed; 2]>,
    /// Every value, a component after the other; empty past 8 bits.
    table: Vec<Fixed>,
    /// The values of a component in the table.
    width: usize,
}

/// How samples lie in rows (8.9.3).
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct Layout {
    /// Bits per sample.
    pub bits: u32,
    /// Columns.
    pub columns: u32,
    /// Samples per pixel.
    pub components: usize,
    /// Rows.
    pub height: u32,
    /// Bytes per row.
    pub stride: usize,
}

/// What the rows of one image mean.
pub(super) struct Samples {
    /// The colours; none for a mask. Boxed, as the stack is small on WebAssembly.
    colours: Option<Box<Colours>>,
    /// The map to values.
    decode: Decode,
    /// The layout.
    layout: Layout,
    /// The texels of each sample value, unveiled, as they are met; empty: not yet kept.
    memo: Vec<Option<Cell>>,
    /// A mask's values are opacity, not the share painted less.
    soft: bool,
}

impl Colours {
    /// The texel at `column` of `row` under `veil`.
    fn cell(
        &mut self,
        job: &Job<'_>,
        (layout, decode): (&Layout, &Decode),
        (row, column): (&[u8], u32),
        veil: Cell,
    ) -> Result<Cell, PdfError> {
        let skipped = self.how.alpha.unwrap_or(usize::MAX);
        for (index, slot) in self.raw.iter_mut().enumerate() {
            let component = index.saturating_add(usize::from(index >= skipped));
            *slot = layout.raw(row, column, component);
        }
        let mut cell = veil;
        if self.how.opacity
            && let Some(channel) = self.how.alpha
        {
            let factor = FULL.checked_div(decode.most).unwrap_or(1);
            cell.opacity = layout.raw(row, column, channel).saturating_mul(factor);
        }
        let keyed = !self.how.key.is_empty()
            && self
                .raw
                .iter()
                .zip(&self.how.key)
                .all(|(&value, &[low, high])| (low..=high).contains(&i64::from(value)));
        if keyed {
            cell.shape = 0;
        }
        if cell.shape == 0 || cell.opacity == 0 {
            cell.color = [0; 3];
        } else {
            cell.color = self.color(job, decode, cell.opacity)?;
        }
        Ok(cell)
    }

    /// The colour of the samples read, of opacity `alpha`.
    fn color(&mut self, job: &Job<'_>, decode: &Decode, alpha: u16) -> Result<[u16; 3], PdfError> {
        if self.how.ycc {
            full_colour(&mut self.raw);
        }
        let blended = alpha < FULL && !self.how.matte.is_empty();
        let first = usize::from(self.raw.first().copied().unwrap_or(0));
        if !blended {
            if let Some(found) = self.table.get(first).copied().flatten() {
                return Ok(found);
            }
            if let Some(found) = self.known
                && self.last == self.raw
            {
                return Ok(found);
            }
        }
        for (index, (value, &sample)) in self.values.iter_mut().zip(&self.raw).enumerate() {
            *value = decode.value(index, sample);
        }
        if blended {
            for (value, &matte) in self.values.iter_mut().zip(&self.how.matte) {
                *value = unblended(*value, matte, alpha);
            }
        }
        let color = self.converter.rgb(job, &self.values)?;
        if !blended {
            if let Some(slot) = self.table.get_mut(first) {
                *slot = Some(color);
            }
            self.known = Some(color);
            self.last.copy_from_slice(&self.raw);
        }
        Ok(color)
    }

    /// Whether a texel's colour is its one sample's alone.
    const fn plain(&self) -> bool {
        self.raw.len() == 1
            && self.how.alpha.is_none()
            && self.how.key.is_empty()
            && self.how.matte.is_empty()
            && !self.how.ycc
    }
}

impl Decode {
    /// The low end of each component.
    pub(super) fn lows(&self) -> Vec<Fixed> {
        self.ranges.iter().map(|&[low, _]| low).collect()
    }

    /// The map of samples of `bits` to `ranges`, one per component.
    pub(super) fn new(bits: u32, ranges: Vec<[Fixed; 2]>) -> Result<Self, PdfError> {
        let most = largest(bits);
        let width = if bits > 8 {
            0
        } else {
            usize::from(most).saturating_add(1)
        };
        let mut table = room(width.saturating_mul(ranges.len()))?;
        if width > 0 {
            for &range in &ranges {
                table.extend((0..=most).map(|sample| between(range, sample, most)));
            }
        }
        Ok(Self {
            most,
            ranges,
            table,
            width,
        })
    }

    /// The value of `sample` of `component`.
    fn value(&self, component: usize, sample: u16) -> Fixed {
        let at = component
            .saturating_mul(self.width)
            .saturating_add(usize::from(sample));
        if let Some(found) = self.table.get(at).filter(|_| self.width > 0) {
            return *found;
        }
        let range = self.ranges.get(component).copied().unwrap_or_default();
        between(range, sample, self.most)
    }
}

impl Layout {
    /// The layout of an image of `size`, checked against the decoded limit.
    pub(super) fn new(
        job: &Job<'_>,
        size: [u32; 2],
        components: usize,
        bits: u32,
    ) -> Result<Self, PdfError> {
        let [columns, height] = size;
        if columns == 0 || height == 0 || components == 0 || !matches!(bits, 1 | 2 | 4 | 8 | 16) {
            return Err(job.objects.malformed());
        }
        let stride = u64::from(columns)
            .saturating_mul(u64::try_from(components).unwrap_or(u64::MAX))
            .saturating_mul(u64::from(bits))
            .div_ceil(8);
        job.reserve(stride.saturating_mul(u64::from(height)))?;
        if stride > ROW || i32::try_from(columns.max(height)).is_err() {
            return Err(job.capped());
        }
        Ok(Self {
            bits,
            columns,
            components,
            height,
            stride: usize::try_from(stride).map_err(|_error| job.limit())?,
        })
    }

    /// The texels that `bytes` of a row hold whole.
    fn present(&self, bytes: usize) -> u32 {
        if bytes >= self.stride {
            return self.columns;
        }
        let each = u64::try_from(self.components)
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::from(self.bits));
        u64::try_from(bytes)
            .unwrap_or(u64::MAX)
            .saturating_mul(8)
            .checked_div(each)
            .and_then(|count| u32::try_from(count).ok())
            .map_or(self.columns, |count| count.min(self.columns))
    }

    /// The sample of `component` at `column` of `row`, high bit first.
    fn raw(&self, row: &[u8], column: u32, component: usize) -> u16 {
        let index = usize::try_from(column)
            .unwrap_or(usize::MAX)
            .saturating_mul(self.components)
            .saturating_add(component);
        let byte = |at: usize| u16::from(row.get(at).copied().unwrap_or(0));
        match self.bits {
            8 => byte(index),
            16 => {
                let start = index.saturating_mul(2);
                byte(start).wrapping_shl(8) | byte(start.saturating_add(1))
            }
            bits => {
                let place = u64::try_from(index)
                    .unwrap_or(0)
                    .saturating_mul(u64::from(bits));
                let within = u32::try_from(place & 7).unwrap_or(0);
                let held = byte(usize::try_from(place >> 3_u32).unwrap_or(usize::MAX));
                held.wrapping_shr(8_u32.saturating_sub(bits).saturating_sub(within)) & largest(bits)
            }
        }
    }

    /// The bytes of all its rows.
    pub(super) fn whole(&self) -> u64 {
        u64::try_from(self.stride)
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::from(self.height))
    }
}

impl Samples {
    /// The texels `from` to `to` of `row`, unveiled, as how many there are of each cell, when
    /// they are single bits of one plain sample; `None` otherwise.
    pub(super) fn bits(
        &mut self,
        job: &Job<'_>,
        row: &[u8],
        (from, to): (u32, u32),
    ) -> Result<Option<[(Cell, u64); 2]>, PdfError> {
        let plain = self.colours.as_ref().is_none_or(|colours| colours.plain());
        if self.layout.bits != 1 || self.layout.components != 1 || !plain {
            return Ok(None);
        }
        let end = to.min(self.layout.present(row.len()));
        let ones = ones(row, (from, end));
        let zeros = u64::from(end.saturating_sub(from)).saturating_sub(ones);
        let (Some(dark), Some(light)) = (
            self.unveiled(job, (&[0], 0))?,
            self.unveiled(job, (&[0x80], 0))?,
        ) else {
            return Ok(None);
        };
        Ok(Some([(dark, zeros), (light, ones)]))
    }

    /// The texel at `column` of `row`, its data as far as it came, under `veil`; `None` past it.
    pub(super) fn cell(
        &mut self,
        job: &Job<'_>,
        (row, column): (&[u8], u32),
        veil: Cell,
    ) -> Result<Option<Cell>, PdfError> {
        let layout = &self.layout;
        if column >= layout.present(row.len()) {
            return Ok(None);
        }
        if let Some(colours) = self.colours.as_mut() {
            let found = colours.cell(job, (layout, &self.decode), (row, column), veil)?;
            return Ok(Some(found));
        }
        let value = self.decode.value(0, layout.raw(row, column, 0)).unit();
        let mut cell = veil;
        if self.soft {
            cell.opacity = value;
        } else {
            cell.shape = FULL.saturating_sub(value);
        }
        Ok(Some(cell))
    }

    /// The samples of an image in colour.
    pub(super) fn colour(
        job: &Job<'_>,
        layout: Layout,
        decode: Decode,
        how: Colouring,
    ) -> Result<Self, PdfError> {
        let count = how.space.components();
        // The tables of one draw and a converter's, all to be filled.
        let tables = decode.table.len().saturating_add(CONVERTER);
        job.spend(u64::try_from(tables).unwrap_or(u64::MAX))?;
        let mut table = Vec::new();
        if count == 1 {
            table = room(decode.width)?;
            table.resize(decode.width, None);
        }
        let colours = Colours {
            converter: Converter::new(job, &how.space, how.intent)?,
            how,
            known: None,
            last: vec![0; count],
            raw: vec![0; count],
            table,
            values: vec![Fixed::default(); count],
        };
        Ok(Self {
            colours: Some(Box::new(colours)),
            decode,
            layout,
            memo: Vec::new(),
            soft: false,
        })
    }

    /// The layout.
    pub(super) const fn layout(&self) -> Layout {
        self.layout
    }

    /// The samples of a mask, `soft` or one that cuts out.
    pub(super) const fn share(layout: Layout, decode: Decode, soft: bool) -> Self {
        Self {
            colours: None,
            decode,
            layout,
            memo: Vec::new(),
            soft,
        }
    }

    /// The texel at `column` of `row`, unveiled; those of one sample of up to 8 bits are kept by
    /// value.
    pub(super) fn unveiled(
        &mut self,
        job: &Job<'_>,
        (row, column): (&[u8], u32),
    ) -> Result<Option<Cell>, PdfError> {
        let plain =
            self.layout.bits <= 8 && self.colours.as_ref().is_none_or(|colours| colours.plain());
        if !plain || column >= self.layout.present(row.len()) {
            return self.cell(job, (row, column), WHOLE);
        }
        if self.memo.is_empty() {
            let size = 1_usize.wrapping_shl(self.layout.bits);
            self.memo = room(size)?;
            self.memo.resize(size, None);
        }
        let raw = usize::from(self.layout.raw(row, column, 0));
        if let Some(found) = self.memo.get(raw).copied().flatten() {
            return Ok(Some(found));
        }
        let found = self.cell(job, (row, column), WHOLE)?;
        if let (Some(slot), Some(cell)) = (self.memo.get_mut(raw), found) {
            *slot = Some(cell);
        }
        Ok(found)
    }

    /// The work of a texel's components past the first few, each read and keyed.
    fn wide(&self) -> u64 {
        let components = self
            .colours
            .as_ref()
            .map_or(1, |colours| colours.how.space.components());
        u64::try_from(components >> 2_u32).unwrap_or(u64::MAX)
    }

    /// The work of reading a texel to average: more for several components, which convert.
    pub(super) fn work(&self) -> u64 {
        let several = self
            .colours
            .as_ref()
            .is_some_and(|colours| colours.how.space.components() > 1);
        1_u64
            .saturating_add(u64::from(several))
            .saturating_add(self.wide())
    }
}

/// Where `sample`, of up to `most`, lies in `range`, to the nearest.
fn between(range: [Fixed; 2], sample: u16, most: u16) -> Fixed {
    let [low, high] = range;
    let (span, whole) = (high.minus(low).bits(), i64::from(most));
    let near = span
        .checked_mul(i64::from(sample))
        .and_then(|product| product.checked_add(whole >> 1_u32))
        .and_then(|product| product.checked_div_euclid(whole));
    let way = near.unwrap_or_else(|| {
        let product = i128::from(span).saturating_mul(i128::from(sample));
        saturated(quotient(product, i128::from(most)).unwrap_or(0))
    });
    low.plus(Fixed::from_bits(way))
}

/// YCC samples of 8 bits as RGB (ITU-T T.871).
fn full_colour(raw: &mut [u16]) {
    let Some(pixel) = raw.first_chunk_mut::<3>() else {
        return;
    };
    let [luma, blue, red] = pixel.map(i32::from);
    let (blue_off, red_off) = (blue.saturating_sub(128), red.saturating_sub(128));
    let level = luma.saturating_mul(0x1_0000).saturating_add(0x8000);
    *pixel = [
        level.saturating_add(red_off.saturating_mul(91_881)),
        level
            .saturating_sub(blue_off.saturating_mul(22_554))
            .saturating_sub(red_off.saturating_mul(46_802)),
        level.saturating_add(blue_off.saturating_mul(116_130)),
    ]
    .map(|value| u16::try_from((value >> 16_u32).clamp(0_i32, 255_i32)).unwrap_or(0));
}

/// The largest sample of `bits`.
pub(super) const fn largest(bits: u32) -> u16 {
    FULL.wrapping_shr(16_u32.saturating_sub(bits))
}

/// The bits set from bit `from` to bit `to` of `row`, high bit first.
fn ones(row: &[u8], (from, to): (u32, u32)) -> u64 {
    let mut count = 0_u64;
    let mut at = from;
    while at < to {
        let byte =
            usize::try_from(at >> 3_u32).map_or(0, |index| row.get(index).copied().unwrap_or(0));
        let start = at & 7;
        let stop = to.saturating_sub(at).saturating_add(start).min(8);
        let high = 0xFF_u32 >> start;
        let low = 0xFF_u32.wrapping_shl(8_u32.saturating_sub(stop)) & 0xFF;
        count = count.saturating_add(u64::from((u32::from(byte) & high & low).count_ones()));
        at = at.saturating_add(stop.saturating_sub(start));
    }
    count
}

/// `value` with the matte `matte` undone at opacity `alpha` (11.6.5.3).
fn unblended(value: Fixed, matte: Fixed, alpha: u16) -> Fixed {
    let off = i128::from(value.minus(matte).bits()).saturating_mul(i128::from(FULL));
    let way = quotient(off, i128::from(alpha)).map_or(0, saturated);
    matte.plus(Fixed::from_bits(way))
}
