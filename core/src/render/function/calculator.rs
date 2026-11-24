//! PostScript calculator functions (ISO 32000-2:2020, 7.10.5), compiled to steps that only jump
//! forward.

use core::mem::take;

use crate::PdfError;
use crate::render::fixed::Fixed;
use crate::render::function::elementary;
use crate::render::job::Job;
use crate::render::math;
use crate::syntax::is_white_space;

/// How deep blocks nest, the program's own included.
const NESTING: usize = 33;

/// The bits of a fraction.
const FRACTION: i64 = 0xFFFF_FFFF;

/// Work of an elementary function.
const FUNCTION_WORK: u64 = 96;

/// One half.
const HALF: Fixed = Fixed::from_bits(0x8000_0000);

/// Work of a `roll` besides the values it turns: its division.
const ROLL_WORK: u64 = 4;

/// Work of a run besides its steps: filling the stack and reading the results.
const RUN_WORK: u64 = 8;

/// The stack's size (Annex C).
const STACK: usize = 100;

/// Operators of one number.
#[derive(Clone, Copy)]
enum Alone {
    /// `abs`.
    Abs,
    /// `ceiling`.
    Ceiling,
    /// `cos`.
    Cos,
    /// `cvi`.
    Cvi,
    /// `cvr`.
    Cvr,
    /// `floor`.
    Floor,
    /// `ln`.
    Ln,
    /// `log`.
    Log,
    /// `neg`.
    Neg,
    /// `round`.
    Round,
    /// `sin`.
    Sin,
    /// `sqrt`.
    Sqrt,
    /// `truncate`.
    Truncate,
}

/// Relational, boolean and bitwise operators.
#[derive(Clone, Copy)]
enum Logic {
    /// `and`.
    And,
    /// `bitshift`.
    Bitshift,
    /// `eq`.
    Eq,
    /// `ge`.
    Ge,
    /// `gt`.
    Gt,
    /// `le`.
    Le,
    /// `lt`.
    Lt,
    /// `ne`.
    Ne,
    /// `not`.
    Not,
    /// `or`.
    Or,
    /// `xor`.
    Xor,
}

/// Operators of two numbers.
#[derive(Clone, Copy)]
enum Paired {
    /// `add`.
    Add,
    /// `atan`.
    Atan,
    /// `div`.
    Div,
    /// `exp`.
    Exp,
    /// `idiv`.
    Idiv,
    /// `mod`.
    Mod,
    /// `mul`.
    Mul,
    /// `sub`.
    Sub,
}

/// Stack operators.
#[derive(Clone, Copy)]
enum Shuffle {
    /// `copy`.
    Copy,
    /// `dup`.
    Dup,
    /// `exch`.
    Exch,
    /// `index`.
    Index,
    /// `pop`.
    Pop,
    /// `roll`.
    Roll,
}

/// A step.
#[derive(Clone, Copy)]
enum Step {
    /// An operator of one number.
    Alone(Alone),
    /// Go on at a step further down.
    Jump(usize),
    /// A relational, boolean or bitwise operator.
    Logic(Logic),
    /// An operator of two numbers.
    Paired(Paired),
    /// An operand.
    Push(Value),
    /// A stack operator.
    Shuffle(Shuffle),
    /// Go on at a step further down when false.
    Unless(usize),
}

/// A value on the stack.
#[derive(Clone, Copy)]
enum Value {
    /// A boolean.
    Boolean(bool),
    /// An integer.
    Integer(i32),
    /// A real.
    Real(Fixed),
}

/// What compiling has open.
#[derive(Default)]
struct Compiling {
    /// The blocks closed and waiting for `if` or `ifelse`, by the step in front of each.
    blocks: Vec<usize>,
    /// The blocks open: the step in front and the blocks waiting outside.
    open: Vec<(usize, Vec<usize>)>,
    /// The steps.
    steps: Vec<Step>,
}

/// A compiled program.
pub(super) struct Program {
    /// Its steps.
    steps: Vec<Step>,
}

/// The operand stack.
struct Stack {
    /// The values held.
    count: usize,
    /// The slots.
    values: [Value; STACK],
}

impl Compiling {
    /// Adds the token `word`; `None` when it cannot stand here.
    fn add(&mut self, word: &[u8]) -> Option<()> {
        match word {
            b"{" => {
                if self.blocks.len() >= 2 {
                    return None;
                }
                self.open.push((self.steps.len(), take(&mut self.blocks)));
                self.steps.push(Step::Jump(0));
            }
            b"}" => {
                if !self.blocks.is_empty() {
                    return None;
                }
                let (at, waiting) = self.open.pop()?;
                self.blocks = waiting;
                self.blocks.push(at);
            }
            _ if self.open.is_empty() => return None,
            b"if" | b"ifelse" => self.branch(word == b"ifelse")?,
            _ if !self.blocks.is_empty() => return None,
            _ => self.steps.push(step(word)?),
        }
        Some(())
    }

    /// Links the blocks waiting as the branches of `if` or, with `otherwise`, `ifelse`.
    fn branch(&mut self, otherwise: bool) -> Option<()> {
        let end = self.steps.len();
        let first = self.blocks.first().copied()?;
        let second = self.blocks.get(1).copied();
        let past = match second {
            Some(next) if otherwise => {
                // The step in front of the second block ends the first.
                *self.steps.get_mut(next)? = Step::Jump(end);
                next.saturating_add(1)
            }
            None if !otherwise => end,
            Some(_) | None => return None,
        };
        *self.steps.get_mut(first)? = Step::Unless(past);
        self.blocks.clear();
        Some(())
    }
}

impl Program {
    /// The bytes it holds.
    pub(super) fn bytes(&self) -> u64 {
        u64::try_from(self.steps.len().saturating_mul(size_of::<Step>())).unwrap_or(u64::MAX)
    }

    /// The program of `source`.
    pub(super) fn read(job: &Job<'_>, source: &[u8]) -> Result<Self, PdfError> {
        job.spend(u64::try_from(source.len()).unwrap_or(u64::MAX))?;
        let mut compiling = Compiling::default();
        let mut rest = source;
        while let Some((word, after)) = token(rest) {
            rest = after;
            compiling.add(word).ok_or_else(|| job.objects.malformed())?;
            if compiling.open.len() > NESTING {
                return Err(job.capped());
            }
        }
        let whole = compiling.blocks == [0] && compiling.open.is_empty();
        let mut steps = compiling.steps;
        let front = steps
            .first_mut()
            .filter(|_front| whole)
            .ok_or_else(|| job.objects.malformed())?;
        *front = Step::Jump(1);
        Ok(Self { steps })
    }

    /// Runs it on `input`; false when it fails or leaves other than `out` numbers.
    pub(super) fn run(
        &self,
        job: &Job<'_>,
        input: &[Fixed],
        out: &mut [Fixed],
    ) -> Result<bool, PdfError> {
        let mut stack = Stack {
            count: 0,
            values: [Value::Boolean(false); STACK],
        };
        for &value in input {
            if stack.push(Value::Real(value)).is_none() {
                return Ok(false);
            }
        }
        let (mut at, mut work) = (0_usize, RUN_WORK);
        while let Some(&next) = self.steps.get(at) {
            at = at.saturating_add(1);
            work = work
                .saturating_add(cost(next))
                .saturating_add(stack.moved(next));
            let Some(target) = stack.step(next, at) else {
                job.spend(work)?;
                return Ok(false);
            };
            // Never back, so the program ends.
            at = target.max(at);
        }
        job.spend(work)?;
        Ok(stack.results(out).is_some())
    }
}

impl Stack {
    /// Runs an operator of one number.
    fn alone(&mut self, operator: Alone) -> Option<()> {
        let value = self.pop()?;
        let real = value.number()?;
        let nought = Fixed::default();
        let found = match operator {
            Alone::Abs if real >= nought => value,
            Alone::Abs | Alone::Neg => value.negated()?,
            Alone::Ceiling => value.rounded(Fixed::from_bits(FRACTION))?,
            Alone::Cos => Value::Real(elementary::cosine(real)),
            Alone::Cvi => Value::Integer(i32::try_from(truncated(real).floor()).ok()?),
            Alone::Cvr => Value::Real(real),
            Alone::Floor => value.rounded(nought)?,
            Alone::Ln => Value::Real(elementary::natural(real)?),
            Alone::Log => Value::Real(elementary::decimal(real)?),
            Alone::Round => value.rounded(HALF)?,
            Alone::Sin => Value::Real(elementary::sine(real)),
            Alone::Sqrt => Value::Real((real >= nought).then(|| math::root(real))?),
            Alone::Truncate => match value {
                Value::Integer(_) => value,
                Value::Boolean(_) | Value::Real(_) => Value::Real(truncated(real)),
            },
        };
        self.push(found)
    }

    /// Runs a relational, boolean or bitwise operator.
    fn logic(&mut self, operator: Logic) -> Option<()> {
        let right = self.pop()?;
        if matches!(operator, Logic::Not) {
            return self.push(match right {
                Value::Boolean(truth) => Value::Boolean(!truth),
                Value::Integer(whole) => Value::Integer(!whole),
                Value::Real(_) => return None,
            });
        }
        let left = self.pop()?;
        let found = match operator {
            Logic::And => bitwise(left, right, |one, other| one & other)?,
            Logic::Bitshift => Value::Integer(shifted(left.integer()?, right.integer()?)),
            Logic::Eq => Value::Boolean(left.equals(right)),
            Logic::Ge => Value::Boolean(left.number()? >= right.number()?),
            Logic::Gt => Value::Boolean(left.number()? > right.number()?),
            Logic::Le => Value::Boolean(left.number()? <= right.number()?),
            Logic::Lt => Value::Boolean(left.number()? < right.number()?),
            Logic::Ne => Value::Boolean(!left.equals(right)),
            Logic::Or => bitwise(left, right, |one, other| one | other)?,
            Logic::Not | Logic::Xor => bitwise(left, right, |one, other| one ^ other)?,
        };
        self.push(found)
    }

    /// The values `step` moves beside its own work: those `copy` copies, and those `roll`
    /// turns with its division.
    fn moved(&self, step: Step) -> u64 {
        let (back, turning) = match step {
            Step::Shuffle(Shuffle::Copy) => (1, 0),
            Step::Shuffle(Shuffle::Roll) => (2, ROLL_WORK),
            Step::Alone(_)
            | Step::Jump(_)
            | Step::Logic(_)
            | Step::Paired(_)
            | Step::Push(_)
            | Step::Shuffle(_)
            | Step::Unless(_) => return 0,
        };
        let count = self
            .count
            .checked_sub(back)
            .and_then(|at| self.values.get(at).copied())
            .and_then(Value::integer)
            .and_then(|count| u64::try_from(count).ok())
            .unwrap_or(0)
            .min(u64::try_from(STACK).unwrap_or(u64::MAX));
        // Values move four to a unit.
        (count >> 2_u32).saturating_add(turning)
    }

    /// Runs an operator of two numbers.
    fn paired(&mut self, operator: Paired) -> Option<()> {
        let right = self.pop()?;
        let left = self.pop()?;
        let (one, other) = (left.number()?, right.number()?);
        let whole = left.integer().zip(right.integer());
        let found = match operator {
            Paired::Add => whole
                .and_then(|pair| pair.0.checked_add(pair.1))
                .map_or_else(|| Value::Real(one.plus(other)), Value::Integer),
            Paired::Atan => Value::Real(elementary::arc_tangent(one, other)?),
            Paired::Div => Value::Real(one.over(other)?),
            Paired::Exp => Value::Real(elementary::power(one, other)?),
            Paired::Idiv => Value::Integer(whole.and_then(|pair| pair.0.checked_div(pair.1))?),
            Paired::Mod => Value::Integer(whole.and_then(|pair| pair.0.checked_rem(pair.1))?),
            Paired::Mul => whole
                .and_then(|pair| pair.0.checked_mul(pair.1))
                .map_or_else(|| Value::Real(one.times(other)), Value::Integer),
            Paired::Sub => whole
                .and_then(|pair| pair.0.checked_sub(pair.1))
                .map_or_else(|| Value::Real(one.minus(other)), Value::Integer),
        };
        self.push(found)
    }

    /// The top value, taken.
    fn pop(&mut self) -> Option<Value> {
        self.count = self.count.checked_sub(1)?;
        self.values.get(self.count).copied()
    }

    /// Puts `value` on top; `None` when full.
    fn push(&mut self, value: Value) -> Option<()> {
        *self.values.get_mut(self.count)? = value;
        self.count = self.count.saturating_add(1);
        Some(())
    }

    /// The numbers left, into `out`; `None` unless there are as many.
    fn results(&self, out: &mut [Fixed]) -> Option<()> {
        let left = self.values.get(..self.count)?;
        if left.len() != out.len() {
            return None;
        }
        for (slot, value) in out.iter_mut().zip(left) {
            *slot = value.number()?;
        }
        Some(())
    }

    /// Runs a stack operator.
    fn shuffle(&mut self, operator: Shuffle) -> Option<()> {
        match operator {
            Shuffle::Copy => {
                let count = usize::try_from(self.pop()?.integer()?).ok()?;
                let from = self.count.checked_sub(count)?;
                let end = self.count.checked_add(count).filter(|end| *end <= STACK)?;
                self.values.copy_within(from..self.count, self.count);
                self.count = end;
            }
            Shuffle::Dup => {
                let top = self.pop()?;
                self.push(top)?;
                self.push(top)?;
            }
            Shuffle::Exch => {
                let top = self.pop()?;
                let below = self.pop()?;
                self.push(top)?;
                self.push(below)?;
            }
            Shuffle::Index => {
                let back = usize::try_from(self.pop()?.integer()?).ok()?;
                let at = self.count.checked_sub(1)?.checked_sub(back)?;
                self.push(self.values.get(at).copied()?)?;
            }
            Shuffle::Pop => {
                let _: Value = self.pop()?;
            }
            Shuffle::Roll => {
                let by = i64::from(self.pop()?.integer()?);
                let count = self.pop()?.integer()?;
                let length = usize::try_from(count).ok()?;
                let from = self.count.checked_sub(length)?;
                if length > 0 {
                    let turned = by.checked_rem_euclid(i64::from(count))?;
                    self.values
                        .get_mut(from..self.count)?
                        .rotate_right(usize::try_from(turned).ok()?);
                }
            }
        }
        Some(())
    }

    /// Runs `step`; where to go on, `after` unless it jumps.
    fn step(&mut self, step: Step, after: usize) -> Option<usize> {
        match step {
            Step::Alone(operator) => self.alone(operator)?,
            Step::Jump(target) => return Some(target),
            Step::Logic(operator) => self.logic(operator)?,
            Step::Paired(operator) => self.paired(operator)?,
            Step::Push(value) => self.push(value)?,
            Step::Shuffle(operator) => self.shuffle(operator)?,
            Step::Unless(target) => match self.pop()? {
                Value::Boolean(truth) => return Some(if truth { after } else { target }),
                Value::Integer(_) | Value::Real(_) => return None,
            },
        }
        Some(after)
    }
}

impl Value {
    /// Whether it equals `other`.
    fn equals(self, other: Self) -> bool {
        match (self, other) {
            (Self::Boolean(one), Self::Boolean(two)) => one == two,
            _ => self
                .number()
                .zip(other.number())
                .is_some_and(|pair| pair.0 == pair.1),
        }
    }

    /// Its integer.
    const fn integer(self) -> Option<i32> {
        match self {
            Self::Integer(whole) => Some(whole),
            Self::Boolean(_) | Self::Real(_) => None,
        }
    }

    /// Its negation, of its own kind where that holds it.
    fn negated(self) -> Option<Self> {
        match self.integer().and_then(i32::checked_neg) {
            Some(whole) => Some(Self::Integer(whole)),
            None => Some(Self::Real(Fixed::default().minus(self.number()?))),
        }
    }

    /// Its value; `None` of a boolean.
    fn number(self) -> Option<Fixed> {
        match self {
            Self::Boolean(_) => None,
            Self::Integer(whole) => Some(Fixed::from_i64(i64::from(whole))),
            Self::Real(real) => Some(real),
        }
    }

    /// A real rounded down after adding `up`; an integer unchanged.
    const fn rounded(self, up: Fixed) -> Option<Self> {
        match self {
            Self::Boolean(_) => None,
            Self::Integer(_) => Some(self),
            Self::Real(real) => Some(Self::Real(Fixed::from_i64(real.plus(up).floor()))),
        }
    }
}

/// `and`, `or` or `xor` of two booleans or of two integers.
fn bitwise(left: Value, right: Value, apply: fn(i32, i32) -> i32) -> Option<Value> {
    match (left, right) {
        (Value::Boolean(one), Value::Boolean(other)) => {
            Some(Value::Boolean(apply(i32::from(one), i32::from(other)) != 0))
        }
        _ => Some(Value::Integer(apply(left.integer()?, right.integer()?))),
    }
}

/// The work of `step`.
const fn cost(step: Step) -> u64 {
    match step {
        Step::Alone(Alone::Cos | Alone::Ln | Alone::Log | Alone::Sin | Alone::Sqrt)
        | Step::Paired(Paired::Atan | Paired::Exp) => FUNCTION_WORK,
        Step::Alone(_)
        | Step::Jump(_)
        | Step::Logic(_)
        | Step::Paired(_)
        | Step::Push(_)
        | Step::Shuffle(_)
        | Step::Unless(_) => 1,
    }
}

/// The number or boolean `word`.
fn operand(word: &[u8]) -> Option<Value> {
    match word {
        b"false" => return Some(Value::Boolean(false)),
        b"true" => return Some(Value::Boolean(true)),
        _ => {}
    }
    // Digits and their punctuation only: not the words a float may also be written as.
    let numeric = |byte: &u8| byte.is_ascii_digit() || b"+-.eE".contains(byte);
    if !word.iter().all(numeric) {
        return None;
    }
    let text = str::from_utf8(word).ok()?;
    text.parse::<i32>().map(Value::Integer).ok().or_else(|| {
        let real = text.parse::<f64>().ok()?;
        Some(Value::Real(Fixed::from_f64(real)))
    })
}

/// `value` shifted left by `by`, right when negative.
fn shifted(value: i32, by: i32) -> i32 {
    let mask = i64::from(u32::MAX);
    let bits = i64::from(value) & mask;
    let distance = by.unsigned_abs();
    let moved = if distance >= 32 {
        0
    } else if by < 0_i32 {
        bits.wrapping_shr(distance)
    } else {
        bits.wrapping_shl(distance) & mask
    };
    // The 32 bits as two's complement.
    let sign = 1_i64 << 31_u32;
    i32::try_from((moved ^ sign).saturating_sub(sign)).unwrap_or(0)
}

/// The step the token `word` stands for.
fn step(word: &[u8]) -> Option<Step> {
    let found = match word {
        b"abs" => Step::Alone(Alone::Abs),
        b"add" => Step::Paired(Paired::Add),
        b"and" => Step::Logic(Logic::And),
        b"atan" => Step::Paired(Paired::Atan),
        b"bitshift" => Step::Logic(Logic::Bitshift),
        b"ceiling" => Step::Alone(Alone::Ceiling),
        b"copy" => Step::Shuffle(Shuffle::Copy),
        b"cos" => Step::Alone(Alone::Cos),
        b"cvi" => Step::Alone(Alone::Cvi),
        b"cvr" => Step::Alone(Alone::Cvr),
        b"div" => Step::Paired(Paired::Div),
        b"dup" => Step::Shuffle(Shuffle::Dup),
        b"eq" => Step::Logic(Logic::Eq),
        b"exch" => Step::Shuffle(Shuffle::Exch),
        b"exp" => Step::Paired(Paired::Exp),
        b"floor" => Step::Alone(Alone::Floor),
        b"ge" => Step::Logic(Logic::Ge),
        b"gt" => Step::Logic(Logic::Gt),
        b"idiv" => Step::Paired(Paired::Idiv),
        b"index" => Step::Shuffle(Shuffle::Index),
        b"le" => Step::Logic(Logic::Le),
        b"ln" => Step::Alone(Alone::Ln),
        b"log" => Step::Alone(Alone::Log),
        b"lt" => Step::Logic(Logic::Lt),
        b"mod" => Step::Paired(Paired::Mod),
        b"mul" => Step::Paired(Paired::Mul),
        b"ne" => Step::Logic(Logic::Ne),
        b"neg" => Step::Alone(Alone::Neg),
        b"not" => Step::Logic(Logic::Not),
        b"or" => Step::Logic(Logic::Or),
        b"pop" => Step::Shuffle(Shuffle::Pop),
        b"roll" => Step::Shuffle(Shuffle::Roll),
        b"round" => Step::Alone(Alone::Round),
        b"sin" => Step::Alone(Alone::Sin),
        b"sqrt" => Step::Alone(Alone::Sqrt),
        b"sub" => Step::Paired(Paired::Sub),
        b"truncate" => Step::Alone(Alone::Truncate),
        b"xor" => Step::Logic(Logic::Xor),
        _ => Step::Push(operand(word)?),
    };
    Some(found)
}

/// The first token of `source` and what follows it; `None` at the end.
fn token(source: &[u8]) -> Option<(&[u8], &[u8])> {
    let mut rest = source;
    loop {
        let start = rest.iter().position(|byte| !is_white_space(*byte))?;
        rest = rest.get(start..)?;
        if rest.first() != Some(&b'%') {
            break;
        }
        let end = rest.iter().position(|byte| matches!(*byte, 10 | 13))?;
        rest = rest.get(end..)?;
    }
    let length = if rest.starts_with(b"{") || rest.starts_with(b"}") {
        1
    } else {
        rest.iter()
            .position(|byte| is_white_space(*byte) || b"{}%".contains(byte))
            .unwrap_or(rest.len())
    };
    rest.split_at_checked(length)
}

/// `value` without its fraction.
fn truncated(value: Fixed) -> Fixed {
    if value < Fixed::default() {
        Fixed::default().minus(Fixed::from_i64(Fixed::default().minus(value).floor()))
    } else {
        Fixed::from_i64(value.floor())
    }
}
