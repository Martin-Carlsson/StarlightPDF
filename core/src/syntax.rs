//! Tokens and objects read from a source (ISO 32000-2:2020, 7.2 and 7.3).

use core::mem;
use core::str::from_utf8;

use crate::input::Source;
use crate::objects::ObjectId;
use crate::{ErrorCode, PdfError};

/// Bytes read from the source at a time.
const CHUNK: usize = 512;

/// The dictionary keys the library reads; others are not kept.
///
/// [`PARMS`] are kept only inside `DecodeParms`, the others only in the outermost dictionary.
const KEYS: [&[u8]; 19] = [
    b"BitsPerComponent",
    b"Colors",
    b"Columns",
    b"Count",
    b"DecodeParms",
    b"Encrypt",
    b"Filter",
    b"First",
    b"Index",
    b"Length",
    b"N",
    b"Pages",
    b"Predictor",
    b"Prev",
    b"Root",
    b"Size",
    b"Type",
    b"W",
    b"XRefStm",
];

/// The keys of [`KEYS`] a `DecodeParms` dictionary holds (ISO 32000-2:2020, 7.4.4.4).
const PARMS: [&[u8]; 4] = [b"BitsPerComponent", b"Colors", b"Columns", b"Predictor"];

/// Bytes kept of a name; longer names keep one more, so match no key.
const NAME_LIMIT: usize = 127;

/// Bytes of a number or keyword; longer ones fail with [`ErrorCode::LimitExceeded`].
const RUN_LIMIT: usize = 255;

/// A dictionary's values of [`KEYS`], the outermost's or a `DecodeParms`'s; of duplicates, the last.
#[derive(Default, Eq, PartialEq)]
pub(crate) struct Dictionary(Box<[Option<Object>; KEYS.len()]>);

/// A container being read; of its contents only [`KEYS`] values are kept.
enum Frame {
    /// An array, from the offset of its `[`.
    Array(u64),
    /// A dictionary awaiting a key.
    Dictionary,
    /// A dictionary awaiting the value of a key; its [`KEYS`] index when kept.
    Key(Option<usize>),
}

/// A keyword the library tells apart.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Keyword {
    /// `true`, `false` or `null`.
    Constant,
    /// `obj`.
    Obj,
    /// Any other.
    Other,
    /// `R`.
    Reference,
    /// `stream`.
    Stream,
    /// `trailer`.
    Trailer,
}

/// An object, holding only what the library reads (ISO 32000-2:2020, 7.3).
#[derive(Eq, PartialEq)]
pub(crate) enum Object {
    /// An outermost dictionary's array, from the offset of its `[`; read it again from there.
    Array(u64),
    /// A dictionary.
    Dictionary(Dictionary),
    /// An integer.
    Integer(i64),
    /// A name, `#` escapes decoded, at most [`NAME_LIMIT`] bytes and one.
    Name(Vec<u8>),
    /// A boolean, null, real, string, nested array or nested dictionary.
    Other,
    /// An indirect reference.
    Reference(ObjectId),
}

/// Bytes at an offset.
pub(crate) trait Input {
    /// Fills `buffer` from `offset`, stopping early at the end; returns the count read.
    ///
    /// # Errors
    ///
    /// Whatever the bytes' source fails with; `operation` names the operation.
    fn fill(
        &mut self,
        offset: u64,
        buffer: &mut [u8],
        operation: &'static str,
    ) -> Result<usize, PdfError>;
}

/// Reads tokens and objects from bytes through a small window.
pub(crate) struct Reader<I> {
    /// Whether a token looked at ahead was malformed.
    broken: bool,
    /// The bytes.
    input: I,
    /// The operation errors name.
    operation: &'static str,
    /// The offset of the next byte.
    position: u64,
    /// Tokens looked at ahead and not used, the next last.
    pushed: Vec<Token>,
    /// The offset of `window`.
    start: u64,
    /// Bytes read at `start`.
    window: Vec<u8>,
}

/// A token (ISO 32000-2:2020, 7.2).
#[derive(Eq, PartialEq)]
pub(crate) enum Token {
    /// `]`.
    ArrayEnd,
    /// `[`.
    ArrayStart,
    /// `>>`.
    DictionaryEnd,
    /// `<<`.
    DictionaryStart,
    /// An integer.
    Integer(i64),
    /// A keyword.
    Keyword(Keyword),
    /// A name, as [`Object::Name`].
    Name(Vec<u8>),
    /// A real or a string.
    Other,
}

impl Dictionary {
    /// The value of `key`; `None` for a key not in [`KEYS`].
    pub(crate) fn get(&self, key: &[u8]) -> Option<&Object> {
        KEYS.iter()
            .position(|kept| *kept == key)
            .and_then(|index| self.0.get(index))
            .and_then(Option::as_ref)
    }
}

impl Object {
    /// The offset of the array's `[`, when this is an array.
    pub(crate) const fn array(&self) -> Option<u64> {
        if let Self::Array(start) = *self {
            Some(start)
        } else {
            None
        }
    }

    /// The dictionary, when this is one.
    pub(crate) fn dictionary(self) -> Option<Dictionary> {
        if let Self::Dictionary(dictionary) = self {
            Some(dictionary)
        } else {
            None
        }
    }

    /// The integer, when this is one.
    pub(crate) const fn integer(&self) -> Option<i64> {
        if let Self::Integer(value) = *self {
            Some(value)
        } else {
            None
        }
    }

    /// Whether this is the name `name`.
    pub(crate) fn is_name(&self, name: &[u8]) -> bool {
        matches!(self, Self::Name(found) if found.as_slice() == name)
    }

    /// Whether this is a name.
    pub(crate) const fn is_named(&self) -> bool {
        matches!(*self, Self::Name(_))
    }

    /// The reference, when this is one.
    pub(crate) const fn reference(&self) -> Option<ObjectId> {
        if let Self::Reference(id) = *self {
            Some(id)
        } else {
            None
        }
    }
}

impl Input for &Source {
    fn fill(
        &mut self,
        offset: u64,
        buffer: &mut [u8],
        operation: &'static str,
    ) -> Result<usize, PdfError> {
        self.read(offset, buffer, operation)
    }
}

impl<I> Reader<I>
where
    I: Input,
{
    /// `number` as an integer, or the start of a reference `number generation R`
    /// (ISO 32000-2:2020, 7.3.10).
    fn after_integer(&mut self, number: i64) -> Result<Object, PdfError> {
        let first = self.lookahead()?;
        let Some(Token::Integer(generation)) = first else {
            self.pushed.extend(first);
            return Ok(Object::Integer(number));
        };
        let second = self.lookahead()?;
        if second == Some(Token::Keyword(Keyword::Reference)) {
            return u32::try_from(number)
                .ok()
                .zip(u16::try_from(generation).ok())
                .map(|(object, version)| {
                    Object::Reference(ObjectId {
                        generation: version,
                        number: object,
                    })
                })
                .ok_or_else(|| self.malformed());
        }
        self.pushed.extend(second);
        self.pushed.push(Token::Integer(generation));
        Ok(Object::Integer(number))
    }

    /// Whether the next byte starts a token: neither white-space nor a comment.
    pub(crate) fn at_token(&mut self) -> Result<bool, PdfError> {
        Ok(self
            .peek()?
            .is_some_and(|byte| !is_white_space(byte) && byte != b'%'))
    }

    /// Moves past the current byte.
    const fn bump(&mut self) {
        self.position = self.position.saturating_add(1);
    }

    /// The byte a `#` escape in a name stands for, after the `#`; `None` leaves the position.
    fn escape(&mut self) -> Result<Option<u8>, PdfError> {
        let mark = self.position;
        let high = self.next_byte()?.and_then(hex_value);
        let low = self.next_byte()?.and_then(hex_value);
        let decoded = high
            .zip(low)
            .map(|(left, right)| left.wrapping_shl(4) | right);
        if decoded.is_none() {
            self.position = mark;
        }
        Ok(decoded)
    }

    /// The rest of a hexadecimal string (ISO 32000-2:2020, 7.3.4.3).
    fn hex_string(&mut self) -> Result<Token, PdfError> {
        loop {
            match self.next_byte()? {
                Some(b'>') => return Ok(Token::Other),
                Some(byte) if byte.is_ascii_hexdigit() || is_white_space(byte) => {}
                Some(_) | None => return Err(self.malformed()),
            }
        }
    }

    /// The rest of a literal string (ISO 32000-2:2020, 7.3.4.2).
    fn literal_string(&mut self) -> Result<Token, PdfError> {
        let mut depth: u64 = 1;
        loop {
            match self.next_byte()? {
                Some(b'\\') => {
                    if self.next_byte()?.is_none() {
                        return Err(self.malformed());
                    }
                }
                Some(b'(') => depth = depth.saturating_add(1),
                Some(b')') => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Ok(Token::Other);
                    }
                }
                Some(_) => {}
                None => return Err(self.malformed()),
            }
        }
    }

    /// The next token, or `None` where it is malformed; then every later token is malformed.
    fn lookahead(&mut self) -> Result<Option<Token>, PdfError> {
        match self.token() {
            Ok(token) => Ok(Some(token)),
            Err(error) if error.code() == ErrorCode::Malformed => {
                self.broken = true;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// A [`ErrorCode::Malformed`] error.
    fn malformed(&self) -> PdfError {
        PdfError::new(ErrorCode::Malformed, self.operation)
    }

    /// The rest of a name (ISO 32000-2:2020, 7.3.5).
    fn name(&mut self) -> Result<Token, PdfError> {
        let mut name = Vec::new();
        while let Some(byte) = self.peek()?.filter(|byte| is_regular(*byte)) {
            self.bump();
            let decoded = if byte == b'#' { self.escape()? } else { None };
            if name.len() <= NAME_LIMIT {
                name.push(decoded.unwrap_or(byte));
            }
        }
        Ok(Token::Name(name))
    }

    /// A reader of `input` at `position`, failing as `operation`.
    pub(crate) const fn new(input: I, position: u64, operation: &'static str) -> Self {
        Self {
            broken: false,
            input,
            operation,
            position,
            pushed: Vec::new(),
            start: 0,
            window: Vec::new(),
        }
    }

    /// The next byte, `None` at the end.
    fn next_byte(&mut self) -> Result<Option<u8>, PdfError> {
        let byte = self.peek()?;
        self.bump();
        Ok(byte)
    }

    /// An object (ISO 32000-2:2020, 7.3); nesting deeper than `depth_limit` fails with
    /// [`ErrorCode::LimitExceeded`].
    pub(crate) fn object(&mut self, depth_limit: u32) -> Result<Object, PdfError> {
        let limit = usize::try_from(depth_limit).unwrap_or(usize::MAX);
        let mut stack = Vec::new();
        let mut outermost = None;
        loop {
            let value = match self.token()? {
                Token::ArrayStart | Token::DictionaryStart if stack.len() >= limit => {
                    return Err(PdfError::new(ErrorCode::LimitExceeded, self.operation));
                }
                Token::ArrayStart => {
                    stack.push(Frame::Array(self.position.saturating_sub(1)));
                    continue;
                }
                Token::DictionaryStart => {
                    stack.push(Frame::Dictionary);
                    continue;
                }
                Token::ArrayEnd => match stack.pop() {
                    Some(Frame::Array(start)) if stack.len() == 1 => Object::Array(start),
                    Some(Frame::Array(_)) => Object::Other,
                    Some(Frame::Dictionary | Frame::Key(_)) | None => {
                        return Err(self.malformed());
                    }
                },
                Token::DictionaryEnd => match stack.pop() {
                    Some(Frame::Dictionary) if stack.is_empty() => {
                        Object::Dictionary(mem::take(&mut outermost).unwrap_or_default())
                    }
                    Some(Frame::Dictionary) => Object::Other,
                    Some(Frame::Array(_) | Frame::Key(_)) | None => return Err(self.malformed()),
                },
                Token::Integer(integer) => self.after_integer(integer)?,
                Token::Keyword(Keyword::Constant) | Token::Other => Object::Other,
                Token::Keyword(
                    Keyword::Obj
                    | Keyword::Other
                    | Keyword::Reference
                    | Keyword::Stream
                    | Keyword::Trailer,
                ) => return Err(self.malformed()),
                Token::Name(name) => match stack.pop() {
                    Some(Frame::Dictionary) => {
                        let slot = KEYS.iter().position(|key| *key == name).filter(|_| {
                            let parm = PARMS.contains(&name.as_slice());
                            let own = stack.len() == 1
                                || stack.len() == 2
                                    && matches!(stack.get(1), Some(Frame::Array(_)));
                            let in_parms = own
                                && stack.first().is_some_and(|frame| {
                                    matches!(*frame, Frame::Key(Some(slot))
                                        if KEYS.get(slot) == Some(&b"DecodeParms".as_slice()))
                                });
                            if parm { in_parms } else { stack.is_empty() }
                        });
                        stack.push(Frame::Key(slot));
                        continue;
                    }
                    Some(frame) => {
                        stack.push(frame);
                        Object::Name(name)
                    }
                    None => Object::Name(name),
                },
            };
            match stack.pop() {
                None => return Ok(value),
                Some(Frame::Array(start)) => stack.push(Frame::Array(start)),
                Some(Frame::Key(slot)) => {
                    if let Some(kept) = slot.and_then(|index| {
                        outermost
                            .get_or_insert_with(Dictionary::default)
                            .0
                            .get_mut(index)
                    }) {
                        *kept = Some(value);
                    }
                    stack.push(Frame::Dictionary);
                }
                Some(Frame::Dictionary) => return Err(self.malformed()),
            }
        }
    }

    /// The next byte, without moving past it.
    fn peek(&mut self) -> Result<Option<u8>, PdfError> {
        if let Some(byte) = self
            .position
            .checked_sub(self.start)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| self.window.get(index))
        {
            return Ok(Some(*byte));
        }
        self.window.resize(CHUNK, 0);
        let count = self
            .input
            .fill(self.position, &mut self.window, self.operation)?;
        self.window.truncate(count);
        self.start = self.position;
        Ok(self.window.first().copied())
    }

    /// The offset of the next byte.
    pub(crate) const fn position(&self) -> u64 {
        self.position
    }

    /// The rest of a run of regular characters: a number or a keyword (ISO 32000-2:2020, 7.3.2 and 7.3.3).
    fn regular(&mut self, first: u8) -> Result<Token, PdfError> {
        let mut run = vec![first];
        while let Some(byte) = self.peek()?.filter(|byte| is_regular(*byte)) {
            if run.len() >= RUN_LIMIT {
                return Err(PdfError::new(ErrorCode::LimitExceeded, self.operation));
            }
            self.bump();
            run.push(byte);
        }
        let digits = run
            .strip_prefix(b"+")
            .or_else(|| run.strip_prefix(b"-"))
            .unwrap_or(&run);
        let point = digits.iter().position(|byte| *byte == b'.');
        match run.as_slice() {
            b"R" => Ok(Token::Keyword(Keyword::Reference)),
            b"false" | b"null" | b"true" => Ok(Token::Keyword(Keyword::Constant)),
            b"obj" => Ok(Token::Keyword(Keyword::Obj)),
            b"stream" => Ok(Token::Keyword(Keyword::Stream)),
            b"trailer" => Ok(Token::Keyword(Keyword::Trailer)),
            _ if !digits
                .iter()
                .all(|byte| byte.is_ascii_digit() || *byte == b'.') =>
            {
                Ok(Token::Keyword(Keyword::Other))
            }
            _ if !digits.is_empty() && point.is_none() => Ok(from_utf8(&run)
                .ok()
                .and_then(|text| text.parse().ok())
                .map_or(Token::Other, Token::Integer)),
            _ if point.is_some()
                && point == digits.iter().rposition(|byte| *byte == b'.')
                && digits.iter().any(u8::is_ascii_digit) =>
            {
                Ok(Token::Other)
            }
            _ => Err(self.malformed()),
        }
    }

    /// Moves to `position`.
    pub(crate) const fn seek(&mut self, position: u64) {
        self.position = position;
    }

    /// Skips white-space and comments (ISO 32000-2:2020, 7.2.3 and 7.2.4).
    fn skip_space(&mut self) -> Result<(), PdfError> {
        loop {
            match self.peek()? {
                Some(b'%') => {
                    while self
                        .peek()?
                        .is_some_and(|byte| byte != b'\r' && byte != b'\n')
                    {
                        self.bump();
                    }
                }
                Some(byte) if is_white_space(byte) => self.bump(),
                Some(_) | None => return Ok(()),
            }
        }
    }

    /// Skips white-space only.
    pub(crate) fn skip_white_space(&mut self) -> Result<(), PdfError> {
        while self.peek()?.is_some_and(is_white_space) {
            self.bump();
        }
        Ok(())
    }

    /// The start of a stream's data: `stream` and then LF or CR LF, as the next bytes
    /// (ISO 32000-2:2020, 7.3.8.1); anything else is [`ErrorCode::Malformed`].
    pub(crate) fn stream_start(&mut self) -> Result<u64, PdfError> {
        let line_end = self.token()? == Token::Keyword(Keyword::Stream)
            && match self.next_byte()? {
                Some(b'\n') => true,
                Some(b'\r') => self.next_byte()? == Some(b'\n'),
                Some(_) | None => false,
            };
        if line_end {
            Ok(self.position)
        } else {
            Err(self.malformed())
        }
    }

    /// The next token; the end of the source is [`ErrorCode::Malformed`].
    pub(crate) fn token(&mut self) -> Result<Token, PdfError> {
        if self.broken {
            return Err(self.malformed());
        }
        if let Some(token) = self.pushed.pop() {
            return Ok(token);
        }
        self.skip_space()?;
        let byte = self.next_byte()?.ok_or_else(|| self.malformed())?;
        match byte {
            b'[' => Ok(Token::ArrayStart),
            b']' => Ok(Token::ArrayEnd),
            b'<' if self.peek()? == Some(b'<') => {
                self.bump();
                Ok(Token::DictionaryStart)
            }
            b'<' => self.hex_string(),
            b'>' if self.peek()? == Some(b'>') => {
                self.bump();
                Ok(Token::DictionaryEnd)
            }
            b'(' => self.literal_string(),
            b'/' => self.name(),
            b')' | b'>' | b'{' | b'}' => Err(self.malformed()),
            _ => self.regular(byte),
        }
    }
}

/// The value of a hexadecimal digit.
fn hex_value(byte: u8) -> Option<u8> {
    let value = char::from(byte).to_digit(16)?;
    u8::try_from(value).ok()
}

/// Whether `byte` is a regular character (ISO 32000-2:2020, 7.2.3).
const fn is_regular(byte: u8) -> bool {
    !is_white_space(byte)
        && !matches!(
            byte,
            b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
        )
}

/// Whether `byte` is white-space (ISO 32000-2:2020, 7.2.3).
pub(crate) const fn is_white_space(byte: u8) -> bool {
    matches!(byte, 0 | b'\t' | b'\n' | 0x0C | b'\r' | b' ')
}
