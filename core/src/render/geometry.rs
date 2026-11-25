//! Points, affine maps and pixel areas (ISO 32000-2:2020, 8.3).

use crate::render::fixed::{Fixed, quotient, saturated};

/// Pixels, the bottom and the right excluded.
#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(crate) struct Area {
    /// Past the last row.
    pub bottom: i32,
    /// The first column.
    pub left: i32,
    /// Past the last column.
    pub right: i32,
    /// The first row.
    pub top: i32,
}

/// An affine map: x' = a x + c y + e, y' = b x + d y + f.
#[derive(Clone, Copy)]
pub(crate) struct Matrix {
    /// a, b, c, d, e and f, as `cm` takes them.
    pub terms: [Fixed; 6],
}

/// A point or a vector.
#[derive(Clone, Copy, Default)]
pub(crate) struct Point {
    /// x.
    pub across: Fixed,
    /// y.
    pub down: Fixed,
}

impl Area {
    /// Rows.
    pub(crate) fn height(self) -> usize {
        usize::try_from(self.bottom.saturating_sub(self.top)).unwrap_or(0)
    }

    /// The pixels in both; none is an area without rows or columns.
    pub(crate) fn intersect(self, other: Self) -> Self {
        let (left, top) = (self.left.max(other.left), self.top.max(other.top));
        Self {
            bottom: self.bottom.min(other.bottom).max(top),
            left,
            right: self.right.min(other.right).max(left),
            top,
        }
    }

    /// Whether it has no pixels.
    pub(crate) const fn is_empty(self) -> bool {
        self.bottom <= self.top || self.right <= self.left
    }

    /// The least area holding both; the other where one has no pixels.
    pub(crate) fn joined(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Self {
            bottom: self.bottom.max(other.bottom),
            left: self.left.min(other.left),
            right: self.right.max(other.right),
            top: self.top.min(other.top),
        }
    }

    /// Columns.
    pub(crate) fn width(self) -> usize {
        usize::try_from(self.right.saturating_sub(self.left)).unwrap_or(0)
    }
}

impl Matrix {
    /// Where `point` maps to.
    pub(crate) fn apply(&self, point: Point) -> Point {
        let [_, _, _, _, across, down] = self.terms;
        let turned = self.linear(point);
        Point {
            across: turned.across.plus(across),
            down: turned.down.plus(down),
        }
    }

    /// The pixels the rectangle `corners`, left, bottom, right and top, touches.
    pub(crate) fn bounds(&self, corners: [Fixed; 4]) -> Area {
        let [left, top, right, bottom] = self.extent(corners);
        Area {
            bottom: past(bottom),
            left: first_pixel(left),
            right: past(right),
            top: first_pixel(top),
        }
    }

    /// The least x and y, then the most, of where the rectangle `corners` maps to.
    pub(crate) fn extent(&self, corners: [Fixed; 4]) -> [Fixed; 4] {
        let [left, bottom, right, top] = corners;
        let mapped = [(left, bottom), (right, bottom), (right, top), (left, top)]
            .map(|(across, down)| self.apply(Point { across, down }));
        let [first, _, _, _] = mapped;
        let (mut low, mut high) = (first, first);
        for point in mapped {
            low.across = low.across.min(point.across);
            low.down = low.down.min(point.down);
            high.across = high.across.max(point.across);
            high.down = high.down.max(point.down);
        }
        [low.across, low.down, high.across, high.down]
    }

    /// The map that changes nothing.
    pub(crate) const fn identity() -> Self {
        let (one, none) = (Fixed::from_i64(1), Fixed::from_i64(0));
        Self {
            terms: [one, none, none, one, none, none],
        }
    }

    /// The inverse; `None` when singular or out of range.
    pub(crate) fn inverse(&self) -> Option<Self> {
        let [aa, bb, cc, dd, ee, ff] = self.terms.map(|term| i128::from(term.bits()));
        let minus = |plus: [i128; 2], less: [i128; 2]| {
            let [first, second] = plus;
            let [third, fourth] = less;
            first
                .checked_mul(second)?
                .checked_sub(third.checked_mul(fourth)?)
        };
        // 64 fraction bits.
        let determinant = minus([aa, dd], [bb, cc])?;
        let linear = |term: i128| quotient(term.checked_mul(1_i128 << 64_u32)?, determinant);
        let moved = |term: i128| {
            term.checked_mul(1_i128 << 32_u32).map_or_else(
                // Too large to take the bits: the determinant gives its low ones up instead.
                || quotient(term, determinant >> 32_u32),
                |exact| quotient(exact, determinant),
            )
        };
        let found = [
            linear(dd)?,
            linear(bb.checked_neg()?)?,
            linear(cc.checked_neg()?)?,
            linear(aa)?,
            moved(minus([cc, ff], [dd, ee])?)?,
            moved(minus([bb, ee], [aa, ff])?)?,
        ];
        let mut terms = [Fixed::default(); 6];
        for (slot, bits) in terms.iter_mut().zip(found) {
            let held = saturated(bits);
            if i128::from(held) != bits {
                return None;
            }
            *slot = Fixed::from_bits(held);
        }
        Some(Self { terms })
    }

    /// Where the vector `vector` maps to.
    pub(crate) fn linear(&self, vector: Point) -> Point {
        let [aa, bb, cc, dd, _, _] = self.terms;
        Point {
            across: vector.across.times(aa).plus(vector.down.times(cc)),
            down: vector.across.times(bb).plus(vector.down.times(dd)),
        }
    }

    /// This map, then `after`.
    pub(crate) fn then(&self, after: &Self) -> Self {
        let [aa, bb, cc, dd, ee, ff] = self.terms;
        let turned = |across, down| after.linear(Point { across, down });
        let (first, second) = (turned(aa, bb), turned(cc, dd));
        let origin = after.apply(Point {
            across: ee,
            down: ff,
        });
        Self {
            terms: [
                first.across,
                first.down,
                second.across,
                second.down,
                origin.across,
                origin.down,
            ],
        }
    }
}

/// The pixel `value` lies in.
fn first_pixel(value: Fixed) -> i32 {
    i32::try_from(value.floor()).unwrap_or(i32::MAX)
}

/// The pixel after the last that the span up to `value` touches.
fn past(value: Fixed) -> i32 {
    let last = Fixed::from_bits(value.bits().saturating_add(0xFFFF_FFFF));
    i32::try_from(last.floor()).unwrap_or(i32::MAX)
}
