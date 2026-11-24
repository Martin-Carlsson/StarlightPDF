//! Embedded font programs: Type 1, CFF, TrueType and OpenType (ISO 32000-2:2020, 9.9).

mod cff;
mod charstring;
pub(crate) mod encoding;
mod truetype;
mod type1;

use crate::pull::number;
use crate::render::fixed::{quotient, saturated};

/// Work of a glyph a program's reading lists.
const GLYPH_WORK: u64 = 16;

/// The bytes a program's fixed tables take, about.
const BESIDE: u64 = 0x2000;

/// One, with 16 fraction bits.
const ONE: i64 = 0x1_0000;

/// The `FontMatrix` of a program that gives none: a thousandth.
const THOUSANDTH: [i64; 6] = [4_294_967, 0, 0, 4_294_967, 0, 0];

/// Why a program or a glyph cannot be used.
pub(crate) enum Fault {
    /// It breaks its format.
    Broken,
    /// It passes a limit.
    Limit,
}

/// A character set of a `cmap` table.
#[derive(Clone, Copy)]
pub(crate) enum Table {
    /// Macintosh Roman (1, 0).
    MacRoman,
    /// Windows symbol (3, 0).
    Symbol,
    /// Unicode.
    Unicode,
}

/// Receives an outline, in font units with 16 fraction bits; quadratic curves arrive as cubics.
pub(crate) trait Outline {
    /// Closes the subpath.
    fn close(&mut self);

    /// Adds a cubic Bezier curve.
    fn curve(&mut self, points: [[i32; 2]; 3]);

    /// Adds a line.
    fn line(&mut self, to: [i32; 2]);

    /// Starts a subpath.
    fn start(&mut self, at: [i32; 2]);
}

/// A font program, held whole. Glyph 0 is `.notdef`.
pub(crate) struct Program {
    /// Its bytes; a Type 1 program's are decrypted.
    bytes: Vec<u8>,
    /// Its CFF data.
    cff: Option<cff::Font>,
    /// Its sfnt tables.
    sfnt: Option<truetype::Font>,
    /// Its Type 1 data.
    type1: Option<type1::Font>,
}

/// Where a piece of a program lies.
#[derive(Clone, Copy, Default)]
pub(crate) struct Span {
    /// Its first byte.
    at: u32,
    /// Its length.
    length: u32,
}

impl Program {
    /// The glyph of `code` in the built-in encoding; for TrueType, 9.6.5.4's way for symbolic fonts.
    pub(crate) fn built_in(&self, code: u8) -> Option<u32> {
        if let Some(font) = self.type1.as_ref() {
            return font.built_in(code);
        }
        let encoded = self.cff.as_ref().and_then(|font| font.built_in(code));
        let found = encoded.or_else(|| {
            let sfnt = self.sfnt.as_ref()?;
            if self.cff.is_none() && !sfnt.has_cmap() {
                return Some(u32::from(code));
            }
            sfnt.coded(&self.bytes, code)
        });
        self.glyph(found?)
    }

    /// The glyph of `cid`: through a CID-keyed charset, else the same number (9.7.4.2).
    pub(crate) fn cid(&self, cid: u32) -> Option<u32> {
        let keyed = self.cff.as_ref().filter(|font| font.is_keyed());
        keyed.map_or_else(|| self.glyph(cid), |font| font.cid(cid))
    }

    /// `index`, when it is a glyph's.
    fn glyph(&self, index: u32) -> Option<u32> {
        (index < self.glyphs()).then_some(index)
    }

    /// How many glyphs it has.
    pub(crate) fn glyphs(&self) -> u32 {
        let own = self.type1.as_ref().map(type1::Font::glyphs);
        own.or_else(|| self.cff.as_ref().map(cff::Font::glyphs))
            .or_else(|| self.sfnt.as_ref().map(truetype::Font::glyphs))
            .unwrap_or(0)
    }

    /// The bytes it holds: the data, and the tables read of it for its glyphs, names and routines.
    pub(crate) fn held(&self) -> u64 {
        let copies = if self.type1.is_some() { 2 } else { 1 };
        let tables = [
            self.type1.as_ref().map(type1::Font::held),
            self.cff.as_ref().map(cff::Font::held),
            self.sfnt.as_ref().map(truetype::Font::held),
        ];
        u64::try_from(self.bytes.capacity())
            .unwrap_or(u64::MAX)
            .saturating_mul(copies)
            .saturating_add(tables.into_iter().flatten().fold(0, u64::saturating_add))
            .saturating_add(BESIDE)
    }

    /// The glyph of `character` in the `cmap` table `table`.
    pub(crate) fn mapped(&self, table: Table, character: u32) -> Option<u32> {
        let found = self.sfnt.as_ref()?.mapped(&self.bytes, table, character)?;
        self.glyph(found)
    }

    /// The glyph's `FontMatrix`, with 32 fraction bits.
    pub(crate) fn matrix(&self, glyph: u32) -> [i64; 6] {
        let own = self.type1.as_ref().map(type1::Font::matrix);
        let cff = || self.cff.as_ref()?.matrix(&self.bytes, glyph);
        let sfnt = || self.sfnt.as_ref()?.matrix();
        own.or_else(cff).or_else(sfnt).unwrap_or(THOUSANDTH)
    }

    /// The glyph called `name`; for TrueType through the `cmap`, then `post` (9.6.5.4).
    pub(crate) fn named(&self, name: &[u8]) -> Option<u32> {
        if let Some(font) = self.type1.as_ref() {
            return font.named(&self.bytes, name);
        }
        if name == b".notdef" {
            return self.glyph(0);
        }
        let charset = self
            .cff
            .as_ref()
            .and_then(|font| font.named(&self.bytes, name));
        let found = charset.or_else(|| self.sfnt.as_ref()?.named(&self.bytes, name));
        self.glyph(found?)
    }

    /// Passes the outline of `glyph` to `out`; returns the steps it took, after an error beside it.
    /// After an error, what came is to be dropped.
    pub(crate) fn outline(&self, glyph: u32, out: &mut dyn Outline) -> Result<u64, (Fault, u64)> {
        if let Some(font) = self.type1.as_ref() {
            font.outline(&self.bytes, glyph, out)
        } else if let Some(font) = self.cff.as_ref() {
            font.outline(&self.bytes, glyph, out)
        } else if let Some(font) = self.sfnt.as_ref() {
            font.outline(&self.bytes, glyph, out)
        } else {
            Err((Fault::Broken, 0))
        }
    }

    /// The program in `bytes`, whatever its dictionary calls it; adds the work reading it took
    /// past its bytes to `spent`, though it fails.
    pub(crate) fn parse(bytes: Vec<u8>, spent: &mut u64) -> Result<Self, Fault> {
        // Spans count in 32 bits.
        let Ok(length) = u32::try_from(bytes.len()) else {
            return Err(Fault::Limit);
        };
        let mut program = Self {
            bytes,
            cff: None,
            sfnt: None,
            type1: None,
        };
        if truetype::matches(&program.bytes) {
            let mut font = truetype::Font::parse(&program.bytes, spent)?;
            if let Some(span) = font.compact() {
                let compact = cff::Font::parse(&program.bytes, span, spent)?;
                font.name(&program.bytes, compact.glyphs(), spent);
                program.cff = Some(compact);
            }
            program.sfnt = Some(font);
        } else if cff::matches(&program.bytes) {
            let whole = Span { at: 0, length };
            program.cff = Some(cff::Font::parse(&program.bytes, whole, spent)?);
        } else {
            program.type1 = Some(type1::Font::parse(&mut program.bytes, spent)?);
        }
        Ok(program)
    }
}

impl Span {
    /// The span of `length` bytes from `at`, cut at `end`.
    fn clipped(at: usize, length: usize, end: usize) -> Self {
        let first = at.min(end);
        let kept = length.min(end.saturating_sub(first));
        Self {
            at: u32::try_from(first).unwrap_or(0),
            length: u32::try_from(kept).unwrap_or(0),
        }
    }

    /// The position past it.
    fn end(self) -> usize {
        self.start().saturating_add(size(self.length))
    }

    /// Its bytes in `bytes`.
    fn of(self, bytes: &[u8]) -> Option<&[u8]> {
        bytes.get(self.start()..self.end())
    }

    /// Its first position.
    fn start(self) -> usize {
        size(self.at)
    }
}

/// `value` saturated to what an outline carries.
fn coordinate(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

/// The decimal number `text`, with 32 fraction bits.
fn decimal(text: &[u8]) -> Option<i64> {
    let (negative, unsigned) = signed(text);
    let mark = unsigned
        .iter()
        .position(|byte| matches!(*byte, b'e' | b'E'));
    let split = mark.unwrap_or(unsigned.len());
    let (mut mantissa, mut scale) = (0_i128, 0_i32);
    let (mut seen, mut fraction) = (false, false);
    for byte in unsigned.get(..split)? {
        match *byte {
            b'.' if !fraction => fraction = true,
            b'0'..=b'9' => {
                seen = true;
                // Digits past the 18 that are kept only move the point.
                if mantissa < 100_000_000_000_000_000 {
                    let digit = i128::from(byte.wrapping_sub(b'0'));
                    mantissa = mantissa.saturating_mul(10).saturating_add(digit);
                    scale = scale.saturating_sub(i32::from(fraction));
                } else {
                    scale = scale.saturating_add(i32::from(!fraction));
                }
            }
            _ => return None,
        }
    }
    if let Some(at) = mark {
        let (below, digits) = signed(unsigned.get(at.saturating_add(1)..)?);
        let mut power = 0_i32;
        for byte in digits {
            if !byte.is_ascii_digit() {
                return None;
            }
            let digit = i32::from(byte.wrapping_sub(b'0'));
            power = power.saturating_mul(10).saturating_add(digit);
        }
        let change = if below { power.saturating_neg() } else { power };
        scale = scale.saturating_add(change);
    }
    if !seen {
        return None;
    }
    let factor = 10_i128.checked_pow(scale.unsigned_abs());
    let wide = mantissa.saturating_mul(1_i128 << 32_u32);
    let bits = if scale < 0_i32 {
        factor
            .and_then(|divisor| quotient(wide, divisor))
            .map_or(0, saturated)
    } else if mantissa == 0 {
        0
    } else {
        saturated(wide.saturating_mul(factor.unwrap_or(i128::MAX)))
    };
    Some(if negative {
        bits.saturating_neg()
    } else {
        bits
    })
}

/// The integer that `first` and the bytes after it encode, as DICTs and charstrings share it: 28, then
/// 32 to 254 (Adobe TN 5176, Table 3; TN 5177, 3.2); and the bytes after it.
fn encoded(first: u8, rest: &[u8]) -> Option<(i64, &[u8])> {
    let length = match first {
        28 => 2,
        247..=254 => 1,
        32..=246 => 0,
        _ => return None,
    };
    let (read, after) = rest.split_at_checked(length)?;
    let code = i64::from(first);
    let bits = number(read);
    let low = i64::from(bits);
    let value = match first {
        28 => i64::from(u16::try_from(bits).ok()?.cast_signed()),
        247..=250 => (code.saturating_sub(247) << 8_u32)
            .saturating_add(low)
            .saturating_add(108),
        251..=254 => (code.saturating_sub(251) << 8_u32)
            .saturating_add(low)
            .saturating_add(108)
            .saturating_neg(),
        _ => code.saturating_sub(139),
    };
    Some((value, after))
}

/// The big-endian number of `width` bytes at `at`.
fn field(bytes: &[u8], at: usize, width: usize) -> Option<u32> {
    bytes.get(at..at.checked_add(width)?).map(number)
}

/// The 16-bit number at `at`.
fn short(bytes: &[u8], at: usize) -> Option<u16> {
    u16::try_from(field(bytes, at, 2)?).ok()
}

/// Whether `text` has a minus, and the rest of it.
fn signed(text: &[u8]) -> (bool, &[u8]) {
    text.strip_prefix(b"-").map_or_else(
        || (false, text.strip_prefix(b"+").unwrap_or(text)),
        |rest| (true, rest),
    )
}

/// `value` as an offset or a count.
fn size(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// The product of numbers with 16 fraction bits.
fn times(left: i64, right: i64) -> i64 {
    let product = i128::from(left).saturating_mul(i128::from(right));
    saturated(product.saturating_add(0x8000) >> 16_u32)
}
