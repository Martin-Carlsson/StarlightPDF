//! CFF programs (Adobe Technical Note 5176).

use crate::font_program::charstring::{self, Dialect, Source};
use crate::font_program::encoding::{
    self, EXPERT_CHARSET, EXPERT_ENCODING, EXPERT_SUBSET, ISO_ADOBE, Label, Names,
    STANDARD_ENCODING,
};
use crate::font_program::{Fault, GLYPH_WORK, Outline, Span, decimal, encoded, field, short, size};
use crate::render::fixed::saturated;

/// Bytes of one program's DICTs, which may overlap.
const DICTS: usize = 0x4_0000;

/// Operands a DICT holds.
const OPERANDS: usize = 48;

/// What the library needs of a DICT.
#[derive(Default)]
struct Dict {
    /// `CharStrings`.
    chars: Option<usize>,
    /// `charset`.
    charset: usize,
    /// `Encoding`.
    encoding: usize,
    /// `FDArray`.
    fonts: Option<usize>,
    /// `ROS` is there.
    keyed: bool,
    /// `CharstringType`, when given.
    kind: Option<usize>,
    /// `FontMatrix`.
    matrix: Option<[i64; 6]>,
    /// `Private`: its length and position.
    private: Option<[usize; 2]>,
    /// `Subrs`, from its DICT's start.
    routines: Option<usize>,
    /// `FDSelect`.
    select: Option<usize>,
}

/// A CFF font.
pub(crate) struct Font {
    /// Each id of the charset with its glyph, sorted: SIDs, or CIDs when keyed.
    charset: Vec<[u16; 2]>,
    /// The glyph of each code.
    codes: Vec<u16>,
    /// The global subroutines.
    globals: Index,
    /// The charstrings.
    glyphs: Index,
    /// Each font DICT's part; one when not keyed.
    groups: Vec<Group>,
    /// Whether CID-keyed.
    keyed: bool,
    /// The glyphs by name; none when keyed.
    names: Names,
    /// `FDSelect`'s position.
    select: Option<usize>,
    /// Where it lies in the program.
    span: Span,
}

/// What a glyph takes from its font DICT.
#[derive(Clone, Copy)]
struct Group {
    /// The whole `FontMatrix`, when one is given.
    matrix: Option<[i64; 6]>,
    /// The local subroutines.
    routines: Index,
}

/// An INDEX (TN 5176, 5).
#[derive(Clone, Copy, Default)]
struct Index {
    /// How many objects it has.
    count: u16,
    /// The byte before the objects.
    data: usize,
    /// The offsets' position.
    offsets: usize,
    /// The bytes of an offset.
    width: usize,
}

/// A glyph's surroundings while its charstring runs.
struct View<'font> {
    /// The font's bytes.
    data: &'font [u8],
    /// The font.
    font: &'font Font,
    /// The local subroutines.
    routines: Index,
}

impl Dict {
    /// The DICT in `data` (TN 5176, 4), taken from what is `left` of the limit.
    fn read(data: &[u8], left: &mut usize) -> Result<Self, Fault> {
        *left = left.checked_sub(data.len()).ok_or(Fault::Limit)?;
        let mut dict = Self::default();
        let mut operands = Vec::with_capacity(OPERANDS);
        let mut rest = data;
        while let Some((first, tail)) = rest.split_first() {
            rest = tail;
            let operator = match *first {
                12 => {
                    let (second, after) = rest.split_first().ok_or(Fault::Broken)?;
                    rest = after;
                    0x0C00 | u16::from(*second)
                }
                // Reserved ones too (TN 5176, Table 3), which are then ignored.
                plain @ (0..=27 | 31 | 255) => u16::from(plain),
                lead => {
                    let (value, after) = operand(lead, rest).ok_or(Fault::Broken)?;
                    rest = after;
                    if operands.len() < OPERANDS {
                        operands.push(value);
                    }
                    continue;
                }
            };
            dict.set(operator, &operands);
            operands.clear();
        }
        Ok(dict)
    }

    /// Takes the entry of `operator`.
    fn set(&mut self, operator: u16, operands: &[i64]) {
        let offset = operands.last().and_then(|value| natural(*value));
        match operator {
            15 => self.charset = offset.unwrap_or(0),
            16 => self.encoding = offset.unwrap_or(0),
            17 => self.chars = offset,
            18 => {
                if let Some([length, at]) = operands.last_chunk().copied() {
                    self.private = natural(length).zip(natural(at)).map(<[usize; 2]>::from);
                }
            }
            19 => self.routines = offset,
            0x0C06 => self.kind = offset,
            0x0C07 => {
                self.matrix = operands
                    .first_chunk()
                    .copied()
                    .filter(|matrix| *matrix != [0; 6]);
            }
            0x0C1E => self.keyed = true,
            0x0C24 => self.fonts = offset,
            0x0C25 => self.select = offset,
            _ => {}
        }
    }
}

impl Font {
    /// The glyph of `code` in its encoding.
    pub(crate) fn built_in(&self, code: u8) -> Option<u32> {
        let glyph = *self.codes.get(usize::from(code))?;
        (glyph != 0).then(|| u32::from(glyph))
    }

    /// The glyph of `cid`.
    pub(crate) fn cid(&self, cid: u32) -> Option<u32> {
        self.find(u16::try_from(cid).ok()?).map(u32::from)
    }

    /// The first glyph of the charset's `id`.
    fn find(&self, id: u16) -> Option<u16> {
        find(&self.charset, id)
    }

    /// How many glyphs it has.
    pub(crate) fn glyphs(&self) -> u32 {
        u32::from(self.glyphs.count)
    }

    /// The part of the font DICT of `glyph`.
    fn group(&self, data: &[u8], glyph: u32) -> Option<Group> {
        let chosen = match self.select {
            Some(at) => selected(data, at, glyph)?,
            None => 0,
        };
        self.groups.get(chosen).copied()
    }

    /// The bytes its tables hold.
    pub(crate) fn held(&self) -> u64 {
        let charset = self
            .charset
            .capacity()
            .saturating_mul(size_of::<[u16; 2]>());
        let codes = self.codes.capacity().saturating_mul(size_of::<u16>());
        let groups = self.groups.capacity().saturating_mul(size_of::<Group>());
        let total = charset.saturating_add(codes).saturating_add(groups);
        u64::try_from(total)
            .unwrap_or(u64::MAX)
            .saturating_add(self.names.held())
    }

    /// Whether CID-keyed.
    pub(crate) const fn is_keyed(&self) -> bool {
        self.keyed
    }

    /// The glyph's `FontMatrix`, when the program gives one.
    pub(crate) fn matrix(&self, bytes: &[u8], glyph: u32) -> Option<[i64; 6]> {
        self.group(self.span.of(bytes)?, glyph)?.matrix
    }

    /// The glyph called `name`.
    pub(crate) fn named(&self, bytes: &[u8], name: &[u8]) -> Option<u32> {
        self.names.find(self.span.of(bytes)?, name)
    }

    /// Passes the outline of `glyph` to `out`.
    pub(crate) fn outline(
        &self,
        bytes: &[u8],
        glyph: u32,
        out: &mut dyn Outline,
    ) -> Result<u64, (Fault, u64)> {
        let data = self.span.of(bytes).ok_or((Fault::Broken, 0))?;
        let code = self
            .glyphs
            .object(data, size(glyph))
            .ok_or((Fault::Broken, 0))?;
        let view = View {
            data,
            font: self,
            routines: self.group(data, glyph).ok_or((Fault::Broken, 0))?.routines,
        };
        charstring::draw(Dialect::Two, &view, code, out)
    }

    /// The font in `span` of `bytes`; adds the work reading it took past its bytes to `spent`,
    /// though it fails: DICT bytes, 16 a glyph and sorting names.
    pub(crate) fn parse(bytes: &[u8], span: Span, spent: &mut u64) -> Result<Self, Fault> {
        let mut left = DICTS;
        let parsed = Self::read(bytes, span, &mut left, spent);
        *spent =
            spent.saturating_add(u64::try_from(DICTS.saturating_sub(left)).unwrap_or(u64::MAX));
        parsed
    }

    /// [`Self::parse`], DICT bytes taken from `left`.
    fn read(bytes: &[u8], span: Span, left: &mut usize, spent: &mut u64) -> Result<Self, Fault> {
        let data = span
            .of(bytes)
            .filter(|part| matches(part))
            .ok_or(Fault::Broken)?;
        let header = usize::from(*data.get(2).ok_or(Fault::Broken)?);
        let (_, after_names) = Index::read(data, header)?;
        let (tops, after_tops) = Index::read(data, after_names)?;
        let (strings, after_strings) = Index::read(data, after_tops)?;
        let (globals, _) = Index::read(data, after_strings)?;
        let top = Dict::read(tops.object(data, 0).ok_or(Fault::Broken)?, left)?;
        if top.kind.is_some_and(|kind| kind != 2) {
            return Err(Fault::Broken);
        }
        let (glyphs, _) = Index::read(data, top.chars.ok_or(Fault::Broken)?)?;
        if glyphs.count == 0 {
            return Err(Fault::Broken);
        }
        *spent = spent.saturating_add(u64::from(glyphs.count).saturating_mul(GLYPH_WORK));
        let ids = charset(data, &top, glyphs.count).ok_or(Fault::Broken)?;
        let mut sorted: Vec<[u16; 2]> = ids
            .iter()
            .zip(0_u16..)
            .map(|(id, glyph)| [*id, glyph])
            .collect();
        sorted.sort_unstable();
        let groups = groups(data, &top, left)?;
        let (codes, names) = if top.keyed {
            (Vec::new(), Names::default())
        } else {
            let codes = codes(data, top.encoding, &sorted, glyphs.count).ok_or(Fault::Broken)?;
            (codes, named(data, strings, &ids))
        };
        *spent = spent.saturating_add(names.work());
        Ok(Self {
            charset: sorted,
            codes,
            globals,
            glyphs,
            groups,
            keyed: top.keyed,
            select: top.select.filter(|_at| top.keyed),
            span,
            names,
        })
    }
}

impl Index {
    /// The bytes of the object `index`.
    fn object(self, data: &[u8], index: usize) -> Option<&[u8]> {
        self.place(data, index)?.of(data)
    }

    /// The position of the object `index`.
    fn offset(self, data: &[u8], index: usize) -> Option<usize> {
        let at = self.offsets.checked_add(index.checked_mul(self.width)?)?;
        self.data.checked_add(size(field(data, at, self.width)?))
    }

    /// Where the object `index` lies.
    fn place(self, data: &[u8], index: usize) -> Option<Span> {
        if index >= usize::from(self.count) {
            return None;
        }
        let first = self.offset(data, index)?;
        let end = self.offset(data, index.checked_add(1)?)?;
        let length = end.checked_sub(first)?;
        (end <= data.len()).then(|| Span::clipped(first, length, data.len()))
    }

    /// The INDEX at `at` and the position past it.
    fn read(data: &[u8], at: usize) -> Result<(Self, usize), Fault> {
        let count = short(data, at).ok_or(Fault::Broken)?;
        let offsets = at.saturating_add(3);
        if count == 0 {
            return Ok((Self::default(), at.saturating_add(2)));
        }
        let width = usize::from(*data.get(at.saturating_add(2)).ok_or(Fault::Broken)?);
        if !(1..=4).contains(&width) {
            return Err(Fault::Broken);
        }
        let table = usize::from(count).saturating_add(1).saturating_mul(width);
        let index = Self {
            count,
            data: offsets.saturating_add(table).saturating_sub(1),
            offsets,
            width,
        };
        let end = index
            .offset(data, usize::from(count))
            .ok_or(Fault::Broken)?;
        Ok((index, end))
    }
}

impl Source for View<'_> {
    fn component(&self, code: u8) -> Option<&[u8]> {
        if self.font.keyed {
            return None;
        }
        let glyph = self
            .font
            .find(encoding::listed(&STANDARD_ENCODING, code)?)?;
        self.font.glyphs.object(self.data, usize::from(glyph))
    }

    fn routine(&self, global: bool, index: i64) -> Option<&[u8]> {
        let table = if global {
            self.font.globals
        } else {
            self.routines
        };
        // TN 5177, 4.7.
        let bias = match table.count {
            0..=1239 => 107,
            1240..=33_899 => 1131,
            _ => 0x8000,
        };
        table.object(self.data, usize::try_from(index.checked_add(bias)?).ok()?)
    }
}

/// The id of each glyph (TN 5176, 13); a predefined charset may name fewer.
fn charset(data: &[u8], top: &Dict, glyphs: u16) -> Option<Vec<u16>> {
    let count = usize::from(glyphs);
    let runs: &[(u16, u16)] = match top.charset {
        0 if top.keyed => return Some((0..glyphs).collect()),
        0 => &ISO_ADOBE,
        1 => &EXPERT_CHARSET,
        2 => &EXPERT_SUBSET,
        at => return charset_table(data, at, count),
    };
    let mut ids = Vec::new();
    for run in runs {
        let (first, length) = *run;
        ids.extend(first..first.saturating_add(length));
    }
    ids.truncate(count);
    Some(ids)
}

/// The charset at `at` of `count` glyphs.
fn charset_table(data: &[u8], at: usize, count: usize) -> Option<Vec<u16>> {
    let format = *data.get(at)?;
    let mut cursor = at.checked_add(1)?;
    let mut ids = vec![0_u16];
    while ids.len() < count {
        let first = short(data, cursor)?;
        cursor = cursor.checked_add(2)?;
        let more = match format {
            0 => 0,
            1 => {
                let left = *data.get(cursor)?;
                cursor = cursor.checked_add(1)?;
                u16::from(left)
            }
            2 => {
                let left = short(data, cursor)?;
                cursor = cursor.checked_add(2)?;
                left
            }
            _ => return None,
        };
        let wanted = count.saturating_sub(ids.len());
        ids.extend((first..=first.saturating_add(more)).take(wanted));
    }
    Some(ids)
}

/// The glyph of each code (TN 5176, 12).
fn codes(data: &[u8], at: usize, charset: &[[u16; 2]], glyphs: u16) -> Option<Vec<u16>> {
    let predefined: Option<&[(u8, u8, u16)]> = match at {
        0 => Some(&STANDARD_ENCODING),
        1 => Some(&EXPERT_ENCODING),
        _ => None,
    };
    if let Some(runs) = predefined {
        let glyph = |code| find(charset, encoding::listed(runs, code)?);
        return Some((0..=u8::MAX).map(|code| glyph(code).unwrap_or(0)).collect());
    }
    let mut codes = vec![0_u16; 256];
    let format = *data.get(at)?;
    let count = usize::from(*data.get(at.checked_add(1)?)?);
    let mut cursor = at.checked_add(2)?;
    let mut glyph = 0_u16;
    for _ in 0..count {
        let first = *data.get(cursor)?;
        cursor = cursor.checked_add(1)?;
        let more = match format & 0x7F {
            0 => 0,
            1 => {
                let left = *data.get(cursor)?;
                cursor = cursor.checked_add(1)?;
                left
            }
            _ => return None,
        };
        for code in first..=first.saturating_add(more) {
            glyph = glyph.saturating_add(1);
            if glyph < glyphs {
                *codes.get_mut(usize::from(code))? = glyph;
            }
        }
    }
    if format & 0x80 != 0 {
        let supplements = usize::from(*data.get(cursor)?);
        cursor = cursor.checked_add(1)?;
        for _ in 0..supplements {
            let code = *data.get(cursor)?;
            let id = short(data, cursor.checked_add(1)?)?;
            cursor = cursor.checked_add(3)?;
            if let Some(found) = find(charset, id) {
                *codes.get_mut(usize::from(code))? = found;
            }
        }
    }
    Some(codes)
}

/// The first glyph of `id` in a sorted charset.
fn find(charset: &[[u16; 2]], id: u16) -> Option<u16> {
    let at = charset.partition_point(|entry| *entry < [id, 0]);
    let [found, glyph] = *charset.get(at)?;
    (found == id).then_some(glyph)
}

/// What each font DICT gives its glyphs.
fn groups(data: &[u8], top: &Dict, left: &mut usize) -> Result<Vec<Group>, Fault> {
    let Some(at) = top.fonts.filter(|_at| top.keyed) else {
        return Ok(vec![Group {
            matrix: top.matrix,
            routines: routines(data, top, left)?,
        }]);
    };
    let (fonts, _) = Index::read(data, at)?;
    // Font DICTs may share a Private DICT: each is read once.
    let mut read: Vec<([usize; 2], Index)> = Vec::new();
    // FDSelect counts in a byte.
    (0..usize::from(fonts.count).min(256))
        .map(|index| {
            let dict = Dict::read(fonts.object(data, index).ok_or(Fault::Broken)?, left)?;
            let matrix = match (dict.matrix, top.matrix) {
                (Some(own), Some(outer)) => Some(product(own, outer)),
                (own, outer) => own.or(outer),
            };
            let key = dict.private.unwrap_or_default();
            let found = read
                .iter()
                .find(|known| known.0 == key)
                .map(|known| known.1);
            let local = if let Some(known) = found {
                known
            } else {
                let made = routines(data, &dict, left)?;
                read.push((key, made));
                made
            };
            Ok(Group {
                matrix,
                routines: local,
            })
        })
        .collect()
}

/// Whether `bytes` open as a CFF does (TN 5176, 6).
pub(crate) fn matches(bytes: &[u8]) -> bool {
    let header = bytes.first_chunk().copied();
    header.is_some_and(|[major, _, length, width]: [u8; 4]| {
        major == 1 && length >= 4 && (1..=4).contains(&width)
    })
}

/// The glyphs of `ids` by name.
fn named(data: &[u8], strings: Index, ids: &[u16]) -> Names {
    let list = encoding::standard_strings();
    let standard = u16::try_from(list.len()).unwrap_or(0);
    let entries = ids
        .iter()
        .zip(0_u16..)
        .filter_map(|(id, glyph)| {
            let label = match id.checked_sub(standard) {
                Some(own) => Label::held(strings.place(data, usize::from(own))?)?,
                None => Label::Listed(*id),
            };
            Some((label, glyph))
        })
        .collect();
    Names::new(data, list, entries)
}

/// A DICT's number with 32 fraction bits as an offset or a count.
fn natural(value: i64) -> Option<usize> {
    usize::try_from(value >> 32_u32).ok()
}

/// The DICT operand that starts with `first`, with 32 fraction bits, and the bytes after it.
fn operand(first: u8, rest: &[u8]) -> Option<(i64, &[u8])> {
    let (value, after) = match first {
        29 => {
            let (read, after) = rest.split_at_checked(4)?;
            (i64::from(field(read, 0, 4)?.cast_signed()), after)
        }
        30 => return packed(rest),
        _ => encoded(first, rest)?,
    };
    Some((value.saturating_mul(1_i64 << 32_u32), after))
}

/// A real number in nibbles.
fn packed(rest: &[u8]) -> Option<(i64, &[u8])> {
    let mut text = Vec::new();
    for (index, byte) in rest.iter().enumerate() {
        for nibble in [*byte >> 4_u32, *byte & 0xF] {
            // No honest number is longer.
            if text.len() > 64 {
                return None;
            }
            match nibble {
                0..=9 => text.push(b'0'.saturating_add(nibble)),
                0xA => text.push(b'.'),
                0xB => text.push(b'E'),
                0xC => text.extend_from_slice(b"E-"),
                0xE => text.push(b'-'),
                0xF => return Some((decimal(&text)?, rest.get(index.checked_add(1)?..)?)),
                _ => return None,
            }
        }
    }
    None
}

/// The matrix of `first` and then `after`.
fn product(first: [i64; 6], after: [i64; 6]) -> [i64; 6] {
    let ([a1, b1, c1, d1, e1, f1], [a2, b2, c2, d2, e2, f2]) = (first, after);
    let cell = |left: i64, with: i64, right: i64, by: i64, plus: i64| {
        let sum = i128::from(left)
            .saturating_mul(i128::from(with))
            .saturating_add(i128::from(right).saturating_mul(i128::from(by)))
            .saturating_add(0x8000_0000);
        saturated((sum >> 32_u32).saturating_add(i128::from(plus)))
    };
    [
        cell(a1, a2, b1, c2, 0),
        cell(a1, b2, b1, d2, 0),
        cell(c1, a2, d1, c2, 0),
        cell(c1, b2, d1, d2, 0),
        cell(e1, a2, f1, c2, e2),
        cell(e1, b2, f1, d2, f2),
    ]
}

/// The local subroutines of `dict`.
fn routines(data: &[u8], dict: &Dict, left: &mut usize) -> Result<Index, Fault> {
    let Some([length, at]) = dict.private.filter(|entry| *entry != [0; 2]) else {
        return Ok(Index::default());
    };
    let private = at
        .checked_add(length)
        .and_then(|end| data.get(at..end))
        .ok_or(Fault::Broken)?;
    let Some(offset) = Dict::read(private, left)?.routines else {
        return Ok(Index::default());
    };
    Ok(Index::read(data, at.saturating_add(offset))?.0)
}

/// The font DICT of `glyph` (TN 5176, 19).
fn selected(data: &[u8], at: usize, glyph: u32) -> Option<usize> {
    let body = at.checked_add(1)?;
    match *data.get(at)? {
        0 => data
            .get(body.checked_add(size(glyph))?)
            .map(|font| usize::from(*font)),
        3 => {
            let ranges = body.checked_add(2)?;
            let (mut low, mut high) = (0_usize, usize::from(short(data, body)?));
            // The last range that starts at the glyph or before it.
            while low < high {
                let middle = low.midpoint(high);
                let first = short(data, ranges.checked_add(middle.checked_mul(3)?)?)?;
                if u32::from(first) <= glyph {
                    low = middle.checked_add(1)?;
                } else {
                    high = middle;
                }
            }
            let range = ranges.checked_add(low.checked_sub(1)?.checked_mul(3)?)?;
            data.get(range.checked_add(2)?)
                .map(|font| usize::from(*font))
        }
        _ => None,
    }
}
