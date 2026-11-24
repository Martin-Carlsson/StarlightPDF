//! Signed 31.32 fixed point that saturates, and unit values of which 65,535 is one.

use crate::overlay::Object;

/// Fraction bits.
const BITS: u32 = 32;

/// Half of the last bit a product keeps.
const HALF: i128 = 0x8000_0000;

/// A number with 32 fraction bits; every operation saturates.
#[derive(Clone, Copy, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Fixed(i64);

impl Fixed {
    /// The value times 2 to the 32.
    pub(crate) const fn bits(self) -> i64 {
        self.0
    }

    /// The largest integer not above it.
    pub(crate) const fn floor(self) -> i64 {
        self.0 >> BITS
    }

    /// The number whose value times 2 to the 32 is `bits`.
    pub(crate) const fn from_bits(bits: i64) -> Self {
        Self(bits)
    }

    /// The nearest to `value`, saturated; 0 when not finite.
    pub(crate) fn from_f64(value: f64) -> Self {
        let bits = value.to_bits();
        let exponent = i64::try_from((bits >> 52_u32) & 0x7FF).unwrap_or(0);
        if exponent == 0 || exponent == 0x7FF {
            return Self(0);
        }
        let mantissa = (bits & 0x000F_FFFF_FFFF_FFFF) | 0x0010_0000_0000_0000;
        // The mantissa counts units of 2 to the exponent less 1075.
        let shift = exponent
            .saturating_sub(1075)
            .saturating_add(i64::from(BITS));
        let magnitude = match u32::try_from(shift) {
            Ok(left) if left <= 10 => mantissa.wrapping_shl(left),
            Ok(_) => u64::MAX,
            Err(_) => match u32::try_from(shift.saturating_neg()) {
                Ok(right) if right < 64 => {
                    let half = 1_u64.wrapping_shl(right.saturating_sub(1));
                    mantissa.saturating_add(half).wrapping_shr(right)
                }
                Ok(_) | Err(_) => 0,
            },
        };
        let held = i64::try_from(magnitude).unwrap_or(i64::MAX);
        Self(if value.is_sign_negative() {
            held.saturating_neg()
        } else {
            held
        })
    }

    /// The integer `value`, saturated.
    pub(crate) const fn from_i64(value: i64) -> Self {
        Self(value.saturating_mul(1_i64 << BITS))
    }

    /// The number `value`; `None` for another object.
    pub(crate) fn from_object(value: &Object) -> Option<Self> {
        match *value {
            Object::Integer(integer) => Some(Self::from_i64(integer)),
            Object::Real(real) => Some(Self::from_f64(real)),
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Dictionary(_)
            | Object::Name(_)
            | Object::Null
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_) => None,
        }
    }

    /// The difference.
    pub(crate) const fn minus(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// The quotient, rounded to the nearest, halves up; `None` by zero.
    pub(crate) fn over(self, other: Self) -> Option<Self> {
        quotient(i128::from(self.0) << BITS, i128::from(other.0)).map(|bits| Self(saturated(bits)))
    }

    /// The pixel coordinate: 8 fraction bits, rounded to the nearest, halves up.
    pub(crate) fn pixel(self) -> i32 {
        let rounded = self.subpixel();
        i32::try_from(rounded).unwrap_or(if rounded < 0 { i32::MIN } else { i32::MAX })
    }

    /// The sum.
    pub(crate) const fn plus(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// [`Fixed::pixel`] before it saturates to 32 bits.
    pub(crate) const fn subpixel(self) -> i64 {
        self.0.saturating_add(0x80_0000) >> 24_u32
    }

    /// The product, rounded to the nearest, halves up.
    pub(crate) fn times(self, other: Self) -> Self {
        let product = i128::from(self.0).saturating_mul(i128::from(other.0));
        Self(saturated(product.saturating_add(HALF) >> BITS))
    }

    /// The value clamped to 0 to 1, as a unit value.
    pub(crate) fn unit(self) -> u16 {
        let one = 1_i64 << BITS;
        let scaled = self.0.clamp(0, one).saturating_mul(0xFFFF);
        u16::try_from(scaled.saturating_add(1_i64 << 31_u32) >> BITS).unwrap_or(u16::MAX)
    }
}

/// The unit value `value` in 8 bits, rounded to the nearest.
pub(crate) fn byte(value: u16) -> u8 {
    let wide = u32::from(value).wrapping_add(128);
    u8::try_from(wide.wrapping_sub(wide >> 8_u32) >> 8_u32).unwrap_or(u8::MAX)
}

/// `dividend` over `divisor`, rounded to the nearest, halves up; `None` by zero.
pub(crate) fn quotient(dividend: i128, divisor: i128) -> Option<i128> {
    let (above, below) = if divisor < 0 {
        (dividend.checked_neg()?, divisor.checked_neg()?)
    } else {
        (dividend, divisor)
    };
    let floor = above.checked_div_euclid(below)?;
    let rest = above.checked_rem_euclid(below)?;
    let up = rest > 0 && rest >= below.saturating_sub(rest);
    floor.checked_add(i128::from(up))
}

/// `bits` saturated to 64.
pub(crate) fn saturated(bits: i128) -> i64 {
    i64::try_from(bits).unwrap_or(if bits < 0 { i64::MIN } else { i64::MAX })
}

/// The product of two unit values, rounded exactly.
pub(crate) fn unit(left: u16, right: u16) -> u16 {
    let wide = u32::from(left)
        .wrapping_mul(u32::from(right))
        .wrapping_add(0x8000);
    u16::try_from(wide.wrapping_add(wide >> 16_u32) >> 16_u32).unwrap_or(u16::MAX)
}
