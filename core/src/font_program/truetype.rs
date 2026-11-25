//! TrueType and OpenType programs (ISO/IEC 14496-22).

use core::iter::repeat_n;

use crate::font_program::encoding::{self, Label, Names};
use crate::font_program::{
    Fault, GLYPH_WORK, ONE, Outline, Span, Table, coordinate, field, short, size, times,
};
use crate::render::fixed::{quotient, saturated};

/// Components of one glyph.
const COMPONENTS: u32 = 256;

/// Composites inside each other.
const DEPTH: u32 = 8;

/// Points, and contours, of one glyph.
const POINTS: usize = 0x1_0000;

/// An sfnt's tables.
pub(crate) struct Font {
    /// The `CFF` table.
    compact: Option<Span>,
    /// The number of glyphs.
    count: u16,
    /// The `glyf` table.
    glyf: Option<Span>,
    /// The `loca` table.
    loca: Span,
    /// Whether `loca` holds 32-bit offsets.
    long: bool,
    /// The `cmap` subtables used.
    maps: Maps,
    /// `post`'s names.
    names: Names,
    /// The `post` table, its names not yet read.
    post: Option<Span>,
    /// Units of the em.
    units: Option<u16>,
}

/// The positions of `cmap` subtables.
#[derive(Clone, Copy, Default)]
struct Maps {
    /// Macintosh Roman.
    roman: Option<usize>,
    /// Windows symbol.
    symbol: Option<usize>,
    /// Unicode.
    unicode: Option<usize>,
}

/// A point of a glyph.
#[derive(Clone, Copy)]
struct Point {
    /// Its place, with 16 fraction bits.
    at: [i32; 2],
    /// Whether it is on the curve.
    on: bool,
}

/// The tables the library reads.
#[derive(Default)]
struct Tables {
    /// `cmap`.
    cmap: Option<Span>,
    /// `CFF`.
    compact: Option<Span>,
    /// `glyf`.
    glyf: Option<Span>,
    /// `head`.
    head: Option<Span>,
    /// `loca`.
    loca: Option<Span>,
    /// `maxp`.
    maxp: Option<Span>,
    /// `post`.
    post: Option<Span>,
}

/// Gathers one glyph.
struct Walk<'font> {
    /// The program.
    bytes: &'font [u8],
    /// The components taken.
    components: u32,
    /// The contours read.
    contours: usize,
    /// The position past each contour.
    ends: Vec<usize>,
    /// The font.
    font: &'font Font,
    /// The points each component's placing moved, counted once per level.
    moved: usize,
    /// The points, in font units.
    points: Vec<Point>,
}

impl Font {
    /// The glyph of `code` in a symbolic font (9.6.5.4).
    pub(crate) fn coded(&self, bytes: &[u8], code: u8) -> Option<u32> {
        let plain = u32::from(code);
        let symbol = || {
            [0, 0xF000, 0xF100, 0xF200]
                .into_iter()
                .find_map(|high: u32| self.mapped(bytes, Table::Symbol, high | plain))
        };
        symbol()
            .or_else(|| self.mapped(bytes, Table::MacRoman, plain))
            .or_else(|| self.mapped(bytes, Table::Unicode, plain))
    }

    /// The `CFF` table, when it holds the glyphs.
    pub(crate) fn compact(&self) -> Option<Span> {
        self.compact.filter(|_span| self.glyf.is_none())
    }

    /// The glyph's bytes; empty for a blank one.
    fn data<'font>(&self, bytes: &'font [u8], glyph: u32) -> Option<&'font [u8]> {
        if glyph >= u32::from(self.count) {
            return None;
        }
        let width = if self.long { 4 } else { 2 };
        let at = size(glyph).checked_mul(width)?;
        let loca = self.loca.of(bytes)?;
        let scale = if self.long { 1 } else { 2 };
        let first = size(field(loca, at, width)?).checked_mul(scale)?;
        let next = at.checked_add(width)?;
        let end = size(field(loca, next, width)?).checked_mul(scale)?;
        if first == end {
            return Some(&[]);
        }
        let glyf = self.glyf?.of(bytes)?;
        // The last glyph may lack its padding.
        glyf.get(first..end.min(glyf.len()))
    }

    /// How many glyphs it has.
    pub(crate) fn glyphs(&self) -> u32 {
        u32::from(self.count)
    }

    /// Whether a `cmap` subtable can be used.
    pub(crate) const fn has_cmap(&self) -> bool {
        self.maps.roman.is_some() || self.maps.symbol.is_some() || self.maps.unicode.is_some()
    }

    /// The bytes its tables hold.
    pub(crate) fn held(&self) -> u64 {
        self.names.held()
    }

    /// The glyph of `character` in `table`.
    pub(crate) fn mapped(&self, bytes: &[u8], table: Table, character: u32) -> Option<u32> {
        let at = match table {
            Table::MacRoman => self.maps.roman,
            Table::Symbol => self.maps.symbol,
            Table::Unicode => self.maps.unicode,
        };
        lookup(bytes.get(at?..)?, character).filter(|glyph| *glyph != 0)
    }

    /// The matrix of the em.
    pub(crate) fn matrix(&self) -> Option<[i64; 6]> {
        let side = saturated(quotient(1_i128 << 32_u32, i128::from(self.units?))?);
        Some([side, 0, 0, side, 0, 0])
    }

    /// Reads `post`'s names of the first `glyphs`, which the program has: no more are asked for.
    pub(crate) fn name(&mut self, bytes: &[u8], glyphs: u32, spent: &mut u64) {
        if let Some(span) = self.post.take() {
            let count = u16::try_from(glyphs).unwrap_or(u16::MAX).min(self.count);
            self.names = post(bytes, span, count).unwrap_or_default();
            *spent = spent.saturating_add(self.names.work());
        }
    }

    /// The glyph called `name`.
    pub(crate) fn named(&self, bytes: &[u8], name: &[u8]) -> Option<u32> {
        let unicode = || {
            encoding::characters(name)
                .into_iter()
                .find_map(|character| self.mapped(bytes, Table::Unicode, character))
        };
        let roman = || {
            let code = encoding::roman_code(name).filter(|_code| self.maps.roman.is_some())?;
            self.mapped(bytes, Table::MacRoman, u32::from(code))
        };
        unicode()
            .or_else(roman)
            .or_else(|| self.names.find(bytes, name))
    }

    /// Passes the outline of `glyph` to `out`.
    pub(crate) fn outline(
        &self,
        bytes: &[u8],
        glyph: u32,
        out: &mut dyn Outline,
    ) -> Result<u64, (Fault, u64)> {
        let mut walk = Walk {
            bytes,
            components: 0,
            contours: 0,
            ends: Vec::new(),
            font: self,
            moved: 0,
            points: Vec::new(),
        };
        let loaded = walk.load(glyph, 0);
        let taken = walk
            .points
            .len()
            .saturating_add(walk.contours)
            .saturating_add(walk.moved)
            .saturating_add(size(walk.components));
        let steps = u64::try_from(taken).unwrap_or(u64::MAX).saturating_add(1);
        loaded.map_err(|fault| (fault, steps))?;
        let mut first = 0_usize;
        for end in &walk.ends {
            let points = walk.points.get(first..*end).ok_or((Fault::Broken, steps))?;
            trace(points, out);
            first = *end;
        }
        Ok(steps)
    }

    /// The sfnt in `bytes`; adds the work reading it took past its bytes to `spent`: 16 a glyph
    /// of `glyf` and sorting names.
    pub(crate) fn parse(bytes: &[u8], spent: &mut u64) -> Result<Self, Fault> {
        let tables = Tables::read(bytes).ok_or(Fault::Broken)?;
        let head = tables.head.and_then(|span| span.of(bytes));
        let units = head
            .and_then(|data| short(data, 18))
            .filter(|value| *value != 0);
        let declared = tables
            .maxp
            .and_then(|span| span.of(bytes))
            .and_then(|data| short(data, 4));
        let mut font = Self {
            compact: tables.compact,
            count: declared.unwrap_or(0),
            glyf: tables.glyf,
            loca: tables.loca.unwrap_or_default(),
            long: false,
            maps: tables.cmap.map_or_default(|span| Maps::read(bytes, span)),
            names: Names::default(),
            post: tables.post,
            units,
        };
        if tables.glyf.is_some() {
            let format = head.and_then(|data| short(data, 50));
            let (Some(form), Some(_), Some(loca)) = (format, units, tables.loca) else {
                return Err(Fault::Broken);
            };
            font.long = form != 0;
            let slots = size(loca.length) >> if font.long { 2_u32 } else { 1_u32 };
            let fitting = u16::try_from(slots.saturating_sub(1)).unwrap_or(u16::MAX);
            font.count = declared.map_or(fitting, |count| count.min(fitting));
            if font.count == 0 {
                return Err(Fault::Broken);
            }
        } else if tables.compact.is_none() {
            return Err(Fault::Broken);
        } else {
            // The CFF table counts the glyphs.
        }
        if font.glyf.is_some() {
            *spent = spent.saturating_add(u64::from(font.count).saturating_mul(GLYPH_WORK));
            font.name(bytes, u32::from(font.count), spent);
        }
        Ok(font)
    }
}

impl Maps {
    /// The subtables of `cmap` to use.
    fn read(bytes: &[u8], cmap: Span) -> Self {
        let mut maps = Self::default();
        let mut rank = u32::MAX;
        let Some(data) = cmap.of(bytes) else {
            return maps;
        };
        let records = short(data, 2).unwrap_or(0);
        for index in 0..usize::from(records) {
            let at = index.saturating_mul(8).saturating_add(4);
            let (Some(platform), Some(kind), Some(offset)) = (
                short(data, at),
                short(data, at.saturating_add(2)),
                field(data, at.saturating_add(4), 4),
            ) else {
                break;
            };
            if !matches!(short(data, size(offset)), Some(0 | 4 | 6 | 12)) {
                continue;
            }
            let start = cmap.start().saturating_add(size(offset));
            let ranked = match (platform, kind) {
                (1, 0) => {
                    maps.roman = maps.roman.or(Some(start));
                    continue;
                }
                (3, 0) => {
                    maps.symbol = maps.symbol.or(Some(start));
                    continue;
                }
                (3, 10) => 0,
                (0, 4 | 6) => 1,
                (3, 1) => 2,
                (0, 0..=3) => 3,
                _ => continue,
            };
            if ranked < rank {
                rank = ranked;
                maps.unicode = Some(start);
            }
        }
        maps
    }
}

impl Tables {
    /// The table directory.
    fn read(bytes: &[u8]) -> Option<Self> {
        let mut tables = Self::default();
        for index in 0..usize::from(short(bytes, 4)?) {
            let at = index.checked_mul(16)?.checked_add(12)?;
            let Some(tag) = bytes.get(at..at.checked_add(4)?) else {
                break;
            };
            let slot = match tag {
                b"CFF " => &mut tables.compact,
                b"cmap" => &mut tables.cmap,
                b"glyf" => &mut tables.glyf,
                b"head" => &mut tables.head,
                b"loca" => &mut tables.loca,
                b"maxp" => &mut tables.maxp,
                b"post" => &mut tables.post,
                _ => continue,
            };
            let first = field(bytes, at.checked_add(8)?, 4)?;
            let length = field(bytes, at.checked_add(12)?, 4)?;
            *slot = Some(Span::clipped(size(first), size(length), bytes.len()));
        }
        Some(tables)
    }
}

impl<'font> Walk<'font> {
    /// Reads the component records of a composite, whose points follow `start`.
    fn composite(&mut self, data: &'font [u8], start: usize, depth: u32) -> Result<(), Fault> {
        let mut at = 10_usize;
        loop {
            self.components = self.components.saturating_add(1);
            if self.components > COMPONENTS {
                return Err(Fault::Limit);
            }
            let flags = short(data, at).ok_or(Fault::Broken)?;
            let glyph = short(data, at.saturating_add(2)).ok_or(Fault::Broken)?;
            let (arguments, after) =
                arguments(data, at.saturating_add(4), flags).ok_or(Fault::Broken)?;
            let (linear, next) = scaling(data, after, flags).ok_or(Fault::Broken)?;
            at = next;
            let first = self.points.len();
            self.load(u32::from(glyph), depth.saturating_add(1))?;
            // Each level moves every point below it again.
            let below = self.points.len().saturating_sub(first);
            self.moved = self.moved.saturating_add(below);
            if linear != [ONE, 0, 0, ONE] {
                for point in self.points.get_mut(first..).ok_or(Fault::Broken)? {
                    point.at = transformed(linear, point.at.map(i64::from)).map(coordinate);
                }
            }
            let offset = if flags & 2 == 0 {
                self.matched(start, first, arguments)?
            } else {
                let given = arguments.map(|value| value.saturating_mul(ONE));
                // SCALED_COMPONENT_OFFSET without UNSCALED_COMPONENT_OFFSET.
                if flags & 0x1800 == 0x800 {
                    transformed(linear, given)
                } else {
                    given
                }
            };
            let [right, rise] = offset;
            for point in self.points.get_mut(first..).ok_or(Fault::Broken)? {
                let [across, up] = point.at.map(i64::from);
                point.at = [across.saturating_add(right), up.saturating_add(rise)].map(coordinate);
            }
            if flags & 0x20 == 0 {
                return Ok(());
            }
        }
    }

    /// Adds the points of `glyph`.
    fn load(&mut self, glyph: u32, depth: u32) -> Result<(), Fault> {
        if depth > DEPTH {
            return Err(Fault::Limit);
        }
        let data = self.font.data(self.bytes, glyph).ok_or(Fault::Broken)?;
        if data.is_empty() {
            return Ok(());
        }
        let contours = short(data, 0).ok_or(Fault::Broken)?;
        let start = self.points.len();
        if contours.cast_signed() < 0 {
            self.composite(data, start, depth)
        } else {
            self.simple(data, usize::from(contours), start)
        }
    }

    /// The offset that lays a component's point on one before it.
    fn matched(&self, start: usize, first: usize, which: [i64; 2]) -> Result<[i64; 2], Fault> {
        let [parent, child] = which.map(|value| usize::try_from(value).unwrap_or(usize::MAX));
        let before = self
            .points
            .get(start..first)
            .and_then(|part| part.get(parent));
        let own = self.points.get(first..).and_then(|part| part.get(child));
        let (Some(fixed), Some(moving)) = (before, own) else {
            return Err(Fault::Broken);
        };
        let ([to_across, to_up], [across, up]) =
            (fixed.at.map(i64::from), moving.at.map(i64::from));
        Ok([to_across.saturating_sub(across), to_up.saturating_sub(up)])
    }

    /// Reads a simple glyph of `contours`, whose points follow `start`.
    fn simple(&mut self, data: &'font [u8], contours: usize, start: usize) -> Result<(), Fault> {
        self.contours = self.contours.saturating_add(contours);
        if self.contours > POINTS {
            return Err(Fault::Limit);
        }
        if contours == 0 {
            return Ok(());
        }
        let mut count = 0_usize;
        for index in 0..contours {
            let at = index.saturating_mul(2).saturating_add(10);
            let end = usize::from(short(data, at).ok_or(Fault::Broken)?).saturating_add(1);
            if end < count {
                return Err(Fault::Broken);
            }
            if end > count {
                count = end;
                self.ends.push(start.saturating_add(end));
            }
        }
        let total = start.saturating_add(count);
        if total > POINTS {
            return Err(Fault::Limit);
        }
        let hints = contours.saturating_mul(2).saturating_add(10);
        let length = usize::from(short(data, hints).ok_or(Fault::Broken)?);
        let body = hints.saturating_add(2).saturating_add(length);
        let mut rest = data.get(body..).ok_or(Fault::Broken)?;
        let mut flags = Vec::with_capacity(count);
        while flags.len() < count {
            let (flag, tail) = rest.split_first().ok_or(Fault::Broken)?;
            rest = tail;
            let mut repeat = 1_usize;
            if *flag & 8 != 0 {
                let (more, after) = rest.split_first().ok_or(Fault::Broken)?;
                rest = after;
                repeat = repeat.saturating_add(usize::from(*more));
            }
            let wanted = repeat.min(count.saturating_sub(flags.len()));
            flags.extend(repeat_n(*flag, wanted));
        }
        let (mut across, mut up) = (0_i64, 0_i64);
        for flag in &flags {
            let (delta, after) = delta(rest, *flag, 2, 0x10).ok_or(Fault::Broken)?;
            rest = after;
            across = across.saturating_add(delta);
            self.points.push(Point {
                at: [coordinate(across.saturating_mul(ONE)), 0_i32],
                on: *flag & 1 != 0,
            });
        }
        let placed = self.points.get_mut(start..).ok_or(Fault::Broken)?;
        for (flag, point) in flags.iter().zip(placed) {
            let (delta, after) = delta(rest, *flag, 4, 0x20).ok_or(Fault::Broken)?;
            rest = after;
            up = up.saturating_add(delta);
            let [x, _] = point.at;
            point.at = [x, coordinate(up.saturating_mul(ONE))];
        }
        Ok(())
    }
}

/// A component's two arguments and the position past them.
fn arguments(data: &[u8], at: usize, flags: u16) -> Option<([i64; 2], usize)> {
    let width = if flags & 1 == 0 { 1 } else { 2 };
    let second = at.checked_add(width)?;
    let mut pair = [0_i64; 2];
    for (slot, place) in pair.iter_mut().zip([at, second]) {
        let raw = field(data, place, width)?;
        *slot = match (flags & 3, width) {
            (2, _) => i64::from(u8::try_from(raw).ok()?.cast_signed()),
            (3, _) => i64::from(u16::try_from(raw).ok()?.cast_signed()),
            _ => i64::from(raw),
        };
    }
    Some((pair, second.checked_add(width)?))
}

/// A point's step along one axis and the bytes after it.
fn delta(rest: &[u8], flag: u8, narrow: u8, same: u8) -> Option<(i64, &[u8])> {
    let positive = flag & same != 0;
    if flag & narrow != 0 {
        let (byte, after) = rest.split_first()?;
        let value = i64::from(*byte);
        Some((
            if positive {
                value
            } else {
                value.saturating_neg()
            },
            after,
        ))
    } else if positive {
        Some((0, rest))
    } else {
        Some((i64::from(short(rest, 0)?.cast_signed()), rest.get(2..)?))
    }
}

/// The glyph of `character` in a `cmap` subtable.
fn lookup(table: &[u8], character: u32) -> Option<u32> {
    let index = size(character);
    match short(table, 0)? {
        0 => {
            let code = u8::try_from(character).ok()?;
            table
                .get(usize::from(code).checked_add(6)?)
                .map(|glyph| u32::from(*glyph))
        }
        4 => segmented(table, u16::try_from(character).ok()?),
        6 => {
            let place = index.checked_sub(usize::from(short(table, 6)?))?;
            if place >= usize::from(short(table, 8)?) {
                return None;
            }
            short(table, place.checked_mul(2)?.checked_add(10)?).map(u32::from)
        }
        12 => {
            let (mut low, mut high) = (0_usize, size(field(table, 12, 4)?));
            while low < high {
                let middle = low.midpoint(high);
                let group = middle.checked_mul(12)?.checked_add(16)?;
                let first = field(table, group, 4)?;
                if character < first {
                    high = middle;
                } else if character > field(table, group.checked_add(4)?, 4)? {
                    low = middle.checked_add(1)?;
                } else {
                    let base = field(table, group.checked_add(8)?, 4)?;
                    return base.checked_add(character.checked_sub(first)?);
                }
            }
            None
        }
        _ => None,
    }
}

/// Whether `bytes` open as an sfnt does.
pub(crate) fn matches(bytes: &[u8]) -> bool {
    matches!(bytes.get(..4), Some([0, 1, 0, 0] | b"true" | b"OTTO"))
}

/// The middle of two values.
fn middle(first: i64, second: i64) -> i64 {
    quotient(i128::from(first.saturating_add(second)), 2).map_or(0, saturated)
}

/// The names in `post` of the first `glyphs`: the names of others are never asked for.
fn post(bytes: &[u8], span: Span, glyphs: u16) -> Option<Names> {
    let data = span.of(bytes)?;
    let version = field(data, 0, 4)?;
    let list = encoding::macintosh_glyphs();
    let listed = u16::try_from(list.len()).ok()?;
    let mut entries: Vec<(Label, u16)> = match version {
        0x1_0000 => (0..listed.min(glyphs))
            .map(|glyph| (Label::Listed(glyph), glyph))
            .collect(),
        0x2_0000 => {
            let declared = short(data, 32)?;
            let mut own: Vec<(u16, u16)> = (0..declared.min(glyphs))
                .map_while(|glyph| {
                    let place = usize::from(glyph).saturating_mul(2).saturating_add(34);
                    short(data, place).map(|index| (index, glyph))
                })
                .collect();
            let mut entries: Vec<(Label, u16)> = own
                .iter()
                .filter(|pair| pair.0 < listed)
                .map(|&(index, glyph)| (Label::Listed(index), glyph))
                .collect();
            own.retain(|pair| pair.0 >= listed);
            own.sort_unstable();
            // The names past the standard ones, walked once.
            let mut at = usize::from(declared).checked_mul(2)?.checked_add(34)?;
            let mut walked = listed;
            for (index, glyph) in own {
                while walked < index {
                    let Some(length) = data.get(at).map(|byte| usize::from(*byte)) else {
                        break;
                    };
                    at = at.checked_add(length)?.checked_add(1)?;
                    walked = walked.saturating_add(1);
                }
                let Some(length) = data.get(at).map(|byte| usize::from(*byte)) else {
                    break;
                };
                let first = span.start().checked_add(at)?.checked_add(1)?;
                if let Some(label) = Label::held(Span::clipped(first, length, span.end())) {
                    entries.push((label, glyph));
                }
            }
            entries
        }
        _ => return None,
    };
    // In glyph order, so that the first of equal names wins.
    entries.sort_unstable_by_key(|entry| entry.1);
    Some(Names::new(bytes, list, entries))
}

/// A component's 2 x 2 matrix with 16 fraction bits and the position past it.
fn scaling(data: &[u8], at: usize, flags: u16) -> Option<([i64; 4], usize)> {
    let term = |index: usize| -> Option<i64> {
        let raw = short(data, at.checked_add(index.checked_mul(2)?)?)?;
        // From 14 fraction bits.
        Some(i64::from(raw.cast_signed()).saturating_mul(4))
    };
    if flags & 8 != 0 {
        let scale = term(0)?;
        Some(([scale, 0, 0, scale], at.checked_add(2)?))
    } else if flags & 0x40 != 0 {
        Some(([term(0)?, 0, 0, term(1)?], at.checked_add(4)?))
    } else if flags & 0x80 != 0 {
        Some(([term(0)?, term(1)?, term(2)?, term(3)?], at.checked_add(8)?))
    } else {
        Some(([ONE, 0, 0, ONE], at))
    }
}

/// The glyph of `character` in a format 4 subtable.
fn segmented(table: &[u8], character: u16) -> Option<u32> {
    let doubled = usize::from(short(table, 6)? & 0xFFFE);
    let (mut low, mut high) = (0_usize, doubled >> 1_u32);
    while low < high {
        let middle = low.midpoint(high);
        if short(table, middle.checked_mul(2)?.checked_add(14)?)? < character {
            low = middle.checked_add(1)?;
        } else {
            high = middle;
        }
    }
    let place = low.checked_mul(2)?;
    if place >= doubled {
        return None;
    }
    let starts = doubled.checked_add(16)?.checked_add(place)?;
    let within = character.checked_sub(short(table, starts)?)?;
    let deltas = starts.checked_add(doubled)?;
    let shift = short(table, deltas)?;
    let ranges = deltas.checked_add(doubled)?;
    let range = short(table, ranges)?;
    if range == 0 {
        return Some(u32::from(character.wrapping_add(shift)));
    }
    let at = ranges
        .checked_add(usize::from(range))?
        .checked_add(usize::from(within).checked_mul(2)?)?;
    let glyph = short(table, at)?;
    (glyph != 0).then(|| u32::from(glyph.wrapping_add(shift)))
}

/// Passes one contour to `out` (TrueType's quadratic curves as cubics).
fn trace(points: &[Point], out: &mut dyn Outline) {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return;
    };
    // A lone point is an anchor.
    if points.len() < 2 {
        return;
    }
    let (start, round) = if first.on {
        (first.at.map(i64::from), points.get(1..))
    } else if last.on {
        (
            last.at.map(i64::from),
            points.get(..points.len().saturating_sub(1)),
        )
    } else {
        let ([x0, y0], [x1, y1]) = (first.at.map(i64::from), last.at.map(i64::from));
        ([middle(x0, x1), middle(y0, y1)], Some(points))
    };
    out.start(start.map(coordinate));
    let mut at = start;
    let mut control: Option<[i64; 2]> = None;
    for point in round.unwrap_or_default() {
        let place = point.at.map(i64::from);
        if point.on {
            match control.take() {
                Some(pull) => out.curve(cubic(at, pull, place)),
                None => out.line(point.at),
            }
            at = place;
        } else {
            if let Some(pull) = control {
                let ([x0, y0], [x1, y1]) = (pull, place);
                let between = [middle(x0, x1), middle(y0, y1)];
                out.curve(cubic(at, pull, between));
                at = between;
            }
            control = Some(place);
        }
    }
    if let Some(pull) = control {
        out.curve(cubic(at, pull, start));
    }
    out.close();
}

/// `vector` through `linear`.
fn transformed(linear: [i64; 4], vector: [i64; 2]) -> [i64; 2] {
    let ([xx, yx, xy, yy], [across, up]) = (linear, vector);
    [
        times(xx, across).saturating_add(times(xy, up)),
        times(yx, across).saturating_add(times(yy, up)),
    ]
}

/// The cubic of the quadratic from `from` by `pull` to `to`.
fn cubic(from: [i64; 2], pull: [i64; 2], to: [i64; 2]) -> [[i32; 2]; 3] {
    let ([x0, y0], [x1, y1], [x2, y2]) = (from, pull, to);
    let toward = |end: i64, by: i64| {
        let twice = i128::from(by.saturating_sub(end)).saturating_mul(2);
        coordinate(end.saturating_add(quotient(twice, 3).map_or(0, saturated)))
    };
    [
        [toward(x0, x1), toward(y0, y1)],
        [toward(x2, x1), toward(y2, y1)],
        to.map(coordinate),
    ]
}
