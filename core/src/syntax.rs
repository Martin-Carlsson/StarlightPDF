//! Tokens and objects read from bytes, and objects written in canonical syntax
//! (ISO 32000-2:2020, 7.2 and 7.3).

use core::str::from_utf8;

use crate::input::Source;
use crate::objects::ObjectId;
use crate::stream::Feeder;
use crate::{ErrorCode, PdfError};

/// Bytes read at a time.
const CHUNK: usize = 512;

/// Bytes kept of a name read in passing; longer names keep one more, so match no key.
const NAME_LIMIT: usize = 127;

/// Bytes of a number or keyword; longer ones are [`ErrorCode::Malformed`].
const RUN_LIMIT: usize = 255;

/// Where a direct object starts, in the source or in an object stream's decoded data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct At {
    /// The offset of its first byte.
    offset: u64,
    /// The object stream holding it, by object number; none in the source.
    packed: Option<u32>,
}

/// A container being skipped or written.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Frame {
    /// An array.
    Array,
    /// A dictionary awaiting a key.
    Key,
    /// A dictionary awaiting a value.
    Value,
}

/// A keyword the library tells apart (ISO 32000-2:2020, 7.3).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Keyword {
    /// `endobj`.
    EndObj,
    /// `endstream`.
    EndStream,
    /// `false`.
    False,
    /// `null`.
    Null,
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
    /// `true`.
    True,
}

/// The bytes of the source, or of an object stream's decoded data.
pub(crate) struct Located<'data> {
    /// The object stream's decoded data, when one.
    packed: Option<Feeder<'data>>,
    /// The source.
    source: &'data Source,
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
    /// The bytes of `window` read.
    filled: usize,
    /// The bytes.
    input: I,
    /// Whether strings and names are kept whole; otherwise strings are dropped and names cut.
    keep: bool,
    /// The operation errors name.
    operation: &'static str,
    /// What the objects read belong to, for [`At`].
    place: At,
    /// The offset of the next byte.
    position: u64,
    /// Numbers and keywords looked at ahead and not used, at most two, the next last.
    pushed: [Option<Token>; 2],
    /// The offset of `window`.
    start: u64,
    /// Bytes read at `start`.
    window: [u8; CHUNK],
}

/// A token, or a reference read as one (ISO 32000-2:2020, 7.2 and 7.3.10).
#[derive(Debug, PartialEq)]
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
    /// A name, `#` escapes decoded.
    Name(Vec<u8>),
    /// A real.
    Real(f64),
    /// `number generation R`, from [`Reader::item`] only.
    Reference(ObjectId),
    /// A string's bytes, escapes decoded; empty unless kept.
    String(Vec<u8>),
}

/// A direct object; arrays and dictionaries are read again where they start (ISO 32000-2:2020, 7.3).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    /// An array.
    Array(At),
    /// A boolean.
    Boolean(bool),
    /// A dictionary.
    Dictionary(At),
    /// An integer.
    Integer(i64),
    /// A name.
    Name(Vec<u8>),
    /// The null object.
    Null,
    /// A real.
    Real(f64),
    /// An indirect reference.
    Reference(ObjectId),
    /// A string's bytes.
    String(Vec<u8>),
}

impl At {
    /// A place in the source or, with `packed`, in that object stream's data.
    pub(crate) const fn new(offset: u64, packed: Option<u32>) -> Self {
        Self { offset, packed }
    }

    /// The offset of its first byte.
    pub(crate) const fn offset(self) -> u64 {
        self.offset
    }

    /// The object stream holding it.
    pub(crate) const fn packed(self) -> Option<u32> {
        self.packed
    }
}

impl Value {
    /// Where the array starts, when this is one.
    pub(crate) const fn array(&self) -> Option<At> {
        if let Self::Array(at) = *self {
            Some(at)
        } else {
            None
        }
    }

    /// Where the dictionary starts, when this is one.
    pub(crate) const fn dictionary(&self) -> Option<At> {
        if let Self::Dictionary(at) = *self {
            Some(at)
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

impl Input for Located<'_> {
    fn fill(
        &mut self,
        offset: u64,
        buffer: &mut [u8],
        operation: &'static str,
    ) -> Result<usize, PdfError> {
        match self.packed.as_mut() {
            Some(feeder) => feeder.fill(offset, buffer, operation),
            None => self.source.read(offset, buffer, operation),
        }
    }
}

impl<'data> Located<'data> {
    /// The source, or, with `packed`, that object stream's decoded data.
    pub(crate) const fn new(source: &'data Source, packed: Option<Feeder<'data>>) -> Self {
        Self { packed, source }
    }
}

impl<I> Reader<I>
where
    I: Input,
{
    /// Moves past `count` bytes.
    fn advance(&mut self, count: usize) {
        self.position = self
            .position
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
    }

    /// The bytes from the next to the window's end, reading the window first when it holds none;
    /// empty at the end.
    fn ahead(&mut self) -> Result<&[u8], PdfError> {
        let _next = self.peek()?;
        let index = usize::try_from(self.position.saturating_sub(self.start)).unwrap_or(usize::MAX);
        Ok(self
            .window
            .get(..self.filled)
            .and_then(|read| read.get(index..))
            .unwrap_or_default())
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

    /// The value starting with `token`; a container is skipped when `skip`, nesting at most
    /// `depth_limit` deep.
    pub(crate) fn complete(
        &mut self,
        token: Token,
        depth_limit: u32,
        skip: bool,
    ) -> Result<Value, PdfError> {
        let (frame, back) = match token {
            Token::ArrayStart => (Frame::Array, 1),
            Token::DictionaryStart => (Frame::Key, 2),
            Token::Integer(value) => return Ok(Value::Integer(value)),
            Token::Keyword(Keyword::False) => return Ok(Value::Boolean(false)),
            Token::Keyword(Keyword::Null) => return Ok(Value::Null),
            Token::Keyword(Keyword::True) => return Ok(Value::Boolean(true)),
            Token::Name(name) => return Ok(Value::Name(name)),
            Token::Real(value) => return Ok(Value::Real(value)),
            Token::Reference(id) => return Ok(Value::Reference(id)),
            Token::String(bytes) => return Ok(Value::String(bytes)),
            Token::ArrayEnd
            | Token::DictionaryEnd
            | Token::Keyword(
                Keyword::EndObj
                | Keyword::EndStream
                | Keyword::Obj
                | Keyword::Other
                | Keyword::Reference
                | Keyword::Stream
                | Keyword::Trailer,
            ) => return Err(self.malformed()),
        };
        let at = At {
            offset: self.position.saturating_sub(back),
            ..self.place
        };
        if skip {
            self.skip(frame, depth_limit)?;
        }
        Ok(if frame == Frame::Array {
            Value::Array(at)
        } else {
            Value::Dictionary(at)
        })
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

    /// The first value of the bytes, a container not skipped; nesting at most `depth_limit` deep.
    pub(crate) fn head(&mut self, depth_limit: u32) -> Result<Value, PdfError> {
        let token = self.item()?;
        self.complete(token, depth_limit, false)
    }

    /// The rest of a hexadecimal string (ISO 32000-2:2020, 7.3.4.3).
    fn hex_string(&mut self) -> Result<Token, PdfError> {
        let mut bytes = Vec::new();
        let mut high: Option<u8> = None;
        loop {
            match self.next_byte()? {
                Some(b'>') => break,
                Some(byte) if is_white_space(byte) => {}
                Some(byte) => {
                    let digit = hex_value(byte).ok_or_else(|| self.malformed())?;
                    if let Some(left) = high.take() {
                        if self.keep {
                            bytes.push(left.wrapping_shl(4) | digit);
                        }
                    } else {
                        high = Some(digit);
                    }
                }
                None => return Err(self.malformed()),
            }
        }
        if self.keep {
            bytes.extend(high.map(|left: u8| left.wrapping_shl(4)));
        }
        Ok(Token::String(bytes))
    }

    /// The next token, a reference `number generation R` as one (ISO 32000-2:2020, 7.3.10).
    pub(crate) fn item(&mut self) -> Result<Token, PdfError> {
        let first = self.token()?;
        let Token::Integer(number) = first else {
            return Ok(first);
        };
        let Some(second) = self.small()? else {
            return Ok(first);
        };
        let Token::Integer(generation) = second else {
            self.push_back(second);
            return Ok(first);
        };
        let Some(third) = self.small()? else {
            self.push_back(second);
            return Ok(first);
        };
        if third == Token::Keyword(Keyword::Reference) {
            return u32::try_from(number)
                .ok()
                .zip(u16::try_from(generation).ok())
                .map(|(object, version)| {
                    Token::Reference(ObjectId {
                        generation: version,
                        number: object,
                    })
                })
                .ok_or_else(|| self.malformed());
        }
        self.push_back(third);
        self.push_back(second);
        Ok(first)
    }

    /// The rest of a literal string (ISO 32000-2:2020, 7.3.4.2).
    fn literal_string(&mut self) -> Result<Token, PdfError> {
        let mut bytes = Vec::new();
        let mut depth: u64 = 1;
        loop {
            let keep = self.keep;
            let rest = self.ahead()?;
            let mut plain = 0_usize;
            let mut closed = false;
            for &byte in rest {
                match byte {
                    b'\\' | b'\r' => break,
                    b'(' => depth = depth.saturating_add(1),
                    b')' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            closed = true;
                            break;
                        }
                    }
                    _ => {}
                }
                plain = plain.wrapping_add(1);
            }
            if keep {
                bytes.extend_from_slice(rest.get(..plain).unwrap_or_default());
            }
            self.advance(plain);
            if closed {
                self.bump();
                return Ok(Token::String(bytes));
            }
            let Some(byte) = self.next_byte()? else {
                return Err(self.malformed());
            };
            let kept = match byte {
                b'\\' => self.string_escape()?,
                b'(' => {
                    depth = depth.saturating_add(1);
                    Some(byte)
                }
                b')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Ok(Token::String(bytes));
                    }
                    Some(byte)
                }
                b'\r' => {
                    if self.peek()? == Some(b'\n') {
                        self.bump();
                    }
                    Some(b'\n')
                }
                _ => Some(byte),
            };
            if self.keep {
                bytes.extend(kept);
            }
        }
    }

    /// A [`ErrorCode::Malformed`] error.
    fn malformed(&self) -> PdfError {
        PdfError::new(ErrorCode::Malformed, self.operation)
    }

    /// The rest of a name (ISO 32000-2:2020, 7.3.5).
    fn name(&mut self) -> Result<Token, PdfError> {
        let mut name = Vec::new();
        loop {
            let keep = self.keep;
            let rest = self.ahead()?;
            let count = rest
                .iter()
                .position(|byte| !is_regular(*byte) || *byte == b'#')
                .unwrap_or(rest.len());
            let room = if keep {
                count
            } else {
                NAME_LIMIT.saturating_add(1).saturating_sub(name.len())
            };
            name.extend_from_slice(rest.get(..count.min(room)).unwrap_or_default());
            let next = rest.get(count).copied();
            let more = count > 0 && count == rest.len();
            self.advance(count);
            if next == Some(b'#') {
                self.bump();
                let decoded = self.escape()?.unwrap_or(b'#');
                if keep || name.len() <= NAME_LIMIT {
                    name.push(decoded);
                }
                continue;
            }
            if !more {
                return Ok(Token::Name(name));
            }
        }
    }

    /// A reader of `input` at `position`, failing as `operation`.
    pub(crate) const fn new(input: I, position: u64, operation: &'static str) -> Self {
        Self {
            input,
            keep: false,
            operation,
            place: At::new(0, None),
            position,
            filled: 0,
            pushed: [None, None],
            start: 0,
            window: [0; CHUNK],
        }
    }

    /// The next byte, `None` at the end.
    fn next_byte(&mut self) -> Result<Option<u8>, PdfError> {
        let byte = self.peek()?;
        self.bump();
        Ok(byte)
    }

    /// The next token, `None` at the end.
    pub(crate) fn next_token(&mut self) -> Result<Option<Token>, PdfError> {
        if let Some(token) = self.pop_back() {
            return Ok(Some(token));
        }
        self.skip_space()?;
        let Some(byte) = self.next_byte()? else {
            return Ok(None);
        };
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
        .map(Some)
    }

    /// The next byte, without moving past it.
    fn peek(&mut self) -> Result<Option<u8>, PdfError> {
        if let Some(byte) = self
            .position
            .checked_sub(self.start)
            .and_then(|index| usize::try_from(index).ok())
            .filter(|index| *index < self.filled)
            .and_then(|index| self.window.get(index))
        {
            return Ok(Some(*byte));
        }
        self.filled = self
            .input
            .fill(self.position, &mut self.window, self.operation)?
            .min(CHUNK);
        self.start = self.position;
        Ok(self.window.first().copied().filter(|_| self.filled > 0))
    }

    /// The token looked at ahead last, taken.
    fn pop_back(&mut self) -> Option<Token> {
        self.pushed.iter_mut().rev().find_map(Option::take)
    }

    /// The offset of the next byte.
    pub(crate) const fn position(&self) -> u64 {
        self.position
    }

    /// Keeps `token`, looked at ahead, to be read next.
    fn push_back(&mut self, token: Token) {
        if let Some(slot) = self.pushed.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(token);
        }
    }

    /// The rest of a run of regular characters: a number or a keyword (ISO 32000-2:2020, 7.3.2 and 7.3.3).
    fn regular(&mut self, first: u8) -> Result<Token, PdfError> {
        let mut buffer = [0; RUN_LIMIT];
        buffer[0] = first;
        let mut length = 1_usize;
        let operation = self.operation;
        loop {
            let rest = self.ahead()?;
            let count = rest
                .iter()
                .position(|byte| !is_regular(*byte))
                .unwrap_or(rest.len());
            let end = length.saturating_add(count);
            buffer
                .get_mut(length..end)
                .ok_or_else(|| PdfError::new(ErrorCode::Malformed, operation))?
                .copy_from_slice(rest.get(..count).unwrap_or_default());
            length = end;
            let more = count > 0 && count == rest.len();
            self.advance(count);
            if !more {
                break;
            }
        }
        let run = buffer.get(..length).unwrap_or_default();
        let digits = run
            .strip_prefix(b"+")
            .or_else(|| run.strip_prefix(b"-"))
            .unwrap_or(run);
        let point = digits.iter().position(|byte| *byte == b'.');
        let text = from_utf8(run).unwrap_or_default();
        match run {
            b"R" => Ok(Token::Keyword(Keyword::Reference)),
            b"endobj" => Ok(Token::Keyword(Keyword::EndObj)),
            b"endstream" => Ok(Token::Keyword(Keyword::EndStream)),
            b"false" => Ok(Token::Keyword(Keyword::False)),
            b"null" => Ok(Token::Keyword(Keyword::Null)),
            b"obj" => Ok(Token::Keyword(Keyword::Obj)),
            b"stream" => Ok(Token::Keyword(Keyword::Stream)),
            b"trailer" => Ok(Token::Keyword(Keyword::Trailer)),
            b"true" => Ok(Token::Keyword(Keyword::True)),
            _ if !digits
                .iter()
                .all(|byte| byte.is_ascii_digit() || *byte == b'.') =>
            {
                Ok(Token::Keyword(Keyword::Other))
            }
            _ if !digits.is_empty() && point.is_none() => Ok(text.parse().map_or_else(
                |_| text.parse().map_or(Token::Real(0.0), Token::Real),
                Token::Integer,
            )),
            _ if point.is_some()
                && point == digits.iter().rposition(|byte| *byte == b'.')
                && digits.iter().any(u8::is_ascii_digit) =>
            {
                text.parse()
                    .ok()
                    .map(Token::Real)
                    .ok_or_else(|| self.malformed())
            }
            _ => Err(self.malformed()),
        }
    }

    /// Moves to `position`; only forward over decoded stream data.
    pub(crate) fn seek(&mut self, position: u64) {
        self.position = position;
        self.pushed = [None, None];
    }

    /// Keeps strings and names whole from now on, or not.
    pub(crate) const fn set_keep(&mut self, keep: bool) {
        self.keep = keep;
    }

    /// Sets the object stream whose data is read, by object number.
    pub(crate) const fn set_place(&mut self, packed: Option<u32>) {
        self.place = At::new(0, packed);
    }

    /// Skips the rest of a container opened as `frame`, nesting at most `depth_limit` deep;
    /// dictionary keys must be names.
    fn skip(&mut self, frame: Frame, depth_limit: u32) -> Result<(), PdfError> {
        let limit = usize::try_from(depth_limit).unwrap_or(usize::MAX);
        let mut frames = Vec::new();
        frames.push(frame);
        while let Some(&top) = frames.last() {
            let token = self.item()?;
            if top == Frame::Key {
                match token {
                    Token::DictionaryEnd => drop(frames.pop()),
                    Token::Name(_) => frames.push(Frame::Value),
                    Token::ArrayEnd
                    | Token::ArrayStart
                    | Token::DictionaryStart
                    | Token::Integer(_)
                    | Token::Keyword(_)
                    | Token::Real(_)
                    | Token::Reference(_)
                    | Token::String(_) => return Err(self.malformed()),
                }
                continue;
            }
            if top == Frame::Value {
                let _closed = frames.pop();
            }
            match token {
                Token::ArrayEnd if top == Frame::Array => drop(frames.pop()),
                Token::ArrayStart | Token::DictionaryStart if frames.len() >= limit => {
                    return Err(PdfError::new(ErrorCode::LimitExceeded, self.operation));
                }
                Token::ArrayStart => frames.push(Frame::Array),
                Token::DictionaryStart => frames.push(Frame::Key),
                Token::Integer(_)
                | Token::Keyword(Keyword::False | Keyword::Null | Keyword::True)
                | Token::Name(_)
                | Token::Real(_)
                | Token::Reference(_)
                | Token::String(_) => {}
                Token::ArrayEnd
                | Token::DictionaryEnd
                | Token::Keyword(
                    Keyword::EndObj
                    | Keyword::EndStream
                    | Keyword::Obj
                    | Keyword::Other
                    | Keyword::Reference
                    | Keyword::Stream
                    | Keyword::Trailer,
                ) => return Err(self.malformed()),
            }
        }
        Ok(())
    }

    /// Skips white-space and comments (ISO 32000-2:2020, 7.2.3 and 7.2.4).
    fn skip_space(&mut self) -> Result<(), PdfError> {
        let mut comment = false;
        loop {
            let rest = self.ahead()?;
            let Some(&first) = rest.first() else {
                return Ok(());
            };
            comment = comment || first == b'%';
            let count = if comment {
                let end = rest.iter().position(|byte| matches!(byte, b'\r' | b'\n'));
                comment = end.is_none();
                end.unwrap_or(rest.len())
            } else {
                let count = rest
                    .iter()
                    .position(|byte| !is_white_space(*byte))
                    .unwrap_or(rest.len());
                if count == 0 {
                    return Ok(());
                }
                count
            };
            self.advance(count);
        }
    }

    /// Skips white-space only.
    pub(crate) fn skip_white_space(&mut self) -> Result<(), PdfError> {
        while self.peek()?.is_some_and(is_white_space) {
            self.bump();
        }
        Ok(())
    }

    /// The next token if it is a number, a keyword or `>>`; `None` before another or at the end.
    fn small(&mut self) -> Result<Option<Token>, PdfError> {
        if let Some(token) = self.pop_back() {
            return Ok(Some(token));
        }
        self.skip_space()?;
        match self.peek()? {
            Some(byte) if is_regular(byte) || matches!(byte, b')' | b'>' | b'{' | b'}') => {
                self.next_token()
            }
            Some(_) | None => Ok(None),
        }
    }

    /// The start of a stream's data after its dictionary: `stream` and LF or CR LF
    /// (ISO 32000-2:2020, 7.3.8.1); the offset, and whether only a CR ended the line.
    pub(crate) fn stream_start(&mut self) -> Result<(u64, bool), PdfError> {
        if self.token()? != Token::Keyword(Keyword::Stream) {
            return Err(self.malformed());
        }
        let lone_cr = match self.next_byte()? {
            Some(b'\n') => false,
            Some(b'\r') if self.peek()? == Some(b'\n') => {
                self.bump();
                false
            }
            Some(b'\r') => true,
            Some(_) | None => return Err(self.malformed()),
        };
        Ok((self.position, lone_cr))
    }

    /// The byte a `\` escape in a literal string stands for, after the `\`; `None` for a line
    /// continuation (ISO 32000-2:2020, 7.3.4.2).
    fn string_escape(&mut self) -> Result<Option<u8>, PdfError> {
        let Some(byte) = self.next_byte()? else {
            return Err(self.malformed());
        };
        Ok(match byte {
            b'n' => Some(b'\n'),
            b'r' => Some(b'\r'),
            b't' => Some(b'\t'),
            b'b' => Some(0x08),
            b'f' => Some(0x0C),
            b'\r' => {
                if self.peek()? == Some(b'\n') {
                    self.bump();
                }
                None
            }
            b'\n' => None,
            b'0'..=b'7' => {
                let mut value = byte.wrapping_sub(b'0');
                for _ in 0_u8..2 {
                    match self.peek()? {
                        Some(digit @ b'0'..=b'7') => {
                            self.bump();
                            value = value.wrapping_shl(3) | digit.wrapping_sub(b'0');
                        }
                        Some(_) | None => break,
                    }
                }
                Some(value)
            }
            _ => Some(byte),
        })
    }

    /// The next token; the end of the bytes is [`ErrorCode::Malformed`].
    pub(crate) fn token(&mut self) -> Result<Token, PdfError> {
        self.next_token()?.ok_or_else(|| self.malformed())
    }

    /// The next value, a container skipped; nesting at most `depth_limit` deep.
    pub(crate) fn value(&mut self, depth_limit: u32) -> Result<Value, PdfError> {
        let token = self.item()?;
        self.complete(token, depth_limit, true)
    }

    /// The next value in canonical syntax, its strings through `strings`; nesting at most
    /// `depth_limit` deep.
    pub(crate) fn write(
        &mut self,
        depth_limit: u32,
        strings: &mut dyn FnMut(Vec<u8>) -> Result<Vec<u8>, PdfError>,
    ) -> Result<String, PdfError> {
        let limit = usize::try_from(depth_limit).unwrap_or(usize::MAX);
        self.keep = true;
        let mut out = String::new();
        let mut frames = Vec::new();
        let mut open = true;
        loop {
            let token = self.item()?;
            let top = frames.last().copied();
            if !open && token != Token::ArrayEnd {
                out.push(' ');
            }
            open = false;
            if top == Some(Frame::Key) {
                match token {
                    Token::DictionaryEnd => {
                        let _closed = frames.pop();
                        out.push_str(">>");
                    }
                    Token::Name(name) => {
                        write_name(&mut out, &name);
                        frames.push(Frame::Value);
                    }
                    Token::ArrayEnd
                    | Token::ArrayStart
                    | Token::DictionaryStart
                    | Token::Integer(_)
                    | Token::Keyword(_)
                    | Token::Real(_)
                    | Token::Reference(_)
                    | Token::String(_) => return Err(self.malformed()),
                }
            } else {
                if top == Some(Frame::Value) {
                    let _closed = frames.pop();
                }
                match token {
                    Token::ArrayEnd if top == Some(Frame::Array) => {
                        let _closed = frames.pop();
                        out.push(']');
                    }
                    Token::ArrayStart | Token::DictionaryStart if frames.len() >= limit => {
                        return Err(PdfError::new(ErrorCode::LimitExceeded, self.operation));
                    }
                    Token::ArrayStart => {
                        frames.push(Frame::Array);
                        out.push('[');
                        open = true;
                    }
                    Token::DictionaryStart => {
                        frames.push(Frame::Key);
                        out.push_str("<<");
                    }
                    Token::Integer(value) => out.push_str(&value.to_string()),
                    Token::Keyword(Keyword::False) => out.push_str("false"),
                    Token::Keyword(Keyword::Null) => out.push_str("null"),
                    Token::Keyword(Keyword::True) => out.push_str("true"),
                    Token::Name(name) => write_name(&mut out, &name),
                    Token::Real(value) => write_real(&mut out, value),
                    Token::Reference(id) => {
                        out.push_str(&id.number.to_string());
                        out.push(' ');
                        out.push_str(&id.generation.to_string());
                        out.push_str(" R");
                    }
                    Token::String(bytes) => write_string(&mut out, &strings(bytes)?),
                    Token::ArrayEnd
                    | Token::DictionaryEnd
                    | Token::Keyword(
                        Keyword::EndObj
                        | Keyword::EndStream
                        | Keyword::Obj
                        | Keyword::Other
                        | Keyword::Reference
                        | Keyword::Stream
                        | Keyword::Trailer,
                    ) => return Err(self.malformed()),
                }
            }
            if frames.is_empty() {
                return Ok(out);
            }
        }
    }
}

/// The bytes of `word` that equal `byte`, each as its top bit, by arithmetic alone.
pub(crate) fn matching(word: u64, byte: u8) -> u64 {
    const LOW: u64 = 0x7F7F_7F7F_7F7F_7F7F;
    let differ = word ^ 0x0101_0101_0101_0101_u64.wrapping_mul(u64::from(byte));
    !((differ & LOW).wrapping_add(LOW) | differ | LOW)
}

/// The value of a hexadecimal digit.
fn hex_value(byte: u8) -> Option<u8> {
    let value = char::from(byte).to_digit(16)?;
    u8::try_from(value).ok()
}

/// Eight bytes as a word, the first most significant: one load, not eight.
pub(crate) fn word(group: [u8; 8]) -> u64 {
    group
        .into_iter()
        .fold(0, |word, byte| (word << 8_u8) | u64::from(byte))
}

/// Whether `byte` is a regular character (ISO 32000-2:2020, 7.2.3).
pub(crate) const fn is_regular(byte: u8) -> bool {
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

/// Appends the hexadecimal digits of `byte`, uppercase when `upper`.
fn push_hex(out: &mut String, byte: u8, upper: bool) {
    for nibble in [byte.wrapping_shr(4), byte & 0x0F] {
        let digit = char::from_digit(u32::from(nibble), 16).unwrap_or('0');
        out.push(if upper {
            digit.to_ascii_uppercase()
        } else {
            digit
        });
    }
}

/// Appends `name` in canonical syntax: `#xx` for bytes outside `!` to `~` and delimiters
/// (ISO 32000-2:2020, 7.3.5).
pub(crate) fn write_name(out: &mut String, name: &[u8]) {
    out.push('/');
    for &byte in name {
        if byte.is_ascii_graphic() && is_regular(byte) && byte != b'#' {
            out.push(char::from(byte));
        } else {
            out.push('#');
            push_hex(out, byte, true);
        }
    }
}

/// Appends `number` in canonical syntax, as read back: 0 when not finite, else when longer
/// than a number read, the nearest that is not (ISO 32000-2:2020, 7.3.3).
pub(crate) fn write_real(out: &mut String, number: f64) {
    let text = number.to_string();
    if !number.is_finite() {
        out.push('0');
    } else if text.len() <= RUN_LIMIT {
        out.push_str(&text);
    } else {
        let sign = if number.is_sign_negative() { "-" } else { "" };
        let room = RUN_LIMIT.saturating_sub(sign.len());
        if number.abs() >= 1.0_f64 {
            // Past the largest integer part room holds: all nines.
            out.push_str(sign);
            out.push_str(&"9".repeat(room));
        } else {
            // Without the leading 0, rounded to the places left after the point.
            let places = room.saturating_sub(1);
            let rounded = format!("{:.places$}", number.abs());
            let fraction = rounded.trim_start_matches('0').trim_end_matches('0');
            if fraction.len() > 1 {
                out.push_str(sign);
                out.push_str(fraction);
            } else {
                out.push('0');
            }
        }
    }
}

/// Appends `bytes` in canonical syntax: literal when all printable ASCII, else hexadecimal
/// (ISO 32000-2:2020, 7.3.4).
pub(crate) fn write_string(out: &mut String, bytes: &[u8]) {
    if bytes.iter().all(|byte| (0x20..=0x7E).contains(byte)) {
        out.push('(');
        for &byte in bytes {
            if matches!(byte, b'\\' | b'(' | b')') {
                out.push('\\');
            }
            out.push(char::from(byte));
        }
        out.push(')');
    } else {
        out.push('<');
        for &byte in bytes {
            push_hex(out, byte, false);
        }
        out.push('>');
    }
}
