//! Elementary functions in fixed point (ISO 32000-2:2020, 7.10.3, 7.10.5): worked out with 60
//! fraction bits, rounded to 32.

use crate::render::fixed::{Fixed, quotient, saturated};

/// Fraction bits while working.
const BITS: u32 = 60;

/// Degrees in a radian.
const DEGREES: i128 = 0x0003_94BB_834C_783E_F70C;

/// One over the natural logarithm of 2.
const INVERSE_LN_2: i128 = 0x1715_4765_2B82_FE17;

/// The natural logarithm of 2.
const LN_2: i128 = 0x0B17_217F_7D1C_F79B;

/// The common logarithm of 2.
const LOG10_2: i128 = 0x04D1_04D4_27DE_7FBD;

/// One while working.
const ONE: i128 = 0x1000_0000_0000_0000;

/// Radians in a degree.
const RADIANS: i128 = 0x0047_7D1A_894A_74E4;

/// The square root of 2.
const ROOT_2: i128 = 0x16A0_9E66_7F3B_CC91;

/// The tangent of an eighth of pi.
const TANGENT: i128 = 0x06A0_9E66_7F3B_CC91;

/// The angle of `rise` over `run` in degrees, 0 to 360; `None` of 0 over 0.
pub(crate) fn arc_tangent(rise: Fixed, run: Fixed) -> Option<Fixed> {
    let up = i128::from(rise.bits().unsigned_abs());
    let along = i128::from(run.bits().unsigned_abs());
    let tangent = quotient(up.min(along) << BITS, up.max(along))?;
    let (base, reduced) = if tangent > TANGENT {
        let below = tangent.saturating_sub(ONE);
        (45, quotient(below << BITS, tangent.saturating_add(ONE))?)
    } else {
        (0, tangent)
    };
    let mut angle = scaled(odd_series(reduced, true), DEGREES).saturating_add(whole(base));
    if up > along {
        angle = whole(90).saturating_sub(angle);
    }
    if run.bits() < 0 {
        angle = whole(180).saturating_sub(angle);
    }
    if rise.bits() < 0 {
        angle = whole(360).saturating_sub(angle);
    }
    let found = rounded(angle);
    Some(if found == Fixed::from_i64(360) {
        Fixed::default()
    } else {
        found
    })
}

/// The cosine of `degrees`.
pub(crate) fn cosine(degrees: Fixed) -> Fixed {
    sine_of(turn(degrees).saturating_add(Fixed::from_i64(90).bits()))
}

/// The common logarithm; `None` unless `value` is above 0.
pub(crate) fn decimal(value: Fixed) -> Option<Fixed> {
    binary(value).map(|power| rounded(scaled(power, LOG10_2)))
}

/// The natural logarithm; `None` unless `value` is above 0.
pub(crate) fn natural(value: Fixed) -> Option<Fixed> {
    binary(value).map(|power| rounded(scaled(power, LN_2)))
}

/// `base` to the `exponent`, saturated; `None` of a negative to a fraction and of 0 to a
/// negative.
pub(crate) fn power(base: Fixed, exponent: Fixed) -> Option<Fixed> {
    let one = Fixed::from_i64(1);
    let nought = Fixed::default();
    if base < nought || exponent == Fixed::from_i64(exponent.floor()) {
        return whole_power(base, exponent);
    }
    if base == nought {
        return (exponent > nought).then_some(nought);
    }
    if base == one {
        return Some(one);
    }
    let product = i128::from(exponent.bits()).saturating_mul(binary(base)?);
    Some(exponential(product >> 32_u32))
}

/// The sine of `degrees`.
pub(crate) fn sine(degrees: Fixed) -> Fixed {
    sine_of(turn(degrees))
}

/// The logarithm to base 2, with the working bits; `None` unless `value` is above 0.
fn binary(value: Fixed) -> Option<i128> {
    let bits = u64::try_from(value.bits()).ok()?;
    let top = bits.checked_ilog2()?;
    let mut mantissa = i128::from(bits) << BITS >> top;
    let mut exponent = i64::from(top).saturating_sub(32);
    if mantissa > ROOT_2 {
        mantissa >>= 1_u32;
        exponent = exponent.saturating_add(1);
    }
    let below = mantissa.saturating_sub(ONE);
    let part = quotient(below << BITS, mantissa.saturating_add(ONE))?;
    let fraction = scaled(odd_series(part, false) << 1_u32, INVERSE_LN_2);
    Some(whole(exponent).saturating_add(fraction))
}

/// 2 to the `power`, given with the working bits.
fn exponential(power: i128) -> Fixed {
    let whole_part = power >> BITS;
    let natural_part = scaled(power.saturating_sub(whole_part << BITS), LN_2);
    let (mut sum, mut term) = (ONE, ONE);
    for at in 1_i128..=20_i128 {
        term = scaled(term, natural_part).checked_div(at).unwrap_or(0);
        sum = sum.saturating_add(term);
    }
    // The sum has 60 fraction bits, the result 32.
    let drop = 28_i128.saturating_sub(whole_part);
    if drop <= -3 {
        return Fixed::from_bits(i64::MAX);
    }
    if drop >= 100 {
        return Fixed::default();
    }
    let Ok(right) = u32::try_from(drop) else {
        let left = u32::try_from(drop.saturating_neg()).unwrap_or(0);
        return Fixed::from_bits(saturated(sum.wrapping_shl(left)));
    };
    let half = 1_i128.wrapping_shl(right) >> 1_u32;
    Fixed::from_bits(saturated(sum.saturating_add(half).wrapping_shr(right)))
}

/// The sum of x^k / k over odd k, of alternating sign when `alternating`: the inverse hyperbolic
/// and plain tangents, for x of at most the square root of 2 less 1.
fn odd_series(value: i128, alternating: bool) -> i128 {
    let square = scaled(value, value);
    let (mut sum, mut term) = (value, value);
    for odd in (3_i128..50_i128).step_by(2) {
        term = scaled(term, square);
        if alternating {
            term = term.saturating_neg();
        }
        sum = sum.saturating_add(term.checked_div(odd).unwrap_or(0));
    }
    sum
}

/// A working value as fixed point, rounded.
fn rounded(value: i128) -> Fixed {
    Fixed::from_bits(saturated(value.saturating_add(0x0800_0000) >> 28_u32))
}

/// The product of two working values.
const fn scaled(left: i128, right: i128) -> i128 {
    left.saturating_mul(right).saturating_add(ONE >> 1_u32) >> BITS
}

/// The sine of an angle of `bits`, from 0 to 450 degrees.
fn sine_of(bits: i64) -> Fixed {
    let [quarter, half, all] = [90, 180, 360].map(|degrees| Fixed::from_i64(degrees).bits());
    let mut angle = if bits >= all {
        bits.saturating_sub(all)
    } else {
        bits
    };
    let negative = angle >= half;
    if negative {
        angle = angle.saturating_sub(half);
    }
    if angle > quarter {
        angle = half.saturating_sub(angle);
    }
    let complement = angle > quarter >> 1_u32;
    if complement {
        angle = quarter.saturating_sub(angle);
    }
    let radians = i128::from(angle).saturating_mul(RADIANS) >> 32_u32;
    let square = scaled(radians, radians);
    let (mut term, first) = if complement {
        (ONE, 1_i128)
    } else {
        (radians, 2_i128)
    };
    let mut sum = term;
    for at in (first..first.saturating_add(20)).step_by(2) {
        let below = at.saturating_mul(at.saturating_add(1));
        term = scaled(term, square).checked_div(below).unwrap_or(0);
        term = term.saturating_neg();
        sum = sum.saturating_add(term);
    }
    let found = rounded(sum);
    if negative {
        Fixed::default().minus(found)
    } else {
        found
    }
}

/// The bits of `degrees` within a turn.
fn turn(degrees: Fixed) -> i64 {
    let all = Fixed::from_i64(360).bits();
    degrees.bits().checked_rem_euclid(all).unwrap_or(0)
}

/// `value` as a working value.
fn whole(value: i64) -> i128 {
    i128::from(value) << BITS
}

/// `base` to a whole `exponent`; `None` for a fraction and of 0 to a negative.
fn whole_power(base: Fixed, exponent: Fixed) -> Option<Fixed> {
    let times = exponent.floor();
    let one = Fixed::from_i64(1);
    if exponent != Fixed::from_i64(times) || (times < 0 && base == Fixed::default()) {
        return None;
    }
    let small = base.bits().unsigned_abs() < one.bits().unsigned_abs();
    let inverted = times < 0 && small;
    let mut square = if inverted { one.over(base)? } else { base };
    let mut left = times.unsigned_abs();
    let mut found = one;
    while left > 0 {
        if left & 1 == 1 {
            found = found.times(square);
        }
        square = square.times(square);
        left >>= 1_u32;
    }
    if times < 0 && !inverted {
        return Some(one.over(found).unwrap_or_default());
    }
    Some(found)
}
