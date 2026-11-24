//! A document's shared state, and its objects read through it (ISO 32000-2:2020, 7.3.8, 7.3.10
//! and 7.5).

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::rc::Rc;
use alloc::sync::Arc;
use core::cell::{Cell, RefCell};
use core::fmt::{self, Debug, Formatter};
use std::collections::HashSet;
use std::sync::{Mutex, PoisonError, RwLock};

use crate::compliance::Deviation;
use crate::decryption::{self, Handler};
use crate::input::{BytesSourceOptions, Reads, Source};
use crate::limits::ResourcePolicy;
use crate::object_stream::{self, Sequences};
use crate::objects::{ObjectId, XrefEntry, XrefKind};
use crate::overlay::{Data, Object};
use crate::pull::{Held, Pull};
use crate::repair;
use crate::security::{Cipher, Encryption};
use crate::stream::{Feeder, decoder};
use crate::syntax::{At, Input, Keyword, Located, Reader, Token, Value};
use crate::xref;
use crate::{ErrorCode, PdfError, Version, parse_header};

/// The most bytes of one object stream a render keeps decoded.
const CONTAINER: usize = 0x10_0000;

/// The most bytes of the newest cross-reference stream kept decoded.
pub(crate) const SECTION: usize = 0x1_0000;

/// The most object streams a render reads as outside renders it records.
const LAZY: usize = 4096;

/// The most bytes of object streams a render keeps decoded.
const CONTAINERS: usize = 0x40_0000;

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

/// Slots of the entries looked up.
const RECENT: usize = 4096;

/// Work of a token of an object read while work is bounded, beside its bytes'.
const TOKEN_WORK: u64 = 10;

/// What bounds one operation, a render: its work, and each deviation recorded once.
#[derive(Default)]
pub(crate) struct Bounds {
    /// Object streams and the newest cross-reference stream kept decoded: each read would decode
    /// them again from their start.
    pub containers: RefCell<Decoded>,
    /// The entries looked up, absent ones too, one a slot by number, where a lookup walked more
    /// than one section or decoded one: bands look them up again.
    pub entries: RefCell<Vec<Recent>>,
    /// The bytes the object being read may still take.
    pub held: Cell<u64>,
    /// The bytes free beside what the operation holds, where it counts them; see
    /// [`Objects::allow`].
    pub room: Cell<Option<u64>>,
    /// The deviations recorded, by clause and message.
    pub warned: RefCell<HashSet<(&'static str, &'static str)>>,
    /// The work left, which reads and decoding take from.
    pub work: Cell<u64>,
}

/// An object stream a render keeps decoded.
struct Container {
    /// Its data decoded.
    data: Rc<[u8]>,
    /// Where its dictionary is.
    dictionary: At,
    /// Its members' numbers and where their objects start, after `First`, once one was read.
    members: Option<Listing>,
}

impl Container {
    /// The bytes it holds.
    fn bytes(&self) -> usize {
        let members = match self.members {
            Some(Listing::Listed(_, ref listed)) => listed.len(),
            Some(Listing::Unlisted) | None => 0,
        };
        self.data
            .len()
            .saturating_add(members.saturating_mul(size_of::<(u32, u64)>()))
    }
}

/// What a render keeps decoded.
#[derive(Default)]
pub(crate) struct Decoded {
    /// The bytes of `streams`.
    bytes: usize,
    /// Whether an object stream was refused for room since last asked.
    crowded: bool,
    /// The object streams not kept, too long or damaged, whose members are read as outside
    /// renders; at most [`LAZY`].
    lazy: BTreeSet<u32>,
    /// The numbers of `streams`, the first kept first.
    order: VecDeque<u32>,
    /// The bytes of `sections`.
    section_bytes: usize,
    /// The offsets of `sections`, the first kept first.
    section_order: VecDeque<u64>,
    /// Cross-reference streams' data, by offset, but the newest where the store keeps it.
    sections: BTreeMap<u64, Arc<[u8]>>,
    /// Object streams refused once for room; refused again, they are not kept.
    starved: BTreeSet<u32>,
    /// The object streams kept, by number.
    streams: BTreeMap<u32, Container>,
}

impl Decoded {
    /// Drops the first kept but `keep` until all fit in [`CONTAINERS`].
    fn evict(&mut self, keep: u32) {
        let mut turns = self.order.len();
        while self.bytes > CONTAINERS && turns > 0 {
            turns = turns.saturating_sub(1);
            let Some(number) = self.order.pop_front() else {
                break;
            };
            if number == keep {
                self.order.push_back(number);
                continue;
            }
            if let Some(gone) = self.streams.remove(&number) {
                self.bytes = self.bytes.saturating_sub(gone.bytes());
            }
        }
    }

    /// Reads object stream `number`'s members as outside renders from now on.
    fn not_kept(&mut self, number: u32) {
        if self.lazy.len() >= LAZY {
            self.lazy.clear();
        }
        let _new = self.lazy.insert(number);
    }
}

/// An object stream's members' numbers and offsets, or that they are read as outside renders.
enum Listing {
    /// `First` and each member's number and offset after it.
    Listed(u64, Vec<(u32, u64)>),
    /// Too many to list, or damaged.
    Unlisted,
}

/// A kept object stream's data and where a member's object starts in it.
type Member = (Rc<[u8]>, u64);

/// An entry looked up: its number and what was found.
type Recent = Option<(u32, Option<XrefEntry>)>;

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

/// What reports progress and staleness as work is taken.
pub(crate) trait Pacing {
    /// Reports progress and checks staleness when due.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Cancelled`] when the caller cancels, or the document changed.
    fn pace(&self) -> Result<(), PdfError>;
}

/// The work left, where it is bounded, and what reports progress as it is taken.
#[derive(Clone, Copy)]
pub(crate) struct Meter<'store> {
    /// The work left.
    left: &'store Cell<u64>,
    /// What reports progress and staleness.
    pace: Option<&'store dyn Pacing>,
}

impl Debug for Meter<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Meter")
            .field("left", &self.left.get())
            .finish_non_exhaustive()
    }
}

/// A document's objects, read for one operation.
#[derive(Clone, Copy)]
pub(crate) struct Objects<'store> {
    /// What bounds the operation, where something does.
    bounds: Option<&'store Bounds>,
    /// The operation errors name.
    operation: &'static str,
    /// Its pace, where reads report progress.
    pace: Option<&'store dyn Pacing>,
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
    /// Held while [`Objects::repair`] runs; whether it recorded the table damaged.
    repairing: Mutex<bool>,
    /// The newest cross-reference section's data decoded, where a stream's and at most
    /// [`SECTION`] bytes: each lookup would decode it again.
    section: Mutex<Option<Arc<[u8]>>>,
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
    /// Lets the object to be read take a quarter of the decoded limit, and of the room free beside
    /// what the operation holds, under bounds: growing its lists and strings may take three times
    /// what they hold.
    fn allow(self) {
        if let Some(bounds) = self.bounds {
            let free = bounds.room.get().map_or(u64::MAX, |room| room >> 2_u32);
            bounds.held.set((self.decoded_limit() >> 2_u32).min(free));
        }
    }

    /// Keeps a reader's strings and names within what an object may take, under bounds, so that
    /// one fails while it grows.
    fn bound<I>(self, reader: &mut Reader<I>)
    where
        I: Input,
    {
        if self.bounds.is_some() {
            let most = self.decoded_limit() >> 2_u32;
            reader.set_keep_limit(usize::try_from(most).unwrap_or(usize::MAX));
        }
    }

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

    /// Takes the work of the tokens `reader` read since last asked, where work is bounded.
    pub(crate) fn charge<I>(self, reader: &mut Reader<I>) -> Result<(), PdfError>
    where
        I: Input,
    {
        let tokens = reader.tokens();
        self.work().map_or(Ok(()), |work| {
            work.take(tokens.saturating_mul(TOKEN_WORK), self.operation)
        })
    }

    /// The data of object stream `number` and its dictionary (ISO 32000-2:2020, 7.5.7).
    fn container(self, number: u32) -> Result<(Box<dyn Pull + 'store>, At), PdfError> {
        if let Some(bounds) = self.bounds
            && let Some(kept) = bounds.containers.borrow().streams.get(&number)
        {
            return Ok((Box::new(Held::new(Rc::clone(&kept.data))), kept.dictionary));
        }
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
        let mut data = decoder(self, Some(id), start, length, dictionary, false)?;
        // A render keeps what fits, beside what it holds, for the members read after.
        let Some(bounds) = self.bounds else {
            return Ok((data, dictionary));
        };
        if bounds.containers.borrow().lazy.contains(&number) {
            return Ok((data, dictionary));
        }
        match self.whole(&mut *data, CONTAINER) {
            Ok(Some(bytes))
                if self
                    .fits_room(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
                    .is_err() =>
            {
                // No room: the caches give way first, and it is refused only when that was
                // not enough.
                let mut kept = bounds.containers.borrow_mut();
                if kept.starved.insert(number) {
                    kept.crowded = true;
                } else {
                    kept.not_kept(number);
                }
                drop(kept);
                drop(bytes);
                data = decoder(self, Some(id), start, length, dictionary, false)?;
                return Ok((data, dictionary));
            }
            Ok(Some(bytes)) => {
                let held: Rc<[u8]> = Rc::from(bytes);
                let mut kept = bounds.containers.borrow_mut();
                kept.bytes = kept.bytes.saturating_add(held.len());
                kept.order.push_back(number);
                let _replaced = kept.streams.insert(
                    number,
                    Container {
                        data: Rc::clone(&held),
                        dictionary,
                        members: None,
                    },
                );
                kept.evict(number);
                return Ok((Box::new(Held::new(held)), dictionary));
            }
            // Damaged: its members before the damage are read as outside renders.
            Ok(None) => {}
            Err(error) if error.code() == ErrorCode::Malformed => {}
            Err(error) => return Err(error),
        }
        // Longer than is kept, no room, or damaged: read anew, as each time after.
        bounds.containers.borrow_mut().not_kept(number);
        data = decoder(self, Some(id), start, length, dictionary, false)?;
        Ok((data, dictionary))
    }

    /// The bytes of the object streams an operation keeps and of the cross-reference stream kept.
    #[cfg(feature = "rendering")]
    pub(crate) fn containers(self) -> u64 {
        let section = self
            .store
            .section
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or(0, |kept| kept.len());
        let total = self.bounds.map_or(section, |bounds| {
            let kept = bounds.containers.borrow();
            section
                .saturating_add(kept.section_bytes)
                .saturating_add(kept.bytes)
        });
        u64::try_from(total).unwrap_or(u64::MAX)
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
        clause: &'static str,
        message: &'static str,
        offset: Option<u64>,
    ) -> Result<(), PdfError> {
        if self.store.strict {
            return Err(PdfError {
                message: String::from(message),
                offset,
                ..self.malformed()
            });
        }
        if let Some(bounds) = self.bounds {
            // A render reads again for every band, whose count is not the document's.
            if !bounds.warned.borrow_mut().insert((clause, message)) {
                return Ok(());
            }
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
    ) -> Result<Reader<Reads<'store>>, PdfError> {
        let mut reader = Reader::new(self.source(), offset, self.operation);
        self.bound(&mut reader);
        if xref::object_header(&mut reader, self)?
            != Some((i64::from(id.number), i64::from(id.generation)))
        {
            return Err(self.malformed());
        }
        Ok(reader)
    }

    /// Gives up the object and cross-reference streams a render keeps, so that what needs the
    /// room has it.
    #[cfg(feature = "rendering")]
    pub(crate) fn drop_containers(self) {
        if let Some(bounds) = self.bounds {
            let mut kept = bounds.containers.borrow_mut();
            kept.streams.clear();
            kept.order.clear();
            kept.bytes = 0;
            kept.sections.clear();
            kept.section_order.clear();
            kept.section_bytes = 0;
        }
    }

    /// Whether `endstream` follows `offset`, after white-space.
    fn ends(self, offset: u64) -> Result<bool, PdfError> {
        let mut reader = Reader::new(self.source(), offset, self.operation);
        match reader.next_token() {
            Ok(found) => Ok(found == Some(Token::Keyword(Keyword::EndStream))),
            // Only damage means no `endstream`; the operation's end is not damage.
            Err(error) if error.code() == ErrorCode::Malformed => Ok(false),
            Err(error) => Err(error),
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
        let looked = self.bounds.and_then(|bounds| {
            let held = bounds.entries.borrow();
            held.get(slot(number, held.len())).copied().flatten()
        });
        if let Some((held, found)) = looked
            && held == number
        {
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
        let (found, further) = match xref::entry(self, self.store.newest, number) {
            Err(error) if error.code() == ErrorCode::Malformed => {
                self.repair()?;
                return Ok(rebuilt().flatten());
            }
            other => other?,
        };
        if found.is_some() {
            *recent() = found;
        }
        if further {
            self.remember(number, found);
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
        let size = self.source().size(self.operation)?;
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
        let count = self.source().read(
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

    /// Fails [`ErrorCode::LimitExceeded`] where `bytes` more are past the room free beside what
    /// the operation holds, where it counts it.
    pub(crate) fn fits_room(self, bytes: u64) -> Result<(), PdfError> {
        match self.bounds.and_then(|bounds| bounds.room.get()) {
            Some(room) if bytes > room => {
                Err(PdfError::new(ErrorCode::LimitExceeded, self.operation))
            }
            Some(_) | None => Ok(()),
        }
    }

    /// Whether a cross-reference stream of `bytes` decoded fits the room a render leaves; when
    /// not, what is kept for reuse gives way.
    pub(crate) fn fits_section(self, bytes: u64) -> bool {
        let fits = self.fits_room(bytes).is_ok();
        if !fits && let Some(bounds) = self.bounds {
            bounds.containers.borrow_mut().crowded = true;
        }
        fits
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
                    let value = reader.value(self.inner_depth())?;
                    self.charge(&mut reader)?;
                    value
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

    /// Counts an object and `payload` more bytes of the object being read, under bounds.
    fn hold(self, payload: usize) -> Result<(), PdfError> {
        let Some(bounds) = self.bounds else {
            return Ok(());
        };
        let bytes = size_of::<Object>().saturating_add(payload);
        let left = bounds
            .held
            .get()
            .checked_sub(u64::try_from(bytes).unwrap_or(u64::MAX))
            .ok_or_else(|| PdfError::new(ErrorCode::LimitExceeded, self.operation))?;
        bounds.held.set(left);
        Ok(())
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
                    self.charge(&mut reader)?;
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

    /// Keeps `data`, the cross-reference stream at `offset` decoded: the newest in the store
    /// where small, else in a render.
    pub(crate) fn keep_section(self, offset: u64, data: Arc<[u8]>) {
        if offset == self.store.newest && data.len() <= SECTION {
            *self
                .store
                .section
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(data);
            return;
        }
        let Some(bounds) = self.bounds else {
            return;
        };
        let most = self.sections_most();
        let mut kept = bounds.containers.borrow_mut();
        kept.section_bytes = kept.section_bytes.saturating_add(data.len());
        kept.section_order.push_back(offset);
        if let Some(old) = kept.sections.insert(offset, data) {
            kept.section_bytes = kept.section_bytes.saturating_sub(old.len());
        }
        // The first kept give way to the last.
        while kept.section_bytes > most && kept.section_order.len() > 1 {
            let Some(first) = kept.section_order.pop_front() else {
                break;
            };
            if let Some(gone) = kept.sections.remove(&first) {
                kept.section_bytes = kept.section_bytes.saturating_sub(gone.len());
            }
        }
    }

    /// The data of the cross-reference stream at `offset` decoded, where kept.
    pub(crate) fn kept_section(self, offset: u64) -> Option<Arc<[u8]>> {
        let shared = (offset == self.store.newest)
            .then(|| {
                self.store
                    .section
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
            })
            .flatten();
        shared.or_else(|| {
            let bounds = self.bounds?;
            bounds.containers.borrow().sections.get(&offset).cloned()
        })
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
    pub(crate) fn located(self, id: ObjectId) -> Result<Option<Reader<Reads<'store>>>, PdfError> {
        if self.work().is_some_and(|work| work.left.get() == 0) {
            return Err(PdfError::new(ErrorCode::LimitExceeded, self.operation));
        }
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

    /// Whether the stored object `id` is an array of more than `most` items, counted without
    /// reading them into memory.
    #[cfg(feature = "rendering")]
    pub(crate) fn longer(self, id: ObjectId, most: usize) -> Result<bool, PdfError> {
        let Value::Array(at) = self.resolve(id)? else {
            return Ok(false);
        };
        let mut reader = self.reader(at)?;
        drop(reader.token()?);
        for _ in 0..=most {
            let token = reader.item()?;
            if token == Token::ArrayEnd {
                return Ok(false);
            }
            drop(reader.complete(token, self.inner_depth(), true)?);
            self.charge(&mut reader)?;
        }
        Ok(true)
    }

    /// A [`ErrorCode::Malformed`] error.
    pub(crate) fn malformed(self) -> PdfError {
        PdfError::new(ErrorCode::Malformed, self.operation)
    }

    /// Where object `number`, at the entry's index in a kept object stream, starts in its data,
    /// its members' places read once; none where it is not kept.
    fn member_start(self, entry: XrefEntry, number: u32) -> Result<Option<Member>, PdfError> {
        let Some(bounds) = self.bounds else {
            return Ok(None);
        };
        let mut kept = bounds.containers.borrow_mut();
        let Some(container) = kept.streams.get_mut(&entry.stream) else {
            return Ok(None);
        };
        if container.members.is_none() {
            let data: Box<dyn Pull + '_> = Box::new(Held::new(Rc::clone(&container.data)));
            let listed = match object_stream::members(
                self,
                data,
                container.dictionary,
                container.data.len(),
            ) {
                Ok(Some((first, members))) => Listing::Listed(first, members),
                // Too many, or damaged: read as outside renders.
                Ok(None) => Listing::Unlisted,
                Err(error) if error.code() == ErrorCode::Malformed => Listing::Unlisted,
                Err(error) => return Err(error),
            };
            container.members = Some(listed);
            let grown = container.bytes().saturating_sub(container.data.len());
            kept.bytes = kept.bytes.saturating_add(grown);
            kept.evict(entry.stream);
        }
        let Some(listed) = kept.streams.get(&entry.stream) else {
            return Ok(None);
        };
        let Some(Listing::Listed(first, ref members)) = listed.members else {
            return Ok(None);
        };
        let index = usize::try_from(entry.index).unwrap_or(usize::MAX);
        match members.get(index) {
            Some(&(found, offset)) if found == number => Ok(Some((
                Rc::clone(&listed.data),
                first.checked_add(offset).ok_or_else(|| self.malformed())?,
            ))),
            Some(_) | None => Err(self.malformed()),
        }
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
        self.allow();
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

    /// The value of `key` of the dictionary at `dictionary` in object `id`, read into memory as
    /// [`Objects::owned`] reads it; the other entries are passed over unread, so a large one takes no
    /// memory (ISO 32000-2:2020, 7.3.7).
    #[cfg(feature = "rendering")]
    pub(crate) fn owned_entry(
        self,
        id: ObjectId,
        dictionary: At,
        key: &[u8],
    ) -> Result<Option<Object>, PdfError> {
        let Some(value) = self.get(dictionary, key)? else {
            return Ok(None);
        };
        // Strings in an object stream are not encrypted on their own (7.5.7, 7.6.2).
        let handler = self
            .security()
            .filter(|found| found.encrypts(id) && dictionary.packed().is_none());
        let mut strings = |bytes: Vec<u8>| match handler {
            Some(found) => found.string(id, &bytes, self.operation),
            None => Ok(bytes),
        };
        let at = match value {
            Value::Array(at) | Value::Dictionary(at) => at,
            Value::Boolean(flag) => return Ok(Some(Object::Boolean(flag))),
            Value::Integer(number) => return Ok(Some(Object::Integer(number))),
            Value::Name(name) => return Ok(Some(Object::Name(name))),
            Value::Null => return Ok(Some(Object::Null)),
            Value::Real(number) => return Ok(Some(Object::Real(number))),
            Value::Reference(target) => return Ok(Some(Object::Reference(target))),
            Value::String(bytes) => return Ok(Some(Object::String(strings(bytes)?))),
        };
        self.allow();
        let mut reader = self.reader(at)?;
        reader.set_keep(true);
        let token = reader.item()?;
        self.read(&mut reader, token, self.depth_limit(), &mut strings)
            .map(Some)
    }

    /// Object `id`, as [`Objects::owned`] reads it; a member of an object stream is read on in
    /// `sequences` from the member read before, where that gives what reading it alone gives.
    pub(crate) fn owned_in(
        self,
        id: ObjectId,
        sequences: &mut Sequences<'store>,
    ) -> Result<Object, PdfError> {
        sequences.renew(self.store.is_rebuilt());
        if let Some(entry) = self.entry(id.number)?
            && entry.kind == XrefKind::Compressed
            && id.generation == 0
            && !sequences.refuses(entry.stream)
        {
            match self.sequenced(id, entry, sequences) {
                Ok(Some(object)) => return Ok(object),
                Ok(None) => {}
                // Read alone, so that it fails or not as it would alone.
                Err(_) => sequences.refuse(entry.stream),
            }
        }
        self.owned(id)
    }

    /// The trailer dictionary read into memory; an empty one without a source (ISO 32000-2:2020,
    /// 7.5.5).
    pub(crate) fn owned_trailer(self) -> Result<Object, PdfError> {
        let Some(at) = self.store.trailer else {
            return Ok(Object::Dictionary(Vec::new()));
        };
        let mut reader = self.reader(at)?;
        reader.set_keep(true);
        self.allow();
        self.room(&mut reader);
        let token = reader.item()?;
        self.read(&mut reader, token, self.depth_limit(), &mut Ok)
    }

    /// These objects, reporting progress through `pace` as their reads take work.
    #[cfg(feature = "rendering")]
    pub(crate) const fn paced<'view>(self, pace: &'view dyn Pacing) -> Objects<'view>
    where
        'store: 'view,
    {
        Objects {
            bounds: self.bounds,
            operation: self.operation,
            pace: Some(pace),
            scanned: self.scanned,
            store: self.store,
        }
    }

    /// A reader at object `number` in the object stream `entry` names, at the entry's index
    /// (ISO 32000-2:2020, 7.5.7).
    fn packed(self, entry: XrefEntry, number: u32) -> Result<Reader<Feeder<'store>>, PdfError> {
        let (data, dictionary) = self.container(entry.stream)?;
        if let Some((bytes, start)) = self.member_start(entry, number)? {
            // Its members' places are known: only its own object is read.
            drop(data);
            let at = usize::try_from(start).unwrap_or(usize::MAX);
            let held: Box<dyn Pull + 'store> = Box::new(Held::at(bytes, at));
            let mut reader = Reader::new(Feeder::at(held, start), start, self.operation);
            reader.set_place(Some(entry.stream));
            self.bound(&mut reader);
            return Ok(reader);
        }
        let mut reader = object_stream::member(self, data, dictionary, number, entry.index)?;
        reader.set_place(Some(entry.stream));
        self.bound(&mut reader);
        Ok(reader)
    }

    /// Whether the stream whose dictionary is at `dictionary` has only Flate filters and no
    /// parameters, or no filter and no encryption: a failure in its data then falls at the same
    /// byte for any read sizes, short of the decoded limit.
    fn plain(self, dictionary: At) -> Result<bool, PdfError> {
        if self.get(dictionary, b"DecodeParms")?.is_some() {
            return Ok(false);
        }
        let flate = |value: &Value| value.is_name(b"FlateDecode");
        Ok(match self.get(dictionary, b"Filter")? {
            None => self.store.security.is_none(),
            Some(Value::Array(at)) => {
                let mut items = self.reader(at)?;
                let mut any = false;
                let mut all = true;
                loop {
                    match items.token()? {
                        Token::ArrayStart => {}
                        Token::ArrayEnd => break,
                        Token::Name(name) => {
                            any = true;
                            all &= name.as_slice() == b"FlateDecode";
                        }
                        Token::DictionaryEnd
                        | Token::DictionaryStart
                        | Token::Integer(_)
                        | Token::Keyword(_)
                        | Token::Real(_)
                        | Token::Reference(_)
                        | Token::String(_) => return Ok(false),
                    }
                }
                any && all
            }
            Some(ref single) => flate(single),
        })
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
        self.hold(0)?;
        self.charge(reader)?;
        Ok(match token {
            Token::ArrayStart => {
                let below = inner()?;
                let mut items = Vec::new();
                loop {
                    self.room(reader);
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
                    self.room(reader);
                    match reader.item()? {
                        Token::DictionaryEnd => break Object::Dictionary(last_values(entries)),
                        Token::Name(key) => {
                            self.hold(key.len())?;
                            self.room(reader);
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
            Token::Name(name) => {
                self.hold(name.len())?;
                Object::Name(name)
            }
            Token::Real(value) => Object::Real(value),
            Token::Reference(id) => Object::Reference(id),
            Token::String(bytes) => {
                self.hold(bytes.len())?;
                Object::String(strings(bytes)?)
            }
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
        let input = Located::new(self.source(), packed);
        let mut reader = Reader::new(input, at.offset(), self.operation);
        reader.set_place(at.packed());
        self.bound(&mut reader);
        Ok(reader)
    }

    /// Keeps what a lookup of object `number` found, where an operation keeps it, so that it is
    /// not looked up again.
    fn remember(self, number: u32, found: Option<XrefEntry>) {
        let Some(bounds) = self.bounds else {
            return;
        };
        let mut kept = bounds.entries.borrow_mut();
        if kept.is_empty() {
            kept.resize(RECENT, None);
        }
        let at = slot(number, kept.len());
        if let Some(place) = kept.get_mut(at) {
            *place = Some((number, found));
        }
    }

    /// Rebuilds the entries by scanning the file, once, keeping the trailer and security handler,
    /// and records the deviation (ISO 32000-2:2020, 7.5.4).
    fn repair(self) -> Result<(), PdfError> {
        // Charged to the operation, which repairs again if it ends first, and not paced, as the
        // lock is held.
        let objects = Self { pace: None, ..self };
        let mut held = self
            .store
            .repairing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.store.is_rebuilt() {
            return Ok(());
        }
        // Recorded before the scan, once, though a scan cut short is done again.
        if !*held {
            self.deviation("7.5.4", "cross-reference table damaged or missing", None)?;
            *held = true;
        }
        let mut found = repair::scan(objects, self.store.object_limit)?;
        let entries = repair::newest(found.take_entries());
        self.store
            .set_rebuilt(objects.with_members(&entries, found.streams())?);
        drop(held);
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

    /// Keeps a reader's next string or name within the room left for the object being read, under
    /// bounds, so that it fails while it grows.
    fn room<I>(self, reader: &mut Reader<I>)
    where
        I: Input,
    {
        if let Some(bounds) = self.bounds {
            let left = bounds
                .held
                .get()
                .saturating_sub(u64::try_from(size_of::<Object>()).unwrap_or(u64::MAX));
            reader.set_keep_limit(usize::try_from(left).unwrap_or(usize::MAX));
        }
    }

    /// Where the first `endstream` from `start` lies, if any.
    fn search_endstream(self, start: u64, size: u64) -> Result<Option<u64>, PdfError> {
        let mut buffer = vec![0; SCAN];
        let mut at = start;
        let step = u64::try_from(SCAN.saturating_sub(ENDSTREAM.len())).unwrap_or(1);
        while at < size {
            let count = self.source().read(at, &mut buffer, self.operation)?;
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

    /// The most bytes of the cross-reference stream at `offset` kept decoded: the newest only,
    /// and small, but in a render, which keeps any and gives them up when it needs the room.
    pub(crate) fn section_most(self, offset: u64) -> usize {
        if self.bounds.is_some() {
            self.sections_most()
        } else if offset == self.store.newest {
            SECTION
        } else {
            0
        }
    }

    /// The most bytes of cross-reference streams a render keeps decoded: a quarter of the decoded
    /// limit, at least [`SECTION`].
    fn sections_most(self) -> usize {
        usize::try_from(self.decoded_limit() >> 2_u32)
            .unwrap_or(usize::MAX)
            .max(SECTION)
    }

    /// The security handler, when encrypted.
    pub(crate) const fn security(self) -> Option<&'store Handler> {
        self.store.security.as_ref()
    }

    /// Object `id`, at `entry` in an object stream whose data is only Flate or not filtered,
    /// read on in `sequences`; none where it must be read alone.
    fn sequenced(
        self,
        id: ObjectId,
        entry: XrefEntry,
        sequences: &mut Sequences<'store>,
    ) -> Result<Option<Object>, PdfError> {
        let open = sequences.of(entry.stream, || {
            let (pairs, dictionary) = self.container(entry.stream)?;
            if !self.plain(dictionary)? {
                return Err(self.malformed());
            }
            let (data, _) = self.container(entry.stream)?;
            object_stream::sequence(self, (pairs, data), dictionary, entry.stream)
        })?;
        let Some(mut reader) = open.member(self, id.number, entry.index)? else {
            return Ok(None);
        };
        self.allow();
        reader.set_place(Some(entry.stream));
        self.bound(&mut reader);
        reader.set_keep(true);
        let token = reader.item()?;
        self.read(&mut reader, token, self.depth_limit(), &mut Ok)
            .map(Some)
    }

    /// Sets the bytes free beside what the operation holds, where it is bounded.
    #[cfg(feature = "rendering")]
    pub(crate) fn set_room(self, room: u64) {
        if let Some(bounds) = self.bounds {
            bounds.room.set(Some(room));
        }
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
    pub(crate) const fn source(self) -> Reads<'store> {
        Reads::new(&self.store.source, self.work())
    }

    /// Whether the first deviation fails.
    #[cfg(feature = "rendering")]
    pub(crate) const fn strict(self) -> bool {
        self.store.strict
    }

    /// The handler decrypting the strings of object `id`, whose value `reader` is at: none when
    /// unencrypted, for the `Encrypt` dictionary and for a cross-reference stream
    /// (ISO 32000-2:2020, 7.5.8.2 and 7.6.2).
    fn strings(
        self,
        reader: &mut Reader<Reads<'store>>,
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
            bounds: self.bounds,
            operation: self.operation,
            pace: self.pace,
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

    /// Whether an object stream was refused for room since last asked, so that the caches give
    /// way.
    #[cfg(feature = "rendering")]
    pub(crate) fn wants_room(self) -> bool {
        self.bounds.is_some_and(|bounds| {
            let mut kept = bounds.containers.borrow_mut();
            let wanted = kept.crowded;
            kept.crowded = false;
            wanted
        })
    }

    /// All of `data`, where at most `most` bytes.
    fn whole(self, data: &mut dyn Pull, most: usize) -> Result<Option<Vec<u8>>, PdfError> {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let count = data.pull(&mut buffer)?;
            if count == 0 {
                break;
            }
            if bytes.len().saturating_add(count) > most {
                return Ok(None);
            }
            bytes
                .try_reserve(count)
                .map_err(|_error| PdfError::new(ErrorCode::BudgetExceeded, self.operation))?;
            bytes.extend_from_slice(buffer.get(..count).unwrap_or_default());
        }
        Ok(Some(bytes))
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

    /// These objects, read and decoded within `bounds`.
    #[cfg(feature = "rendering")]
    pub(crate) const fn within<'view>(self, bounds: &'view Bounds) -> Objects<'view>
    where
        'store: 'view,
    {
        Objects {
            bounds: Some(bounds),
            operation: self.operation,
            pace: self.pace,
            scanned: self.scanned,
            store: self.store,
        }
    }

    /// The work left, where it is bounded.
    pub(crate) const fn work(self) -> Option<Meter<'store>> {
        match self.bounds {
            Some(bounds) => Some(Meter {
                left: &bounds.work,
                pace: self.pace,
            }),
            None => None,
        }
    }
}

impl Meter<'_> {
    /// Whether no work is left.
    pub(crate) const fn exhausted(self) -> bool {
        self.left.get() == 0
    }

    /// Takes `units`, failing past the work left as `operation`, and reports progress when due.
    pub(crate) fn take(self, units: u64, operation: &'static str) -> Result<(), PdfError> {
        let left = self.left.get().checked_sub(units);
        self.left.set(left.unwrap_or(0));
        if left.is_none() {
            return Err(PdfError::new(ErrorCode::LimitExceeded, operation));
        }
        self.pace.map_or(Ok(()), Pacing::pace)
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
            section: Mutex::default(),
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
            bounds: None,
            operation,
            pace: None,
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
            section: Mutex::default(),
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
/// The slot of object `number` among `slots`, a power of two.
fn slot(number: u32, slots: usize) -> usize {
    usize::try_from(number).unwrap_or(0) & slots.wrapping_sub(1)
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
