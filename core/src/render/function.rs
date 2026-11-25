//! Functions of types 0, 2, 3 and 4 (ISO 32000-2:2020, 7.10).

mod calculator;
pub(crate) mod elementary;

use alloc::rc::Rc;
use core::slice;

use crate::overlay::Object;
use crate::pull;
use crate::render::OPERATION;
use crate::render::data::{self, Resolved};
use crate::render::fixed::{Fixed, quotient, saturated};
use crate::render::job::{Job, RESOLVE_WORK};
use crate::{ErrorCode, PdfError};

/// How deep stitching nests.
const DEPTH: u8 = 8;

/// The most bytes one function's samples and programs hold.
const HELD: u64 = 0x40_0000;

/// The most numbers of an entry; only `Bounds` and `Encode` of stitching come near it.
const LISTED: usize = 8192;

/// Work of mapping an input or output of a sampled function: a wide division.
const MAPPING_WORK: usize = 3;

/// The most inputs interpolated between; with more the nearest sample is taken.
const MIXED: usize = 8;

/// The most functions one is made of.
const NODES: u32 = 4096;

/// Bytes a function holds besides samples and programs.
const NODE_SIZE: u64 = 256;

/// Work of a power.
const POWER_WORK: u64 = 96;

/// Work of a sample a spline reads and weighs.
const SPLINE_WORK: usize = 2;

/// The most bytes of a calculator's program.
const PROGRAM: u64 = 0x1_0000;

/// The most inputs or outputs.
pub(crate) const VALUES: usize = 32;

/// A calculator function.
struct Calculated {
    /// What it gives when it fails: the low ends of `Range`.
    low: Vec<Fixed>,
    /// The program.
    program: calculator::Program,
}

/// An exponential function (7.10.3).
struct Exponential {
    /// `C0` and the way to `C1`, per output.
    ends: Vec<[Fixed; 2]>,
    /// `N`.
    exponent: Fixed,
}

/// A function, or an array of them.
pub(crate) struct Function {
    /// The bytes it holds.
    bytes: u64,
    /// The bounds of each input.
    domain: Vec<[Fixed; 2]>,
    /// What gives the outputs.
    kind: Box<dyn Evaluate>,
    /// Its outputs.
    outputs: usize,
    /// The bounds of each output; none without `Range`.
    range: Vec<[Fixed; 2]>,
}

/// What reading one function adds up.
struct Reader<'job, 'doc> {
    /// The bytes held.
    held: u64,
    /// The job.
    job: &'job Job<'doc>,
    /// The functions read.
    nodes: u32,
}

/// One input of a sampled function.
struct Axis {
    /// Where its domain maps to.
    encode: [Fixed; 2],
    /// Its samples less one.
    last: Fixed,
    /// The samples from one of its values to the next.
    stride: usize,
}

/// A sampled function (7.10.2).
struct Sampled {
    /// The inputs.
    axes: Vec<Axis>,
    /// The bits of a sample.
    bits: u32,
    /// Whether it interpolates by cubic spline, `Order` 3, not linearly.
    cubic: bool,
    /// The samples.
    data: Vec<u8>,
    /// Where each output's samples map to.
    decode: Vec<[Fixed; 2]>,
}

/// Where an input falls between a sampled function's samples on one axis.
#[derive(Clone, Copy, Default)]
struct Span {
    /// The way from the sample `whole` to the next.
    fraction: Fixed,
    /// The last sample.
    last: i64,
    /// The samples from one of its values to the next.
    stride: usize,
    /// The sample at or before it.
    whole: i64,
}

/// An array of functions of one output each.
struct Several {
    /// The functions.
    parts: Vec<Function>,
}

/// A stitching function (7.10.4).
struct Stitched {
    /// `Bounds`.
    bounds: Vec<Fixed>,
    /// Where each subdomain maps to.
    encode: Vec<[Fixed; 2]>,
    /// The functions stitched.
    parts: Vec<Function>,
}

/// What gives a function's outputs for inputs within its domain.
trait Evaluate {
    /// The outputs for `input`, within `domain`, into `out`, which has their number.
    fn evaluate(
        &self,
        job: &Job<'_>,
        domain: &[[Fixed; 2]],
        input: &[Fixed],
        out: &mut [Fixed],
    ) -> Result<(), PdfError>;
}

impl Evaluate for Calculated {
    fn evaluate(
        &self,
        job: &Job<'_>,
        _domain: &[[Fixed; 2]],
        input: &[Fixed],
        out: &mut [Fixed],
    ) -> Result<(), PdfError> {
        if self.program.run(job, input, out)? {
            return Ok(());
        }
        job.warn("7.10.5", "a calculator function fails")?;
        for (slot, low) in out.iter_mut().zip(&self.low) {
            *slot = *low;
        }
        Ok(())
    }
}

impl Evaluate for Exponential {
    fn evaluate(
        &self,
        job: &Job<'_>,
        _domain: &[[Fixed; 2]],
        input: &[Fixed],
        out: &mut [Fixed],
    ) -> Result<(), PdfError> {
        let value = input.first().copied().unwrap_or_default();
        let part = if self.exponent == Fixed::from_i64(1) {
            job.spend(1)?;
            value
        } else {
            job.spend(POWER_WORK)?;
            if let Some(found) = elementary::power(value, self.exponent) {
                found
            } else {
                job.warn("7.10.3", "an exponential function's Domain allows no power")?;
                Fixed::default()
            }
        };
        for (slot, &[start, way]) in out.iter_mut().zip(&self.ends) {
            *slot = start.plus(part.times(way));
        }
        Ok(())
    }
}

impl Evaluate for Sampled {
    fn evaluate(
        &self,
        job: &Job<'_>,
        domain: &[[Fixed; 2]],
        input: &[Fixed],
        out: &mut [Fixed],
    ) -> Result<(), PdfError> {
        let nought = Fixed::default();
        let mixed = self.axes.len() <= MIXED;
        let mut base = 0_usize;
        let mut between = [(0_usize, nought); MIXED];
        let mut spans = [Span::default(); MIXED];
        let mut dimensions = 0_usize;
        for ((axis, &from), &value) in self.axes.iter().zip(domain).zip(input) {
            let place = interpolate(value, from, axis.encode).clamp(nought, axis.last);
            let nearest = place.plus(Fixed::from_bits(0x8000_0000)).floor();
            let whole = if mixed { place.floor() } else { nearest };
            let step = usize::try_from(whole).unwrap_or(0);
            base = base.saturating_add(step.saturating_mul(axis.stride));
            let fraction = place.minus(Fixed::from_i64(whole));
            if mixed
                && fraction > nought
                && let Some(slot) = between.get_mut(dimensions)
                && let Some(span) = spans.get_mut(dimensions)
            {
                *slot = (axis.stride, fraction);
                *span = Span {
                    fraction,
                    last: axis.last.floor(),
                    stride: axis.stride,
                    whole,
                };
                dimensions = dimensions.saturating_add(1);
            }
        }
        let used = between.get(..dimensions).unwrap_or_default();
        // Linear mixing reads 2 samples a dimension, the spline 4 at a cost of its own.
        let (shift, each) = if self.cubic { (2, SPLINE_WORK) } else { (1, 1) };
        let count =
            1_usize.wrapping_shl(u32::try_from(dimensions.saturating_mul(shift)).unwrap_or(0));
        let mappings = self.axes.len().saturating_add(out.len());
        let work = count
            .saturating_mul(each)
            .saturating_mul(out.len())
            .saturating_add(mappings.saturating_mul(MAPPING_WORK))
            .saturating_add(1);
        job.spend(u64::try_from(work).unwrap_or(u64::MAX))?;
        if self.cubic {
            self.splined(base, spans.get(..dimensions).unwrap_or_default(), out);
            return Ok(());
        }
        // Arrays sized to the corners, as filling them costs as much as mixing.
        match dimensions {
            0..=2 => self.mixed::<4>(base, used, out),
            3..=4 => self.mixed::<16>(base, used, out),
            5..=6 => self.mixed::<64>(base, used, out),
            _ => self.mixed::<{ 1 << MIXED }>(base, used, out),
        }
        Ok(())
    }
}

impl Evaluate for Several {
    fn evaluate(
        &self,
        job: &Job<'_>,
        _domain: &[[Fixed; 2]],
        input: &[Fixed],
        out: &mut [Fixed],
    ) -> Result<(), PdfError> {
        for (slot, part) in out.iter_mut().zip(&self.parts) {
            part.evaluate(job, input, slice::from_mut(slot))?;
        }
        Ok(())
    }
}

impl Evaluate for Stitched {
    fn evaluate(
        &self,
        job: &Job<'_>,
        domain: &[[Fixed; 2]],
        input: &[Fixed],
        out: &mut [Fixed],
    ) -> Result<(), PdfError> {
        job.spend(1)?;
        let value = input.first().copied().unwrap_or_default();
        let [low, high] = domain.first().copied().unwrap_or_default();
        // Half open, but the first is its one point when Bounds starts with the domain.
        let chosen = if value == low && self.bounds.first() == Some(&low) {
            0
        } else {
            self.bounds.partition_point(|bound| *bound <= value)
        };
        let start = chosen
            .checked_sub(1)
            .and_then(|before| self.bounds.get(before))
            .copied()
            .unwrap_or(low);
        let end = self.bounds.get(chosen).copied().unwrap_or(high);
        let (Some(part), Some(&encode)) = (self.parts.get(chosen), self.encode.get(chosen)) else {
            return Err(job.objects.malformed());
        };
        part.evaluate(job, &[interpolate(value, [start, end], encode)], out)
    }
}

impl Function {
    /// The bytes it holds.
    pub(crate) const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// The outputs for `input`, into `out`.
    pub(crate) fn evaluate(
        &self,
        job: &Job<'_>,
        input: &[Fixed],
        out: &mut [Fixed],
    ) -> Result<(), PdfError> {
        let mut held = [Fixed::default(); VALUES];
        let (Some(values), Some(given), Some(found)) = (
            held.get_mut(..self.domain.len()),
            input.get(..self.domain.len()),
            out.get_mut(..self.outputs),
        ) else {
            return Err(job.objects.malformed());
        };
        for ((slot, &value), &[low, high]) in values.iter_mut().zip(given).zip(&self.domain) {
            *slot = value.clamp(low, high);
        }
        self.kind.evaluate(job, &self.domain, values, found)?;
        for (slot, &[low, high]) in found.iter_mut().zip(&self.range) {
            *slot = (*slot).clamp(low, high);
        }
        Ok(())
    }

    /// Its inputs.
    pub(crate) const fn inputs(&self) -> usize {
        self.domain.len()
    }

    /// Its outputs.
    pub(crate) const fn outputs(&self) -> usize {
        self.outputs
    }

    /// The function `value`, or the array of functions of one output each.
    pub(crate) fn read(job: &Job<'_>, value: &Resolved) -> Result<Rc<Self>, PdfError> {
        let mut reader = Reader {
            held: 0,
            job,
            nodes: 0,
        };
        let mut found = match data::items(value.value()) {
            Some(items) => reader.several(items)?,
            None => reader.function(value, 0)?,
        };
        found.bytes = u64::from(reader.nodes)
            .saturating_mul(NODE_SIZE)
            .saturating_add(reader.held);
        Ok(Rc::new(found))
    }
}

impl Reader<'_, '_> {
    /// A calculator function of `outputs`.
    fn calculated(
        &mut self,
        value: &Resolved,
        range: &[[Fixed; 2]],
    ) -> Result<Box<dyn Evaluate>, PdfError> {
        if range.is_empty() || !matches!(*value.value(), Object::Stream(..)) {
            return Err(self.job.objects.malformed());
        }
        let (source, ended) = data::start(self.job, value, PROGRAM)?;
        if !ended {
            return Err(self.job.capped());
        }
        let program = calculator::Program::read(self.job, &source)?;
        self.hold(program.bytes())?;
        Ok(Box::new(Calculated {
            low: range.iter().map(|pair| pair[0]).collect(),
            program,
        }))
    }

    /// An exponential function.
    fn exponential(&self, owner: &Object) -> Result<(Box<dyn Evaluate>, usize), PdfError> {
        let job = self.job;
        let or = |found: Vec<Fixed>, usual: i64| {
            if found.is_empty() {
                vec![Fixed::from_i64(usual)]
            } else {
                found
            }
        };
        let start = or(numbers(job, owner, b"C0")?, 0);
        let end = or(numbers(job, owner, b"C1")?, 1);
        let exponent = Fixed::from_object(data::entry(job, owner, b"N")?.value())
            .ok_or_else(|| job.objects.malformed())?;
        if start.len() != end.len() {
            return Err(job.objects.malformed());
        }
        let ends = start.iter().zip(&end);
        let exponential = Exponential {
            ends: ends.map(|(&from, &to)| [from, to.minus(from)]).collect(),
            exponent,
        };
        Ok((Box::new(exponential), start.len()))
    }

    /// The function `value`, `depth` functions down.
    fn function(&mut self, value: &Resolved, depth: u8) -> Result<Function, PdfError> {
        let job = self.job;
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > NODES || depth > DEPTH {
            return Err(job.capped());
        }
        // Reading copies it.
        job.spend(RESOLVE_WORK)?;
        let owner = value.value();
        let domain = pairs(job, &numbers(job, owner, b"Domain")?)?;
        let range = pairs(job, &numbers(job, owner, b"Range")?)?;
        if domain.is_empty() {
            return Err(job.objects.malformed());
        }
        if domain.len() > VALUES || range.len() > VALUES {
            return Err(job.capped());
        }
        let single = domain.len() == 1;
        let (kind, outputs) = match *data::entry(job, owner, b"FunctionType")?.value() {
            Object::Integer(0) => (self.sampled(value, domain.len(), &range)?, range.len()),
            Object::Integer(2) if single => self.exponential(owner)?,
            Object::Integer(3) if single => self.stitched(owner, depth)?,
            Object::Integer(4) => (self.calculated(value, &range)?, range.len()),
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Dictionary(_)
            | Object::Integer(_)
            | Object::Name(_)
            | Object::Null
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_) => return Err(job.objects.malformed()),
        };
        if outputs > VALUES {
            return Err(job.capped());
        }
        if !range.is_empty() && range.len() != outputs {
            return Err(job.objects.malformed());
        }
        Ok(Function {
            bytes: 0,
            domain,
            kind,
            outputs,
            range,
        })
    }

    /// Counts `bytes` more.
    fn hold(&mut self, bytes: u64) -> Result<(), PdfError> {
        self.held = self.held.saturating_add(bytes);
        if self.held > HELD {
            return Err(self.job.capped());
        }
        self.job.fits(self.held)
    }

    /// A sampled function of `inputs` and the outputs of `range`.
    fn sampled(
        &mut self,
        value: &Resolved,
        inputs: usize,
        range: &[[Fixed; 2]],
    ) -> Result<Box<dyn Evaluate>, PdfError> {
        let job = self.job;
        let broken = || job.objects.malformed();
        let owner = value.value();
        let sizes = numbers(job, owner, b"Size")?;
        let bits = Fixed::from_object(data::entry(job, owner, b"BitsPerSample")?.value())
            .and_then(|found| u32::try_from(found.floor()).ok())
            .filter(|found| matches!(*found, 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32))
            .ok_or_else(broken)?;
        let encode = couples(&numbers(job, owner, b"Encode")?);
        let mut decode = couples(&numbers(job, owner, b"Decode")?);
        if decode.is_empty() {
            decode = range.to_vec();
        }
        let wrong = sizes.len() != inputs
            || range.is_empty()
            || decode.len() != range.len()
            || (!encode.is_empty() && encode.len() != inputs)
            || !matches!(*owner, Object::Stream(..));
        if wrong {
            return Err(broken());
        }
        let order = Fixed::from_object(data::entry(job, owner, b"Order")?.value());
        let cubic = order.is_some_and(|found| found.floor() == 3);
        let mut axes = Vec::with_capacity(inputs);
        let mut count = 1_usize;
        for (at, size) in sizes.iter().map(|size| size.floor()).enumerate() {
            let step = usize::try_from(size).ok().filter(|step| *step > 0);
            let last = Fixed::from_i64(size.saturating_sub(1));
            axes.push(Axis {
                encode: encode
                    .get(at)
                    .copied()
                    .unwrap_or_else(|| [Fixed::default(), last]),
                last,
                stride: count,
            });
            count = count
                .checked_mul(step.ok_or_else(broken)?)
                .ok_or_else(|| job.capped())?;
        }
        let needed = u64::try_from(count)
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(range.len()).unwrap_or(u64::MAX))
            .saturating_mul(u64::from(bits))
            .div_ceil(8);
        self.hold(needed)?;
        let length = usize::try_from(needed).map_err(|_error| job.limit())?;
        let mut samples = room(length)?;
        samples.resize(length, 0);
        let mut opened = data::coded(job, value)?;
        if opened.image.is_some() {
            return Err(broken());
        }
        if pull::fill(&mut opened.data, &mut samples)? < length {
            job.warn("7.10.2", "a sampled function is short of samples")?;
        }
        Ok(Box::new(Sampled {
            axes,
            bits,
            cubic,
            data: samples,
            decode,
        }))
    }

    /// An array of functions of one output each.
    fn several(&mut self, items: &[Object]) -> Result<Function, PdfError> {
        let job = self.job;
        if items.len() > VALUES {
            return Err(job.capped());
        }
        let mut parts = Vec::with_capacity(items.len());
        for item in items {
            let part = self.function(&data::resolve(job, item)?, 0)?;
            if part.outputs != 1 {
                return Err(job.objects.malformed());
            }
            parts.push(part);
        }
        let inputs = parts
            .first()
            .map(|part| part.domain.len())
            .filter(|inputs| parts.iter().all(|part| part.domain.len() == *inputs))
            .ok_or_else(|| job.objects.malformed())?;
        // Each part clips to its own domain.
        let open = [Fixed::from_bits(i64::MIN), Fixed::from_bits(i64::MAX)];
        Ok(Function {
            bytes: 0,
            domain: vec![open; inputs],
            outputs: parts.len(),
            kind: Box::new(Several { parts }),
            range: Vec::new(),
        })
    }

    /// A stitching function, `depth` functions down.
    fn stitched(
        &mut self,
        owner: &Object,
        depth: u8,
    ) -> Result<(Box<dyn Evaluate>, usize), PdfError> {
        let job = self.job;
        let broken = || job.objects.malformed();
        let listed = data::entry(job, owner, b"Functions")?;
        let items = data::items(listed.value()).ok_or_else(broken)?;
        let nodes = usize::try_from(NODES.saturating_sub(self.nodes)).unwrap_or(0);
        if items.len() > nodes {
            return Err(job.capped());
        }
        let bounds = numbers(job, owner, b"Bounds")?;
        let encode = couples(&numbers(job, owner, b"Encode")?);
        if bounds.len().saturating_add(1) != items.len() || encode.len() != items.len() {
            return Err(broken());
        }
        let mut parts: Vec<Function> = room(items.len())?;
        for item in items {
            let found = data::resolve(job, item)?;
            let part = self.function(&found, depth.saturating_add(1))?;
            let differs = parts
                .first()
                .is_some_and(|first| first.outputs != part.outputs);
            if differs || part.domain.len() != 1 {
                return Err(broken());
            }
            parts.push(part);
        }
        let outputs = parts.first().ok_or_else(broken)?.outputs;
        let stitched = Stitched {
            bounds,
            encode,
            parts,
        };
        Ok((Box::new(stitched), outputs))
    }
}

impl Sampled {
    /// Mixes each output at the corners from `base` over `used`, at most `N` corners.
    fn mixed<const N: usize>(&self, base: usize, used: &[(usize, Fixed)], out: &mut [Fixed]) {
        let nought = Fixed::default();
        let mut corners = [base; N];
        let mut count = 1_usize;
        for &(stride, _) in used {
            let (low, high) = corners.split_at_mut(count);
            for (to, from) in high.iter_mut().zip(low.iter()) {
                *to = from.saturating_add(stride);
            }
            count = count.saturating_mul(2);
        }
        let most = i64::from(u32::MAX >> 32_u32.saturating_sub(self.bits));
        // A sample has 16 fraction bits more while it is mixed.
        let full = [nought, Fixed::from_bits(most << 16_u32)];
        let mut values = [nought; N];
        for ((output, slot), &decode) in out.iter_mut().enumerate().zip(&self.decode) {
            let held = values.iter_mut().zip(&corners).take(count);
            for (value, &corner) in held {
                let index = corner
                    .saturating_mul(self.decode.len())
                    .saturating_add(output);
                *value = Fixed::from_bits(i64::from(self.sample(index)) << 16_u32);
            }
            let mut left = count;
            for &(_, fraction) in used.iter().rev() {
                left >>= 1_u32;
                let (low, high) = values.split_at_mut(left);
                for (near, &far) in low.iter_mut().zip(high.iter()) {
                    *near = near.plus(far.minus(*near).times(fraction));
                }
            }
            let sample = values.first().copied().unwrap_or_default();
            *slot = interpolate(sample, full, decode);
        }
    }

    /// Sample `index`; 0 past the end.
    fn sample(&self, index: usize) -> u32 {
        let bit = u64::try_from(index)
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::from(self.bits));
        let Ok(first) = usize::try_from(bit >> 3_u32) else {
            return 0;
        };
        let within = u32::try_from(bit & 7)
            .unwrap_or(0)
            .saturating_add(self.bits);
        // No width allowed crosses into a fifth byte.
        let taken = usize::try_from(within.div_ceil(8)).unwrap_or(0);
        let Some(bytes) = first
            .checked_add(taken)
            .and_then(|end| self.data.get(first..end))
        else {
            return 0;
        };
        let spare = within.next_multiple_of(8).saturating_sub(within);
        pull::number(bytes).wrapping_shr(spare) & (u32::MAX >> 32_u32.saturating_sub(self.bits))
    }

    /// Output `output` by cubic spline from the samples about `base` over `spans` (7.10.2), with
    /// 16 fraction bits more; each axis takes the edge sample past its ends.
    fn spline(&self, base: usize, spans: &[Span], output: usize) -> Fixed {
        let Some((span, rest)) = spans.split_first() else {
            let index = base
                .saturating_mul(self.decode.len())
                .saturating_add(output);
            return Fixed::from_bits(i64::from(self.sample(index)) << 16_u32);
        };
        let mut values = [Fixed::default(); 4];
        let stride = isize::try_from(span.stride).unwrap_or(isize::MAX);
        for (offset, value) in (-1_i64..).zip(&mut values) {
            let moved = span
                .whole
                .saturating_add(offset)
                .clamp(0, span.last)
                .saturating_sub(span.whole);
            let step = isize::try_from(moved).unwrap_or(0).saturating_mul(stride);
            *value = self.spline(base.saturating_add_signed(step), rest, output);
        }
        catmull(values, span.fraction)
    }

    /// Each output by cubic spline from the samples about `base` over `spans`.
    fn splined(&self, base: usize, spans: &[Span], out: &mut [Fixed]) {
        let most = i64::from(u32::MAX >> 32_u32.saturating_sub(self.bits));
        let full = [Fixed::default(), Fixed::from_bits(most << 16_u32)];
        for ((output, slot), &decode) in out.iter_mut().enumerate().zip(&self.decode) {
            // The spline may overshoot the samples; it stays within what they can hold.
            let sample = self.spline(base, spans, output).clamp(full[0], full[1]);
            *slot = interpolate(sample, full, decode);
        }
    }
}

/// The Catmull-Rom spline through `values`, a sample either side of the two it is between, at
/// `fraction` of the way from the second to the third.
fn catmull(values: [Fixed; 4], fraction: Fixed) -> Fixed {
    let [before, from, to, after] = values;
    let times = |value: Fixed, count: i64| Fixed::from_bits(value.bits().saturating_mul(count));
    let cubed = after.minus(before).plus(times(from.minus(to), 3));
    let squared = times(before, 2)
        .minus(times(from, 5))
        .plus(times(to, 4))
        .minus(after);
    let linear = to.minus(before);
    let sum = cubed
        .times(fraction)
        .plus(squared)
        .times(fraction)
        .plus(linear)
        .times(fraction);
    from.plus(Fixed::from_bits(sum.bits() >> 1_u32))
}

/// `values` two by two.
fn couples(values: &[Fixed]) -> Vec<[Fixed; 2]> {
    values.as_chunks::<2>().0.to_vec()
}

/// `value` carried from the interval `from` onto `onto`; its start when `from` is empty.
pub(crate) fn interpolate(value: Fixed, from: [Fixed; 2], onto: [Fixed; 2]) -> Fixed {
    let [[low, high], [start, end]] =
        [from, onto].map(|pair| pair.map(|part| i128::from(part.bits())));
    let product = i128::from(value.bits())
        .saturating_sub(low)
        .saturating_mul(end.saturating_sub(start));
    let way = quotient(product, high.saturating_sub(low)).unwrap_or(0);
    Fixed::from_bits(saturated(start.saturating_add(way)))
}

/// The numbers of the array `key` of `owner`; none when null.
pub(crate) fn numbers(job: &Job<'_>, owner: &Object, key: &[u8]) -> Result<Vec<Fixed>, PdfError> {
    let listed = data::entry(job, owner, key)?;
    if matches!(*listed.value(), Object::Null) {
        return Ok(Vec::new());
    }
    let items = data::items(listed.value()).ok_or_else(|| job.objects.malformed())?;
    if items.len() > LISTED {
        return Err(job.capped());
    }
    job.spend(u64::try_from(items.len()).unwrap_or(u64::MAX))?;
    items
        .iter()
        .map(|item| {
            Fixed::from_object(data::resolve(job, item)?.value())
                .ok_or_else(|| job.objects.malformed())
        })
        .collect()
}

/// `values` as intervals, none of them backwards.
pub(crate) fn pairs(job: &Job<'_>, values: &[Fixed]) -> Result<Vec<[Fixed; 2]>, PdfError> {
    let (found, odd) = values.as_chunks::<2>();
    if odd.is_empty() && found.iter().all(|&[low, high]| low <= high) {
        Ok(found.to_vec())
    } else {
        Err(job.objects.malformed())
    }
}

/// An empty list with room for `count`, sized by the file.
pub(crate) fn room<T>(count: usize) -> Result<Vec<T>, PdfError> {
    let mut list = Vec::new();
    list.try_reserve_exact(count)
        .map_err(|_error| PdfError::new(ErrorCode::BudgetExceeded, OPERATION))?;
    Ok(list)
}
