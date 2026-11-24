//! Square roots, lengths and directions in fixed point.

use crate::render::fixed::{Fixed, saturated};
use crate::render::geometry::Point;

/// The significant bits a vector gets before its direction is taken.
const DIRECTION_BITS: u32 = 46;

/// The length of `vector`.
pub(crate) fn hypotenuse(vector: Point) -> Fixed {
    let square = |part: Fixed| {
        let length = u128::from(part.bits().unsigned_abs());
        length.saturating_mul(length)
    };
    let sum = square(vector.across).saturating_add(square(vector.down));
    Fixed::from_bits(saturated(i128::try_from(sum.isqrt()).unwrap_or(i128::MAX)))
}

/// `vector` turned half round.
pub(crate) const fn opposite(vector: Point) -> Point {
    let none = Fixed::from_bits(0);
    Point {
        across: none.minus(vector.across),
        down: none.minus(vector.down),
    }
}

/// `vector` times a power of two that leaves its direction with enough bits to map.
pub(crate) fn scaled(vector: Point) -> Point {
    let largest = vector
        .across
        .bits()
        .unsigned_abs()
        .max(vector.down.bits().unsigned_abs());
    if largest == 0 {
        return vector;
    }
    let used = u64::BITS.saturating_sub(largest.leading_zeros());
    let shift = |part: Fixed| {
        Fixed::from_bits(if used < DIRECTION_BITS {
            part.bits()
                .wrapping_shl(DIRECTION_BITS.saturating_sub(used))
        } else {
            part.bits()
                .wrapping_shr(used.saturating_sub(DIRECTION_BITS))
        })
    };
    Point {
        across: shift(vector.across),
        down: shift(vector.down),
    }
}

/// The square root of `value`; 0 of a negative.
pub(crate) fn root(value: Fixed) -> Fixed {
    let wide = u128::try_from(value.bits()).unwrap_or(0) << 32_u32;
    Fixed::from_bits(saturated(i128::try_from(wide.isqrt()).unwrap_or(i128::MAX)))
}

/// `vector` turned a quarter, from across to down.
pub(crate) const fn turned(vector: Point) -> Point {
    Point {
        across: Fixed::from_bits(0).minus(vector.down),
        down: vector.across,
    }
}

/// The direction of `vector`, one long; `None` of none.
pub(crate) fn unit(vector: Point) -> Option<Point> {
    let long = scaled(vector);
    let length = hypotenuse(long);
    Some(Point {
        across: long.across.over(length)?,
        down: long.down.over(length)?,
    })
}
