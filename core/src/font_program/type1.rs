//! Type 1 programs (ISO 32000-2:2020, 9.9.1; Adobe Type 1 Font Format).

use alloc::collections::BTreeMap;

use crate::font_program::charstring::{self, Dialect, Source};
use crate::font_program::encoding::{Base, Label, Names};
use crate::font_program::{Fault, GLYPH_WORK, Outline, Span, THOUSANDTH, decimal, size};
use crate::pull::number;
use crate::syntax::{hex_value, is_regular, is_white_space};

/// The key of a charstring (Type 1 Font Format, 7.2).
const CHARSTRING_KEY: u16 = 4330;

/// The bytes a subroutine takes in its map, its share of a node with it: about twice the entry.
const ENTRY: usize = 2 * size_of::<(usize, Span)>();

/// The key of the eexec part.
const EEXEC_KEY: u16 = 55_665;

/// Glyphs, and subroutines, of one program; `.notdef` may come on top.
const MOST: usize = 0xFFFE;

/// A Type 1 font.
pub(crate) struct Font {
    /// The glyph of each code.
    codes: Vec<Option<u32>>,
    /// The decrypted charstrings; the first is `.notdef`'s.
    glyphs: Vec<Span>,
    /// The `FontMatrix`.
    matrix: [i64; 6],
    /// The glyphs by name.
    names: Names,
    /// The decrypted subroutines, by index.
    routines: BTreeMap<usize, Span>,
}

/// What the part before eexec gives.
struct Header {
    /// The predefined encoding.
    base: Base,
    /// Each code's name.
    codes: Vec<Option<Span>>,
    /// Whether the program lists its own codes.
    listed: bool,
    /// The `FontMatrix`.
    matrix: [i64; 6],
}

/// What the eexec part gives.
struct Private {
    /// Each glyph's name and charstring.
    glyphs: Vec<(Span, Span)>,
    /// The subroutines, by index.
    routines: BTreeMap<usize, Span>,
    /// Bytes to drop from each charstring; negative means it is plain.
    skip: i64,
}

/// Reads tokens of PostScript.
struct Scanner {
    /// The next unread byte.
    at: usize,
}

/// A glyph's surroundings while its charstring runs.
struct View<'font> {
    /// The program's bytes.
    bytes: &'font [u8],
    /// The font.
    font: &'font Font,
}

impl Font {
    /// The glyph of `code` in its encoding.
    pub(crate) fn built_in(&self, code: u8) -> Option<u32> {
        self.codes.get(usize::from(code)).copied().flatten()
    }

    /// How many glyphs it has.
    pub(crate) fn glyphs(&self) -> u32 {
        u32::try_from(self.glyphs.len()).unwrap_or(0)
    }

    /// The bytes its tables hold.
    pub(crate) fn held(&self) -> u64 {
        let codes = self
            .codes
            .capacity()
            .saturating_mul(size_of::<Option<u32>>());
        let glyphs = self.glyphs.capacity().saturating_mul(size_of::<Span>());
        let routines = self.routines.len().saturating_mul(ENTRY);
        let total = codes.saturating_add(glyphs).saturating_add(routines);
        u64::try_from(total)
            .unwrap_or(u64::MAX)
            .saturating_add(self.names.held())
    }

    /// The `FontMatrix`.
    pub(crate) const fn matrix(&self) -> [i64; 6] {
        self.matrix
    }

    /// The glyph called `name`.
    pub(crate) fn named(&self, bytes: &[u8], name: &[u8]) -> Option<u32> {
        self.names.find(bytes, name)
    }

    /// Passes the outline of `glyph` to `out`.
    pub(crate) fn outline(
        &self,
        bytes: &[u8],
        glyph: u32,
        out: &mut dyn Outline,
    ) -> Result<u64, (Fault, u64)> {
        let span = self.glyphs.get(size(glyph)).ok_or((Fault::Broken, 0))?;
        let code = span.of(bytes).ok_or((Fault::Broken, 0))?;
        let view = View { bytes, font: self };
        charstring::draw(Dialect::One, &view, code, out)
    }

    /// The program in `bytes`, which it decrypts in place; adds the work reading it took past its
    /// bytes to `spent`, though it fails: 16 a glyph and sorting names.
    pub(crate) fn parse(bytes: &mut Vec<u8>, spent: &mut u64) -> Result<Self, Fault> {
        unframe(bytes);
        let mut scanner = Scanner { at: 0 };
        let header = Header::read(bytes, &mut scanner).ok_or(Fault::Broken)?;
        scanner.at = decrypt_private(bytes, scanner.at).ok_or(Fault::Broken)?;
        let mut private = Private::read(bytes, &mut scanner)?;
        let count = u64::try_from(private.glyphs.len()).unwrap_or(u64::MAX);
        *spent = spent.saturating_add(count.saturating_mul(GLYPH_WORK));
        if private.glyphs.is_empty() {
            return Err(Fault::Broken);
        }
        let blank = private
            .glyphs
            .iter()
            .position(|entry| entry.0.of(bytes) == Some(b".notdef".as_slice()));
        match blank {
            Some(at) => private.glyphs.swap(0, at),
            None => private.glyphs.insert(0, (Span::default(), Span::default())),
        }
        let mut entries = vec![(Label::Listed(0), 0_u16)];
        for (entry, glyph) in private.glyphs.iter().zip(0_u16..).skip(1) {
            if let Some(label) = Label::held(entry.0) {
                entries.push((label, glyph));
            }
        }
        let names = Names::new(bytes, vec![b".notdef"], entries);
        *spent = spent.saturating_add(names.work());
        let codes = (0..=u8::MAX)
            .map(|code| {
                let own = header.codes.get(usize::from(code)).copied().flatten();
                let name = match own {
                    Some(span) => span.of(bytes),
                    None if header.listed => None,
                    None => header.base.glyph_name(code),
                };
                names.find(bytes, name?).filter(|glyph| *glyph != 0)
            })
            .collect();
        Ok(Self {
            codes,
            glyphs: private.glyphs.iter().map(|entry| entry.1).collect(),
            matrix: header.matrix,
            names,
            routines: private.routines,
        })
    }
}

impl Header {
    /// Reads up to eexec; `None` without one.
    fn read(bytes: &[u8], scanner: &mut Scanner) -> Option<Self> {
        let mut header = Self {
            base: Base::Standard,
            codes: vec![None; 256],
            listed: false,
            matrix: THOUSANDTH,
        };
        let (mut has_encoding, mut has_matrix) = (false, false);
        loop {
            let token = scanner.token(bytes)?.of(bytes)?;
            match token {
                b"eexec" => return Some(header),
                b"/Encoding" if !has_encoding => {
                    has_encoding = true;
                    header.read_encoding(bytes, scanner)?;
                }
                b"/FontMatrix" if !has_matrix => {
                    has_matrix = true;
                    if let Some(matrix) = matrix(bytes, scanner) {
                        header.matrix = matrix;
                    }
                }
                _ => {}
            }
        }
    }

    /// Reads `Encoding`'s value.
    fn read_encoding(&mut self, bytes: &[u8], scanner: &mut Scanner) -> Option<()> {
        let first = scanner.token(bytes)?.of(bytes)?;
        if let Some(base) = Base::named(first) {
            self.base = base;
            return Some(());
        }
        // Anything but a list is taken for the standard encoding.
        self.listed = first == b"[" || integer(first).is_some();
        if first == b"[" {
            let mut code = 0_usize;
            loop {
                let span = scanner.token(bytes)?;
                if span.of(bytes)? == b"]" {
                    return Some(());
                }
                if let Some(slot) = self.codes.get_mut(code) {
                    *slot = named(bytes, span);
                }
                code = code.saturating_add(1);
            }
        }
        let mut before = [Span::default(); 3];
        loop {
            let span = scanner.token(bytes)?;
            let token = span.of(bytes)?;
            let [third, second, last] = before;
            match token {
                b"def" => return Some(()),
                // A program that never ends its encoding still has its eexec.
                b"eexec" => {
                    scanner.at = span.start();
                    return Some(());
                }
                b"put" if third.of(bytes)? == b"dup" => {
                    let code = integer(second.of(bytes)?).and_then(|at| usize::try_from(at).ok());
                    if let Some(slot) = code.and_then(|at| self.codes.get_mut(at)) {
                        *slot = named(bytes, last);
                    }
                }
                _ => {}
            }
            before = [second, last, span];
        }
    }
}

impl Private {
    /// Reads `lenIV`, `Subrs` and `CharStrings`, and decrypts each charstring in place.
    fn read(bytes: &mut [u8], scanner: &mut Scanner) -> Result<Self, Fault> {
        let mut private = Self {
            glyphs: Vec::new(),
            routines: BTreeMap::new(),
            skip: 4,
        };
        let mut before = [Span::default(); 3];
        while let Some(span) = scanner.token(bytes) {
            let [third, second, last] = before;
            before = [second, last, span];
            let token = span.of(bytes).unwrap_or_default();
            let count = last.of(bytes).and_then(integer);
            if last.of(bytes) == Some(b"/lenIV".as_slice())
                && let Some(skip) = integer(token)
            {
                private.skip = skip;
            }
            if !matches!(token, b"RD" | b"-|") {
                continue;
            }
            let Some(length) = count.and_then(|value| usize::try_from(value).ok()) else {
                continue;
            };
            // One blank, then the bytes.
            let code = Span::clipped(scanner.at.saturating_add(1), length, bytes.len());
            scanner.at = code.end();
            let name = named(bytes, second);
            let index = (third.of(bytes) == Some(b"dup".as_slice()))
                .then(|| second.of(bytes).and_then(integer))
                .flatten();
            private.take(bytes, code, name, index)?;
        }
        Ok(private)
    }

    /// Keeps the charstring `code` of the glyph `name` or the subroutine `index`.
    fn take(
        &mut self,
        bytes: &mut [u8],
        code: Span,
        name: Option<Span>,
        index: Option<i64>,
    ) -> Result<(), Fault> {
        let plain = usize::try_from(self.skip).map_or(code, |skip| {
            if let Some(slice) = bytes.get_mut(code.start()..code.end()) {
                decrypt(slice, CHARSTRING_KEY);
            }
            let dropped = skip.min(size(code.length));
            let first = code.start().saturating_add(dropped);
            Span::clipped(first, code.end().saturating_sub(first), bytes.len())
        });
        if let Some(label) = name {
            if self.glyphs.len() >= MOST {
                return Err(Fault::Limit);
            }
            self.glyphs.push((label, plain));
        } else if let Some(Ok(place)) = index.map(usize::try_from) {
            if place >= MOST {
                return Err(Fault::Limit);
            }
            // The first of two sets, as with glyphs.
            let _: &mut Span = self.routines.entry(place).or_insert(plain);
        } else {
            // It is neither, so nothing can use it.
        }
        Ok(())
    }
}

impl Scanner {
    /// Skips blanks and comments.
    fn skip(&mut self, bytes: &[u8]) {
        let mut comment = false;
        while let Some(byte) = bytes.get(self.at).copied() {
            match byte {
                b'\r' | b'\n' => comment = false,
                b'%' => comment = true,
                _ if comment || is_white_space(byte) => {}
                _ => break,
            }
            self.at = self.at.saturating_add(1);
        }
    }

    /// Moves past the string that is open.
    fn string(&mut self, bytes: &[u8]) {
        let mut depth = 1_u32;
        while let Some(byte) = bytes.get(self.at).copied() {
            self.at = self.at.saturating_add(1);
            match byte {
                b'\\' => self.at = self.at.saturating_add(1).min(bytes.len()),
                b'(' => depth = depth.saturating_add(1),
                b')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    /// The next token; `None` at the end.
    fn token(&mut self, bytes: &[u8]) -> Option<Span> {
        self.skip(bytes);
        let first = self.at;
        let byte = *bytes.get(first)?;
        self.at = first.saturating_add(1);
        if byte == b'(' {
            self.string(bytes);
        } else if byte == b'/' || is_regular(byte) || is_white_space(byte) {
            let rest = bytes.get(self.at..)?;
            let length = rest
                .iter()
                .position(|next| !is_regular(*next))
                .unwrap_or(rest.len());
            self.at = self.at.saturating_add(length);
        } else {
            // A delimiter stands alone.
        }
        Some(Span::clipped(
            first,
            self.at.saturating_sub(first),
            bytes.len(),
        ))
    }
}

impl Source for View<'_> {
    fn component(&self, code: u8) -> Option<&[u8]> {
        let glyph = self
            .font
            .named(self.bytes, Base::Standard.glyph_name(code)?)?;
        self.font.glyphs.get(size(glyph))?.of(self.bytes)
    }

    fn routine(&self, global: bool, index: i64) -> Option<&[u8]> {
        if global {
            return None;
        }
        let span = self.font.routines.get(&usize::try_from(index).ok()?)?;
        span.of(self.bytes)
    }
}

/// Decrypts `bytes` with `key` (Type 1 Font Format, 7.1).
fn decrypt(bytes: &mut [u8], key: u16) {
    let mut state = key;
    for byte in bytes {
        let cipher = *byte;
        *byte = cipher ^ u8::try_from(state >> 8_u32).unwrap_or(0);
        state = u16::from(cipher)
            .wrapping_add(state)
            .wrapping_mul(52_845)
            .wrapping_add(22_719);
    }
}

/// Decrypts the part after eexec, which ends at `after`; returns where its tokens start.
fn decrypt_private(bytes: &mut Vec<u8>, after: usize) -> Option<usize> {
    let blanks = bytes
        .get(after..)?
        .iter()
        .position(|byte| !matches!(*byte, b' ' | b'\t' | b'\r' | b'\n'))?;
    let start = after.checked_add(blanks)?;
    let four = bytes.get(start..start.checked_add(4)?)?;
    if four.iter().all(u8::is_ascii_hexdigit) {
        let (mut written, mut high) = (start, None);
        for at in start..bytes.len() {
            let byte = *bytes.get(at)?;
            if is_white_space(byte) {
                continue;
            }
            let Some(digit) = hex_value(byte) else {
                break;
            };
            match high.take() {
                Some(upper) => {
                    *bytes.get_mut(written)? = u8::wrapping_shl(upper, 4) | digit;
                    written = written.checked_add(1)?;
                }
                None => high = Some(digit),
            }
        }
        bytes.truncate(written);
    }
    decrypt(bytes.get_mut(start..)?, EEXEC_KEY);
    start.checked_add(4)
}

/// The integer `text`.
fn integer(text: &[u8]) -> Option<i64> {
    str::from_utf8(text).ok()?.parse().ok()
}

/// The six numbers of `FontMatrix`.
fn matrix(bytes: &[u8], scanner: &mut Scanner) -> Option<[i64; 6]> {
    if !matches!(scanner.token(bytes)?.of(bytes)?, b"[" | b"{") {
        return None;
    }
    let mut matrix = [0_i64; 6];
    for term in &mut matrix {
        *term = decimal(scanner.token(bytes)?.of(bytes)?)?;
    }
    (matrix != [0; 6]).then_some(matrix)
}

/// The name in the token `span`, without its solidus.
fn named(bytes: &[u8], span: Span) -> Option<Span> {
    let token = span.of(bytes)?;
    let name = token.strip_prefix(b"/")?;
    Some(Span::clipped(
        span.start().checked_add(1)?,
        name.len(),
        bytes.len(),
    ))
}

/// Takes the segment headers of a PFB file out.
fn unframe(bytes: &mut Vec<u8>) {
    if !bytes.starts_with(&[0x80, 1]) {
        return;
    }
    let (mut at, mut written) = (0_usize, 0_usize);
    while let Some([0x80, 1 | 2, first, second, third, fourth]) =
        bytes.get(at..).and_then(<[u8]>::first_chunk::<6>).copied()
    {
        // Least significant first.
        let length = [fourth, third, second, first];
        let body = at.saturating_add(6);
        let end = body.saturating_add(size(number(&length))).min(bytes.len());
        bytes.copy_within(body..end, written);
        written = written.saturating_add(end.saturating_sub(body));
        at = end;
    }
    bytes.truncate(written);
}
