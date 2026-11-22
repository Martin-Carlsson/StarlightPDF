//! Saving documents and splitting them. Expectations come from the output bytes, read here without
//! the library; saved output has no object streams (ISO 32000-2:2020, 7.5.7).

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::sync::Arc;
use core::fmt::Write as _;
use core::mem::take;
use core::ops::Range;
use core::sync::atomic::{AtomicU32, Ordering};
use std::fs;
use std::io::{self, ErrorKind};
#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt as _, symlink};
#[cfg(windows)]
use std::os::windows::fs::symlink_file as symlink;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use starlightpdf::arrange::{SplitBy, SplitOptions};
use starlightpdf::document::{Document, OpenOptions, SaveOptions};
use starlightpdf::input::{BytesSourceOptions, FileSourceOptions, Source};
use starlightpdf::metadata::SetInfoDictionaryOptions;
use starlightpdf::output::{Append, AppendSinkOptions, FileSinkOptions, MemorySinkOptions, Sink};
use starlightpdf::progress::Progress;
use starlightpdf::{ErrorCode, PdfError};

/// The corpus directory.
const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/corpus");

/// MD5 rotations per step (RFC 1321, 3.4).
const MD5_SHIFTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// MD5 constants, the integer part of 2^32 × |sin(i)| (RFC 1321, 3.4).
const MD5_SINES: [u32; 64] = [
    0xd76a_a478,
    0xe8c7_b756,
    0x2420_70db,
    0xc1bd_ceee,
    0xf57c_0faf,
    0x4787_c62a,
    0xa830_4613,
    0xfd46_9501,
    0x6980_98d8,
    0x8b44_f7af,
    0xffff_5bb1,
    0x895c_d7be,
    0x6b90_1122,
    0xfd98_7193,
    0xa679_438e,
    0x49b4_0821,
    0xf61e_2562,
    0xc040_b340,
    0x265e_5a51,
    0xe9b6_c7aa,
    0xd62f_105d,
    0x0244_1453,
    0xd8a1_e681,
    0xe7d3_fbc8,
    0x21e1_cde6,
    0xc337_07d6,
    0xf4d5_0d87,
    0x455a_14ed,
    0xa9e3_e905,
    0xfcef_a3f8,
    0x676f_02d9,
    0x8d2a_4c8a,
    0xfffa_3942,
    0x8771_f681,
    0x6d9d_6122,
    0xfde5_380c,
    0xa4be_ea44,
    0x4bde_cfa9,
    0xf6bb_4b60,
    0xbebf_bc70,
    0x289b_7ec6,
    0xeaa1_27fa,
    0xd4ef_3085,
    0x0488_1d05,
    0xd9d4_d039,
    0xe6db_99e5,
    0x1fa2_7cf8,
    0xc4ac_5665,
    0xf429_2244,
    0x432a_ff97,
    0xab94_23a7,
    0xfc93_a039,
    0x655b_59c3,
    0x8f0c_cc92,
    0xffef_f47d,
    0x8584_5dd1,
    0x6fa8_7e4f,
    0xfe2c_e6e0,
    0xa301_4314,
    0x4e08_11a1,
    0xf753_7e82,
    0xbd3a_f235,
    0x2ad7_d2bb,
    0xeb86_d391,
];

/// The mode `restrict` gives a file: neither a default nor a temporary file's (POSIX `chmod`).
#[cfg(unix)]
const MODE: u32 = 0o640;

/// A dictionary's entries.
type Dictionary = BTreeMap<Vec<u8>, Value>;

/// Collects what a save appends.
struct Collected(Mutex<Vec<u8>>);

/// Cancels at its second call, the first after bytes are written, noting the bytes its
/// directory's other files then hold.
struct Cancel {
    /// Calls so far.
    calls: AtomicU32,
    /// The directory written into.
    dir: PathBuf,
    /// Bytes of the files other than the previous one at the second call.
    written: Mutex<u64>,
}

/// A file read here: its objects, a later definition replacing an earlier (7.5.6), and trailer.
struct File {
    /// Indirect objects by number.
    objects: BTreeMap<u32, Object>,
    /// The trailer dictionary.
    trailer: Dictionary,
}

/// Reads tokens (7.2, 7.3).
struct Lexer<'data> {
    /// The next byte's offset.
    at: usize,
    /// The file.
    bytes: &'data [u8],
}

/// An indirect object.
struct Object {
    /// A stream's stored bytes (7.3.8).
    data: Option<Vec<u8>>,
    /// The object, or a stream's dictionary.
    value: Value,
}

/// A page, read through its tree with inheritance (7.7.3.4).
struct Page {
    /// Its content streams' stored bytes, in order (7.7.3.3).
    contents: Vec<Vec<u8>>,
    /// Its object number.
    number: u32,
    /// The stored bytes of every stream its resources reach.
    resources: BTreeSet<Vec<u8>>,
}

/// A token (7.2.3).
enum Token {
    /// `]`.
    ArrayEnd,
    /// `[`.
    ArrayStart,
    /// `>>`.
    DictionaryEnd,
    /// `<<`.
    DictionaryStart,
    /// A keyword or other regular characters.
    Keyword(Vec<u8>),
    /// A name, `#` escapes decoded.
    Name(Vec<u8>),
    /// A number, normalized.
    Number(String),
    /// A string's bytes.
    String(Vec<u8>),
}

/// An object (7.3); numbers are compared by normalized text, references by object number.
#[derive(Clone, Eq, PartialEq)]
enum Value {
    /// An array.
    Array(Vec<Self>),
    /// A boolean.
    Bool(bool),
    /// A dictionary.
    Dictionary(Dictionary),
    /// A name.
    Name(Vec<u8>),
    /// Null, or a missing entry.
    Null,
    /// A number without sign on zero, leading or trailing zeros.
    Number(String),
    /// An indirect reference's object number.
    Reference(u32),
    /// A string.
    String(Vec<u8>),
}

impl Append for Collected {
    fn append(&self, bytes: &[u8]) -> io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(())
    }
}

impl Progress for Cancel {
    fn update(&self, _done: u64, _total: u64) -> bool {
        if self.calls.fetch_add(1, Ordering::Relaxed) < 1 {
            return true;
        }
        let others: u64 = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_name() != "previous.pdf")
            // `DirEntry::metadata` on Windows gives the directory's size, stale while open.
            .filter_map(|entry| fs::metadata(entry.path()).ok())
            .map(|metadata| metadata.len())
            .sum();
        *self.written.lock().unwrap_or_else(PoisonError::into_inner) = others;
        false
    }
}

impl File {
    /// Annotation `index` of the page object `page` (12.5.2).
    fn annotation(&self, page: u32, index: usize) -> Result<Dictionary, String> {
        let annotations = self
            .get(&self.dictionary(Value::Reference(page))?, "Annots")
            .array()
            .ok_or("no annotations")?;
        self.dictionary(
            annotations
                .get(index)
                .cloned()
                .ok_or("no such annotation")?,
        )
    }

    /// The stored bytes of the normal appearance of `annotation` (12.5.5).
    fn appearance(&self, annotation: &Dictionary) -> Option<Vec<u8>> {
        self.data(
            self.dictionary(self.get(annotation, "AP"))
                .ok()?
                .get(b"N".as_slice())?,
        )
    }

    /// The stored bytes of the stream `value` refers to.
    fn data(&self, value: &Value) -> Option<Vec<u8>> {
        let Value::Reference(number) = *value else {
            return None;
        };
        self.objects.get(&number)?.data.clone()
    }

    /// `value`, resolved, as a dictionary.
    fn dictionary(&self, value: Value) -> Result<Dictionary, String> {
        self.resolve(value)
            .dictionary()
            .ok_or_else(|| String::from("not a dictionary"))
    }

    /// `key` of `dictionary`, resolved; `Null` when missing.
    fn get(&self, dictionary: &Dictionary, key: &str) -> Value {
        dictionary
            .get(key.as_bytes())
            .cloned()
            .map_or(Value::Null, |value| self.resolve(value))
    }

    /// The number of page objects (7.7.3.3).
    fn page_objects(&self) -> usize {
        self.objects
            .values()
            .filter(|object| {
                object.value.dictionary().is_some_and(|dictionary| {
                    dictionary.get(b"Type".as_slice()) == Some(&name("Page"))
                })
            })
            .count()
    }

    /// The pages in order (7.7.3).
    fn pages(&self) -> Result<Vec<Page>, String> {
        let mut pages = Vec::new();
        self.walk(
            self.root()?
                .get(b"Pages".as_slice())
                .cloned()
                .ok_or("no page tree")?,
            None,
            0,
            &mut pages,
        )?;
        Ok(pages)
    }

    /// The objects `value` reaches, and the stored bytes of the streams among them; a `Parent`
    /// entry is followed only when `parents`.
    fn reach(
        &self,
        value: &Value,
        parents: bool,
        seen: &mut BTreeSet<u32>,
        streams: &mut BTreeSet<Vec<u8>>,
    ) {
        match value.clone() {
            Value::Reference(number) => {
                if seen.insert(number)
                    && let Some(object) = self.objects.get(&number)
                {
                    if let Some(data) = object.data.as_ref() {
                        let _new = streams.insert(data.clone());
                    }
                    self.reach(&object.value, parents, seen, streams);
                }
            }
            Value::Array(items) => {
                for item in &items {
                    self.reach(item, parents, seen, streams);
                }
            }
            Value::Dictionary(entries) => {
                for (key, item) in &entries {
                    if parents || key.as_slice() != b"Parent" {
                        self.reach(item, parents, seen, streams);
                    }
                }
            }
            Value::Bool(_) | Value::Name(_) | Value::Null | Value::Number(_) | Value::String(_) => {
            }
        }
    }

    /// `value`, references followed.
    fn resolve(&self, value: Value) -> Value {
        let mut current = value;
        for _ in 0..32_u8 {
            let Value::Reference(number) = current else {
                break;
            };
            current = self
                .objects
                .get(&number)
                .map_or(Value::Null, |object| object.value.clone());
        }
        current
    }

    /// The catalog (7.7.2).
    fn root(&self) -> Result<Dictionary, String> {
        self.dictionary(self.get(&self.trailer, "Root"))
    }

    /// The stored bytes of every stream.
    fn streams(&self) -> BTreeSet<Vec<u8>> {
        self.objects
            .values()
            .filter_map(|object| object.data.clone())
            .collect()
    }

    /// Appends the pages under `node`, inheriting `resources` (7.7.3.4).
    fn walk(
        &self,
        node: Value,
        inherited: Option<Value>,
        depth: u8,
        pages: &mut Vec<Page>,
    ) -> Result<(), String> {
        let Value::Reference(number) = node else {
            return Err(String::from("page tree node not a reference"));
        };
        if depth > 32 {
            return Err(String::from("page tree too deep"));
        }
        let dictionary = self.dictionary(node)?;
        let resources = dictionary
            .get(b"Resources".as_slice())
            .cloned()
            .or(inherited);
        if self.get(&dictionary, "Type") == name("Pages") {
            for kid in self.get(&dictionary, "Kids").array().ok_or("no kids")? {
                self.walk(kid, resources.clone(), depth.saturating_add(1), pages)?;
            }
            return Ok(());
        }
        let stored = dictionary
            .get(b"Contents".as_slice())
            .cloned()
            .unwrap_or(Value::Null);
        let parts = self
            .resolve(stored.clone())
            .array()
            .unwrap_or_else(|| vec![stored]);
        let contents = parts
            .iter()
            .map(|part| self.data(part).ok_or("content not a stream"))
            .collect::<Result<_, _>>()?;
        let mut streams = BTreeSet::new();
        if let Some(value) = resources.as_ref() {
            self.reach(value, false, &mut BTreeSet::new(), &mut streams);
        }
        pages.push(Page {
            contents,
            number,
            resources: streams,
        });
        Ok(())
    }
}

impl Lexer<'_> {
    /// Moves past the next byte.
    const fn bump(&mut self) {
        self.at = self.at.saturating_add(1);
    }

    /// The next byte.
    fn byte(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    /// A hexadecimal string's bytes, after its `<` (7.3.4.3).
    fn hex(&mut self) -> Vec<u8> {
        let mut digits = Vec::new();
        while let Some(byte) = self.byte() {
            self.bump();
            if byte == b'>' {
                break;
            }
            if let Some(digit) = char::from(byte).to_digit(16) {
                digits.push(u8::try_from(digit).unwrap_or_default());
            }
        }
        if digits.len() & 1 == 1 {
            digits.push(0);
        }
        digits
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[high, low]| (high << 4_u8) | low)
            .collect()
    }

    /// A literal string's bytes, after its `(` (7.3.4.2).
    fn literal(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut depth = 0_u32;
        while let Some(byte) = self.byte() {
            self.bump();
            match byte {
                b'\\' => {
                    let Some(escaped) = self.byte() else { break };
                    self.bump();
                    match escaped {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut code = u32::from(escaped.wrapping_sub(b'0'));
                            for _ in 0..2_u8 {
                                let Some(digit @ b'0'..=b'7') = self.byte() else {
                                    break;
                                };
                                self.bump();
                                code = code
                                    .wrapping_mul(8)
                                    .wrapping_add(u32::from(digit.wrapping_sub(b'0')));
                            }
                            out.push(u8::try_from(code & 0xFF).unwrap_or_default());
                        }
                        b'\r' => {
                            if self.byte() == Some(b'\n') {
                                self.bump();
                            }
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
                b')' if depth == 0 => break,
                b')' => {
                    depth = depth.saturating_sub(1);
                    out.push(byte);
                }
                b'(' => {
                    depth = depth.saturating_add(1);
                    out.push(byte);
                }
                other => out.push(other),
            }
        }
        out
    }

    /// A name's bytes, after its `/` (7.3.5).
    fn name(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(byte) = self.byte()
            && regular(byte)
        {
            self.bump();
            let escaped = self
                .bytes
                .get(self.at..self.at.saturating_add(2))
                .filter(|_pair| byte == b'#')
                .and_then(|pair| str::from_utf8(pair).ok())
                .and_then(|pair| u8::from_str_radix(pair, 16).ok());
            if let Some(decoded) = escaped {
                self.at = self.at.saturating_add(2);
                out.push(decoded);
            } else {
                out.push(byte);
            }
        }
        out
    }

    /// Moves past white space and comments (7.2.3, 7.2.4).
    fn skip(&mut self) {
        while let Some(byte) = self.byte() {
            if byte == b'%' {
                while let Some(next) = self.byte()
                    && next != b'\n'
                    && next != b'\r'
                {
                    self.bump();
                }
            } else if b"\0\t\n\x0c\r ".contains(&byte) {
                self.bump();
            } else {
                break;
            }
        }
    }

    /// A stream's bytes up to `endstream`, after its `stream` keyword (7.3.8.1).
    fn stream(&mut self) -> Result<Vec<u8>, String> {
        if self.byte() == Some(b'\r') {
            self.bump();
        }
        if self.byte() == Some(b'\n') {
            self.bump();
        }
        let start = self.at;
        let rest = self.bytes.get(start..).unwrap_or_default();
        let length = rest
            .windows(9)
            .position(|window| window == b"endstream")
            .ok_or("no endstream")?;
        self.at = start.saturating_add(length).saturating_add(9);
        Ok(rest.get(..length).unwrap_or_default().to_vec())
    }

    /// The next token, if any.
    fn token(&mut self) -> Option<Token> {
        self.skip();
        let start = self.at;
        let first = self.byte()?;
        self.bump();
        Some(match first {
            b'[' => Token::ArrayStart,
            b']' => Token::ArrayEnd,
            b'<' if self.byte() == Some(b'<') => {
                self.bump();
                Token::DictionaryStart
            }
            b'>' if self.byte() == Some(b'>') => {
                self.bump();
                Token::DictionaryEnd
            }
            b'<' => Token::String(self.hex()),
            b'(' => Token::String(self.literal()),
            b'/' => Token::Name(self.name()),
            _ => {
                while let Some(byte) = self.byte()
                    && regular(byte)
                    && regular(first)
                {
                    self.bump();
                }
                let word = self.bytes.get(start..self.at).unwrap_or_default();
                if word.iter().any(u8::is_ascii_digit)
                    && word.iter().all(|byte| b"+-.0123456789".contains(byte))
                {
                    Token::Number(normalize(word))
                } else {
                    Token::Keyword(word.to_vec())
                }
            }
        })
    }

    /// The object that begins with `token` (7.3).
    fn value(&mut self, token: Token) -> Result<Value, String> {
        Ok(match token {
            Token::ArrayStart => {
                let mut items = Vec::new();
                loop {
                    match self.token().ok_or("unterminated array")? {
                        Token::ArrayEnd => break,
                        item @ (Token::ArrayStart
                        | Token::DictionaryEnd
                        | Token::DictionaryStart
                        | Token::Keyword(_)
                        | Token::Name(_)
                        | Token::Number(_)
                        | Token::String(_)) => items.push(self.value(item)?),
                    }
                }
                Value::Array(items)
            }
            Token::DictionaryStart => {
                let mut entries = Dictionary::new();
                loop {
                    match self.token().ok_or("unterminated dictionary")? {
                        Token::DictionaryEnd => break,
                        Token::Name(key) => {
                            let next = self.token().ok_or("dictionary without value")?;
                            let _old = entries.insert(key, self.value(next)?);
                        }
                        Token::ArrayEnd
                        | Token::ArrayStart
                        | Token::DictionaryStart
                        | Token::Keyword(_)
                        | Token::Number(_)
                        | Token::String(_) => {
                            return Err(String::from("dictionary key not a name"));
                        }
                    }
                }
                Value::Dictionary(entries)
            }
            Token::Number(text) => {
                let saved = self.at;
                if !text.contains(['.', '-'])
                    && let Some(Token::Number(generation)) = self.token()
                    && !generation.contains(['.', '-'])
                    && let Some(Token::Keyword(word)) = self.token()
                    && word == b"R"
                {
                    return text
                        .parse()
                        .map(Value::Reference)
                        .map_err(|_err| String::from("bad object number"));
                }
                self.at = saved;
                Value::Number(text)
            }
            Token::Keyword(word) => match word.as_slice() {
                b"true" => Value::Bool(true),
                b"false" => Value::Bool(false),
                b"null" => Value::Null,
                _ => return Err(String::from("unexpected keyword")),
            },
            Token::Name(bytes) => Value::Name(bytes),
            Token::String(bytes) => Value::String(bytes),
            Token::ArrayEnd | Token::DictionaryEnd => {
                return Err(String::from("unexpected delimiter"));
            }
        })
    }
}

impl Value {
    /// The items, when an array.
    fn array(&self) -> Option<Vec<Self>> {
        if let Self::Array(items) = self.clone() {
            Some(items)
        } else {
            None
        }
    }

    /// The entries, when a dictionary.
    fn dictionary(&self) -> Option<Dictionary> {
        if let Self::Dictionary(entries) = self.clone() {
            Some(entries)
        } else {
            None
        }
    }
}

/// A split by `count` pages.
fn by_pages(count: u32) -> SplitOptions {
    let mut options = SplitOptions::default();
    options.by = SplitBy::Pages;
    options.count = count;
    options
}

/// A split at `starts`.
fn by_ranges(starts: &[u32]) -> SplitOptions {
    let mut options = SplitOptions::default();
    options.by = SplitBy::Ranges;
    options.starts = starts.to_vec();
    options
}

/// The path of a corpus file.
fn corpus(relative: &str) -> PathBuf {
    PathBuf::from(format!("{CORPUS}/{relative}"))
}

/// An error's code and message.
fn failure(error: &PdfError) -> String {
    format!("{}: {error}", error.code().name())
}

/// The file identifier's two elements and the MD5 of the bytes before the trailer (14.4).
fn identifier(bytes: &[u8]) -> Result<([Value; 2], Vec<u8>), String> {
    let at = bytes
        .windows(7)
        .rposition(|window| window == b"trailer")
        .ok_or("no trailer")?;
    let digest = md5(bytes.get(..at).unwrap_or_default()).to_vec();
    let file = parse(bytes)?;
    let elements = file.get(&file.trailer, "ID").array().ok_or("no ID")?;
    let pair = <[Value; 2]>::try_from(elements).map_err(|_items| String::from("ID not a pair"))?;
    Ok((pair, digest))
}

/// The output intent's condition and profile bytes (14.11.5).
fn intent(file: &File) -> Result<(Value, Option<Vec<u8>>), String> {
    let first = file
        .get(&file.root()?, "OutputIntents")
        .array()
        .and_then(|intents| intents.into_iter().next())
        .ok_or("no output intent")?;
    let intent = file.dictionary(first)?;
    Ok((
        file.get(&intent, "OutputConditionIdentifier"),
        intent
            .get(b"DestOutputProfile".as_slice())
            .and_then(|profile| file.data(profile)),
    ))
}

/// The names of the entries of `dir`, sorted.
fn listing(dir: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for found in fs::read_dir(dir).map_err(|error| error.to_string())? {
        let entry = found.map_err(|error| error.to_string())?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}

/// The MD5 digest of `data` (RFC 1321).
fn md5(data: &[u8]) -> [u8; 16] {
    let mut message = data.to_vec();
    let mut bits = u64::try_from(data.len())
        .unwrap_or_default()
        .wrapping_mul(8);
    message.push(0x80);
    while message.len() & 63 != 56 {
        message.push(0);
    }
    for _ in 0..8_u8 {
        message.push(u8::try_from(bits & 0xFF).unwrap_or_default());
        bits >>= 8_u8;
    }
    let mut state = [0x6745_2301_u32, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    for block in message.as_chunks::<64>().0 {
        let words: Vec<u32> = block
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| {
                bytes
                    .iter()
                    .rev()
                    .fold(0, |word, &byte| (word << 8_u8) | u32::from(byte))
            })
            .collect();
        let [mut first, mut second, mut third, mut fourth] = state;
        for (step, (&sine, &shift)) in MD5_SINES.iter().zip(MD5_SHIFTS.iter()).enumerate() {
            let (mixed, word) = match step {
                0..=15 => ((second & third) | (!second & fourth), step),
                16..=31 => (
                    (second & fourth) | (third & !fourth),
                    step.wrapping_mul(5).wrapping_add(1) & 15,
                ),
                32..=47 => (
                    second ^ third ^ fourth,
                    step.wrapping_mul(3).wrapping_add(5) & 15,
                ),
                _ => (third ^ (second | !fourth), step.wrapping_mul(7) & 15),
            };
            let sum = mixed
                .wrapping_add(first)
                .wrapping_add(sine)
                .wrapping_add(words.get(word).copied().unwrap_or_default());
            first = fourth;
            fourth = third;
            third = second;
            second = second.wrapping_add(sum.rotate_left(shift));
        }
        let [was_first, was_second, was_third, was_fourth] = state;
        state = [
            was_first.wrapping_add(first),
            was_second.wrapping_add(second),
            was_third.wrapping_add(third),
            was_fourth.wrapping_add(fourth),
        ];
    }
    let mut digest = [0; 16];
    let bytes = state.iter().flat_map(|&word| {
        [0_u32, 8, 16, 24]
            .map(|shift| u8::try_from(word.wrapping_shr(shift) & 0xFF).unwrap_or_default())
    });
    for (out, byte) in digest.iter_mut().zip(bytes) {
        *out = byte;
    }
    digest
}

/// The name `text`.
fn name(text: &str) -> Value {
    Value::Name(text.as_bytes().to_vec())
}

/// A number's text without sign on zero, leading or trailing zeros.
fn normalize(word: &[u8]) -> String {
    let text = String::from_utf8_lossy(word);
    let (negative, unsigned) = text.strip_prefix('-').map_or_else(
        || (false, text.strip_prefix('+').unwrap_or(&text)),
        |rest| (true, rest),
    );
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let integral = whole.trim_start_matches('0');
    let fractional = fraction.trim_end_matches('0');
    let mut out = String::new();
    if negative && (!integral.is_empty() || !fractional.is_empty()) {
        out.push('-');
    }
    out.push_str(if integral.is_empty() { "0" } else { integral });
    if !fractional.is_empty() {
        out.push('.');
        out.push_str(fractional);
    }
    out
}

/// The number `text`.
fn number(text: &str) -> Value {
    Value::Number(normalize(text.as_bytes()))
}

/// The first kid of `file`'s page tree root, a page here (7.7.3).
fn first_page(file: &File) -> Result<Dictionary, String> {
    let tree = file.dictionary(file.get(&file.root()?, "Pages"))?;
    let kid = file
        .get(&tree, "Kids")
        .array()
        .and_then(|kids| kids.first().cloned())
        .ok_or("no kid")?;
    file.dictionary(kid)
}

/// Opens the file at `path`.
fn open(path: &Path) -> Result<Document, String> {
    let mut options = FileSourceOptions::default();
    options.file = path.to_path_buf();
    Source::from_file(&options)
        .and_then(|source| Document::open(&source, None, &OpenOptions::default()))
        .map_err(|error| failure(&error))
}

/// Reads `bytes`: every indirect object, stream bytes cut to `Length` (7.3.8.2), and the last
/// trailer, or a cross-reference stream's dictionary (7.5.8).
fn parse(bytes: &[u8]) -> Result<File, String> {
    let mut lexer = Lexer { at: 0, bytes };
    let mut objects = BTreeMap::new();
    let mut trailer = None;
    let mut numbers: [Option<String>; 2] = [None, None];
    while let Some(token) = lexer.token() {
        let [before, last] = take(&mut numbers);
        match token {
            Token::Number(text) => numbers = [last, Some(text)],
            Token::Keyword(word) if word == b"trailer" => {
                let next = lexer.token().ok_or("empty trailer")?;
                trailer = lexer.value(next)?.dictionary();
            }
            Token::Keyword(word) if word == b"obj" => {
                let number = before
                    .and_then(|text| text.parse().ok())
                    .ok_or("object without number")?;
                let next = lexer.token().ok_or("empty object")?;
                let value = lexer.value(next)?;
                let saved = lexer.at;
                let data = if let Some(Token::Keyword(keyword)) = lexer.token()
                    && keyword == b"stream"
                {
                    Some(lexer.stream()?)
                } else {
                    lexer.at = saved;
                    None
                };
                let _old = objects.insert(number, Object { data, value });
            }
            Token::ArrayEnd
            | Token::ArrayStart
            | Token::DictionaryEnd
            | Token::DictionaryStart
            | Token::Keyword(_)
            | Token::Name(_)
            | Token::String(_) => {}
        }
    }
    let mut file = File {
        objects,
        trailer: Dictionary::new(),
    };
    let lengths: BTreeMap<u32, Option<usize>> = file
        .objects
        .iter()
        .filter(|entry| entry.1.data.is_some())
        .map(|(&number, object)| {
            let length = object
                .value
                .dictionary()
                .map(|dictionary| file.get(&dictionary, "Length"))
                .and_then(|length| {
                    if let Value::Number(text) = length {
                        text.parse().ok()
                    } else {
                        None
                    }
                });
            (number, length)
        })
        .collect();
    for (number, length) in lengths {
        let data = file
            .objects
            .get_mut(&number)
            .and_then(|object| object.data.as_mut())
            .ok_or("stream vanished")?;
        if let Some(stored) = length {
            if stored > data.len() {
                return Err(format!("object {number}: Length past endstream"));
            }
            data.truncate(stored);
        } else {
            for &end in b"\n\r" {
                if data.last() == Some(&end) {
                    let _end = data.pop();
                }
            }
        }
    }
    file.trailer = if let Some(dictionary) = trailer {
        dictionary
    } else {
        let xref = file
            .objects
            .iter()
            .find(|entry| {
                entry.1.value.dictionary().is_some_and(|dictionary| {
                    dictionary.get(b"Type".as_slice()) == Some(&name("XRef"))
                })
            })
            .map(|(&number, _object)| number)
            .ok_or("no trailer")?;
        file.objects
            .remove(&xref)
            .and_then(|object| object.value.dictionary())
            .ok_or("no trailer")?
    };
    Ok(file)
}

/// Splits the corpus file `relative` with `options` and saves each part.
fn parts(relative: &str, options: &SplitOptions) -> Result<Vec<Vec<u8>>, String> {
    let documents = open(&corpus(relative))?
        .split(options)
        .map_err(|error| failure(&error))?;
    let mut saved = Vec::new();
    while let Some(part) = documents.next().map_err(|error| failure(&error))? {
        saved.push(save(&part)?);
    }
    Ok(saved)
}

/// Gives `path` the mode `MODE` and returns `mode`; Windows has no mode bits, so there it only
/// returns `mode`.
fn restrict(path: &Path) -> io::Result<u32> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(MODE))?;
    mode(path)
}

/// The mode bits of `path`.
#[cfg(unix)]
fn mode(path: &Path) -> io::Result<u32> {
    fs::metadata(path).map(|metadata| metadata.permissions().mode() & 0o777)
}

/// Whether `path` is read-only: Windows has no mode bits, and std reads no access control list.
#[cfg(windows)]
fn mode(path: &Path) -> io::Result<u32> {
    fs::metadata(path).map(|metadata| u32::from(metadata.permissions().readonly()))
}

/// `document` saved with the default options to memory.
fn save(document: &Document) -> Result<Vec<u8>, String> {
    let sink = Sink::in_memory(&MemorySinkOptions::default()).map_err(|error| failure(&error))?;
    let _report = document
        .save(&sink, None, &SaveOptions::default())
        .map_err(|error| failure(&error))?;
    sink.take_bytes().map_err(|error| failure(&error))
}

/// A sink over the file `path`.
fn file_sink(path: &Path) -> Result<Sink, String> {
    let mut options = FileSinkOptions::default();
    options.file = path.to_path_buf();
    Sink::from_file(&options).map_err(|error| failure(&error))
}

/// An empty directory for test `name`.
fn scratch(name: &str) -> Result<PathBuf, String> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("save")
        .join(name);
    match fs::remove_dir_all(&dir) {
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(error.to_string()),
        Ok(()) | Err(_) => {}
    }
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

/// Whether `bytes` holds `marker`.
fn holds(bytes: &[u8], marker: &str) -> bool {
    bytes
        .windows(marker.len())
        .any(|window| window == marker.as_bytes())
}

/// Fails with `message` unless `holds`.
fn ensure(holds: bool, message: &str) -> Result<(), String> {
    if holds {
        Ok(())
    } else {
        Err(String::from(message))
    }
}

/// The bytes of the file at `path`.
fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| format!("{}: {error}", path.display()))
}

/// The memory, append and file sinks receive the same bytes.
#[test]
fn sinks_receive_the_same_bytes() -> Result<(), String> {
    let document = open(&corpus("split/pages.pdf"))?;
    let memory = save(&document)?;
    let collected = Arc::new(Collected(Mutex::new(Vec::new())));
    let append: Arc<dyn Append> = Arc::<Collected>::clone(&collected);
    let sink = Sink::from_append(&append, &AppendSinkOptions::default())
        .map_err(|error| failure(&error))?;
    let _report = document
        .save(&sink, None, &SaveOptions::default())
        .map_err(|error| failure(&error))?;
    let appended = collected
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let file = scratch("sinks")?.join("out.pdf");
    let _written = document
        .save(&file_sink(&file)?, None, &SaveOptions::default())
        .map_err(|error| failure(&error))?;
    ensure(!memory.is_empty(), "memory sink empty")?;
    ensure(appended == memory, "append sink differs")?;
    ensure(read(&file)? == memory, "file sink differs")
}

/// A save that fails, refused (ISO 32000-2:2020, 12.8) or cancelled after writing, leaves the
/// file it would replace and no temporary file.
#[test]
fn failed_save_keeps_the_previous_file() -> Result<(), String> {
    let dir = scratch("failed")?;
    let target = dir.join("previous.pdf");
    let previous = b"previous bytes".to_vec();
    let stop = Cancel {
        calls: AtomicU32::new(0),
        dir: dir.clone(),
        written: Mutex::new(0),
    };
    let cancel: &dyn Progress = &stop;
    for (relative, progress, code) in [
        ("save/signed.pdf", None, ErrorCode::SignaturesWouldBreak),
        ("split/pages.pdf", Some(cancel), ErrorCode::Cancelled),
    ] {
        fs::write(&target, &previous).map_err(|error| error.to_string())?;
        let document = open(&corpus(relative))?;
        let sink = file_sink(&target)?;
        let saved = document.save(&sink, progress, &SaveOptions::default());
        drop(sink);
        let got = saved.map_or_else(|error| error.code().name(), |_report| "ok");
        ensure(
            got == code.name(),
            &format!("{relative}: {got}, not {}", code.name()),
        )?;
        ensure(
            read(&target)? == previous,
            &format!("{relative}: previous file changed"),
        )?;
        ensure(
            listing(&dir)? == ["previous.pdf"],
            &format!("{relative}: files left"),
        )?;
    }
    ensure(
        *stop.written.lock().unwrap_or_else(PoisonError::into_inner) > 0,
        "cancelled before writing",
    )
}

/// Saving over the opened file through a relative symbolic link replaces its target, keeps the
/// link and the target's permissions.
#[test]
fn saving_over_the_opened_file_keeps_permissions_and_resolves_symlinks() -> Result<(), String> {
    let dir = scratch("over")?;
    let target = dir.join("target.pdf");
    let link = dir.join("link.pdf");
    let _copied =
        fs::copy(corpus("split/pages.pdf"), &target).map_err(|error| error.to_string())?;
    let permissions = restrict(&target).map_err(|error| error.to_string())?;
    symlink(Path::new("target.pdf"), &link).map_err(|error| format!("symlink: {error}"))?;
    let document = open(&link)?;
    let expected = save(&document)?;
    let _report = document
        .save(&file_sink(&link)?, None, &SaveOptions::default())
        .map_err(|error| failure(&error))?;
    ensure(
        fs::symlink_metadata(&link).is_ok_and(|metadata| metadata.file_type().is_symlink()),
        "link replaced",
    )?;
    ensure(
        fs::read_link(&link).is_ok_and(|to| to == Path::new("target.pdf")),
        "link target changed",
    )?;
    ensure(read(&target)? == expected, "target not the saved document")?;
    ensure(
        mode(&target).map_err(|error| error.to_string())? == permissions,
        "permissions changed",
    )?;
    ensure(listing(&dir)? == ["link.pdf", "target.pdf"], "files left")
}

/// Saving to a dangling relative symbolic link keeps the link and creates its target.
#[test]
fn saving_to_a_dangling_symlink_creates_its_target() -> Result<(), String> {
    let dir = scratch("dangling")?;
    let link = dir.join("link.pdf");
    symlink(Path::new("target.pdf"), &link).map_err(|error| format!("symlink: {error}"))?;
    let document = open(&corpus("split/pages.pdf"))?;
    let _report = document
        .save(&file_sink(&link)?, None, &SaveOptions::default())
        .map_err(|error| failure(&error))?;
    ensure(
        fs::symlink_metadata(&link).is_ok_and(|metadata| metadata.file_type().is_symlink()),
        "link replaced",
    )?;
    ensure(
        read(&dir.join("target.pdf"))? == save(&document)?,
        "target not the saved document",
    )
}

/// Emptying the only info entry leaves no information dictionary (ISO 32000-2:2020, 14.3.3).
#[test]
fn emptied_info_is_left_out() -> Result<(), String> {
    let document = open(&corpus("info/title-only.pdf"))?;
    let mut options = SetInfoDictionaryOptions::default();
    options.title = Some(String::new());
    document
        .set_info_dictionary(&options)
        .map_err(|error| failure(&error))?;
    let file = parse(&save(&document)?)?;
    ensure(
        !file.trailer.contains_key(b"Info".as_slice()),
        "trailer keeps Info",
    )
}

/// A dictionary of tens of thousands of keys, each repeated, saves each key once with its last
/// value (ISO 32000-2:2020, 7.3.7).
#[test]
fn a_repeated_key_keeps_its_last_value() -> Result<(), String> {
    const KEYS: u32 = 40_000;
    let mut text = String::from("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10]");
    for pass in 0..2 {
        for key in 0..KEYS {
            write!(text, " /K{key} {}", key + pass).map_err(|error| error.to_string())?;
        }
    }
    text.push_str(" >>");
    let mut bytes = b"%PDF-2.0\n".to_vec();
    let mut offsets = Vec::new();
    for body in [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        text.as_str(),
    ] {
        offsets.push(bytes.len());
        bytes.extend(format!("{} 0 obj\n{body}\nendobj\n", offsets.len()).into_bytes());
    }
    let table = bytes.len();
    bytes.extend(b"xref\n0 4\n0000000000 65535 f\r\n");
    for offset in offsets {
        bytes.extend(format!("{offset:010} 00000 n\r\n").into_bytes());
    }
    bytes.extend(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{table}\n%%EOF\n").into_bytes(),
    );
    let mut options = BytesSourceOptions::default();
    options.bytes = Arc::new(bytes);
    let document = Source::from_bytes(&options)
        .and_then(|source| Document::open(&source, None, &OpenOptions::default()))
        .map_err(|error| failure(&error))?;
    let saved = save(&document)?;
    let keys = saved
        .windows(3)
        .filter(|window| window.starts_with(b"/K") && window.get(2).is_some_and(u8::is_ascii_digit))
        .count();
    ensure(keys == 40_000, &format!("{keys} keys written, not {KEYS}"))?;
    let file = parse(&saved)?;
    let page = first_page(&file)?;
    ensure(
        (0..KEYS).all(|key| {
            page.get(format!("K{key}").as_bytes()) == Some(&number(&(key + 1).to_string()))
        }),
        "a key lost its last value",
    )
}

/// An Info added to a file whose trailer `Size` is below its objects' numbers takes no number in
/// use: the page stays a page (ISO 32000-2:2020, 7.5.4, 7.5.5, 14.3.3).
#[test]
fn a_small_size_loses_no_object() -> Result<(), String> {
    let document = open(&corpus("save/small-size.pdf"))?;
    let mut options = SetInfoDictionaryOptions::default();
    options.title = Some(String::from("Small"));
    document
        .set_info_dictionary(&options)
        .map_err(|error| failure(&error))?;
    let file = parse(&save(&document)?)?;
    let page = first_page(&file)?;
    ensure(
        file.get(&page, "MediaBox") == Value::Array(["0", "0", "300", "400"].map(number).to_vec()),
        "the page lost its MediaBox",
    )?;
    let info = file.dictionary(file.get(&file.trailer, "Info"))?;
    ensure(
        file.get(&info, "Title") == Value::String(b"Small".to_vec()),
        "the Info lost its Title",
    )
}

/// A rewrite holds only objects reachable from the trailer: no superseded revision (ISO
/// 32000-2:2020, 7.5.6) nor unreferenced object.
#[test]
fn saved_file_holds_only_reachable_objects() -> Result<(), String> {
    let bytes = save(&open(&corpus("save/unreachable.pdf"))?)?;
    for marker in [
        "old-revision",
        "orphan-dictionary",
        "orphan-stream",
        "orphan-cycle",
    ] {
        ensure(!holds(&bytes, marker), &format!("holds {marker}"))?;
    }
    ensure(holds(&bytes, "new-revision"), "lost the current content")?;
    let file = parse(&bytes)?;
    let mut reached = BTreeSet::new();
    file.reach(
        &Value::Dictionary(file.trailer.clone()),
        true,
        &mut reached,
        &mut BTreeSet::new(),
    );
    ensure(
        file.objects.keys().copied().collect::<BTreeSet<u32>>() == reached,
        "holds an object the trailer does not reach",
    )
}

/// Every stream keeps its stored bytes, whatever its filters or Length (ISO 32000-2:2020, 7.3.8,
/// 7.4).
#[test]
fn unchanged_streams_keep_their_bytes() -> Result<(), String> {
    let path = corpus("save/streams.pdf");
    let source = parse(&read(&path)?)?;
    let saved = parse(&save(&open(&path)?)?)?;
    ensure(saved.streams() == source.streams(), "stream bytes differ")
}

/// Two openings of a file save to identical bytes: no clock, no randomness (ISO 32000-2:2020,
/// 14.4).
#[test]
fn saving_twice_gives_identical_bytes() -> Result<(), String> {
    for relative in ["split/pages.pdf", "split/info-intents-layers.pdf"] {
        let path = corpus(relative);
        ensure(
            save(&open(&path)?)? == save(&open(&path)?)?,
            &format!("{relative}: saves of two openings differ"),
        )?;
    }
    Ok(())
}

/// The file identifier is the MD5 of the bytes before the trailer; a source's first element is
/// kept (ISO 32000-2:2020, 14.4; RFC 1321).
#[test]
fn file_identifier_derives_from_content() -> Result<(), String> {
    let ([first, second], plain) = identifier(&save(&open(&corpus("save/unreachable.pdf"))?)?)?;
    ensure(
        first == Value::String(plain.clone()) && second == first,
        "a new identifier is not the digest twice",
    )?;
    let ([kept, changed], derived) = identifier(&save(&open(&corpus("save/streams.pdf"))?)?)?;
    ensure(
        kept == Value::String(vec![
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD,
            0xEE, 0xFF,
        ]),
        "source's first element not kept",
    )?;
    ensure(
        changed == Value::String(derived.clone()),
        "second element not the digest",
    )?;
    ensure(derived != plain, "different content, same identifier")?;
    let part = parts("save/streams.pdf", &by_pages(1))?;
    let ([own, again], digest) = identifier(part.first().ok_or("no part")?)?;
    ensure(
        own == Value::String(digest) && again == own,
        "a part's identifier is not the digest twice",
    )
}

/// Each part holds its pages' content streams in order, and exactly the streams of their own and
/// inherited resources, byte for byte (ISO 32000-2:2020, 7.7.3.3, 7.7.3.4).
#[test]
fn split_parts_keep_streams_byte_for_byte() -> Result<(), String> {
    let source = parse(&read(&corpus("split/pages.pdf"))?)?.pages()?;
    let saved = parts("split/pages.pdf", &by_ranges(&[1, 3]))?;
    let ranges: [Range<usize>; 2] = [1..3, 3..5];
    ensure(saved.len() == ranges.len(), "not two parts")?;
    for (bytes, range) in saved.iter().zip(ranges) {
        let file = parse(bytes)?;
        let expected = source.get(range.clone()).unwrap_or_default();
        let contents: Vec<Vec<Vec<u8>>> = file
            .pages()?
            .into_iter()
            .map(|page| page.contents)
            .collect();
        let wanted: Vec<Vec<Vec<u8>>> = expected.iter().map(|page| page.contents.clone()).collect();
        let label = format!("part of pages {}..{}", range.start, range.end);
        ensure(contents == wanted, &format!("{label}: contents differ"))?;
        let streams: BTreeSet<Vec<u8>> = expected
            .iter()
            .flat_map(|page| page.contents.iter().chain(&page.resources).cloned())
            .collect();
        ensure(
            file.streams() == streams,
            &format!("{label}: streams differ"),
        )?;
    }
    Ok(())
}

/// A saved part holds as many page objects as it has pages (ISO 32000-2:2020, 7.7.3.3).
#[test]
fn split_part_holds_only_its_page_objects() -> Result<(), String> {
    let counts = parts("split/pages.pdf", &by_pages(2))?
        .iter()
        .map(|bytes| parse(bytes).map(|file| file.page_objects().to_string()))
        .collect::<Result<Vec<_>, _>>()?
        .join(" ");
    ensure(
        counts == "2 2 1",
        &format!("page objects per part: {counts}"),
    )
}

/// Links between pages of one part still lead to them, by destination and by `GoTo` action (ISO
/// 32000-2:2020, 12.3.2.2, 12.6.4.2).
#[test]
fn split_keeps_links_within_a_part() -> Result<(), String> {
    let saved = parts("split/links.pdf", &by_ranges(&[1]))?;
    ensure(saved.len() == 1, "not one part")?;
    let file = parse(saved.first().ok_or("no part")?)?;
    let pages: Vec<u32> = file.pages()?.iter().map(|page| page.number).collect();
    let [first, second] = <[u32; 2]>::try_from(pages).map_err(|_pages| "not two pages")?;
    let link = file.annotation(first, 0)?;
    ensure(
        file.get(&link, "Dest") == Value::Array(vec![Value::Reference(second), name("Fit")]),
        "destination of the first page's link",
    )?;
    let action = file.dictionary(file.get(&file.annotation(second, 0)?, "A"))?;
    ensure(
        file.get(&action, "D")
            == Value::Array(vec![
                Value::Reference(first),
                name("XYZ"),
                number("0"),
                number("200"),
                number("0"),
            ]),
        "destination of the second page's GoTo action",
    )
}

/// Every part carries the output condition, the profile bytes and the shown layer on (ISO
/// 32000-2:2020, 8.11.4, 14.11.5); the conformance suite covers the rest.
#[test]
fn split_parts_carry_info_output_intents_and_layers() -> Result<(), String> {
    let path = "split/info-intents-layers.pdf";
    let source = parse(&read(&corpus(path))?)?;
    let saved = parts(path, &by_pages(1))?;
    ensure(saved.len() == 2, "not two parts")?;
    for bytes in &saved {
        let file = parse(bytes)?;
        let (condition, profile) = intent(&file)?;
        ensure(
            condition == Value::String(b"Carried condition".to_vec()),
            "output condition",
        )?;
        ensure(
            profile.is_some() && profile == intent(&source)?.1,
            "output profile bytes",
        )?;
        let properties = file.dictionary(file.get(&file.root()?, "OCProperties"))?;
        let default = file.dictionary(file.get(&properties, "D"))?;
        let off = file.get(&default, "OFF").array().unwrap_or_default();
        let base_off = file.get(&default, "BaseState") == name("OFF");
        let mut shown = false;
        for group in file.get(&properties, "OCGs").array().ok_or("no OCGs")? {
            let named = file.get(&file.dictionary(group.clone())?, "Name");
            if named == Value::String(b"Shown".to_vec()) {
                let on = file.get(&default, "ON").array().unwrap_or_default();
                shown = !off.contains(&group) && (on.contains(&group) || !base_off);
            }
        }
        ensure(shown, "shown layer")?;
    }
    Ok(())
}

/// With `allow_loss`, a link to a page in another part keeps its rectangle and appearance and
/// loses its destination (ISO 32000-2:2020, 12.5.5, 12.5.6.5).
#[test]
fn allow_loss_keeps_the_appearance_of_an_outside_link() -> Result<(), String> {
    let path = "split/outside-link.pdf";
    let source = parse(&read(&corpus(path))?)?;
    let source_page = source
        .pages()?
        .first()
        .map(|page| page.number)
        .ok_or("no page")?;
    let original = source.annotation(source_page, 0)?;
    let mut options = by_pages(1);
    options.allow_loss = true;
    let saved = parts(path, &options)?;
    ensure(saved.len() == 2, "not two parts")?;
    let file = parse(saved.first().ok_or("no part")?)?;
    let page = file
        .pages()?
        .first()
        .map(|page| page.number)
        .ok_or("no page")?;
    let link = file.annotation(page, 0)?;
    ensure(file.get(&link, "Subtype") == name("Link"), "not a link")?;
    ensure(
        file.get(&link, "Rect") == source.get(&original, "Rect"),
        "rectangle changed",
    )?;
    ensure(
        file.appearance(&link).is_some() && file.appearance(&link) == source.appearance(&original),
        "appearance stream bytes changed",
    )?;
    ensure(
        !link.contains_key(b"Dest".as_slice()) && !link.contains_key(b"A".as_slice()),
        "link keeps a target",
    )
}

/// Every part carries the page mode, viewer preferences and extensions, page labels from its
/// first page and an open action to a page it holds; one to another part's page, view state, is
/// left out without `allow_loss` (ISO 32000-2:2020, 7.7.2, 12.2, 12.3.2, 12.4.2: pages 0 and 1
/// are labelled i and ii, 2 and 3 A-5 and A-6).
#[test]
fn split_parts_carry_catalog_entries() -> Result<(), String> {
    let saved = parts("split/open-action.pdf", &by_ranges(&[1, 3]))?;
    ensure(saved.len() == 2, "not two parts")?;
    let roman = |start| {
        Value::Dictionary(Dictionary::from([
            (b"S".to_vec(), name("r")),
            (b"St".to_vec(), number(start)),
        ]))
    };
    let lettered = |start| {
        Value::Dictionary(Dictionary::from([
            (b"P".to_vec(), Value::String(b"A-".to_vec())),
            (b"S".to_vec(), name("D")),
            (b"St".to_vec(), number(start)),
        ]))
    };
    let labels = [
        vec![number("0"), roman("2"), number("1"), lettered("5")],
        vec![number("0"), lettered("6")],
    ];
    for (at, (bytes, nums)) in saved.iter().zip(labels).enumerate() {
        let file = parse(bytes)?;
        let root = file.root()?;
        ensure(
            file.get(&root, "PageMode") == name("UseThumbs"),
            "page mode",
        )?;
        let preferences = file.dictionary(file.get(&root, "ViewerPreferences"))?;
        ensure(
            file.get(&preferences, "HideToolbar") == Value::Bool(true),
            "viewer preferences",
        )?;
        let adobe =
            file.dictionary(file.get(&file.dictionary(file.get(&root, "Extensions"))?, "ADBE"))?;
        ensure(
            file.get(&adobe, "ExtensionLevel") == number("3"),
            "extensions",
        )?;
        let page_labels = file.dictionary(file.get(&root, "PageLabels"))?;
        let found: Vec<Value> = file
            .get(&page_labels, "Nums")
            .array()
            .ok_or("no Nums")?
            .into_iter()
            .map(|item| {
                file.dictionary(item.clone())
                    .map_or(item, Value::Dictionary)
            })
            .collect();
        ensure(found == nums, &format!("part {at}: page labels"))?;
        let tree = file.dictionary(file.get(&root, "Pages"))?;
        let kids = file.get(&tree, "Kids").array().ok_or("no Kids")?;
        let action = file.get(&root, "OpenAction");
        if at == 0 {
            ensure(
                action == Value::Array(vec![kids.get(1).cloned().ok_or("no page")?, name("Fit")]),
                "open action",
            )?;
        } else {
            ensure(action == Value::Null, "open action to another part kept")?;
        }
    }
    Ok(())
}

/// Page labels whose tree, kid, `Nums`, keys, values and `St` are each behind a reference to a
/// reference are carried and moved on (ISO 32000-2:2020, 7.3.10, 7.9.7, 12.4.2: page 1 starts
/// decimal labels at 7, so page 2, the second part's first, is 8).
#[test]
fn split_carries_page_labels_behind_references() -> Result<(), String> {
    let saved = parts("split/labels-behind-references.pdf", &by_pages(2))?;
    let file = parse(saved.get(1).ok_or("no second part")?)?;
    let page_labels = file.dictionary(file.get(&file.root()?, "PageLabels"))?;
    let found: Vec<Value> = file
        .get(&page_labels, "Nums")
        .array()
        .ok_or("no Nums")?
        .into_iter()
        .map(|item| {
            file.dictionary(item.clone())
                .map_or(item, Value::Dictionary)
        })
        .collect();
    let expected = vec![
        number("0"),
        Value::Dictionary(Dictionary::from([
            (b"S".to_vec(), name("D")),
            (b"St".to_vec(), number("8")),
        ])),
    ];
    ensure(found == expected, "page labels not carried and moved on")
}

/// A reference whose generation is not its object's is null (ISO 32000-2:2020, 7.3.10).
#[test]
fn references_to_another_generation_are_null() -> Result<(), String> {
    let bytes = save(&open(&corpus("save/generations.pdf"))?)?;
    ensure(!holds(&bytes, "wrong-generation"), "a newer reference kept")?;
    ensure(
        !holds(&bytes, "newer-generation"),
        "an older reference kept",
    )?;
    ensure(
        holds(&bytes, "right-generation"),
        "a matching reference lost",
    )
}

/// Whether `byte` is a regular character (7.2.3).
fn regular(byte: u8) -> bool {
    !b"\0\t\n\x0c\r ()<>[]{}/%".contains(&byte)
}
