//! ICC profiles (ICC.1:2022): curves, matrices and tables to the profile connection space.
//! Values in a conversion have 16 fraction bits.

use alloc::rc::Rc;

use crate::PdfError;
use crate::pull;
use crate::render::colour::Intent;
use crate::render::colour::cie::{self, Curve, Linear, ONE};
use crate::render::fixed::Fixed;
use crate::render::function::{elementary, room};
use crate::render::job::Job;

/// The illuminant of the connection space (7.2.16).
const D50: [Fixed; 3] = [
    Fixed::from_bits(0xF6D6_0000),
    Fixed::from_i64(1),
    Fixed::from_bits(0xD32D_0000),
];

/// Where a profile's tags are counted (7.3).
const DIRECTORY: usize = 128;

/// The most bytes a profile's tables hold.
const HELD: u64 = 0x80_0000;

/// Work of a table's corner, for its three outputs.
const CORNER_WORK: u64 = 4;

/// Work of a curve.
const CURVE_WORK: u64 = 2;

/// Work of a transform besides its parts: the matrix to RGB and its transfer.
const TRANSFORM_WORK: u64 = 16;

/// The most inputs.
const INPUTS: usize = 4;

/// What a colour in the legacy 16-bit encoding of L*a*b* is scaled by on the way in: 65,280 over
/// 65,535 (6.3.4.2).
const LEGACY: i64 = 65_281;

/// The most tags.
const TAGS: usize = 1024;

/// How the values at a transform's end give a colour (6.3.4.2).
#[derive(Clone, Copy)]
enum Ending {
    /// L*a*b* of version 4, and in 8 bits.
    Lab,
    /// L*a*b* in the legacy 16-bit encoding.
    Legacy,
    /// L* alone.
    Lightness,
    /// XYZ as it is.
    Plain,
    /// Encoded XYZ, of which 32,768 is one.
    Xyz,
}

/// A colour lookup table.
struct Grid {
    /// The points of each input.
    points: Vec<usize>,
    /// The values, three a point, the first input varying slowest.
    table: Vec<u8>,
    /// Whether a value has two bytes.
    wide: bool,
}

/// What reading one profile keeps.
struct Parser<'run, 'doc> {
    /// The profile.
    bytes: &'run [u8],
    /// The curves read, by where they start.
    curves: Vec<(usize, Rc<Curve>)>,
    /// The bytes held.
    held: u64,
    /// The inputs.
    inputs: usize,
    /// The job.
    job: &'run Job<'doc>,
    /// Whether the connection space is L*a*b*.
    lab: bool,
    /// Whether the colours given are L*a*b*.
    lab_data: bool,
    /// The transforms read, by where they start.
    transforms: Vec<(usize, Rc<Transform>)>,
}

/// A usable profile.
pub(super) struct Profile {
    /// To RGB with the media white kept (6.3.2).
    absolute: Linear,
    /// The bytes it holds.
    bytes: u64,
    /// To RGB with the media white as white.
    relative: Linear,
    /// The perceptual, colorimetric and saturation transforms.
    transforms: [Rc<Transform>; 3],
    /// The work of a colour through its dearest transform.
    work: u64,
}

/// A way from colours to the connection space: curves, table, curves, matrix, curves, each
/// optional.
struct Transform {
    /// How it ends.
    ending: Ending,
    /// The curves of the inputs.
    first: Vec<Rc<Curve>>,
    /// The table.
    grid: Option<Grid>,
    /// The curves of the outputs.
    last: Vec<Rc<Curve>>,
    /// The matrix by rows, then its offsets, s15.16.
    matrix: Option<[i64; 12]>,
    /// The curves before the matrix.
    middle: Vec<Rc<Curve>>,
    /// What the colours given are scaled by.
    scale: i64,
}

impl Grid {
    /// The three outputs at `input`.
    fn mixed(&self, input: [i64; INPUTS]) -> [i64; 3] {
        let mut between = [(0_usize, 0_i64); INPUTS];
        let mut dimensions = 0_usize;
        let (mut base, mut stride) = (0_usize, 1_usize);
        for (&points, &value) in self.points.iter().zip(&input).rev() {
            let last = i64::try_from(points).unwrap_or(1).saturating_sub(1);
            let place = value.clamp(0, ONE).saturating_mul(last);
            let index = usize::try_from(place >> 16_u32).unwrap_or(0);
            base = base.saturating_add(index.saturating_mul(stride));
            let fraction = place & 0xFFFF;
            if fraction > 0
                && let Some(slot) = between.get_mut(dimensions)
            {
                *slot = (stride, fraction);
                dimensions = dimensions.saturating_add(1);
            }
            stride = stride.saturating_mul(points);
        }
        let used = between.get(..dimensions).unwrap_or_default();
        let mut corners = [base; 1 << INPUTS];
        let mut count = 1_usize;
        for &(step, _) in used {
            let (low, high) = corners.split_at_mut(count);
            for (to, from) in high.iter_mut().zip(low.iter()) {
                *to = from.saturating_add(step);
            }
            count = count.saturating_mul(2);
        }
        let mut out = [0_i64; 3];
        for (output, slot) in out.iter_mut().enumerate() {
            let mut values = [0_i64; 1 << INPUTS];
            for (value, &corner) in values.iter_mut().zip(&corners).take(count) {
                *value = self.value(corner.saturating_mul(3).saturating_add(output)) << 16_u32;
            }
            let mut left = count;
            for &(_, fraction) in used.iter().rev() {
                left >>= 1_u32;
                let (low, high) = values.split_at_mut(left);
                for (near, &far) in low.iter_mut().zip(high.iter()) {
                    let way = far.saturating_sub(*near).saturating_mul(fraction) >> 16_u32;
                    *near = near.saturating_add(way);
                }
            }
            *slot = cie::widened(values.first().copied().unwrap_or_default());
        }
        out
    }

    /// Value `index`, 65,535 as one.
    fn value(&self, index: usize) -> i64 {
        if self.wide {
            let at = index.saturating_mul(2);
            let pair = self.table.get(at..at.saturating_add(2));
            i64::from(pair.map_or(0, pull::number))
        } else {
            i64::from(self.table.get(index).copied().unwrap_or_default()).saturating_mul(257)
        }
    }
}

impl<'run> Parser<'run, '_> {
    /// The A-to-B transform at `at` (10.10 to 10.12).
    fn a_to_b(&mut self, at: usize) -> Result<Rc<Transform>, PdfError> {
        if let Some(found) = self.transforms.iter().find(|known| known.0 == at) {
            return Ok(Rc::clone(&found.1));
        }
        let found = match self.taken(at, 4)? {
            b"mft1" => self.tables(at, false)?,
            b"mft2" => self.tables(at, true)?,
            b"mAB " => self.elements(at)?,
            _ => return Err(self.broken()),
        };
        let shared = Rc::new(found);
        self.transforms.push((at, Rc::clone(&shared)));
        Ok(shared)
    }

    /// The error of a profile that cannot be used.
    fn broken(&self) -> PdfError {
        self.job.objects.malformed()
    }

    /// The curve at `at` and the bytes it takes (10.6, 10.18).
    fn curve(&mut self, at: usize) -> Result<(Rc<Curve>, usize), PdfError> {
        let kind = self.taken(at, 4)?;
        let parametric = kind == b"para";
        if !parametric && kind != b"curv" {
            return Err(self.broken());
        }
        let size = if parametric {
            let values = match self.number(at.saturating_add(8), 2)? {
                0 => 1_usize,
                1 => 3,
                2 => 4,
                3 => 5,
                4 => 7,
                _ => return Err(self.broken()),
            };
            values.saturating_mul(4).saturating_add(12)
        } else {
            self.number(at.saturating_add(8), 4)?
                .saturating_mul(2)
                .saturating_add(12)
        };
        if let Some(found) = self.curves.iter().find(|known| known.0 == at) {
            return Ok((Rc::clone(&found.1), size));
        }
        let data = self.taken(at, size)?;
        let body = data.get(12..).unwrap_or_default();
        let found = if parametric {
            self.parametric(body)?
        } else {
            match body.len() {
                0 => Curve::identity(),
                2 => {
                    let gamma = i64::from(pull::number(body)) << 24_u32;
                    Curve::power(self.job, Fixed::from_bits(gamma))?
                }
                _ => {
                    // Checked before the samples are made.
                    let made = u64::try_from(body.len()).unwrap_or(u64::MAX);
                    self.job.fits(self.held.saturating_add(made))?;
                    Curve::sampled(self.samples(body, true)?)
                }
            }
        };
        self.hold(found.bytes())?;
        let shared = Rc::new(found);
        self.curves.push((at, Rc::clone(&shared)));
        Ok((shared, size))
    }

    /// `count` curves one after another from `at`, each padded to 4 bytes; none without `at`.
    fn curve_list(&mut self, at: Option<usize>, count: usize) -> Result<Vec<Rc<Curve>>, PdfError> {
        let Some(start) = at else {
            return Ok(Vec::new());
        };
        let mut list = Vec::with_capacity(count);
        let mut next = start;
        for _ in 0..count {
            let (curve, size) = self.curve(next)?;
            list.push(curve);
            next = next.saturating_add(size.checked_next_multiple_of(4).unwrap_or(size));
        }
        Ok(list)
    }

    /// A lutAToBType transform (10.12).
    fn elements(&mut self, at: usize) -> Result<Transform, PdfError> {
        let inputs = self.number(at.saturating_add(8), 1)?;
        let outputs = self.number(at.saturating_add(9), 1)?;
        let mut offsets = [None; 5];
        let fields = offsets.iter_mut().zip((12_usize..).step_by(4));
        for (slot, field) in fields {
            let offset = self.number(at.saturating_add(field), 4)?;
            *slot = (offset > 0).then(|| at.saturating_add(offset));
        }
        let [second, matrix, middle, table, first] = offsets;
        let bare = inputs != 3 && table.is_none();
        if inputs != self.inputs || outputs != 3 || second.is_none() || bare {
            return Err(self.broken());
        }
        let grid = if let Some(start) = table {
            let mut points = Vec::with_capacity(inputs);
            for input in 0..inputs {
                points.push(self.number(start.saturating_add(input), 1)?);
            }
            let wide = match self.number(start.saturating_add(16), 1)? {
                1 => false,
                2 => true,
                _ => return Err(self.broken()),
            };
            Some(self.grid(start.saturating_add(20), points, wide)?)
        } else {
            None
        };
        let factors = if let Some(start) = matrix {
            let mut found = [0_i64; 12];
            let entries = self.taken(start, 48)?.as_chunks::<4>().0;
            for (slot, field) in found.iter_mut().zip(entries) {
                *slot = sign(pull::number(field));
            }
            Some(found)
        } else {
            None
        };
        Ok(Transform {
            ending: if self.lab { Ending::Lab } else { Ending::Xyz },
            first: self.curve_list(first, inputs)?,
            grid,
            last: self.curve_list(second, 3)?,
            matrix: factors,
            middle: self.curve_list(middle, 3)?,
            scale: ONE,
        })
    }

    /// The table of `points` at `at`.
    fn grid(&mut self, at: usize, points: Vec<usize>, wide: bool) -> Result<Grid, PdfError> {
        let values = points
            .iter()
            .try_fold(if wide { 6 } else { 3 }, |size: usize, &side| {
                (side > 0).then(|| size.checked_mul(side)).flatten()
            })
            .ok_or_else(|| self.broken())?;
        let data = self.taken(at, values)?;
        self.hold(u64::try_from(values).unwrap_or(u64::MAX))?;
        let mut table = room(values)?;
        table.extend_from_slice(data);
        Ok(Grid {
            points,
            table,
            wide,
        })
    }

    /// Counts `bytes` more, beside all that is held.
    fn hold(&mut self, bytes: u64) -> Result<(), PdfError> {
        self.held = self.held.saturating_add(bytes);
        if self.held > HELD {
            return Err(self.job.capped());
        }
        self.job.fits(self.held)?;
        self.job.spend(bytes)
    }

    /// The number of `length` bytes at `at`.
    fn number(&self, at: usize, length: usize) -> Result<usize, PdfError> {
        usize::try_from(pull::number(self.taken(at, length)?)).map_err(|_error| self.broken())
    }

    /// The parametric curve of parameters `body` (10.18).
    fn parametric(&self, body: &[u8]) -> Result<Curve, PdfError> {
        let mut held = [Fixed::default(); 7];
        let fields = body.as_chunks::<4>().0;
        for (slot, field) in held.iter_mut().zip(fields) {
            *slot = fixed(sign(pull::number(field)));
        }
        let [gamma, scale, shift, third, fourth, fifth, sixth] = held;
        let count = fields.len();
        Curve::compiled(self.job, &|value| {
            let inner = if count == 1 {
                value
            } else {
                value.times(scale).plus(shift)
            };
            // Up to four parameters it bends where the base turns negative, else at the fifth.
            let raised = if count <= 4 {
                inner >= Fixed::default()
            } else {
                value >= fourth
            };
            let power = if raised {
                elementary::power(inner.max(Fixed::default()), gamma).unwrap_or_default()
            } else {
                Fixed::default()
            };
            match count {
                4 => power.plus(third),
                5 if !raised => third.times(value),
                7 if raised => power.plus(fifth),
                7 => third.times(value).plus(sixth),
                _ => power,
            }
        })
    }

    /// The samples in `data` of 8 or, when `wide`, 16 bits, as 16 bits.
    fn samples(&self, data: &[u8], wide: bool) -> Result<Vec<u16>, PdfError> {
        let mut found = room(if wide {
            data.len() >> 1_u32
        } else {
            data.len()
        })?;
        if wide {
            for &pair in data.as_chunks::<2>().0 {
                found.push(u16::try_from(pull::number(&pair)).map_err(|_error| self.broken())?);
            }
        } else {
            found.extend(
                data.iter()
                    .map(|&value| u16::from(value).saturating_mul(257)),
            );
        }
        Ok(found)
    }

    /// The transform of one curve a component, for RGB through a matrix (Annex F).
    fn shaped(&mut self) -> Result<Option<Rc<Transform>>, PdfError> {
        let mut first = Vec::new();
        let mut matrix = [0_i64; 12];
        let ending = if self.inputs == 1 {
            let Some(at) = self.tag(*b"kTRC")? else {
                return Ok(None);
            };
            first.push(self.curve(at)?.0);
            if self.lab {
                Ending::Lightness
            } else {
                for (row, white) in D50.iter().enumerate() {
                    if let Some(slot) = matrix.get_mut(row.saturating_mul(3)) {
                        *slot = cie::sixteenths(*white);
                    }
                }
                Ending::Plain
            }
        } else if self.inputs == 3 && !self.lab {
            let names = [(b"rXYZ", b"rTRC"), (b"gXYZ", b"gTRC"), (b"bXYZ", b"bTRC")];
            for (column, (colorant, curve)) in names.into_iter().enumerate() {
                let (Some(from), Some(bend)) = (self.tag(*colorant)?, self.tag(*curve)?) else {
                    return Ok(None);
                };
                first.push(self.curve(bend)?.0);
                let parts = self.triple(from)?;
                for (row, part) in parts.into_iter().enumerate() {
                    let index = row.saturating_mul(3).saturating_add(column);
                    if let Some(slot) = matrix.get_mut(index) {
                        *slot = part;
                    }
                }
            }
            Ending::Plain
        } else {
            return Ok(None);
        };
        Ok(Some(Rc::new(Transform {
            ending,
            first,
            grid: None,
            last: Vec::new(),
            matrix: matches!(ending, Ending::Plain).then_some(matrix),
            middle: Vec::new(),
            scale: ONE,
        })))
    }

    /// The s15.16 number at `at`.
    fn signed(&self, at: usize) -> Result<i64, PdfError> {
        self.taken(at, 4).map(|field| sign(pull::number(field)))
    }

    /// The curve of `length` bytes of table at `at`.
    fn table_curve(&mut self, at: usize, length: usize, wide: bool) -> Result<Rc<Curve>, PdfError> {
        let found = Curve::sampled(self.samples(self.taken(at, length)?, wide)?);
        self.hold(found.bytes())?;
        Ok(Rc::new(found))
    }

    /// A lut8Type or, when `wide`, lut16Type transform (10.10, 10.11).
    fn tables(&mut self, at: usize, wide: bool) -> Result<Transform, PdfError> {
        let inputs = self.number(at.saturating_add(8), 1)?;
        let outputs = self.number(at.saturating_add(9), 1)?;
        let points = self.number(at.saturating_add(10), 1)?;
        if inputs != self.inputs || outputs != 3 {
            return Err(self.broken());
        }
        let (start, before, after, size) = if wide {
            let before = self.number(at.saturating_add(48), 2)?;
            let after = self.number(at.saturating_add(50), 2)?;
            (52_usize, before, after, 2_usize)
        } else {
            (48, 256, 256, 1)
        };
        if before == 0 || after == 0 {
            return Err(self.broken());
        }
        let mut next = at.saturating_add(start);
        let mut first = Vec::with_capacity(inputs);
        for _ in 0..inputs {
            let length = before.saturating_mul(size);
            first.push(self.table_curve(next, length, wide)?);
            next = next.saturating_add(length);
        }
        let grid = self.grid(next, vec![points; inputs], wide)?;
        next = next.saturating_add(grid.table.len());
        let mut last = Vec::with_capacity(3);
        for _ in 0_u8..3_u8 {
            let length = after.saturating_mul(size);
            last.push(self.table_curve(next, length, wide)?);
            next = next.saturating_add(length);
        }
        Ok(Transform {
            ending: match (self.lab, wide) {
                (false, _) => Ending::Xyz,
                (true, false) => Ending::Lab,
                (true, true) => Ending::Legacy,
            },
            first,
            grid: Some(grid),
            last,
            // The matrix of these types is for XYZ given, which no space here has.
            matrix: None,
            middle: Vec::new(),
            scale: if wide && self.lab_data { LEGACY } else { ONE },
        })
    }

    /// Where the tag `name` starts; `None` when missing.
    fn tag(&self, name: [u8; 4]) -> Result<Option<usize>, PdfError> {
        let count = self.number(DIRECTORY, 4)?;
        if count > TAGS {
            return Err(self.job.capped());
        }
        let records = self.taken(DIRECTORY.saturating_add(4), count.saturating_mul(12))?;
        let Some(record) = records
            .as_chunks::<12>()
            .0
            .iter()
            .find(|record| record.starts_with(&name))
        else {
            return Ok(None);
        };
        let start = pull::number(record.get(4..8).unwrap_or_default());
        usize::try_from(start)
            .map(Some)
            .map_err(|_error| self.broken())
    }

    /// `length` bytes from `at`.
    fn taken(&self, at: usize, length: usize) -> Result<&'run [u8], PdfError> {
        at.checked_add(length)
            .and_then(|end| self.bytes.get(at..end))
            .ok_or_else(|| self.broken())
    }

    /// The numbers of the `XYZ ` tag at `at` (10.31).
    fn triple(&self, at: usize) -> Result<[i64; 3], PdfError> {
        if self.taken(at, 4)? != b"XYZ " {
            return Err(self.broken());
        }
        Ok([
            self.signed(at.saturating_add(8))?,
            self.signed(at.saturating_add(12))?,
            self.signed(at.saturating_add(16))?,
        ])
    }
}

impl Profile {
    /// The bytes it holds.
    pub(super) const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// The profile `bytes` of `inputs` components; malformed when it cannot be used.
    pub(super) fn read(job: &Job<'_>, bytes: &[u8], inputs: usize) -> Result<Self, PdfError> {
        let field = |from: usize| bytes.get(from..from.saturating_add(4)).unwrap_or_default();
        let space: &[u8] = field(16);
        let matching = match inputs {
            1 => space == b"GRAY",
            3 => space == b"RGB " || space == b"Lab ",
            4 => space == b"CMYK",
            _ => false,
        };
        let lab = field(20) == b"Lab ";
        let known = matches!(field(12), b"mntr" | b"scnr" | b"prtr" | b"spac")
            && matches!(bytes.get(8), Some(2..=4))
            && field(36) == b"acsp"
            && (lab || field(20) == b"XYZ ");
        let mut parser = Parser {
            bytes,
            curves: Vec::new(),
            held: 0,
            inputs,
            job,
            lab,
            lab_data: space == b"Lab ",
            transforms: Vec::new(),
        };
        if !matching || !known {
            return Err(parser.broken());
        }
        // ICC.1:2022, 8.10: the intent's table, else the perceptual one, else matrix and curves.
        let mut tables = [None, None, None];
        for (slot, name) in tables.iter_mut().zip([b"A2B0", b"A2B1", b"A2B2"]) {
            if let Some(at) = parser.tag(*name)? {
                *slot = Some(parser.a_to_b(at)?);
            }
        }
        let [perceptual, colorimetric, saturation] = tables;
        let either = match perceptual {
            Some(found) => Some(found),
            None => parser.shaped()?,
        };
        let usual = either
            .or_else(|| colorimetric.clone())
            .or_else(|| saturation.clone())
            .ok_or_else(|| parser.broken())?;
        let media = match parser.tag(*b"wtpt")? {
            Some(at) => parser.triple(at)?.map(fixed),
            None => D50,
        };
        let unit = [Fixed::from_i64(1); 3];
        let linear = |scale: [Fixed; 3]| Linear::new(D50, &cie::diagonal(scale));
        let relative = if lab { D50 } else { unit };
        // The media white over D50 times what is relative to it (6.3.2).
        let absolute = if lab {
            Some(media)
        } else {
            let [red, green, blue] = media;
            let [first, second, third] = D50;
            red.over(first)
                .zip(green.over(second))
                .zip(blue.over(third))
                .map(|((one, two), three)| [one, two, three])
        };
        let work = [Some(&usual), colorimetric.as_ref(), saturation.as_ref()]
            .into_iter()
            .flatten()
            .map(|transform| transform.work())
            .fold(0, u64::max);
        Ok(Self {
            absolute: absolute.and_then(linear).ok_or_else(|| parser.broken())?,
            bytes: parser.held,
            relative: linear(relative).ok_or_else(|| parser.broken())?,
            work,
            transforms: [
                Rc::clone(&usual),
                colorimetric.unwrap_or_else(|| Rc::clone(&usual)),
                saturation.unwrap_or(usual),
            ],
        })
    }

    /// The colour of `values` with `intent`.
    pub(super) fn rgb(&self, values: [i64; INPUTS], intent: Intent) -> [u16; 3] {
        let [perceptual, colorimetric, saturation] = self.transforms.each_ref();
        let (transform, linear) = match intent {
            Intent::Absolute => (colorimetric, &self.absolute),
            Intent::Perceptual => (perceptual, &self.relative),
            Intent::Relative => (colorimetric, &self.relative),
            Intent::Saturation => (saturation, &self.relative),
        };
        linear.rgb(transform.connected(values))
    }

    /// The work of converting a colour.
    pub(super) const fn work(&self) -> u64 {
        self.work
    }
}

impl Transform {
    /// What `values` give in the connection space, L*a*b* as its part of the white's.
    fn connected(&self, values: [i64; INPUTS]) -> [i64; 3] {
        let mut held = if self.scale == ONE {
            values
        } else {
            values.map(|value| value.saturating_mul(self.scale) >> 16_u32)
        };
        curved(&self.first, &mut held);
        let [one, two, three, _] = held;
        let mut found = self
            .grid
            .as_ref()
            .map_or([one, two, three], |grid| grid.mixed(held));
        curved(&self.middle, &mut found);
        if let Some(matrix) = self.matrix {
            let given = found;
            let (factors, offsets) = matrix.split_at(9);
            let rows = factors.as_chunks::<3>().0.iter().zip(offsets);
            for (slot, (&row, &offset)) in found.iter_mut().zip(rows) {
                *slot = weighed(row, given).saturating_add(offset);
            }
        }
        curved(&self.last, &mut found);
        let [first, second, third] = found;
        let encoded = |lightness: i64, red: i64, blue: i64| {
            let centred = |value: i64| value.saturating_mul(255).saturating_sub(128 << 16_u32);
            cie::lab([lightness.saturating_mul(100), centred(red), centred(blue)])
        };
        // 65,535 over 65,280 is 257 over 256.
        let legacy = |value: i64| value.saturating_mul(257) >> 8_u32;
        match self.ending {
            Ending::Lab => encoded(first, second, third),
            Ending::Legacy => encoded(legacy(first), legacy(second), legacy(third)),
            Ending::Lightness => cie::lab([first.saturating_mul(100), 0, 0]),
            Ending::Plain => found,
            Ending::Xyz => found.map(|value| value.saturating_mul(0xFFFF) >> 15_u32),
        }
    }

    /// The work of a colour through it, as measured: each curve, each table corner of each output,
    /// the matrix and an L*a*b* ending.
    fn work(&self) -> u64 {
        let corners = self.grid.as_ref().map_or(0, |grid| {
            1_u64.wrapping_shl(u32::try_from(grid.points.len()).unwrap_or(0))
        });
        let curves = self
            .first
            .len()
            .saturating_add(self.middle.len())
            .saturating_add(self.last.len());
        let ending = match self.ending {
            Ending::Lab | Ending::Legacy | Ending::Lightness => cie::LAB_WORK,
            Ending::Plain | Ending::Xyz => 0,
        };
        TRANSFORM_WORK
            .saturating_add(corners.saturating_mul(CORNER_WORK))
            .saturating_add(
                u64::try_from(curves)
                    .unwrap_or(u64::MAX)
                    .saturating_mul(CURVE_WORK),
            )
            .saturating_add(ending)
    }
}

/// Puts each of `values` through its curve.
fn curved(curves: &[Rc<Curve>], values: &mut [i64]) {
    for (value, curve) in values.iter_mut().zip(curves) {
        *value = curve.at(*value);
    }
}

/// An s15.16 number in fixed point.
const fn fixed(value: i64) -> Fixed {
    Fixed::from_bits(value.saturating_mul(ONE))
}

/// An s15.16 number out of its bits.
fn sign(field: u32) -> i64 {
    let top = 1_i64 << 31_u32;
    (i64::from(field) ^ top).saturating_sub(top)
}

/// A row of the matrix applied to `values`.
const fn weighed([first, second, third]: [i64; 3], [one, two, three]: [i64; 3]) -> i64 {
    let sum = first
        .saturating_mul(one)
        .saturating_add(second.saturating_mul(two))
        .saturating_add(third.saturating_mul(three))
        .saturating_add(0x8000);
    sum >> 16_u32
}
