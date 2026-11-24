//! Type 1 and Type 2 charstrings (Adobe Type 1 Font Format, 6 and 8; Technical Note 5177).

use core::mem;

use crate::font_program::{Fault, ONE, Outline, coordinate, encoded, times};
use crate::pull::number;
use crate::render::fixed::{quotient, saturated};

/// Subroutines inside each other (TN 5177, Appendix B).
const DEPTH: usize = 10;

/// Points of one glyph.
const POINTS: u32 = 0x1_0000;

/// Operands held; the standards allow 48.
const STACK: usize = 96;

/// Numbers and operators of one glyph.
const STEPS: u32 = 0x1_0000;

/// Which language a charstring is in.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Dialect {
    /// Type 1.
    One,
    /// Type 2.
    Two,
}

/// What follows an operator.
#[derive(Clone, Copy)]
enum Flow<'code> {
    /// The subroutine.
    Call(&'code [u8]),
    /// The glyph's end.
    End(Option<Accented>),
    /// The next operator.
    Next,
    /// The caller.
    Return,
    /// The next operator, past a mask of this many bytes.
    Skip(usize),
}

/// What a charstring needs of its program.
pub(crate) trait Source {
    /// The charstring of `code` in `StandardEncoding`.
    fn component(&self, code: u8) -> Option<&[u8]>;

    /// The subroutine `index`, as the operand gives it.
    fn routine(&self, global: bool, index: i64) -> Option<&[u8]>;
}

/// An accented character made of two others (`seac`).
#[derive(Clone, Copy)]
struct Accented {
    /// The accent's code.
    accent: u8,
    /// The base's code.
    base: u8,
    /// The accent's origin.
    offset: [i64; 2],
}

/// The state of one glyph.
struct Machine<'code, 'out> {
    /// The left side bearing.
    bearing: [i64; 2],
    /// The language.
    dialect: Dialect,
    /// The points a flex collected.
    flex: Option<Vec<[i64; 2]>>,
    /// Whether a subpath is open.
    open: bool,
    /// The character's origin.
    origin: [i64; 2],
    /// The outline's receiver.
    out: &'out mut dyn Outline,
    /// The current point.
    point: [i64; 2],
    /// The points passed.
    points: u32,
    /// What `pop` returns, the next last.
    results: Vec<i64>,
    /// The program.
    source: &'code dyn Source,
    /// The operands.
    stack: Vec<i64>,
    /// The stem hints.
    stems: usize,
    /// The steps taken.
    steps: u32,
    /// The transient array.
    transient: [i64; 32],
}

impl<'code> Machine<'code, '_> {
    /// An arithmetic, storage or conditional operator of Type 2.
    fn arithmetic(&mut self, operator: u8) -> Result<(), Fault> {
        let last = self.take()?;
        let result = match operator {
            9 => last.saturating_abs(),
            14 => last.saturating_neg(),
            5 => flag(last == 0),
            18 => return Ok(()),
            21 => self.stored(last)?,
            26 => root(last),
            27 => {
                self.push(last)?;
                last
            }
            29 => self.copied(last)?,
            30 => return self.roll(last),
            _ => return self.combine(operator, last),
        };
        self.push(result)
    }

    /// `closepath`, which leaves the point.
    fn close(&mut self) {
        if self.open {
            self.out.close();
            self.open = false;
        }
    }

    /// An operator of two operands or more, the last of them `last`.
    fn combine(&mut self, operator: u8, last: i64) -> Result<(), Fault> {
        let other = self.take()?;
        let result = match operator {
            3 => flag(other != 0 && last != 0),
            4 => flag(other != 0 || last != 0),
            10 => other.saturating_add(last),
            11 => other.saturating_sub(last),
            12 => divided(other, last),
            15 => flag(other == last),
            20 => {
                let index = usize::try_from(whole(last)).map_err(|_error| Fault::Broken);
                *self.transient.get_mut(index?).ok_or(Fault::Broken)? = other;
                return Ok(());
            }
            22 => {
                let second = self.take()?;
                let first = self.take()?;
                if other <= last { first } else { second }
            }
            24 => times(other, last),
            28 => {
                self.push(last)?;
                other
            }
            _ => return Err(Fault::Broken),
        };
        self.push(result)
    }

    /// The operand `index` places below the top.
    fn copied(&self, index: i64) -> Result<i64, Fault> {
        // A negative one copies the top.
        let below = usize::try_from(whole(index)).unwrap_or(0);
        self.stack
            .iter()
            .rev()
            .nth(below)
            .copied()
            .ok_or(Fault::Broken)
    }

    /// A curve to the absolute `points`.
    fn curve(&mut self, points: [[i64; 2]; 3]) -> Result<(), Fault> {
        self.pass(3)?;
        self.out.curve(points.map(|point| point.map(coordinate)));
        let [.., last] = points;
        self.point = last;
        Ok(())
    }

    /// A curve of relative `steps`.
    fn curved(&mut self, steps: [i64; 6]) -> Result<(), Fault> {
        let [x1, y1, x2, y2, x3, y3] = steps;
        let first = moved(self.point, [x1, y1]);
        let second = moved(first, [x2, y2]);
        self.curve([first, second, moved(second, [x3, y3])])
    }

    /// `hvcurveto` and `vhcurveto`.
    fn curves_alternating(&mut self, operands: &[i64], starts_across: bool) -> Result<(), Fault> {
        let (groups, rest) = operands.as_chunks::<4>();
        let mut across = starts_across;
        for (index, group) in groups.iter().enumerate() {
            let [first, x2, y2, third] = *group;
            let last = index.saturating_add(1) == groups.len();
            let extra = rest.first().copied().filter(|_value| last).unwrap_or(0);
            self.curved(if across {
                [first, 0, x2, y2, extra, third]
            } else {
                [0, first, x2, y2, third, extra]
            })?;
            across = !across;
        }
        Ok(())
    }

    /// `hhcurveto` and `vvcurveto`.
    fn curves_parallel(&mut self, operands: &[i64], across: bool) -> Result<(), Fault> {
        let odd = operands.len() & 1;
        let mut extra = operands
            .first()
            .copied()
            .filter(|_value| odd == 1)
            .unwrap_or(0);
        for group in operands.get(odd..).unwrap_or_default().as_chunks::<4>().0 {
            let [first, x2, y2, third] = *group;
            self.curved(if across {
                [first, extra, x2, y2, third, 0]
            } else {
                [extra, first, x2, y2, 0, third]
            })?;
            extra = 0;
        }
        Ok(())
    }

    /// A drawing or moving operator, common to both.
    fn draw(&mut self, operator: u8, operands: &[i64]) -> Result<(), Fault> {
        match operator {
            4 => self.step([0, top(operands)?]),
            21 => self.step(pair(operands)?),
            22 => self.step([top(operands)?, 0]),
            5 => {
                for delta in operands.as_chunks::<2>().0 {
                    self.line(*delta)?;
                }
            }
            6 | 7 => {
                let mut across = operator == 6;
                for length in operands {
                    self.line(if across { [*length, 0] } else { [0, *length] })?;
                    across = !across;
                }
            }
            8 => {
                for steps in operands.as_chunks::<6>().0 {
                    self.curved(*steps)?;
                }
            }
            24 => {
                let split = operands.len().checked_sub(2).ok_or(Fault::Broken)?;
                let (curves, line) = operands.split_at_checked(split).ok_or(Fault::Broken)?;
                for steps in curves.as_chunks::<6>().0 {
                    self.curved(*steps)?;
                }
                self.line(pair(line)?)?;
            }
            25 => {
                let split = operands.len().checked_sub(6).ok_or(Fault::Broken)?;
                let (lines, curve) = operands.split_at_checked(split).ok_or(Fault::Broken)?;
                for delta in lines.as_chunks::<2>().0 {
                    self.line(*delta)?;
                }
                self.curved(*curve.first_chunk().ok_or(Fault::Broken)?)?;
            }
            26 | 27 => self.curves_parallel(operands, operator == 27)?,
            30 | 31 => self.curves_alternating(operands, operator == 31)?,
            _ => return Err(Fault::Broken),
        }
        Ok(())
    }

    /// Runs a drawing operator on the operands and clears them.
    fn drawn(&mut self, operator: u8) -> Result<Flow<'code>, Fault> {
        let mut operands = mem::take(&mut self.stack);
        let done = self.draw(operator, &operands);
        operands.clear();
        self.stack = operands;
        done.map(|()| Flow::Next)
    }

    /// The four flexes of Type 2.
    fn flexes(&mut self, operator: u8) -> Result<(), Fault> {
        let args = &self.stack;
        let halves = match operator {
            34 => {
                let [x1, x2, y2, x3, x4, x5, x6] = *args.first_chunk().ok_or(Fault::Broken)?;
                let back = y2.saturating_neg();
                [[x1, 0, x2, y2, x3, 0], [x4, 0, x5, back, x6, 0]]
            }
            35 => {
                let first = *args.first_chunk().ok_or(Fault::Broken)?;
                let rest = args.get(6..).unwrap_or_default();
                [first, *rest.first_chunk().ok_or(Fault::Broken)?]
            }
            36 => {
                let [x1, y1, x2, y2, x3, x4, x5, y5, x6] =
                    *args.first_chunk().ok_or(Fault::Broken)?;
                let back = y1.saturating_add(y2).saturating_add(y5).saturating_neg();
                [[x1, y1, x2, y2, x3, 0], [x4, 0, x5, y5, x6, back]]
            }
            37 => {
                let [x1, y1, x2, y2, x3, y3, x4, y4, x5, y5, last] =
                    *args.first_chunk().ok_or(Fault::Broken)?;
                let total = [[x1, y1], [x2, y2], [x3, y3], [x4, y4], [x5, y5]]
                    .into_iter()
                    .fold([0; 2], moved);
                let [across, up] = total;
                let end = if across.saturating_abs() > up.saturating_abs() {
                    [last, up.saturating_neg()]
                } else {
                    [across.saturating_neg(), last]
                };
                let [x6, y6] = end;
                [[x1, y1, x2, y2, x3, y3], [x4, y4, x5, y5, x6, y6]]
            }
            _ => return Err(Fault::Broken),
        };
        self.stack.clear();
        for steps in halves {
            self.curved(steps)?;
        }
        Ok(())
    }

    /// Runs the glyph of `code`, with an accented one's two parts.
    fn glyph(&mut self, code: &'code [u8]) -> Result<(), Fault> {
        let Some(pieces) = self.run(code, [0; 2])? else {
            return Ok(());
        };
        let source: &'code dyn Source = self.source;
        for (part, origin) in [(pieces.base, [0; 2]), (pieces.accent, pieces.offset)] {
            let found = source.component(part).ok_or(Fault::Broken)?;
            if self.run(found, origin)?.is_some() {
                return Err(Fault::Broken);
            }
        }
        Ok(())
    }

    /// A line of `delta`.
    fn line(&mut self, delta: [i64; 2]) -> Result<(), Fault> {
        self.pass(1)?;
        self.point = moved(self.point, delta);
        self.out.line(self.point.map(coordinate));
        Ok(())
    }

    /// A number, whose first byte is `first`; returns the bytes after it.
    fn number(&mut self, first: u8, tail: &'code [u8]) -> Result<&'code [u8], Fault> {
        let (value, rest) = if first == 255 {
            let (read, rest) = tail.split_at_checked(4).ok_or(Fault::Broken)?;
            (i64::from(number(read).cast_signed()), rest)
        } else {
            encoded(first, tail).ok_or(Fault::Broken)?
        };
        let fraction = first == 255 && self.dialect == Dialect::Two;
        self.push(if fraction {
            value
        } else {
            value.saturating_mul(ONE)
        })?;
        Ok(rest)
    }

    /// An operator of Type 1.
    fn operator_one(&mut self, operator: u8) -> Result<Flow<'code>, Fault> {
        match operator {
            1 | 3 => self.stack.clear(),
            9 => {
                self.close();
                self.stack.clear();
            }
            10 => return self.routine(false),
            11 => return Ok(Flow::Return),
            13 => self.sidebearing([*self.stack.first().ok_or(Fault::Broken)?, 0]),
            14 => return Ok(Flow::End(None)),
            4..=8 | 21 | 22 | 30 | 31 => return self.drawn(operator),
            _ => return Err(Fault::Broken),
        }
        Ok(Flow::Next)
    }

    /// An escaped operator of Type 1.
    fn operator_one_escaped(&mut self, operator: u8) -> Result<Flow<'code>, Fault> {
        match operator {
            0..=2 => self.stack.clear(),
            6 => {
                let [side, across, up, base, accent] =
                    *self.stack.last_chunk().ok_or(Fault::Broken)?;
                let [bearing, _] = self.bearing;
                let offset = [bearing.saturating_add(across).saturating_sub(side), up];
                return accented(base, accent, offset);
            }
            7 => self.sidebearing(*self.stack.first_chunk().ok_or(Fault::Broken)?),
            12 => {
                let divisor = self.take()?;
                let dividend = self.take()?;
                self.push(divided(dividend, divisor))?;
            }
            16 => self.other_routine()?,
            17 => {
                let result = self.results.pop().ok_or(Fault::Broken)?;
                self.push(result)?;
            }
            33 => {
                let at = pair(&self.stack)?;
                self.point = moved(self.origin, at);
                self.stack.clear();
            }
            _ => return Err(Fault::Broken),
        }
        Ok(Flow::Next)
    }

    /// An operator of Type 2; a leading width needs no care, as each takes what is last.
    fn operator_two(&mut self, operator: u8) -> Result<Flow<'code>, Fault> {
        match operator {
            1 | 3 | 18 | 23 => self.stem(),
            10 => return self.routine(false),
            11 => return Ok(Flow::Return),
            14 => {
                if matches!(self.stack.len(), 4 | 5) {
                    let [across, up, base, accent] =
                        *self.stack.last_chunk().ok_or(Fault::Broken)?;
                    return accented(base, accent, [across, up]);
                }
                return Ok(Flow::End(None));
            }
            19 | 20 => {
                self.stem();
                return Ok(Flow::Skip(self.stems.div_ceil(8)));
            }
            29 => return self.routine(true),
            4..=8 | 21 | 22 | 24..=27 | 30 | 31 => return self.drawn(operator),
            _ => return Err(Fault::Broken),
        }
        Ok(Flow::Next)
    }

    /// An escaped operator of Type 2.
    fn operator_two_escaped(&mut self, operator: u8) -> Result<Flow<'code>, Fault> {
        match operator {
            // dotsection, which hints.
            0 => self.stack.clear(),
            23 => self.push(ONE)?,
            34..=37 => self.flexes(operator)?,
            _ => self.arithmetic(operator)?,
        }
        Ok(Flow::Next)
    }

    /// `callothersubr`: flex, hint replacement, or the arguments back.
    fn other_routine(&mut self) -> Result<(), Fault> {
        let which = whole(self.take()?);
        let count = usize::try_from(whole(self.take()?)).map_err(|_error| Fault::Broken)?;
        let from = self.stack.len().checked_sub(count).ok_or(Fault::Broken)?;
        self.results.clear();
        self.results.extend(self.stack.drain(from..).rev());
        match which {
            // The first point is where the flex starts, the second its reference point.
            1 => self.flex = Some(vec![self.point]),
            2 => {
                if let Some(points) = self.flex.as_mut() {
                    if points.len() >= 8 {
                        return Err(Fault::Broken);
                    }
                    points.push(self.point);
                }
            }
            0 => {
                let points = self.flex.take().ok_or(Fault::Broken)?;
                let [start, _, first, second, third, fourth, fifth, sixth] =
                    *points.first_chunk().ok_or(Fault::Broken)?;
                self.point = start;
                self.curve([first, second, third])?;
                self.curve([fourth, fifth, sixth])?;
                if let [y, x, _] = *self.results.as_slice() {
                    self.results.clear();
                    self.results.extend([y, x]);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Counts `count` points to come and opens the subpath.
    fn pass(&mut self, count: u32) -> Result<(), Fault> {
        self.points = self
            .points
            .saturating_add(count)
            .saturating_add(u32::from(!self.open));
        if self.points > POINTS {
            return Err(Fault::Limit);
        }
        if !self.open {
            self.out.start(self.point.map(coordinate));
            self.open = true;
        }
        Ok(())
    }

    /// Holds an operand.
    fn push(&mut self, value: i64) -> Result<(), Fault> {
        if self.stack.len() >= STACK {
            return Err(Fault::Broken);
        }
        self.stack.push(value);
        Ok(())
    }

    /// `roll` by `shift`.
    fn roll(&mut self, shift: i64) -> Result<(), Fault> {
        let count = usize::try_from(whole(self.take()?)).map_err(|_error| Fault::Broken)?;
        let from = self.stack.len().checked_sub(count).ok_or(Fault::Broken)?;
        let width = i64::try_from(count).map_err(|_error| Fault::Broken)?;
        if let Some(by) = whole(shift).checked_rem_euclid(width) {
            let places = usize::try_from(by).map_err(|_error| Fault::Broken)?;
            self.stack
                .get_mut(from..)
                .ok_or(Fault::Broken)?
                .rotate_right(places);
        }
        Ok(())
    }

    /// `callsubr` and `callgsubr`.
    fn routine(&mut self, global: bool) -> Result<Flow<'code>, Fault> {
        let index = whole(self.take()?);
        let source: &'code dyn Source = self.source;
        source
            .routine(global, index)
            .map(Flow::Call)
            .ok_or(Fault::Broken)
    }

    /// Runs the character `code` with its origin at `origin`.
    fn run(&mut self, code: &'code [u8], origin: [i64; 2]) -> Result<Option<Accented>, Fault> {
        self.bearing = [0; 2];
        self.flex = None;
        self.origin = origin;
        self.point = origin;
        self.results.clear();
        self.stack.clear();
        self.stems = 0;
        self.transient = [0; 32];
        let mut callers: Vec<&'code [u8]> = Vec::new();
        let mut rest = code;
        loop {
            let next = rest.split_first();
            let Some((first, tail)) = next else {
                match callers.pop() {
                    Some(caller) => rest = caller,
                    None => break,
                }
                continue;
            };
            self.steps = self.steps.saturating_add(1);
            if self.steps > STEPS {
                return Err(Fault::Limit);
            }
            let operator = *first;
            if operator >= 32 || (operator == 28 && self.dialect == Dialect::Two) {
                rest = self.number(operator, tail)?;
                continue;
            }
            let (flow, after) = if operator == 12 {
                let (second, after) = tail.split_first().ok_or(Fault::Broken)?;
                let flow = match self.dialect {
                    Dialect::One => self.operator_one_escaped(*second),
                    Dialect::Two => self.operator_two_escaped(*second),
                };
                (flow?, after)
            } else {
                let flow = match self.dialect {
                    Dialect::One => self.operator_one(operator),
                    Dialect::Two => self.operator_two(operator),
                };
                (flow?, tail)
            };
            rest = after;
            match flow {
                Flow::Call(_) if callers.len() >= DEPTH => return Err(Fault::Limit),
                Flow::Call(called) => {
                    callers.push(after);
                    rest = called;
                }
                Flow::End(accent) => {
                    self.close();
                    return Ok(accent);
                }
                Flow::Next => {}
                Flow::Return => rest = callers.pop().ok_or(Fault::Broken)?,
                Flow::Skip(count) => rest = after.get(count..).ok_or(Fault::Broken)?,
            }
        }
        self.close();
        Ok(None)
    }

    /// `hsbw` and `sbw`.
    fn sidebearing(&mut self, bearing: [i64; 2]) {
        self.bearing = bearing;
        self.point = moved(self.origin, bearing);
        self.stack.clear();
    }

    /// Counts stem hints.
    fn stem(&mut self) {
        self.stems = self.stems.saturating_add(self.stack.len() >> 1_u32);
        self.stack.clear();
    }

    /// A move of `delta`; inside a flex it only places a point.
    fn step(&mut self, delta: [i64; 2]) {
        if self.flex.is_none() {
            self.close();
        }
        self.point = moved(self.point, delta);
    }

    /// The transient value at `index`.
    fn stored(&self, index: i64) -> Result<i64, Fault> {
        let place = usize::try_from(whole(index)).map_err(|_error| Fault::Broken)?;
        self.transient.get(place).copied().ok_or(Fault::Broken)
    }

    /// The last operand.
    fn take(&mut self) -> Result<i64, Fault> {
        self.stack.pop().ok_or(Fault::Broken)
    }
}

/// The end of an accented character of the codes `base` and `accent`.
fn accented<'code>(base: i64, accent: i64, offset: [i64; 2]) -> Result<Flow<'code>, Fault> {
    let code = |value: i64| u8::try_from(whole(value)).map_err(|_error| Fault::Broken);
    Ok(Flow::End(Some(Accented {
        accent: code(accent)?,
        base: code(base)?,
        offset,
    })))
}

/// The quotient of numbers with 16 fraction bits; 0 by zero.
fn divided(dividend: i64, divisor: i64) -> i64 {
    quotient(i128::from(dividend) << 16_u32, i128::from(divisor)).map_or(0, saturated)
}

/// Passes the glyph of the charstring `code` to `out`; returns the steps it took, after an error
/// beside it.
pub(crate) fn draw(
    dialect: Dialect,
    source: &dyn Source,
    code: &[u8],
    out: &mut dyn Outline,
) -> Result<u64, (Fault, u64)> {
    let mut machine = Machine {
        bearing: [0; 2],
        dialect,
        flex: None,
        open: false,
        origin: [0; 2],
        out,
        point: [0; 2],
        points: 0,
        results: Vec::new(),
        source,
        stack: Vec::with_capacity(STACK),
        stems: 0,
        steps: 0,
        transient: [0; 32],
    };
    let done = machine.glyph(code);
    let taken = u64::from(machine.steps).saturating_add(u64::from(machine.points));
    done.map(|()| taken).map_err(|fault| (fault, taken))
}

/// One for true.
const fn flag(value: bool) -> i64 {
    if value { ONE } else { 0 }
}

/// `point` moved by `delta`.
const fn moved(point: [i64; 2], delta: [i64; 2]) -> [i64; 2] {
    let ([across, up], [right, rise]) = (point, delta);
    [across.saturating_add(right), up.saturating_add(rise)]
}

/// The last two operands.
fn pair(operands: &[i64]) -> Result<[i64; 2], Fault> {
    operands.last_chunk().copied().ok_or(Fault::Broken)
}

/// The square root.
fn root(value: i64) -> i64 {
    let square = u128::from(value.max(0).unsigned_abs()) << 16_u32;
    saturated(i128::try_from(square.isqrt()).unwrap_or(0))
}

/// The last operand of `operands`.
fn top(operands: &[i64]) -> Result<i64, Fault> {
    operands.last().copied().ok_or(Fault::Broken)
}

/// The integer part.
const fn whole(value: i64) -> i64 {
    value >> 16_u32
}
