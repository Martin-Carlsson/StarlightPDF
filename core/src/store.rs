//! A document's shared state, and its objects read through it (ISO 32000-2:2020, 7.3.8, 7.3.10
//! and 7.5).

use alloc::sync::Arc;
use std::sync::{Mutex, PoisonError, RwLock};

use crate::compliance::Deviation;
use crate::decryption::{self, Handler};
use crate::input::{BytesSourceOptions, Source};
use crate::limits::ResourcePolicy;
use crate::object_stream;
use crate::objects::{ObjectId, XrefEntry, XrefKind};
use crate::overlay::{Data, Object};
use crate::pull::Pull;
use crate::repair;
use crate::security::{Cipher, Encryption};
use crate::stream::{Feeder, decoder};
use crate::syntax::{At, Input, Keyword, Located, Reader, Token, Value};
use crate::xref;
use crate::{ErrorCode, PdfError, Version, parse_header};

/// The keyword after a stream's data (ISO 32000-2:2020, 7.3.8.1).
const ENDSTREAM: &[u8] = b"endstream";

/// Bytes searched for the header (ISO 32000-2:2020, 7.5.2).
const HEADER_SEARCH: usize = 1024;

/// Bytes of each key of the dictionary [`Store`] keeps, at most.
const KEY_BYTES: usize = 16;

/// Keys of the dictionary [`Store`] keeps, at most.
const KEYS: usize = 12;

/// The operation of [`Store::open`].
const OPEN: &str = "Document::open";

/// Bytes scanned at a time for [`ENDSTREAM`].
const SCAN: usize = 4096;

/// A dictionary's keys and their values.
#[derive(Clone, Debug)]
struct Indexed {
    /// Where the dictionary starts.
    at: At,
    /// Whether `keys` holds all its keys short enough to look up.
    complete: bool,
    /// Its keys, each once, in the order first found.
    keys: [Option<Key>; KEYS],
}

/// A key of a dictionary and its value.
#[derive(Clone, Debug)]
struct Key {
    /// The key's length.
    length: u8,
    /// The key's bytes, then zeros.
    name: [u8; KEY_BYTES],
    /// Its last value.
    value: Kept,
}

/// A value kept, or, for one holding bytes, where it starts.
#[derive(Clone, Copy, Debug)]
enum Kept {
    /// No such key.
    Absent,
    /// An array.
    Array(At),
    /// A name or string at this offset, read again.
    At(u64),
    /// A boolean.
    Boolean(bool),
    /// A dictionary.
    Dictionary(At),
    /// An integer.
    Integer(i64),
    /// The null object.
    Null,
    /// A real.
    Real(f64),
    /// An indirect reference.
    Reference(ObjectId),
}

/// A document's objects, read for one operation.
#[derive(Clone, Copy)]
pub(crate) struct Objects<'store> {
    /// The operation errors name.
    operation: &'static str,
    /// Entries found by scanning the file, read in place of the store's, sorted by number.
    scanned: Option<&'store [XrefEntry]>,
    /// The state.
    store: &'store Store,
}

/// A document's shared state: its source, limits, where its objects are and its warnings.
#[derive(Debug)]
pub(crate) struct Store {
    /// One more than the highest object number the cross-reference sections declare.
    declared: u64,
    /// Bytes one stream may decode to.
    decoded_limit: u64,
    /// Nesting depth of arrays and dictionaries.
    depth_limit: u32,
    /// The small dictionary read last, by key, so reading another key of it needs no new parse.
    dictionary: Mutex<Option<Indexed>>,
    /// The last search for `endstream`: where it started and where it found the keyword.
    endstream: Mutex<Option<(u64, Option<u64>)>>,
    /// The offset of the newest cross-reference section, read on demand unless `rebuilt`.
    newest: u64,
    /// Objects allowed.
    object_limit: u64,
    /// Entries rebuilt by scanning the file, sorted by object number.
    rebuilt: RwLock<Option<Vec<XrefEntry>>>,
    /// The entry looked up last, so reading it again needs no new lookup.
    recent: Mutex<Option<XrefEntry>>,
    /// Held while [`Objects::repair`] runs.
    repairing: Mutex<()>,
    /// The security handler, when encrypted.
    security: Option<Handler>,
    /// The source.
    source: Source,
    /// Whether a deviation fails.
    strict: bool,
    /// The newest trailer dictionary, when found.
    trailer: Option<At>,
    /// The header's version.
    version: Version,
    /// Warnings kept.
    warning_limit: u32,
    /// The deviations tolerated.
    warnings: Mutex<Warnings>,
}

/// Deviations tolerated.
#[derive(Debug, Default)]
struct Warnings {
    /// Those past the limit.
    dropped: u64,
    /// Those kept, in the order found.
    kept: Vec<Deviation>,
}

impl Indexed {
    /// Adds key `name` with `value`, which starts at `start`; a key longer than any looked up is
    /// left out. False when there are too many keys.
    fn add(&mut self, name: &[u8], start: u64, value: &Value) -> bool {
        let mut bytes = [0; KEY_BYTES];
        let Some(prefix) = bytes.get_mut(..name.len()) else {
            return true;
        };
        prefix.copy_from_slice(name);
        let Some(slot) = self.keys.iter_mut().find(|slot| {
            slot.as_ref()
                .is_none_or(|found| found.name.get(..usize::from(found.length)) == Some(name))
        }) else {
            return false;
        };
        *slot = Some(Key {
            length: u8::try_from(name.len()).unwrap_or_default(),
            name: bytes,
            value: match *value {
                Value::Array(at) => Kept::Array(at),
                Value::Boolean(flag) => Kept::Boolean(flag),
                Value::Dictionary(at) => Kept::Dictionary(at),
                Value::Integer(number) => Kept::Integer(number),
                Value::Name(_) | Value::String(_) => Kept::At(start),
                Value::Null => Kept::Null,
                Value::Real(number) => Kept::Real(number),
                Value::Reference(id) => Kept::Reference(id),
            },
        });
        true
    }

    /// The value of `key` when this is the whole dictionary at `at`, [`Kept::Absent`] when it has
    /// none.
    fn find(&self, at: At, key: &[u8]) -> Option<Kept> {
        (self.at == at && self.complete).then(|| {
            self.keys
                .iter()
                .flatten()
                .find(|found| found.name.get(..usize::from(found.length)) == Some(key))
                .map_or(Kept::Absent, |found| found.value)
        })
    }
}

impl<'store> Objects<'store> {
    /// The catalog's id, the trailer's `Root` (ISO 32000-2:2020, 7.5.5 and 7.7.2).
    pub(crate) fn catalog(self) -> Result<ObjectId, PdfError> {
        let root = self.get(self.trailer()?, b"Root")?;
        let id = root
            .as_ref()
            .and_then(Value::reference)
            .ok_or_else(|| self.malformed())?;
        let _catalog = self.dictionary(root)?;
        Ok(id)
    }

    /// The data of object stream `number` and its dictionary (ISO 32000-2:2020, 7.5.7).
    fn container(self, number: u32) -> Result<(Box<dyn Pull + 'store>, At), PdfError> {
        let generation = self
            .entry(number)?
            .ok_or_else(|| self.malformed())?
            .generation;
        let id = ObjectId { generation, number };
        let mut reader = self.located(id)?.ok_or_else(|| self.malformed())?;
        let dictionary = reader
            .value(self.depth_limit())?
            .dictionary()
            .ok_or_else(|| self.malformed())?;
        let (start, length) = self.extent(&mut reader, dictionary, true)?;
        Ok((
            decoder(self, Some(id), start, length, dictionary, false)?,
            dictionary,
        ))
    }

    /// Bytes one stream may decode to.
    pub(crate) const fn decoded_limit(self) -> u64 {
        self.store.decoded_limit
    }

    /// Nesting depth of arrays and dictionaries.
    pub(crate) const fn depth_limit(self) -> u32 {
        self.store.depth_limit
    }

    /// Records a deviation from `clause` of ISO 32000-2:2020, or fails with
    /// [`ErrorCode::Malformed`] when strict.
    pub(crate) fn deviation(
        self,
        clause: &str,
        message: &str,
        offset: Option<u64>,
    ) -> Result<(), PdfError> {
        if self.store.strict {
            return Err(PdfError {
                message: String::from(message),
                offset,
                ..self.malformed()
            });
        }
        let mut warnings = self
            .store
            .warnings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if warnings.kept.len() < usize::try_from(self.store.warning_limit).unwrap_or(usize::MAX) {
            warnings.kept.push(Deviation {
                clause: format!("ISO 32000-2:2020, {clause}"),
                message: String::from(message),
                object_number: None,
                offset,
                page: None,
            });
        } else {
            warnings.dropped = warnings.dropped.saturating_add(1);
        }
        drop(warnings);
        Ok(())
    }

    /// The dictionary `value` is or refers to; anything else is [`ErrorCode::Malformed`].
    pub(crate) fn dictionary(self, value: Option<Value>) -> Result<At, PdfError> {
        match value {
            Some(Value::Reference(id)) => self.resolve(id)?.dictionary(),
            Some(found) => found.dictionary(),
            None => None,
        }
        .ok_or_else(|| self.malformed())
    }

    /// A reader after the header `id.number id.generation obj` at `offset`
    /// (ISO 32000-2:2020, 7.3.10); another header is [`ErrorCode::Malformed`].
    pub(crate) fn direct(
        self,
        offset: u64,
        id: ObjectId,
    ) -> Result<Reader<&'store Source>, PdfError> {
        let mut reader = Reader::new(&self.store.source, offset, self.operation);
        if xref::object_header(&mut reader, self.operation)?
            != Some((i64::from(id.number), i64::from(id.generation)))
        {
            return Err(self.malformed());
        }
        Ok(reader)
    }

    /// Whether `endstream` follows `offset`, after white-space.
    fn ends(self, offset: u64) -> Result<bool, PdfError> {
        let mut reader = Reader::new(&self.store.source, offset, self.operation);
        match reader.next_token() {
            Ok(found) => Ok(found == Some(Token::Keyword(Keyword::EndStream))),
            Err(error) if error.code() == ErrorCode::Io => Err(error),
            Err(_) => Ok(false),
        }
    }

    /// The entry of object `number`, the newest; a malformed table rebuilds it
    /// (ISO 32000-2:2020, 7.5.4, 7.5.6 and 7.5.8).
    pub(crate) fn entry(self, number: u32) -> Result<Option<XrefEntry>, PdfError> {
        let find = |entries: &[XrefEntry]| {
            entries
                .binary_search_by_key(&number, |entry| entry.number)
                .ok()
                .and_then(|at| entries.get(at))
                .copied()
        };
        let rebuilt = || {
            self.scanned.map_or_else(
                || {
                    self.store
                        .rebuilt
                        .read()
                        .unwrap_or_else(PoisonError::into_inner)
                        .as_deref()
                        .map(find)
                },
                |entries| Some(find(entries)),
            )
        };
        if let Some(found) = rebuilt() {
            return Ok(found);
        }
        let recent = || {
            self.store
                .recent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
        };
        let hit = (*recent()).filter(|entry| entry.number == number);
        if hit.is_some() {
            return Ok(hit);
        }
        let found = match xref::entry(self, self.store.newest, number) {
            Err(error) if error.code() == ErrorCode::Malformed => {
                self.repair()?;
                return Ok(rebuilt().flatten());
            }
            other => other?,
        };
        if found.is_some() {
            *recent() = found;
        }
        Ok(found)
    }

    /// The offset and length of the data of the stream whose dictionary `reader` has just read;
    /// `indirect_length` allows a `Length` that refers to an object. A wrong or missing `Length`
    /// is a deviation: the data then ends before `endstream` (ISO 32000-2:2020, 7.3.8).
    pub(crate) fn extent<I>(
        self,
        reader: &mut Reader<I>,
        dictionary: At,
        indirect_length: bool,
    ) -> Result<(u64, u64), PdfError>
    where
        I: Input,
    {
        let (start, lone_cr) = reader.stream_start()?;
        if lone_cr {
            self.deviation("7.3.8.1", "stream keyword ends with CR alone", Some(start))?;
        }
        let length = match self.get(dictionary, b"Length")? {
            Some(Value::Integer(value)) => u64::try_from(value).ok(),
            Some(Value::Reference(id)) if indirect_length => self.length(id)?,
            Some(_) | None => None,
        };
        let size = self.store.source.size(self.operation)?;
        if let Some(bytes) = length
            && let Some(end) = start.checked_add(bytes).filter(|end| *end <= size)
            && self.ends(end)?
        {
            return Ok((start, bytes));
        }
        self.deviation("7.3.8.2", "stream Length wrong or missing", Some(start))?;
        let end = self.find_endstream(start, size)?;
        Ok((start, end.saturating_sub(start)))
    }

    /// The end of the data before the first `endstream` from `start`, an end-of-line marker
    /// before it left out; none is [`ErrorCode::Malformed`].
    fn find_endstream(self, start: u64, size: u64) -> Result<u64, PdfError> {
        let last = *self
            .store
            .endstream
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // A search from before `start` that found none before its result found that result.
        let keyword = match last {
            Some((from, found)) if from <= start && found.is_none_or(|at| start <= at) => found,
            Some(_) | None => {
                let found = self.search_endstream(start, size)?;
                *self
                    .store
                    .endstream
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some((start, found));
                found
            }
        };
        let at = keyword.ok_or_else(|| self.malformed())?;
        let from = at.saturating_sub(2).max(start);
        let mut buffer = [0; 2];
        let count = self.store.source.read(
            from,
            buffer
                .get_mut(..usize::try_from(at.saturating_sub(from)).unwrap_or_default())
                .unwrap_or_default(),
            self.operation,
        )?;
        let before = buffer.get(..count).unwrap_or_default();
        let eol = if before.ends_with(b"\r\n") {
            2
        } else {
            u64::from(
                before
                    .last()
                    .is_some_and(|byte| matches!(byte, b'\r' | b'\n')),
            )
        };
        Ok(at.saturating_sub(eol))
    }

    /// The value of key `key` of the dictionary at `dictionary`, the last if repeated, reading the
    /// whole dictionary (ISO 32000-2:2020, 7.3.7).
    pub(crate) fn get(self, dictionary: At, key: &[u8]) -> Result<Option<Value>, PdfError> {
        let kept = self
            .store
            .dictionary
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|indexed| indexed.find(dictionary, key));
        if let Some(found) = kept {
            return Ok(Some(match found {
                Kept::Absent => return Ok(None),
                Kept::Array(at) => Value::Array(at),
                Kept::At(offset) => {
                    let mut reader = self.reader(At::new(offset, dictionary.packed()))?;
                    reader.set_keep(true);
                    reader.value(self.inner_depth())?
                }
                Kept::Boolean(flag) => Value::Boolean(flag),
                Kept::Dictionary(at) => Value::Dictionary(at),
                Kept::Integer(number) => Value::Integer(number),
                Kept::Null => Value::Null,
                Kept::Real(number) => Value::Real(number),
                Kept::Reference(id) => Value::Reference(id),
            }));
        }
        self.index(dictionary, key)
    }

    /// The value of key `key` of the dictionary at `dictionary`, reading it whole and keeping its
    /// keys (ISO 32000-2:2020, 7.3.7).
    fn index(self, dictionary: At, key: &[u8]) -> Result<Option<Value>, PdfError> {
        let inner = self.inner_depth();
        let rebuilt = self.store.is_rebuilt();
        let mut reader = self.reader(dictionary)?;
        let _start = reader.token()?;
        let mut indexed = Indexed {
            at: dictionary,
            complete: true,
            keys: Default::default(),
        };
        let mut found = None;
        loop {
            match reader.token()? {
                Token::DictionaryEnd => {
                    // Not kept when the entries were rebuilt meanwhile: `dictionary` may then be
                    // another; locks taken in `set_rebuilt`'s order.
                    let current = self
                        .store
                        .rebuilt
                        .read()
                        .unwrap_or_else(PoisonError::into_inner);
                    if current.is_some() == rebuilt {
                        *self
                            .store
                            .dictionary
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner) = Some(indexed);
                    }
                    drop(current);
                    return Ok(found);
                }
                Token::Name(name) => {
                    let start = reader.position();
                    let matches = name == key;
                    reader.set_keep(matches);
                    let value = reader.value(inner)?;
                    reader.set_keep(false);
                    if indexed.complete && !indexed.add(&name, start, &value) {
                        indexed.complete = false;
                    }
                    if matches {
                        found = Some(value);
                    }
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
        }
    }

    /// Nesting depth of a dictionary's values.
    fn inner_depth(self) -> u32 {
        self.depth_limit().saturating_sub(1).max(1)
    }

    /// The integer `Length` of a stream in object `id`, in use and not in an object stream;
    /// `None` when it is not (ISO 32000-2:2020, 7.3.8.2).
    fn length(self, id: ObjectId) -> Result<Option<u64>, PdfError> {
        let value = self.located(id).and_then(|found| {
            found.map_or(Ok(Value::Null), |mut reader| {
                reader.value(self.depth_limit())
            })
        });
        match value {
            Ok(found) => Ok(found.integer().and_then(|bytes| u64::try_from(bytes).ok())),
            Err(error) if error.code() == ErrorCode::Malformed => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// A reader after the header of object `id`, in use in the file body; `None` when it is not.
    /// A header missing where the table puts it rebuilds the table (ISO 32000-2:2020, 7.3.10 and
    /// 7.5.4).
    pub(crate) fn located(self, id: ObjectId) -> Result<Option<Reader<&'store Source>>, PdfError> {
        loop {
            let table = self.scanned.is_none() && !self.store.is_rebuilt();
            let Some(entry) = self
                .entry(id.number)?
                .filter(|found| found.kind == XrefKind::InUse && found.generation == id.generation)
            else {
                return Ok(None);
            };
            match self.direct(entry.offset, id) {
                Ok(reader) => return Ok(Some(reader)),
                Err(error) if table && error.code() == ErrorCode::Malformed => self.repair()?,
                Err(error) => return Err(error),
            }
        }
    }

    /// A [`ErrorCode::Malformed`] error.
    pub(crate) fn malformed(self) -> PdfError {
        PdfError::new(ErrorCode::Malformed, self.operation)
    }

    /// Object `id` in canonical syntax, a stream's dictionary for a stream; `null` when free or
    /// missing (ISO 32000-2:2020, 7.3.10).
    pub(crate) fn object(self, id: ObjectId) -> Result<String, PdfError> {
        let depth = self.depth_limit();
        if let Some(mut reader) = self.located(id)? {
            let handler = self.strings(&mut reader, id)?;
            let mut strings = |bytes: Vec<u8>| match handler {
                Some(found) => found.string(id, &bytes, self.operation),
                None => Ok(bytes),
            };
            let text = reader.write(depth, &mut strings)?;
            match reader.next_token()? {
                Some(Token::Keyword(Keyword::EndObj | Keyword::Stream)) => {}
                Some(_) | None => {
                    self.deviation("7.3.10", "endobj missing", Some(reader.position()))?;
                }
            }
            return Ok(text);
        }
        match self.entry(id.number)? {
            Some(entry) if entry.kind == XrefKind::Compressed && id.generation == 0 => {
                self.packed(entry, id.number)?.write(depth, &mut Ok)
            }
            Some(_) | None => Ok(String::from("null")),
        }
    }

    /// The operation errors name.
    pub(crate) const fn operation(self) -> &'static str {
        self.operation
    }

    /// Object `id` read into memory, its strings decrypted, a stream's data left in the source,
    /// read decrypted, so without its own `Crypt` filter; null when free or missing
    /// (ISO 32000-2:2020, 7.3.10, 7.4.10, 7.6.2).
    pub(crate) fn owned(self, id: ObjectId) -> Result<Object, PdfError> {
        let depth = self.depth_limit();
        if let Some(mut reader) = self.located(id)? {
            let handler = self.strings(&mut reader, id)?;
            let mut strings = |bytes: Vec<u8>| match handler {
                Some(found) => found.string(id, &bytes, self.operation),
                None => Ok(bytes),
            };
            reader.set_keep(true);
            let token = reader.item()?;
            let object = self.read(&mut reader, token, depth, &mut strings)?;
            return Ok(match object {
                Object::Dictionary(entries)
                    if reader.next_token()? == Some(Token::Keyword(Keyword::Stream)) =>
                {
                    let mut kept: Vec<_> = entries
                        .into_iter()
                        .filter(|entry| entry.0 != b"Length")
                        .collect();
                    if handler.is_some() {
                        // Made direct, so an indirect `Crypt` filter is found.
                        for entry in &mut kept {
                            if matches!(entry.0.as_slice(), b"Filter" | b"DecodeParms") {
                                for _ in 0..depth {
                                    let Object::Reference(target) = entry.1 else {
                                        break;
                                    };
                                    entry.1 = self.owned(target)?;
                                }
                            }
                        }
                        kept = without_crypt(kept);
                    }
                    Object::Stream(kept, Data::Stored(id))
                }
                other @ (Object::Array(_)
                | Object::Boolean(_)
                | Object::Dictionary(_)
                | Object::Integer(_)
                | Object::Name(_)
                | Object::Null
                | Object::Real(_)
                | Object::Reference(_)
                | Object::Stream(..)
                | Object::String(_)) => other,
            });
        }
        match self.entry(id.number)? {
            Some(entry) if entry.kind == XrefKind::Compressed && id.generation == 0 => {
                let mut reader = self.packed(entry, id.number)?;
                reader.set_keep(true);
                let token = reader.item()?;
                self.read(&mut reader, token, depth, &mut Ok)
            }
            Some(_) | None => Ok(Object::Null),
        }
    }

    /// The trailer dictionary read into memory; an empty one without a source (ISO 32000-2:2020,
    /// 7.5.5).
    pub(crate) fn owned_trailer(self) -> Result<Object, PdfError> {
        let Some(at) = self.store.trailer else {
            return Ok(Object::Dictionary(Vec::new()));
        };
        let mut reader = self.reader(at)?;
        reader.set_keep(true);
        let token = reader.item()?;
        self.read(&mut reader, token, self.depth_limit(), &mut Ok)
    }

    /// A reader at object `number` in the object stream `entry` names, at the entry's index
    /// (ISO 32000-2:2020, 7.5.7).
    fn packed(self, entry: XrefEntry, number: u32) -> Result<Reader<Feeder<'store>>, PdfError> {
        let (data, dictionary) = self.container(entry.stream)?;
        let mut reader = object_stream::member(self, data, dictionary, number, entry.index)?;
        reader.set_place(Some(entry.stream));
        Ok(reader)
    }

    /// The object that starts with `token`, read into memory, nesting at most `depth` deep, its
    /// strings through `strings`; a dictionary keeps a repeated key's last value
    /// (ISO 32000-2:2020, 7.3).
    fn read<I>(
        self,
        reader: &mut Reader<I>,
        token: Token,
        depth: u32,
        strings: &mut dyn FnMut(Vec<u8>) -> Result<Vec<u8>, PdfError>,
    ) -> Result<Object, PdfError>
    where
        I: Input,
    {
        let inner = || {
            depth
                .checked_sub(1)
                .ok_or_else(|| PdfError::new(ErrorCode::LimitExceeded, self.operation))
        };
        Ok(match token {
            Token::ArrayStart => {
                let below = inner()?;
                let mut items = Vec::new();
                loop {
                    let item = reader.item()?;
                    if item == Token::ArrayEnd {
                        break Object::Array(items);
                    }
                    items.push(self.read(reader, item, below, strings)?);
                }
            }
            Token::DictionaryStart => {
                let below = inner()?;
                let mut entries: Vec<(Vec<u8>, Object)> = Vec::new();
                loop {
                    match reader.item()? {
                        Token::DictionaryEnd => break Object::Dictionary(last_values(entries)),
                        Token::Name(key) => {
                            let item = reader.item()?;
                            let value = self.read(reader, item, below, strings)?;
                            entries.push((key, value));
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
                }
            }
            Token::Integer(value) => Object::Integer(value),
            Token::Keyword(Keyword::False) => Object::Boolean(false),
            Token::Keyword(Keyword::Null) => Object::Null,
            Token::Keyword(Keyword::True) => Object::Boolean(true),
            Token::Name(name) => Object::Name(name),
            Token::Real(value) => Object::Real(value),
            Token::Reference(id) => Object::Reference(id),
            Token::String(bytes) => Object::String(strings(bytes)?),
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
        })
    }

    /// A reader at `at`.
    pub(crate) fn reader(self, at: At) -> Result<Reader<Located<'store>>, PdfError> {
        let packed = at
            .packed()
            .map(|number| self.container(number).map(|(data, _)| Feeder::new(data)))
            .transpose()?;
        let input = Located::new(&self.store.source, packed);
        let mut reader = Reader::new(input, at.offset(), self.operation);
        reader.set_place(at.packed());
        Ok(reader)
    }

    /// Rebuilds the entries by scanning the file, once, keeping the trailer and security handler,
    /// and records the deviation (ISO 32000-2:2020, 7.5.4).
    fn repair(self) -> Result<(), PdfError> {
        let _held = self
            .store
            .repairing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.store.is_rebuilt() {
            return Ok(());
        }
        self.deviation("7.5.4", "cross-reference table damaged or missing", None)?;
        let mut found = repair::scan(self, self.store.object_limit)?;
        let entries = repair::newest(found.take_entries());
        self.store
            .set_rebuilt(self.with_members(&entries, found.streams())?);
        Ok(())
    }

    /// Indirect object `id`; null when free or missing (ISO 32000-2:2020, 7.3.10).
    pub(crate) fn resolve(self, id: ObjectId) -> Result<Value, PdfError> {
        let depth = self.depth_limit();
        if let Some(mut reader) = self.located(id)? {
            return reader.head(depth);
        }
        match self.entry(id.number)? {
            Some(entry) if entry.kind == XrefKind::Compressed && id.generation == 0 => {
                self.packed(entry, id.number)?.head(depth)
            }
            Some(_) | None => Ok(Value::Null),
        }
    }

    /// Where the first `endstream` from `start` lies, if any.
    fn search_endstream(self, start: u64, size: u64) -> Result<Option<u64>, PdfError> {
        let mut buffer = vec![0; SCAN];
        let mut at = start;
        let step = u64::try_from(SCAN.saturating_sub(ENDSTREAM.len())).unwrap_or(1);
        while at < size {
            let count = self.store.source.read(at, &mut buffer, self.operation)?;
            if let Some(found) = buffer
                .get(..count)
                .unwrap_or_default()
                .windows(ENDSTREAM.len())
                .position(|window| window == ENDSTREAM)
            {
                return Ok(Some(
                    at.saturating_add(u64::try_from(found).unwrap_or_default()),
                ));
            }
            at = at.saturating_add(step);
        }
        Ok(None)
    }

    /// The security handler, when encrypted.
    pub(crate) const fn security(self) -> Option<&'store Handler> {
        self.store.security.as_ref()
    }

    /// One more than the highest object number the cross-reference sections declare or the
    /// rebuilt entries hold; a trailer's `Size` is not trusted (ISO 32000-2:2020, 7.5.4, 7.5.5,
    /// 7.5.8).
    pub(crate) fn size(self) -> u32 {
        let declared = u32::try_from(self.store.declared).unwrap_or(u32::MAX);
        let held = self
            .store
            .rebuilt
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|entries| entries.last())
            .map_or(0, |entry| entry.number.saturating_add(1));
        declared.max(held)
    }

    /// Entries of a table indexed by object number below `next`: [`ErrorCode::LimitExceeded`]
    /// past the policy's objects, whatever a trailer's `Size` declares (ISO 32000-2:2020, 7.5.5).
    pub(crate) fn slots(self, next: u32) -> Result<usize, PdfError> {
        let limit = || PdfError::new(ErrorCode::LimitExceeded, self.operation);
        if u64::from(next) > self.store.object_limit.saturating_add(1) {
            return Err(limit());
        }
        usize::try_from(next).map_err(|_error| limit())
    }

    /// The source.
    pub(crate) const fn source(self) -> &'store Source {
        &self.store.source
    }

    /// The handler decrypting the strings of object `id`, whose value `reader` is at: none when
    /// unencrypted, for the `Encrypt` dictionary and for a cross-reference stream
    /// (ISO 32000-2:2020, 7.5.8.2 and 7.6.2).
    fn strings(
        self,
        reader: &mut Reader<&'store Source>,
        id: ObjectId,
    ) -> Result<Option<&'store Handler>, PdfError> {
        let Some(handler) = self.security().filter(|found| found.encrypts(id)) else {
            return Ok(None);
        };
        let start = reader.position();
        let head = reader.head(self.depth_limit())?;
        reader.seek(start);
        let xref = match head.dictionary() {
            Some(at) => self
                .get(at, b"Type")?
                .is_some_and(|kind| kind.is_name(b"XRef")),
            None => false,
        };
        Ok((!xref).then_some(handler))
    }

    /// These objects, read through `entries`, sorted by number, in place of the store's.
    const fn through<'view>(self, entries: &'view [XrefEntry]) -> Objects<'view>
    where
        'store: 'view,
    {
        Objects {
            operation: self.operation,
            scanned: Some(entries),
            store: self.store,
        }
    }

    /// The newest trailer dictionary; none is [`ErrorCode::Malformed`].
    pub(crate) fn trailer(self) -> Result<At, PdfError> {
        self.store.trailer.ok_or_else(|| self.malformed())
    }

    /// The header's version (ISO 32000-2:2020, 7.5.2).
    pub(crate) const fn version(self) -> Version {
        self.store.version
    }

    /// `entries`, sorted by number, with the objects of those object streams `streams` that
    /// `entries` places where they are added up to the object limit, read through `entries`
    /// (ISO 32000-2:2020, 7.5.7).
    fn with_members(
        self,
        entries: &[XrefEntry],
        streams: &[(u64, ObjectId)],
    ) -> Result<Vec<XrefEntry>, PdfError> {
        let left = self
            .store
            .object_limit
            .saturating_sub(u64::try_from(entries.len()).unwrap_or(u64::MAX));
        let newest: Vec<(u64, ObjectId)> = streams
            .iter()
            .copied()
            .filter(|&(offset, id)| {
                entries
                    .binary_search_by_key(&id.number, |entry| entry.number)
                    .ok()
                    .and_then(|at| entries.get(at))
                    .is_some_and(|entry| entry.offset == offset)
            })
            .collect();
        let members = repair::members(self.through(entries), &newest, left)?;
        Ok(repair::newest(
            entries
                .iter()
                .map(|entry| (entry.offset, *entry))
                .chain(members)
                .collect(),
        ))
    }
}

impl Store {
    /// Fails with [`ErrorCode::LimitExceeded`] when more than the policy's objects are in use.
    fn count(&self, declared: u64) -> Result<(), PdfError> {
        let limit = self.object_limit;
        let rebuilt = self
            .rebuilt
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(Vec::len);
        let used = match rebuilt {
            Some(entries) => u64::try_from(entries).unwrap_or(u64::MAX),
            None if declared > limit => xref::in_use(self.objects(OPEN), self.newest)?,
            None => declared,
        };
        if used > limit {
            Err(PdfError::new(ErrorCode::LimitExceeded, OPEN))
        } else {
            Ok(())
        }
    }

    /// The state of a document with no source, of `version`, with `policy`'s limits.
    pub(crate) fn create(
        policy: Option<&ResourcePolicy>,
        version: Version,
    ) -> Result<Self, PdfError> {
        let limits = policy.map_or_else(ResourcePolicy::defaults, Clone::clone);
        Ok(Self {
            declared: 0,
            decoded_limit: limits.decoded_limit(),
            depth_limit: limits.depth_limit(),
            dictionary: Mutex::default(),
            endstream: Mutex::default(),
            newest: 0,
            object_limit: limits.object_limit(),
            rebuilt: RwLock::new(Some(Vec::new())),
            recent: Mutex::default(),
            repairing: Mutex::default(),
            security: None,
            source: Source::from_bytes(&BytesSourceOptions {
                bytes: Arc::new(Vec::new()),
            })?,
            strict: false,
            trailer: None,
            version,
            warning_limit: limits.warning_limit(),
            warnings: Mutex::default(),
        })
    }

    /// How many warnings the limit kept out.
    pub(crate) fn dropped_warnings(&self) -> u64 {
        self.warnings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .dropped
    }

    /// How the document is protected.
    pub(crate) fn encryption(&self) -> Encryption {
        self.security.as_ref().map_or_else(
            || Encryption {
                certificate: false,
                cipher: Cipher::Unencrypted,
                restrictions: Vec::new(),
            },
            Handler::encryption,
        )
    }

    /// The header's version, tolerating bytes before it (ISO 32000-2:2020, 7.5.2).
    fn header(&self) -> Result<Version, PdfError> {
        let mut bytes = [0; HEADER_SEARCH];
        let count = self.source.read(0, &mut bytes, OPEN)?;
        let read = bytes.get(..count).unwrap_or_default();
        if let Ok(version) = parse_header(read, OPEN) {
            return Ok(version);
        }
        let found = (1..count).find_map(|start| {
            read.get(start..)
                .and_then(|rest| parse_header(rest, OPEN).ok())
                .map(|version| (start, version))
        });
        let (start, version) =
            found.ok_or_else(|| PdfError::new(ErrorCode::InvalidHeader, OPEN))?;
        self.objects(OPEN).deviation(
            "7.5.2",
            "bytes before the header",
            u64::try_from(start).ok(),
        )?;
        Ok(version)
    }

    /// Whether the entries were rebuilt.
    fn is_rebuilt(&self) -> bool {
        self.rebuilt
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// The objects, read for `operation`.
    pub(crate) const fn objects(&self, operation: &'static str) -> Objects<'_> {
        Objects {
            operation,
            scanned: None,
            store: self,
        }
    }

    /// Reads `source` with `policy`'s limits, rebuilding a damaged cross-reference table unless
    /// `strict`, and authenticates `password` when encrypted (ISO 32000-2:2020, 7.5 and 7.6).
    pub(crate) fn open(
        source: &Source,
        policy: Option<&ResourcePolicy>,
        strict: bool,
        password: &str,
    ) -> Result<Self, PdfError> {
        let limits = policy.map_or_else(ResourcePolicy::defaults, Clone::clone);
        let mut store = Self {
            declared: 0,
            decoded_limit: limits.decoded_limit(),
            depth_limit: limits.depth_limit(),
            dictionary: Mutex::default(),
            endstream: Mutex::default(),
            newest: 0,
            object_limit: limits.object_limit(),
            rebuilt: RwLock::default(),
            recent: Mutex::default(),
            repairing: Mutex::default(),
            security: None,
            source: source.clone(),
            strict,
            trailer: None,
            version: Version::default(),
            warning_limit: limits.warning_limit(),
            warnings: Mutex::default(),
        };
        store.version = store.header()?;
        let counted = store
            .sections(password)
            .and_then(|declared| store.count(declared).map_err(|error| (false, error)));
        match counted {
            Ok(()) => {}
            Err((_, error)) if error.code() != ErrorCode::Malformed || strict => {
                return Err(error);
            }
            Err((at_startxref, _)) => store.rebuild(at_startxref, password)?,
        }
        Ok(store)
    }

    /// Indexes the objects found by scanning the file, the newest of each number, and the last
    /// trailer, authenticating `password` when encrypted; records the deviation, of `startxref`
    /// when `at_startxref` and the file has cross-reference data, else of the table
    /// (ISO 32000-2:2020, 7.5.4 and 7.5.5).
    fn rebuild(&mut self, at_startxref: bool, password: &str) -> Result<(), PdfError> {
        self.declared = 0;
        let mut found = repair::scan(self.objects(OPEN), self.object_limit)?;
        let entries = repair::newest(found.take_entries());
        self.trailer = repair::trailer(self.objects(OPEN).through(&entries), found.trailers())?;
        self.security = self
            .trailer
            .map(|trailer| {
                decryption::open(self.objects(OPEN).through(&entries), trailer, password)
            })
            .transpose()?
            .flatten();
        self.set_rebuilt(self.objects(OPEN).with_members(&entries, found.streams())?);
        let (clause, message) = if at_startxref && found.xref() {
            ("7.5.5", "startxref wrong or missing")
        } else {
            ("7.5.4", "cross-reference table damaged or missing")
        };
        self.objects(OPEN).deviation(clause, message, None)?;
        self.count(0)
    }

    /// Locates the cross-reference sections and checks that the catalog and its page tree root
    /// are dictionaries, leaving a limit or an unsupported feature met there to later reads, after
    /// authenticating `password` when encrypted; returns the entries the sections declare. A
    /// failure tells whether `startxref` led nowhere (ISO 32000-2:2020, 7.5.5 to 7.5.8, 7.6, 7.7.2).
    fn sections(&mut self, password: &str) -> Result<u64, (bool, PdfError)> {
        let objects = self.objects(OPEN);
        let newest = xref::startxref(objects).map_err(|error| (true, error))?;
        if !xref::is_section(objects, newest).map_err(|error| (true, error))? {
            return Err((true, objects.malformed()));
        }
        let (trailer, declared, size) =
            xref::walk(objects, newest).map_err(|error| (false, error))?;
        self.declared = size;
        self.newest = newest;
        self.trailer = Some(trailer);
        self.security = decryption::open(self.objects(OPEN), trailer, password)
            .map_err(|error| (false, error))?;
        let checked = self.objects(OPEN);
        match checked
            .get(trailer, b"Root")
            .and_then(|root| checked.dictionary(root))
            .and_then(|catalog| checked.get(catalog, b"Pages"))
            .and_then(|pages| checked.dictionary(pages))
        {
            Err(error) if matches!(error.code(), ErrorCode::Io | ErrorCode::Malformed) => {
                Err((false, error))
            }
            Ok(_) | Err(_) => Ok(declared),
        }
    }

    /// Sets the rebuilt entries, forgetting the dictionary read before.
    fn set_rebuilt(&self, entries: Vec<XrefEntry>) {
        let mut rebuilt = self.rebuilt.write().unwrap_or_else(PoisonError::into_inner);
        *rebuilt = Some(entries);
        *self
            .dictionary
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        drop(rebuilt);
    }

    /// The deviations tolerated so far, up to the limit.
    pub(crate) fn warnings(&self) -> Vec<Deviation> {
        self.warnings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .kept
            .clone()
    }
}

/// `entries` with each key once, at its last value's place and with that value
/// (ISO 32000-2:2020, 7.3.7).
fn last_values(entries: Vec<(Vec<u8>, Object)>) -> Vec<(Vec<u8>, Object)> {
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by(|&left, &right| {
        let key = |at: usize| entries.get(at).map(|entry| entry.0.as_slice());
        key(left).cmp(&key(right)).then(right.cmp(&left))
    });
    let mut kept = vec![false; entries.len()];
    let mut previous: Option<&[u8]> = None;
    for at in order {
        let key = entries.get(at).map(|entry| entry.0.as_slice());
        if key != previous
            && let Some(flag) = kept.get_mut(at)
        {
            *flag = true;
        }
        previous = key;
    }
    entries
        .into_iter()
        .zip(kept)
        .filter_map(|(entry, keep)| keep.then_some(entry))
        .collect()
}

/// A stream's `entries` without a leading `Crypt` filter and its parameters (ISO 32000-2:2020,
/// 7.4.10).
fn without_crypt(entries: Vec<(Vec<u8>, Object)>) -> Vec<(Vec<u8>, Object)> {
    let crypt =
        |object: Option<&Object>| matches!(object, Some(Object::Name(name)) if name == b"Crypt");
    let Some(filter) = entries
        .iter()
        .find(|entry| entry.0 == b"Filter")
        .map(|entry| &entry.1)
    else {
        return entries;
    };
    let alone = crypt(Some(filter));
    if !alone && !matches!(filter, Object::Array(items) if crypt(items.first())) {
        return entries;
    }
    entries
        .into_iter()
        .filter_map(|(key, value)| {
            if !matches!(key.as_slice(), b"Filter" | b"DecodeParms") {
                return Some((key, value));
            }
            match value {
                Object::Array(mut items) if !alone && !items.is_empty() => {
                    drop(items.remove(0));
                    (!items.is_empty()).then(|| (key, Object::Array(items)))
                }
                other @ (Object::Array(_)
                | Object::Boolean(_)
                | Object::Dictionary(_)
                | Object::Integer(_)
                | Object::Name(_)
                | Object::Null
                | Object::Real(_)
                | Object::Reference(_)
                | Object::Stream(..)
                | Object::String(_)) => (!alone).then_some((key, other)),
            }
        })
        .collect()
}
