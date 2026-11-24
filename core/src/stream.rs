//! Stream data: reading it raw, undoing its filters and consuming it in constant memory
//! (ISO 32000-2:2020, 7.3.8 and 7.4).

use crate::ascii_hex::AsciiHex;
use crate::ascii85::Ascii85;
use crate::inflate::Inflate;
use crate::input::Reads;
use crate::lzw::Lzw;
use crate::objects::{ObjectId, XrefKind};
use crate::output::Sink;
use crate::overlay::{Data, Dictionary, Object, Overlay};
use crate::predictor::Predictor;
#[cfg(feature = "rendering")]
use crate::pull::Held;
use crate::pull::Pull;
use crate::run_length::RunLength;
use crate::store::{Meter, Objects};
use crate::syntax::{At, Input, Keyword, Token, Value};
use crate::{ErrorCode, PdfError};

/// Filters one stream may chain.
pub(crate) const FILTERS: usize = 16;

/// Bytes discarded or read at a time.
const STEP: usize = 4096;

/// Decoded stream data as bytes at an offset that only moves forward.
pub(crate) struct Feeder<'data> {
    /// The data.
    data: Box<dyn Pull + 'data>,
    /// The offset of the next byte to pull.
    end: u64,
}

/// Stream data opened up to an image filter.
#[cfg(feature = "rendering")]
pub(crate) struct Opened<'store> {
    /// The data, the filters before an image filter undone.
    pub data: Box<dyn Pull + 'store>,
    /// The bytes its reader holds whole.
    pub held: u64,
    /// The image filter left and its parameters.
    pub image: Option<(Vec<u8>, Object)>,
}

/// A filter's integer parameter by key, or the default given.
type Parameter<'lookup> = dyn Fn(&[u8], i64) -> Result<i64, PdfError> + 'lookup;

/// A stream's stored bytes and, when known, their count.
pub(crate) type Stored<'data> = (Box<dyn Pull + 'data>, Option<u64>);

/// Which indirect references a stream's `Filter` and `DecodeParms` may hold.
#[derive(Clone, Copy)]
enum Indirect {
    /// Any.
    Any,
    /// None.
    Never,
    /// Those to objects outside object streams.
    Outside,
}

/// Data that fails with [`ErrorCode::LimitExceeded`] past a count of bytes or the work left.
struct Limited<'data> {
    /// The data.
    data: Box<dyn Pull + 'data>,
    /// Bytes still allowed.
    left: u64,
    /// The operation errors name.
    operation: &'static str,
    /// The work left, a unit a byte, where it is bounded.
    work: Option<Meter<'data>>,
}

/// The raw bytes of a stream.
struct Raw<'source> {
    /// The operation errors name.
    operation: &'static str,
    /// The offset of the next byte.
    position: u64,
    /// Bytes left.
    remaining: u64,
    /// The source.
    source: Reads<'source>,
}

impl<'data> Feeder<'data> {
    /// A feeder of `data`, whose next byte is at offset `end`.
    pub(crate) const fn at(data: Box<dyn Pull + 'data>, end: u64) -> Self {
        Self { data, end }
    }

    /// A feeder at the start of `data`.
    pub(crate) const fn new(data: Box<dyn Pull + 'data>) -> Self {
        Self { data, end: 0 }
    }

    /// Pulls into `buffer`; returns the count.
    fn pull(&mut self, buffer: &mut [u8]) -> Result<usize, PdfError> {
        let count = self.data.pull(buffer)?;
        self.end = self
            .end
            .saturating_add(u64::try_from(count).unwrap_or_default());
        Ok(count)
    }
}

impl Input for Feeder<'_> {
    fn fill(
        &mut self,
        offset: u64,
        buffer: &mut [u8],
        operation: &'static str,
    ) -> Result<usize, PdfError> {
        if offset < self.end {
            return Err(PdfError::new(ErrorCode::Malformed, operation));
        }
        let mut scratch = [0; STEP];
        while self.end < offset {
            let wanted = usize::try_from(offset.saturating_sub(self.end)).unwrap_or(STEP);
            let step = scratch.get_mut(..wanted.min(STEP)).unwrap_or_default();
            if self.pull(step)? == 0 {
                return Ok(0);
            }
        }
        let mut filled: usize = 0;
        while let Some(rest) = buffer.get_mut(filled..).filter(|rest| !rest.is_empty()) {
            match self.pull(rest)? {
                0 => break,
                count => filled = filled.saturating_add(count),
            }
        }
        Ok(filled)
    }
}

impl Pull for Box<dyn Pull + '_> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        (**self).pull(buf)
    }
}

impl Pull for Limited<'_> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let count = self.data.pull(buf)?;
        let bytes = u64::try_from(count).unwrap_or(u64::MAX);
        let limit = || PdfError::new(ErrorCode::LimitExceeded, self.operation);
        self.left = self.left.checked_sub(bytes).ok_or_else(limit)?;
        if let Some(work) = self.work {
            work.take(bytes, self.operation)?;
        }
        Ok(count)
    }
}

impl Pull for Raw<'_> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let wanted = usize::try_from(self.remaining)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let Some(part) = buf.get_mut(..wanted).filter(|part| !part.is_empty()) else {
            return Ok(0);
        };
        let count = self.source.read(self.position, part, self.operation)?;
        let step = u64::try_from(count).unwrap_or_default();
        self.position = self.position.saturating_add(step);
        self.remaining = self.remaining.saturating_sub(step);
        Ok(count)
    }
}

/// `data` with the filters of the stream whose dictionary is at `dictionary` undone, each stage at
/// most the policy's decoded limit; an image or unknown filter is [`ErrorCode::Unsupported`]
/// (ISO 32000-2:2020, 7.4).
fn chain<'store>(
    store: Objects<'store>,
    stored: Box<dyn Pull + 'store>,
    dictionary: At,
    indirect: Indirect,
) -> Result<Box<dyn Pull + 'store>, PdfError> {
    let operation = store.operation();
    // More than `FILTERS` is too many.
    let list = |key: &[u8]| {
        let values = entries(
            store,
            store.get(dictionary, key)?,
            indirect,
            FILTERS.wrapping_add(1),
        )?;
        if values.len() > FILTERS {
            return Err(PdfError::new(ErrorCode::LimitExceeded, operation));
        }
        Ok(values)
    };
    let names = list(b"Filter")?;
    let mut parameters = list(b"DecodeParms")?.into_iter();
    if names.is_empty() {
        return Ok(limited(store, stored));
    }
    let mut data = stored;
    // The rows of all its predictors, which are alive together.
    let mut held = 0;
    for name in names {
        let found = match parameters.next() {
            None | Some(Value::Null) => None,
            Some(Value::Dictionary(at)) => Some(at),
            Some(_) => return Err(store.malformed()),
        };
        let Value::Name(filter) = name else {
            return Err(store.malformed());
        };
        let integer = |key: &[u8], default: i64| -> Result<i64, PdfError> {
            let Some(at) = found else {
                return Ok(default);
            };
            match store.get(at, key)? {
                None => Ok(default),
                Some(value) => resolve(store, value, indirect)?
                    .integer()
                    .ok_or_else(|| store.malformed()),
            }
        };
        data = limited(store, stage(store, data, &filter, (&integer, &mut held))?);
    }
    Ok(data)
}

/// The data of stream `id`, decrypted unless `id` is none, at `start`, decoded as [`chain`] does.
/// An indirect `Filter` or `DecodeParms` is [`ErrorCode::Malformed`] when `direct_only`
/// (ISO 32000-2:2020, 7.5.8.2), and one in an object stream otherwise, so object streams never
/// need each other.
pub(crate) fn decoder<'store>(
    store: Objects<'store>,
    id: Option<ObjectId>,
    start: u64,
    length: u64,
    dictionary: At,
    direct_only: bool,
) -> Result<Box<dyn Pull + 'store>, PdfError> {
    let indirect = if direct_only {
        Indirect::Never
    } else {
        Indirect::Outside
    };
    chain(
        store,
        stored(store, id, start, length, dictionary, indirect)?,
        dictionary,
        indirect,
    )
}

/// Skips `count` bytes of `data`; the end before that is [`ErrorCode::Malformed`].
pub(crate) fn discard(
    data: &mut dyn Pull,
    count: u64,
    operation: &'static str,
) -> Result<(), PdfError> {
    let mut scratch = [0; STEP];
    let mut left = count;
    while left > 0 {
        let wanted = usize::try_from(left).unwrap_or(STEP).min(STEP);
        let step = scratch.get_mut(..wanted).unwrap_or_default();
        let pulled = data.pull(step)?;
        if pulled == 0 {
            return Err(PdfError::new(ErrorCode::Malformed, operation));
        }
        left = left.saturating_sub(u64::try_from(pulled).unwrap_or_default());
    }
    Ok(())
}

/// Reads and discards the rest of `data`.
pub(crate) fn drain(data: &mut dyn Pull) -> Result<(), PdfError> {
    let mut scratch = [0; STEP];
    while data.pull(&mut scratch)? > 0 {}
    Ok(())
}

/// The first `most` elements of `entry` when it is an array, else `entry` itself, none when
/// absent, references resolved as `indirect` allows.
fn entries(
    store: Objects<'_>,
    entry: Option<Value>,
    indirect: Indirect,
    most: usize,
) -> Result<Vec<Value>, PdfError> {
    let Some(value) = entry
        .map(|found| resolve(store, found, indirect))
        .transpose()?
    else {
        return Ok(Vec::new());
    };
    let Some(start) = value.array() else {
        return Ok(vec![value]);
    };
    let mut reader = store.reader(start)?;
    drop(reader.token()?);
    let mut values = Vec::new();
    while values.len() < most {
        let token = reader.item()?;
        if token == Token::ArrayEnd {
            break;
        }
        values.push(reader.complete(token, store.depth_limit(), true)?);
    }
    values
        .into_iter()
        .map(|found| resolve(store, found, indirect))
        .collect()
}

/// The filters of the stream whose dictionary is `entries` and their parameters, resolved
/// through `overlay`; more than [`FILTERS`] is too many (ISO 32000-2:2020, 7.3.8, 7.4).
fn filters(
    store: Objects<'_>,
    overlay: &Overlay,
    entries: Dictionary,
) -> Result<Vec<(Vec<u8>, Object)>, PdfError> {
    let malformed = || store.malformed();
    let dictionary = Object::Dictionary(entries);
    let list = |key: &[u8]| -> Result<Vec<Object>, PdfError> {
        let items = match overlay.entry(store, &dictionary, key)? {
            Object::Null => Vec::new(),
            Object::Array(items) if items.len() > FILTERS => {
                return Err(PdfError::new(ErrorCode::LimitExceeded, store.operation()));
            }
            Object::Array(items) => items,
            other @ (Object::Boolean(_)
            | Object::Dictionary(_)
            | Object::Integer(_)
            | Object::Name(_)
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_)) => vec![other],
        };
        items
            .into_iter()
            .map(|item| overlay.direct(store, item))
            .collect()
    };
    let mut parameters = list(b"DecodeParms")?.into_iter();
    let mut found = Vec::new();
    for name in list(b"Filter")? {
        let Object::Name(filter) = name else {
            return Err(malformed());
        };
        let given = parameters.next().unwrap_or(Object::Null);
        if !matches!(given, Object::Dictionary(_) | Object::Null) {
            return Err(malformed());
        }
        found.push((filter, given));
    }
    Ok(found)
}

/// Writes `data`, of a stream held in memory whose dictionary is `entries`, to `sink`, decoded
/// when `decode` by its filters resolved through `overlay`; the source's stream as [`write`]
/// writes it (ISO 32000-2:2020, 7.3.8, 7.4).
pub(crate) fn held(
    store: Objects<'_>,
    overlay: &Overlay,
    (entries, data): (Dictionary, Data),
    decode: bool,
    sink: &Sink,
) -> Result<(), PdfError> {
    let operation = store.operation();
    let undo = || {
        if decode {
            filters(store, overlay, entries)
        } else {
            Ok(Vec::new())
        }
    };
    match data {
        Data::Stored(id) => write(store, id, decode, sink),
        Data::Bytes(bytes) => pour(store, overlay, (Box::new(&*bytes), decode), undo()?, sink),
        Data::Produced(producer) => {
            let undone = undo()?;
            if undone.is_empty() {
                return producer.write(&mut |bytes| sink.write(bytes, operation));
            }
            pour(store, overlay, (producer.reader(), true), undone, sink)
        }
    }
}

/// Takes the image filter off the end of `listed`, the only place it may be.
#[cfg(feature = "rendering")]
fn image_filter(
    store: Objects<'_>,
    listed: &mut Vec<(Vec<u8>, Object)>,
) -> Result<Option<(Vec<u8>, Object)>, PdfError> {
    match listed.iter().position(|filter| {
        matches!(
            filter.0.as_slice(),
            b"CCITTFaxDecode" | b"DCTDecode" | b"JBIG2Decode" | b"JPXDecode"
        )
    }) {
        Some(at) if at.saturating_add(1) < listed.len() => Err(store.malformed()),
        Some(_) => Ok(listed.pop()),
        None => Ok(None),
    }
}

/// The bytes of `data`, of a stream held in memory; the source's stream's read through to count
/// them when encrypted (ISO 32000-2:2020, 7.3.8).
pub(crate) fn length(store: Objects<'_>, data: Data) -> Result<u64, PdfError> {
    match data {
        Data::Bytes(bytes) => u64::try_from(bytes.len()).map_err(|_error| store.malformed()),
        Data::Produced(producer) => Ok(producer.length()),
        Data::Stored(id) => {
            let (mut stored, known) = stored_data(store, id)?.ok_or_else(|| store.malformed())?;
            if let Some(found) = known {
                return Ok(found);
            }
            let mut scratch = [0; STEP];
            let mut total = 0_u64;
            loop {
                match stored.pull(&mut scratch)? {
                    0 => return Ok(total),
                    count => {
                        total = total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
                    }
                }
            }
        }
    }
}

/// `data`, failing past the policy's decoded limit.
fn limited<'data>(store: Objects<'data>, data: Box<dyn Pull + 'data>) -> Box<dyn Pull + 'data> {
    Box::new(Limited {
        data,
        left: store.decoded_limit(),
        operation: store.operation(),
        work: store.work(),
    })
}

/// The data of a stream whose dictionary is `entries` with its filters undone up to an image
/// filter, which must be the last, as a reader that borrows the store only
/// (ISO 32000-2:2020, 7.4, 8.9.5).
#[cfg(feature = "rendering")]
pub(crate) fn opened<'store>(
    store: Objects<'store>,
    overlay: &Overlay,
    (entries, data): (Dictionary, Data),
) -> Result<Opened<'store>, PdfError> {
    let mut held = 0;
    let stored: Box<dyn Pull + 'store> = match data {
        Data::Bytes(bytes) => Box::new(Held::new(bytes)),
        Data::Produced(producer) => {
            held = producer.held();
            producer.reader()
        }
        Data::Stored(id) => stored_data(store, id)?.ok_or_else(|| store.malformed())?.0,
    };
    let mut found = undone(store, overlay, entries, stored)?;
    found.held = found.held.saturating_add(held);
    Ok(found)
}

/// Writes `stored` to `sink`, when decoding with `filters` and their parameters undone, each
/// stage at most the policy's decoded limit (ISO 32000-2:2020, 7.4).
fn pour<'data>(
    store: Objects<'data>,
    overlay: &Overlay,
    (stored, decode): (Box<dyn Pull + 'data>, bool),
    filters: Vec<(Vec<u8>, Object)>,
    sink: &Sink,
) -> Result<(), PdfError> {
    let operation = store.operation();
    let first = if decode {
        limited(store, stored)
    } else {
        stored
    };
    let (mut data, _held) = staged(store, overlay, first, filters)?;
    let mut buffer = [0; STEP];
    loop {
        let count = data.pull(&mut buffer)?;
        let Some(bytes) = buffer.get(..count).filter(|bytes| !bytes.is_empty()) else {
            return Ok(());
        };
        sink.write(bytes, operation)?;
    }
}

/// `value`, or the object it refers to as `indirect` allows; a reference it does not allow is
/// [`ErrorCode::Malformed`].
fn resolve(store: Objects<'_>, value: Value, indirect: Indirect) -> Result<Value, PdfError> {
    let Value::Reference(id) = value else {
        return Ok(value);
    };
    let allowed = match indirect {
        Indirect::Any => true,
        Indirect::Never => false,
        Indirect::Outside => store
            .entry(id.number)?
            .is_none_or(|entry| entry.kind != XrefKind::Compressed),
    };
    if allowed {
        store.resolve(id)
    } else {
        Err(store.malformed())
    }
}

/// `data` with filter `name` undone, with the integer parameters `integer` gives by key and
/// default; `Crypt` was undone with the stored data (ISO 32000-2:2020, 7.4.2 to 7.4.5 and 7.4.10).
fn stage<'store>(
    store: Objects<'store>,
    data: Box<dyn Pull + 'store>,
    name: &[u8],
    (integer, held): (&Parameter<'_>, &mut u64),
) -> Result<Box<dyn Pull + 'store>, PdfError> {
    let decoded: Box<dyn Pull + 'store> = match name {
        b"ASCIIHexDecode" => return Ok(Box::new(AsciiHex::new(data))),
        b"ASCII85Decode" => return Ok(Box::new(Ascii85::new(data))),
        b"RunLengthDecode" => return Ok(Box::new(RunLength::new(data))),
        b"Crypt" => return Ok(data),
        b"FlateDecode" => Box::new(Inflate::new(data).within(store.work())),
        b"LZWDecode" => {
            let early_change = match integer(b"EarlyChange", 1)? {
                0 => false,
                1 => true,
                _ => return Err(store.malformed()),
            };
            Box::new(Lzw::new(data, early_change))
        }
        _ => return Err(PdfError::new(ErrorCode::Unsupported, store.operation())),
    };
    let predictor = integer(b"Predictor", 1)?;
    if predictor == 1 {
        return Ok(decoded);
    }
    let predicted = Predictor::with_parameters(
        decoded,
        predictor,
        integer(b"Colors", 1)?,
        integer(b"BitsPerComponent", 8)?,
        integer(b"Columns", 1)?,
    )?;
    // Its rows, held once it reads beside those of the stages before, must fit the room a render
    // leaves.
    let rows = predicted.held();
    store.fits_room(held.saturating_add(rows))?;
    *held = held.saturating_add(rows);
    Ok(Box::new(predicted))
}

/// `first` with `filters` and their parameters undone, each stage at most the policy's decoded
/// limit (ISO 32000-2:2020, 7.4).
fn staged<'data>(
    store: Objects<'data>,
    overlay: &Overlay,
    first: Box<dyn Pull + 'data>,
    filters: Vec<(Vec<u8>, Object)>,
) -> Result<(Box<dyn Pull + 'data>, u64), PdfError> {
    let mut data = first;
    let mut held = 0;
    for (name, parameters) in filters {
        let integer = |key: &[u8], default: i64| -> Result<i64, PdfError> {
            match overlay.entry(store, &parameters, key)? {
                Object::Null => Ok(default),
                Object::Integer(value) => Ok(value),
                Object::Array(_)
                | Object::Boolean(_)
                | Object::Dictionary(_)
                | Object::Name(_)
                | Object::Real(_)
                | Object::Reference(_)
                | Object::Stream(..)
                | Object::String(_) => Err(store.malformed()),
            }
        };
        data = limited(store, stage(store, data, &name, (&integer, &mut held))?);
    }
    Ok((data, held))
}

/// The stored bytes of stream `id`, decrypted unless `id` is none, whose data is `length` bytes
/// at `start` and dictionary at `dictionary`, with the crypt filter its first `Crypt` filter
/// names, else the default (ISO 32000-2:2020, 7.4.10 and 7.6).
fn stored<'store>(
    store: Objects<'store>,
    id: Option<ObjectId>,
    start: u64,
    length: u64,
    dictionary: At,
    indirect: Indirect,
) -> Result<Box<dyn Pull + 'store>, PdfError> {
    let raw = Box::new(Raw {
        operation: store.operation(),
        position: start,
        remaining: length,
        source: store.source(),
    });
    let (Some(object), Some(handler)) = (id, store.security()) else {
        return Ok(raw);
    };
    let first = |key: &[u8]| -> Result<Option<Value>, PdfError> {
        Ok(entries(store, store.get(dictionary, key)?, indirect, 1)?
            .into_iter()
            .next())
    };
    let crypt = if first(b"Filter")?.is_some_and(|found| found.is_name(b"Crypt")) {
        let parameters = first(b"DecodeParms")?.as_ref().and_then(Value::dictionary);
        let name = parameters
            .map(|at| store.get(at, b"Name"))
            .transpose()?
            .flatten();
        Some(if let Some(Value::Name(found)) = name {
            found
        } else {
            b"Identity".to_vec()
        })
    } else {
        None
    };
    handler.stream(store, object, dictionary, crypt.as_deref(), raw)
}

/// The stored bytes of stream `id`, decrypted, and their count when known without reading them:
/// when not decrypted; `None` when `id` is not a stream (ISO 32000-2:2020, 7.3.8, 7.6).
pub(crate) fn stored_data(
    store: Objects<'_>,
    id: ObjectId,
) -> Result<Option<Stored<'_>>, PdfError> {
    let Some(mut reader) = store.located(id)? else {
        return Ok(None);
    };
    let Some(dictionary) = reader.value(store.depth_limit())?.dictionary() else {
        return Ok(None);
    };
    let after = reader.position();
    if reader.next_token()? != Some(Token::Keyword(Keyword::Stream)) {
        return Ok(None);
    }
    reader.seek(after);
    let (start, length) = store.extent(&mut reader, dictionary, true)?;
    let known = store.security().is_none().then_some(length);
    Ok(Some((
        stored(store, Some(id), start, length, dictionary, Indirect::Any)?,
        known,
    )))
}

/// `stored`, the data as stored of a stream or inline image whose dictionary is `entries`, as
/// [`opened`] gives a stream's (ISO 32000-2:2020, 7.4, 8.9.7).
#[cfg(feature = "rendering")]
pub(crate) fn undone<'data>(
    store: Objects<'data>,
    overlay: &Overlay,
    entries: Dictionary,
    stored: Box<dyn Pull + 'data>,
) -> Result<Opened<'data>, PdfError> {
    let mut listed = filters(store, overlay, entries)?;
    let image = image_filter(store, &mut listed)?;
    // What its predictors' rows hold counts while it is read.
    let (data, held) = staged(store, overlay, limited(store, stored), listed)?;
    Ok(Opened { data, held, image })
}

/// Writes the data of stream `id` to `sink`, decoded when `decode`; a missing object or one that
/// is not a stream is [`ErrorCode::InvalidArgument`] (ISO 32000-2:2020, 7.3.8).
pub(crate) fn write(
    store: Objects<'_>,
    id: ObjectId,
    decode: bool,
    sink: &Sink,
) -> Result<(), PdfError> {
    let operation = store.operation();
    let invalid = || PdfError::new(ErrorCode::InvalidArgument, operation);
    let mut reader = store.located(id)?.ok_or_else(invalid)?;
    let dictionary = reader
        .value(store.depth_limit())?
        .dictionary()
        .ok_or_else(invalid)?;
    let after = reader.position();
    if reader.next_token()? != Some(Token::Keyword(Keyword::Stream)) {
        return Err(invalid());
    }
    reader.seek(after);
    let (start, length) = store.extent(&mut reader, dictionary, true)?;
    let mut data = stored(store, Some(id), start, length, dictionary, Indirect::Any)?;
    if decode {
        data = chain(store, data, dictionary, Indirect::Any)?;
    }
    let mut buffer = [0; STEP];
    loop {
        let count = data.pull(&mut buffer)?;
        let Some(bytes) = buffer.get(..count).filter(|bytes| !bytes.is_empty()) else {
            return Ok(());
        };
        sink.write(bytes, operation)?;
    }
}
