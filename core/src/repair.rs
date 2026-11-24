//! Rebuilding a damaged or missing cross-reference table by scanning the file
//! (ISO 32000-2:2020, 7.5.4 and 7.5.7).

use crate::object_stream;
use crate::objects::{ObjectId, XrefEntry, XrefKind};
use crate::store::Objects;
use crate::stream::decoder;
use alloc::collections::VecDeque;
use core::cmp::Reverse;
use core::mem;

use crate::syntax::{At, Reader, Value, is_regular, is_white_space, matching, word};
use crate::{ErrorCode, PdfError};

/// Bytes kept before each chunk, for reading `number generation` back from `obj`.
const BEHIND: usize = 64;

/// Bytes read at a time.
const CHUNK: usize = 0x1_0000;

/// Work of a byte that may start a word, where work is bounded.
const CANDIDATE_WORK: u64 = 2;

/// Bytes after a match a chunk must hold to judge it.
const AHEAD: usize = 16;

/// Trailer candidates kept, the last found.
const TRAILERS: usize = 16;

/// What a scan of the file found.
pub(crate) struct Found {
    /// Each object defined in the body and where; later ones are newer.
    entries: Vec<(u64, XrefEntry)>,
    /// The objects that name `/ObjStm`: where and their ids.
    streams: Vec<(u64, ObjectId)>,
    /// The last [`TRAILERS`] trailer dictionaries: after `trailer`, or objects that name `/XRef`,
    /// in file order.
    trailers: VecDeque<Trailer>,
    /// Whether the file has a `xref` keyword or names `/XRef`.
    xref: bool,
}

impl Found {
    /// Keeps what starts at `at` of `bytes`, read from `base`: an object, at most `limit`, a
    /// trailer, a table or a stream of objects or entries.
    fn judge(
        &mut self,
        (bytes, at, base, file_start): (&[u8], usize, u64, bool),
        limit: u64,
        operation: &'static str,
    ) -> Result<(), PdfError> {
        let offset = base.saturating_add(u64::try_from(at).unwrap_or_default());
        // One word can start at each byte: `/` must be followed by `O` or `X`.
        let first = bytes.get(at).copied().unwrap_or(0);
        let second = bytes.get(at.saturating_add(1)).copied().unwrap_or(0);
        let slashed = first == b'/' && matches!(second, b'O' | b'X');
        if first == b'/' && !slashed {
            return Ok(());
        }
        if first == b'o'
            && token_at(bytes, at, b"obj", file_start)
            && let Some((start, id)) = object_id(bytes, at, file_start)
        {
            if u64::try_from(self.entries.len()).unwrap_or(u64::MAX) >= limit {
                return Err(PdfError::new(ErrorCode::LimitExceeded, operation));
            }
            let begin = base.saturating_add(u64::try_from(start).unwrap_or_default());
            self.entries.push((
                begin,
                XrefEntry {
                    generation: id.generation,
                    index: 0,
                    kind: XrefKind::InUse,
                    number: id.number,
                    offset: begin,
                    stream: 0,
                },
            ));
        }
        if first == b't' && token_at(bytes, at, b"trailer", file_start) {
            self.trailer(Trailer::Keyword(offset.saturating_add(7)));
        }
        if first == b'x' && token_at(bytes, at, b"xref", file_start) {
            self.xref = true;
        }
        let last = self.entries.last().copied();
        if slashed
            && token_at(bytes, at, b"/ObjStm", file_start)
            && let Some((begin, entry)) = last
            && self.streams.last().is_none_or(|stream| stream.0 != begin)
        {
            self.streams.push((begin, id_of(entry)));
        }
        if slashed
            && token_at(bytes, at, b"/XRef", file_start)
            && let Some((begin, entry)) = last
        {
            self.xref = true;
            self.trailer(Trailer::Stream(begin, id_of(entry)));
        }
        Ok(())
    }

    /// The objects that name `/ObjStm`: where and their ids.
    pub(crate) fn streams(&self) -> &[(u64, ObjectId)] {
        &self.streams
    }

    /// Each object defined in the body and where, taken.
    pub(crate) fn take_entries(&mut self) -> Vec<(u64, XrefEntry)> {
        mem::take(&mut self.entries)
    }

    /// Keeps `candidate`, dropping the oldest past [`TRAILERS`].
    fn trailer(&mut self, candidate: Trailer) {
        if self.trailers.len() >= TRAILERS {
            let _oldest = self.trailers.pop_front();
        }
        self.trailers.push_back(candidate);
    }

    /// Where the last trailer dictionaries may be, in file order.
    pub(crate) const fn trailers(&self) -> &VecDeque<Trailer> {
        &self.trailers
    }

    /// Whether the file has a `xref` keyword or names `/XRef`.
    pub(crate) const fn xref(&self) -> bool {
        self.xref
    }
}

/// Where a trailer dictionary may be.
#[derive(Clone, Copy)]
pub(crate) enum Trailer {
    /// After the `trailer` keyword at this offset's end.
    Keyword(u64),
    /// The dictionary of the object at this offset, with this id, when of type `XRef`.
    Stream(u64, ObjectId),
}

/// Whether `bytes` holds `word` at `at` as a whole token: delimited before (unless `word` starts
/// with `/`) and after.
fn token_at(bytes: &[u8], at: usize, word: &[u8], file_start: bool) -> bool {
    let after = at.saturating_add(word.len());
    bytes.get(at..after) == Some(word)
        && bytes.get(after).is_none_or(|byte| !is_regular(*byte))
        && (word.first() == Some(&b'/')
            || at.checked_sub(1).map_or(file_start, |before| {
                bytes.get(before).is_some_and(|byte| !is_regular(*byte))
            }))
}

/// The objects of object streams `streams`, each placed where its stream is, at most `limit`;
/// a stream that cannot be read is left out.
pub(crate) fn members(
    objects: Objects<'_>,
    streams: &[(u64, ObjectId)],
    limit: u64,
) -> Result<Vec<(u64, XrefEntry)>, PdfError> {
    let mut entries = Vec::new();
    for &(offset, id) in streams {
        let left = limit.saturating_sub(u64::try_from(entries.len()).unwrap_or(u64::MAX));
        match numbers(objects, offset, id, left) {
            Ok(numbers) => {
                for (number, index) in numbers.into_iter().zip(0_u32..) {
                    entries.push((
                        offset,
                        XrefEntry {
                            generation: 0,
                            index,
                            kind: XrefKind::Compressed,
                            number,
                            offset: 0,
                            stream: id.number,
                        },
                    ));
                }
            }
            Err(error) if matches!(error.code(), ErrorCode::Io | ErrorCode::LimitExceeded) => {
                return Err(error);
            }
            Err(_) => {}
        }
    }
    Ok(entries)
}

/// The newest entry of each object number in `entries`, sorted by number.
pub(crate) fn newest(mut entries: Vec<(u64, XrefEntry)>) -> Vec<XrefEntry> {
    entries.sort_by_key(|&(offset, entry)| (entry.number, Reverse(offset)));
    entries.dedup_by_key(|&mut (_, entry)| entry.number);
    entries.into_iter().map(|(_, entry)| entry).collect()
}

/// The object numbers in the object stream `id` at `offset`, at most `limit`.
fn numbers(
    objects: Objects<'_>,
    offset: u64,
    id: ObjectId,
    limit: u64,
) -> Result<Vec<u32>, PdfError> {
    let mut reader = objects.direct(offset, id)?;
    let dictionary = reader
        .value(objects.depth_limit())?
        .dictionary()
        .ok_or_else(|| objects.malformed())?;
    if !objects
        .get(dictionary, b"Type")?
        .is_some_and(|kind| kind.is_name(b"ObjStm"))
    {
        return Err(objects.malformed());
    }
    let (start, length) = objects.extent(&mut reader, dictionary, true)?;
    let data = decoder(objects, Some(id), start, length, dictionary, false)?;
    object_stream::numbers(objects, data, dictionary, limit)
}

/// `number generation` ending right before `end` in `bytes`, white-space between and a
/// delimiter or the file's start before; and where it starts.
fn object_id(bytes: &[u8], end: usize, file_start: bool) -> Option<(usize, ObjectId)> {
    let digits_before = |to: usize| {
        let from = bytes
            .get(..to)?
            .iter()
            .rposition(|byte| !byte.is_ascii_digit())
            .map_or(0, |at| at.saturating_add(1));
        let value = bytes
            .get(from..to)
            .filter(|digits| !digits.is_empty() && digits.len() <= 10)?
            .iter()
            .try_fold(0_u64, |value, digit| {
                value
                    .checked_mul(10)?
                    .checked_add(u64::from(digit.wrapping_sub(b'0')))
            })?;
        Some((from, value))
    };
    let space_before = |to: usize| {
        let from = bytes
            .get(..to)?
            .iter()
            .rposition(|byte| !is_white_space(*byte))
            .map_or(0, |at| at.saturating_add(1));
        (from < to).then_some(from)
    };
    let (generation_start, generation) = digits_before(space_before(end)?)?;
    let (start, number) = digits_before(space_before(generation_start)?)?;
    let delimited = start.checked_sub(1).map_or(file_start, |before| {
        bytes.get(before).is_some_and(|byte| !is_regular(*byte))
    });
    delimited.then_some(())?;
    Some((
        start,
        ObjectId {
            generation: u16::try_from(generation).ok()?,
            number: u32::try_from(number).ok()?,
        },
    ))
}

/// Scans the file for `number generation obj`, `trailer`, `xref`, `/ObjStm` and `/XRef`, keeping
/// at most `limit` objects (ISO 32000-2:2020, 7.3.10, 7.5.4, 7.5.5, 7.5.7 and 7.5.8).
pub(crate) fn scan(objects: Objects<'_>, limit: u64) -> Result<Found, PdfError> {
    let source = objects.source();
    let size = source.size(objects.operation())?;
    let mut found = Found {
        entries: Vec::new(),
        streams: Vec::new(),
        trailers: VecDeque::new(),
        xref: false,
    };
    let mut buffer = vec![0; CHUNK];
    let mut base: u64 = 0;
    let mut next: u64 = 0;
    loop {
        let count = source.read(base, &mut buffer, objects.operation())?;
        let bytes = buffer.get(..count).unwrap_or_default();
        let at_end = base.saturating_add(u64::try_from(count).unwrap_or(u64::MAX)) >= size
            || count < buffer.len();
        let judged = if at_end {
            count
        } else {
            count.saturating_sub(AHEAD)
        };
        let from = usize::try_from(next.saturating_sub(base)).unwrap_or(usize::MAX);
        let file_start = base == 0;
        let mut at = from;
        let mut candidates = 0_u64;
        loop {
            // Eight bytes at a time while none starts a word.
            while let Some(group) = bytes.get(at..judged).and_then(<[u8]>::first_chunk) {
                let word = word(*group);
                if matching(word, b'o')
                    | matching(word, b't')
                    | matching(word, b'x')
                    | matching(word, b'/')
                    != 0
                {
                    break;
                }
                at = at.wrapping_add(8);
            }
            let Some(skip) = bytes.get(at..judged).and_then(|rest| {
                rest.iter()
                    .position(|byte| matches!(byte, b'o' | b't' | b'x' | b'/'))
            }) else {
                break;
            };
            at = at.saturating_add(skip);
            candidates = candidates.saturating_add(1);
            found.judge((bytes, at, base, file_start), limit, objects.operation())?;
            at = at.saturating_add(1);
        }
        // Each candidate examined, beside the bytes read.
        if let Some(work) = objects.work() {
            work.take(
                candidates.saturating_mul(CANDIDATE_WORK),
                objects.operation(),
            )?;
        }
        if at_end {
            return Ok(found);
        }
        next = base.saturating_add(u64::try_from(judged).unwrap_or_default());
        base = next.saturating_sub(u64::try_from(BEHIND).unwrap_or_default());
    }
}

/// The id of `entry`.
const fn id_of(entry: XrefEntry) -> ObjectId {
    ObjectId {
        generation: entry.generation,
        number: entry.number,
    }
}

/// The last of `trailers` that is a dictionary with `Root`, else the last that is a dictionary.
pub(crate) fn trailer(
    objects: Objects<'_>,
    trailers: &VecDeque<Trailer>,
) -> Result<Option<At>, PdfError> {
    let mut fallback = None;
    for &candidate in trailers.iter().rev() {
        let read = match candidate {
            Trailer::Keyword(offset) => {
                let mut reader = Reader::new(objects.source(), offset, objects.operation());
                reader.head(objects.depth_limit())
            }
            Trailer::Stream(offset, id) => objects
                .direct(offset, id)
                .and_then(|mut reader| reader.head(objects.depth_limit())),
        };
        let at = match read {
            Ok(Value::Dictionary(at)) => at,
            Err(error) if error.code() == ErrorCode::Io => return Err(error),
            Ok(_) | Err(_) => continue,
        };
        if let Trailer::Stream(..) = candidate
            && !objects
                .get(at, b"Type")
                .is_ok_and(|kind| kind.is_some_and(|name| name.is_name(b"XRef")))
        {
            continue;
        }
        match objects.get(at, b"Root") {
            Ok(Some(_)) => return Ok(Some(at)),
            Err(error) if error.code() == ErrorCode::Io => return Err(error),
            Ok(None) | Err(_) => {
                fallback = fallback.or(Some(at));
            }
        }
    }
    Ok(fallback)
}
