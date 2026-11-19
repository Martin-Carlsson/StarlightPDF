//! Tokens and objects read from a source (ISO 32000-2:2020, 7.2 and 7.3).

use core::mem;
use core::str::from_utf8;

use crate::input::Source;
use crate::objects::ObjectId;
use crate::{ErrorCode, PdfError};

/// Bytes read from the source at a time.
const CHUNK: usize = 512;

/// The dictionary keys the library reads; others are not kept.
const KEYS: [&[u8]; 5] = [b"Count", b"Pages", b"Prev", b"Root", b"Type"];

/// Bytes kept of a name; longer names keep one more, so match no key.
const NAME_LIMIT: usize = 127;

/// Bytes of a number or keyword; longer ones fail with [`ErrorCode::LimitExceeded`].
const RUN_LIMIT: usize = 255;

/// An outermost dictionary's values of [`KEYS`]; of duplicates, the last.
#[derive(Default, Eq, PartialEq)]
pub(crate) struct Dictionary(Box<[Option<Object>; KEYS.len()]>);

/// A container being read; nothing in it is kept but an outermost dictionary's [`KEYS`].
enum Frame {
    /// An array.
    Array,
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
    /// `trailer`.
    Trailer,
}

/// An object, holding only what the library reads (ISO 32000-2:2020, 7.3).
#[derive(Eq, PartialEq)]
pub(crate) enum Object {
    /// A dictionary.
    Dictionary(Dictionary),
    /// An integer.
    Integer(i64),
    /// A name, `#` escapes decoded, at most [`NAME_LIMIT`] bytes and one.
    Name(Vec<u8>),
    /// An array, boolean, null, real, string or nested dictionary.
    Other,
    /// An indirect reference.
    Reference(ObjectId),
}

/// Reads tokens and objects from a source through a small window.
pub(crate) struct Reader<'source> {
    /// The operation errors name.
    operation: &'static str,
    /// The offset of the next byte.
    position: u64,
    /// The source.
    source: &'source Source,
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

    /// The reference, when this is one.
    pub(crate) const fn reference(&self) -> Option<ObjectId> {
        if let Self::Reference(id) = *self {
            Some(id)
        } else {
            None
        }
    }
}

impl<'source> Reader<'source> {
    /// `number` as an integer, or the start of a reference `number generation R`
    /// (ISO 32000-2:2020, 7.3.10).
    fn after_integer(&mut self, number: i64) -> Result<Object, PdfError> {
        let mark = self.position;
        if let Some(Token::Integer(generation)) = self.lookahead()?
            && self.lookahead()? == Some(Token::Keyword(Keyword::Reference))
        {
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
        self.position = mark;
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

    /// The next token, or `None` where it is malformed.
    fn lookahead(&mut self) -> Result<Option<Token>, PdfError> {
        match self.token() {
            Ok(token) => Ok(Some(token)),
            Err(error) if error.code() == ErrorCode::Malformed => Ok(None),
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

    /// A reader of `source` at `position`, failing as `operation`.
    pub(crate) const fn new(
        source: &'source Source,
        position: u64,
        operation: &'static str,
    ) -> Self {
        Self {
            operation,
            position,
            source,
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
        let mut outermost = Dictionary::default();
        loop {
            let value = match self.token()? {
                Token::ArrayStart | Token::DictionaryStart if stack.len() >= limit => {
                    return Err(PdfError::new(ErrorCode::LimitExceeded, self.operation));
                }
                Token::ArrayStart => {
                    stack.push(Frame::Array);
                    continue;
                }
                Token::DictionaryStart => {
                    stack.push(Frame::Dictionary);
                    continue;
                }
                Token::ArrayEnd => match stack.pop() {
                    Some(Frame::Array) => Object::Other,
                    Some(Frame::Dictionary | Frame::Key(_)) | None => {
                        return Err(self.malformed());
                    }
                },
                Token::DictionaryEnd => match stack.pop() {
                    Some(Frame::Dictionary) if stack.is_empty() => {
                        Object::Dictionary(mem::take(&mut outermost))
                    }
                    Some(Frame::Dictionary) => Object::Other,
                    Some(Frame::Array | Frame::Key(_)) | None => return Err(self.malformed()),
                },
                Token::Integer(integer) => self.after_integer(integer)?,
                Token::Keyword(Keyword::Constant) | Token::Other => Object::Other,
                Token::Keyword(
                    Keyword::Obj | Keyword::Other | Keyword::Reference | Keyword::Trailer,
                ) => return Err(self.malformed()),
                Token::Name(name) => match stack.pop() {
                    Some(Frame::Dictionary) => {
                        let slot = KEYS
                            .iter()
                            .position(|key| *key == name)
                            .filter(|_| stack.is_empty());
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
                Some(Frame::Array) => stack.push(Frame::Array),
                Some(Frame::Key(slot)) => {
                    if let Some(kept) = slot.and_then(|index| outermost.0.get_mut(index)) {
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
            .source
            .read(self.position, &mut self.window, self.operation)?;
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

    /// The next token; the end of the source is [`ErrorCode::Malformed`].
    pub(crate) fn token(&mut self) -> Result<Token, PdfError> {
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
