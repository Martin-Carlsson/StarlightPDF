//! Shadings of types 1 to 3: by a function, axial and radial (ISO 32000-2:2020, 8.7.4.5.2 to
//! 8.7.4.5.4).

use alloc::rc::Rc;

use crate::PdfError;
use crate::overlay::Object;
use crate::render::colour::Converter;
use crate::render::data;
use crate::render::fixed::{Fixed, saturated};
use crate::render::function::Function;
use crate::render::geometry::Matrix;
use crate::render::job::Job;
use crate::render::shading::{Common, Ramp, Steps};
use crate::render::surface::{Paint, Sample};

/// One, with 32 fraction bits.
const ONE: i128 = 1 << 32_u32;

/// The most a radial shading's quadratic term is off by rounding.
const TANGENT: u128 = 3;

/// Work of a pixel of a radial shading, besides drawing it.
const RADIAL_WORK: u64 = 7;

/// An axial shading.
struct Axial {
    /// It goes on beyond either end.
    extend: [bool; 2],
    /// The map from pixels to the share of the axis.
    lookup: Matrix,
    /// The colours.
    ramp: Ramp,
}

/// A shading that paints nothing.
struct Blank;

/// A shading of type 1.
struct Computed {
    /// Converts.
    converter: Converter,
    /// The rectangle it is defined in.
    domain: [Fixed; 4],
    /// What gives the values.
    function: Rc<Function>,
    /// The map from pixels to its domain.
    lookup: Matrix,
    /// The values of one colour.
    values: Vec<Fixed>,
}

/// A radial shading, in units of its largest measure from the centre of its first circle; every
/// number has 32 fraction bits.
struct Radial {
    /// The quadratic term: the way of the centre squared, less that of the radius.
    bend: i128,
    /// It goes on beyond either circle.
    extend: [bool; 2],
    /// The map from pixels.
    lookup: Matrix,
    /// The way of the centre and of the radius.
    moved: [i128; 3],
    /// The first radius.
    radius: i128,
    /// The colours.
    ramp: Ramp,
}

impl Paint for Axial {
    fn row(
        &mut self,
        job: &Job<'_>,
        row: i32,
        left: i32,
        out: &mut [Sample],
    ) -> Result<(), PdfError> {
        job.spend(u64::try_from(out.len()).unwrap_or(u64::MAX))?;
        let (none, one) = (Fixed::default(), Fixed::from_i64(1));
        let [before, after] = self.extend;
        let mut steps = Steps::new(&self.lookup, row, left);
        for sample in out.iter_mut() {
            let part = steps.advance().across;
            if (part < none && !before) || (part > one && !after) {
                continue;
            }
            *sample = solid(self.ramp.colour(job, part)?);
        }
        Ok(())
    }
}

impl Paint for Blank {
    fn row(
        &mut self,
        _job: &Job<'_>,
        _row: i32,
        _left: i32,
        _out: &mut [Sample],
    ) -> Result<(), PdfError> {
        Ok(())
    }
}

impl Paint for Computed {
    fn row(
        &mut self,
        job: &Job<'_>,
        row: i32,
        left: i32,
        out: &mut [Sample],
    ) -> Result<(), PdfError> {
        job.spend(u64::try_from(out.len()).unwrap_or(u64::MAX))?;
        let [across_low, across_high, down_low, down_high] = self.domain;
        let mut steps = Steps::new(&self.lookup, row, left);
        for sample in out.iter_mut() {
            let at = steps.advance();
            if at.across < across_low
                || at.across > across_high
                || at.down < down_low
                || at.down > down_high
            {
                continue;
            }
            self.function
                .evaluate(job, &[at.across, at.down], &mut self.values)?;
            *sample = solid(self.converter.rgb(job, &self.values)?);
        }
        Ok(())
    }
}

impl Paint for Radial {
    fn row(
        &mut self,
        job: &Job<'_>,
        row: i32,
        left: i32,
        out: &mut [Sample],
    ) -> Result<(), PdfError> {
        let pixels = u64::try_from(out.len()).unwrap_or(u64::MAX);
        job.spend(pixels.saturating_mul(RADIAL_WORK))?;
        let mut steps = Steps::new(&self.lookup, row, left);
        for sample in out.iter_mut() {
            let at = steps.advance();
            if let Some(part) = self.part([at.across, at.down]) {
                *sample = solid(self.ramp.colour(job, part)?);
            }
        }
        Ok(())
    }
}

impl Radial {
    /// The last circle through `at` that is painted: how far it is from the first to the second.
    fn part(&self, at: [Fixed; 2]) -> Option<Fixed> {
        let [across, down] = at.map(|value| i128::from(value.bits()));
        let [centre_across, centre_down, widening] = self.moved;
        let linear = product([
            (across, centre_across),
            (down, centre_down),
            (self.radius, widening),
        ]);
        let constant = product([
            (across, across),
            (down, down),
            (self.radius, self.radius.saturating_neg()),
        ]);
        let inside = linear
            .saturating_mul(linear)
            .saturating_sub(self.bend.saturating_mul(constant));
        let root = if let Ok(square) = u128::try_from(inside) {
            i128::try_from(square.isqrt()).unwrap_or(i128::MAX)
        } else {
            // Rounding alone can take a circle's only point below 0.
            let slack = linear
                .saturating_abs()
                .saturating_mul(4)
                .saturating_add(self.bend.saturating_abs())
                .saturating_add(constant.saturating_abs());
            if inside < slack.saturating_neg() {
                return None;
            }
            0
        };
        // The two roots without a difference of near equals.
        let sum = if linear < 0 {
            linear.saturating_sub(root)
        } else {
            linear.saturating_add(root)
        };
        let share = |above: i128, below: i128| above.saturating_mul(ONE).checked_div(below);
        let found = [share(sum, self.bend), share(constant, sum)];
        let [before, after] = self.extend;
        let painted = found.into_iter().flatten().filter(|&share_along| {
            let wide = self
                .radius
                .saturating_mul(ONE)
                .saturating_add(share_along.saturating_mul(widening));
            wide >= 0 && (share_along >= 0 || before) && (share_along <= ONE || after)
        });
        painted.max().map(|bits| Fixed::from_bits(saturated(bits)))
    }
}

/// An axial shading of `common` (8.7.4.5.3).
fn axial(job: &Job<'_>, common: &Common<'_>) -> Result<Box<dyn Paint>, PdfError> {
    let owner = common.owner.value();
    let [from_across, from_down, to_across, to_down] =
        data::numbers(job, owner, b"Coords")?.ok_or_else(|| job.objects.malformed())?;
    let (extend, ramp) = varied(job, common)?;
    let way = [to_across.minus(from_across), to_down.minus(from_down)];
    let none = Fixed::default();
    let lookup = if way == [none; 2] {
        // Without a length only an extension is left, in its end's colour.
        let end = match extend {
            [_, true] => Fixed::from_i64(1),
            [true, false] => none,
            [false, false] => return Ok(Box::new(Blank)),
        };
        Matrix {
            terms: [none, none, none, none, end, none],
        }
    } else {
        let [across, down] = way;
        let axis = Matrix {
            terms: [
                across,
                down,
                none.minus(down),
                across,
                from_across,
                from_down,
            ],
        };
        let Some(found) = axis.then(&common.matrix).inverse() else {
            return Ok(Box::new(Blank));
        };
        found
    };
    Ok(Box::new(Axial {
        extend,
        lookup,
        ramp,
    }))
}

/// A shading of type 1 of `common` (8.7.4.5.2).
fn computed(job: &Job<'_>, common: &Common<'_>) -> Result<Box<dyn Paint>, PdfError> {
    let owner = common.owner.value();
    let (none, one) = (Fixed::default(), Fixed::from_i64(1));
    let domain = data::numbers(job, owner, b"Domain")?.unwrap_or([none, one, none, one]);
    let matrix = data::matrix(job, owner, b"Matrix")?;
    let function = common
        .function(job, 2)?
        .ok_or_else(|| job.objects.malformed())?;
    let Some(lookup) = matrix.then(&common.matrix).inverse() else {
        return Ok(Box::new(Blank));
    };
    Ok(Box::new(Computed {
        converter: Converter::new(job, &common.space, common.intent)?,
        domain,
        values: vec![none; function.outputs()],
        function,
        lookup,
    }))
}

/// The sum of `pairs` multiplied, rounded to 32 fraction bits.
fn product(pairs: [(i128, i128); 3]) -> i128 {
    let sum = pairs.into_iter().fold(0, |sum: i128, (left, right)| {
        sum.saturating_add(left.saturating_mul(right))
    });
    sum.saturating_add(1 << 31_u32) >> 32_u32
}

/// A radial shading of `common` (8.7.4.5.4).
fn radial(job: &Job<'_>, common: &Common<'_>) -> Result<Box<dyn Paint>, PdfError> {
    let owner = common.owner.value();
    let [from_across, from_down, first, to_across, to_down, second] =
        data::numbers(job, owner, b"Coords")?.ok_or_else(|| job.objects.malformed())?;
    let none = Fixed::default();
    if first < none || second < none {
        return Err(job.objects.malformed());
    }
    let (extend, ramp) = varied(job, common)?;
    let way = [
        to_across.minus(from_across),
        to_down.minus(from_down),
        second.minus(first),
    ];
    if way == [none; 3] {
        return Ok(Box::new(Blank));
    }
    let [across, down, _] = way;
    let size = across
        .max(none.minus(across))
        .max(down)
        .max(none.minus(down))
        .max(first)
        .max(second);
    let unit = Matrix {
        terms: [size, none, none, size, from_across, from_down],
    };
    let Some(lookup) = unit.then(&common.matrix).inverse() else {
        return Ok(Box::new(Blank));
    };
    let scaled = |value: Fixed| i128::from(value.over(size).unwrap_or_default().bits());
    let moved = way.map(scaled);
    let [centre_across, centre_down, widening] = moved;
    let bend = product([
        (centre_across, centre_across),
        (centre_down, centre_down),
        (widening, widening.saturating_neg()),
    ]);
    Ok(Box::new(Radial {
        // Within rounding of 0 the circles share a tangent.
        bend: if bend.unsigned_abs() <= TANGENT {
            0
        } else {
            bend
        },
        extend,
        lookup,
        moved,
        radius: scaled(first),
        ramp,
    }))
}

/// The shading of type `kind`, 1 to 3, of `common`.
pub(super) fn read(
    job: &Job<'_>,
    common: &Common<'_>,
    kind: i64,
) -> Result<Box<dyn Paint>, PdfError> {
    match kind {
        1 => computed(job, common),
        2 => axial(job, common),
        _ => radial(job, common),
    }
}

/// A pixel of `color`.
const fn solid(color: [u16; 3]) -> Sample {
    Sample {
        color,
        opacity: u16::MAX,
        shape: u16::MAX,
    }
}

/// `Extend` and the colours over `Domain`, which types 2 and 3 share.
fn varied(job: &Job<'_>, common: &Common<'_>) -> Result<([bool; 2], Ramp), PdfError> {
    let owner = common.owner.value();
    let broken = || job.objects.malformed();
    let mut extend = [false; 2];
    let listed = data::array(job, owner, b"Extend", extend.len())?;
    if !listed.is_empty() {
        if listed.len() != extend.len() {
            return Err(broken());
        }
        for (slot, item) in extend.iter_mut().zip(&listed) {
            if !matches!(*item.value(), Object::Boolean(_)) {
                return Err(broken());
            }
            *slot = matches!(*item.value(), Object::Boolean(true));
        }
    }
    let ends = data::numbers(job, owner, b"Domain")?
        .unwrap_or_else(|| [Fixed::default(), Fixed::from_i64(1)]);
    let function = common.function(job, 1)?.ok_or_else(broken)?;
    Ok((extend, Ramp::new(job, common, function, ends)?))
}
