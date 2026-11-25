//! CIE-based colour: curves, L*a*b* and XYZ to sRGB (ISO 32000-2:2020, 8.6.5; IEC 61966-2-1).
//! Values in a conversion have 16 fraction bits.

use alloc::rc::Rc;
use std::sync::OnceLock;

use crate::PdfError;
use crate::render::colour::{Convert, Intent};
use crate::render::fixed::{Fixed, quotient, saturated};
use crate::render::function::{elementary, room};
use crate::render::job::Job;

/// Work of a colour from L*a*b*: cubes and a matrix.
pub(super) const LAB_WORK: u64 = 8;

/// The cone responses of Bradford's matrix.
const CONES: Square = [
    [bits(3_844_425_227), bits(1_144_179_288), bits(-693_207_722)],
    [bits(-3_222_084_465), bits(7_359_426_462), bits(157_625_300)],
    [bits(167_074_228), bits(-294_205_260), bits(4_422_098_328)],
];

/// What linear coefficients are kept below: 4,096 with 24 fraction bits.
const COEFFICIENT: i64 = 1 << 36_u32;

/// One, with 16 fraction bits.
pub(super) const ONE: i64 = 0x1_0000;

/// What linear values are kept below.
const REACH: i64 = 1 << 22_u32;

/// The steps of a compiled curve.
const STEPS: usize = 4096;

/// From cone responses over the white's to linear sRGB: IEC 61966-2-1 and Bradford's adaptation
/// to D65, each row summing to one, so the white is white.
const TO_SRGB: Square = [
    [
        bits(10_262_338_674),
        bits(-5_779_083_798),
        bits(-188_287_580),
    ],
    [bits(-590_351_917), bits(4_989_808_626), bits(-104_489_413)],
    [bits(-171_030_346), bits(-319_981_334), bits(4_785_978_976)],
];

/// The work of a compiled entry.
const WORK: u64 = 32;

/// The transfer function of sRGB in [`STEPS`] steps, for every render.
static ENCODING: OnceLock<[u16; STEPS + 1]> = OnceLock::new();

/// A 3 x 3 matrix, as rows or as columns.
pub(super) type Square = [[Fixed; 3]; 3];

/// A `CalGray` or `CalRGB` space (8.6.5.2, 8.6.5.3).
pub(super) struct Calibrated {
    /// The gamma curve of each component.
    pub curves: Vec<Rc<Curve>>,
    /// To RGB.
    pub linear: Linear,
}

/// A curve from 0 to 1, sampled evenly.
pub(super) struct Curve {
    /// The samples, 65,535 as one.
    samples: Vec<u16>,
}

/// A Lab space (8.6.5.4).
pub(super) struct Lab {
    /// To RGB from what each component gives of the white's.
    pub linear: Linear,
}

/// A linear map to linear sRGB, then its transfer function.
pub(super) struct Linear {
    /// The transfer function.
    encoding: &'static [u16],
    /// The map, with 24 fraction bits.
    rows: [[i64; 3]; 3],
}

impl Convert for Calibrated {
    fn rgb(&self, _job: &Job<'_>, values: &[Fixed], _intent: Intent) -> Result<[u16; 3], PdfError> {
        let mut held = [0_i64; 3];
        for ((slot, &value), curve) in held.iter_mut().zip(values).zip(&self.curves) {
            *slot = curve.at(sixteenths(value));
        }
        Ok(self.linear.rgb(held))
    }
}

impl Convert for Lab {
    fn rgb(&self, job: &Job<'_>, values: &[Fixed], _intent: Intent) -> Result<[u16; 3], PdfError> {
        job.spend(LAB_WORK)?;
        let mut held = [0_i64; 3];
        for (slot, &value) in held.iter_mut().zip(values) {
            *slot = sixteenths(value);
        }
        Ok(self.linear.rgb(lab(held)))
    }
}

impl Curve {
    /// The value at `at`, clamped to the curve.
    pub(super) fn at(&self, at: i64) -> i64 {
        let last = i64::try_from(self.samples.len())
            .unwrap_or(1)
            .saturating_sub(1);
        let place = at.clamp(0, ONE).saturating_mul(last);
        let index = usize::try_from(place >> 16_u32).unwrap_or(0);
        let near = i64::from(self.samples.get(index).copied().unwrap_or_default());
        let after = self.samples.get(index.saturating_add(1)).copied();
        let way = after.map_or(0, i64::from).saturating_sub(near);
        let mixed = if after.is_some() {
            way.saturating_mul(place & 0xFFFF)
        } else {
            0
        };
        widened((near << 16_u32).saturating_add(mixed))
    }

    /// The bytes it holds.
    pub(super) fn bytes(&self) -> u64 {
        u64::try_from(self.samples.len().saturating_mul(2)).unwrap_or(u64::MAX)
    }

    /// The curve of `value` in [`STEPS`] steps.
    pub(super) fn compiled(
        job: &Job<'_>,
        value: &dyn Fn(Fixed) -> Fixed,
    ) -> Result<Self, PdfError> {
        let mut samples = room(STEPS.saturating_add(1))?;
        job.spend(WORK.saturating_mul(u64::try_from(STEPS).unwrap_or(0)))?;
        samples
            .extend((0_i64..=4096_i64).map(|step| value(Fixed::from_bits(step << 20_u32)).unit()));
        Ok(Self { samples })
    }

    /// The curve that changes nothing.
    pub(super) fn identity() -> Self {
        Self {
            samples: vec![0, u16::MAX],
        }
    }

    /// The curve x^`gamma`, `gamma` above 0.
    pub(super) fn power(job: &Job<'_>, gamma: Fixed) -> Result<Self, PdfError> {
        if gamma == Fixed::from_i64(1) {
            return Ok(Self::identity());
        }
        Self::compiled(job, &|value| {
            elementary::power(value, gamma).unwrap_or_default()
        })
    }

    /// The curve through `samples`.
    pub(super) const fn sampled(samples: Vec<u16>) -> Self {
        Self { samples }
    }
}

impl Linear {
    /// From the factors of `columns`, XYZ under the white point `white`; `None` for a white that
    /// cannot be one.
    pub(super) fn new(white: [Fixed; 3], columns: &Square) -> Option<Self> {
        let nought = Fixed::default();
        let cones = CONES.map(|row| product(row, white));
        if white.iter().chain(&cones).any(|part| *part <= nought) {
            return None;
        }
        let mut rows = [[0_i64; 3]; 3];
        for (at, column) in columns.iter().enumerate() {
            let mut adapted = [nought; 3];
            for ((slot, row), &below) in adapted.iter_mut().zip(CONES).zip(&cones) {
                *slot = product(row, *column).over(below)?;
            }
            for (row, weights) in rows.iter_mut().zip(TO_SRGB) {
                if let Some(slot) = row.get_mut(at) {
                    let found = product(weights, adapted).bits().saturating_add(128) >> 8_u32;
                    *slot = found.clamp(COEFFICIENT.saturating_neg(), COEFFICIENT);
                }
            }
        }
        Some(Self {
            encoding: encoding(),
            rows,
        })
    }

    /// The colour of `values`.
    pub(super) fn rgb(&self, values: [i64; 3]) -> [u16; 3] {
        let [first, second, third] = values.map(|value| value.clamp(REACH.saturating_neg(), REACH));
        self.rows.map(|[of_first, of_second, of_third]| {
            let sum = of_first
                .saturating_mul(first)
                .saturating_add(of_second.saturating_mul(second))
                .saturating_add(of_third.saturating_mul(third));
            let linear = (sum.saturating_add(1 << 23_u32) >> 24_u32).clamp(0, ONE);
            let index = usize::try_from(linear >> 4_u32).unwrap_or(0);
            let near = self.encoding.get(index).copied().unwrap_or_default();
            let after = self.encoding.get(index.saturating_add(1)).copied();
            let way = i64::from(after.unwrap_or(near)).saturating_sub(i64::from(near));
            let mixed = (i64::from(near) << 4_u32)
                .saturating_add(way.saturating_mul(linear & 15))
                .saturating_add(8);
            u16::try_from(mixed >> 4_u32).unwrap_or(u16::MAX)
        })
    }
}

/// The number of `held` with 32 fraction bits.
const fn bits(held: i64) -> Fixed {
    Fixed::from_bits(held)
}

/// The columns of a diagonal matrix.
pub(super) fn diagonal([first, second, third]: [Fixed; 3]) -> Square {
    let nought = Fixed::default();
    [
        [first, nought, nought],
        [nought, second, nought],
        [nought, nought, third],
    ]
}

/// The transfer function of sRGB, worked out once.
fn encoding() -> &'static [u16] {
    ENCODING.get_or_init(|| {
        let mut table = [0_u16; STEPS + 1];
        // 1.055, 12.92, 0.0031308 and 1 / 2.4.
        let [scale, slope, knee, exponent] =
            [0x1_0E14_7AE1, 0xC_EB85_1EB8, 0x00CD_2E1C, 0x6AAA_AAAB].map(Fixed::from_bits);
        let offset = scale.minus(Fixed::from_i64(1));
        for (slot, step) in table.iter_mut().zip(0_i64..) {
            let value = Fixed::from_bits(step << 20_u32);
            let found = if value <= knee {
                value.times(slope)
            } else {
                let raised = elementary::power(value, exponent).unwrap_or_default();
                scale.times(raised).minus(offset)
            };
            *slot = found.unit();
        }
        table
    })
}

/// What L*, a* and b* give of the white's X, Y and Z (8.6.5.4).
pub(super) fn lab([lightness, red, blue]: [i64; 3]) -> [i64; 3] {
    let reach = 1_i64 << 26_u32;
    let part = |value: i64, over: i64| {
        value
            .clamp(reach.saturating_neg(), reach)
            .saturating_mul(over)
            >> 24_u32
    };
    // 1 / 116, 1 / 500 and 1 / 200 with 32 fraction bits.
    let middle = part(lightness.saturating_add(16 << 16_u32), 0x0234_F72C);
    let across = middle.saturating_add(part(red, 0x0083_126F));
    let down = middle.saturating_sub(part(blue, 0x0147_AE14));
    [across, middle, down].map(|value| {
        let held = value.clamp(reach.saturating_neg(), reach);
        // Above 6 / 29 the cube, else 108 / 841 (x - 4 / 29), with 24 fraction bits.
        let found = if held >= 0x0034_F72C {
            (held.saturating_mul(held) >> 24_u32).saturating_mul(held) >> 24_u32
        } else {
            held.saturating_sub(0x0023_4F73).saturating_mul(0x0020_E00A) >> 24_u32
        };
        found.saturating_add(128) >> 8_u32
    })
}

/// The sum of the products of `left` and `right`, pair by pair.
fn product(left: [Fixed; 3], right: [Fixed; 3]) -> Fixed {
    let sum = left.iter().zip(&right).fold(0_i128, |sum, (one, other)| {
        sum.saturating_add(i128::from(one.bits()).saturating_mul(i128::from(other.bits())))
    });
    Fixed::from_bits(saturated(quotient(sum, 1 << 32_u32).unwrap_or(0)))
}

/// `value` with 16 fraction bits.
pub(super) const fn sixteenths(value: Fixed) -> i64 {
    value.bits().saturating_add(0x8000) >> 16_u32
}

/// A sample, 65,535 as one, times 65,536, as a value with one as 65,536.
pub(super) const fn widened(mixed: i64) -> i64 {
    mixed.saturating_add(mixed >> 16_u32).saturating_add(0x8000) >> 16_u32
}
