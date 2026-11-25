//! Object streams (ISO 32000-2:2020, 7.5.7).

use crate::pull::Pull;
use crate::store::Objects;
use crate::stream::Feeder;
use alloc::collections::{BTreeSet, VecDeque};

use crate::syntax::{At, Input, Reader, Token, Value};
use crate::{ErrorCode, PdfError};

/// The most bytes a [`Replay`] keeps.
const KEPT: usize = 0x1_0000;

/// The most object streams whose members are read alone that are recorded.
const REFUSED: usize = 4096;

/// The most object streams read in order at once: each holds two readers.
const SEQUENCES: usize = 4;

/// An object stream's decoded data read on once, its last bytes kept, so that a member's reads
/// get what reading it again from the start gets, a failure where that meets one included.
struct Replay<'data> {
    /// The data.
    data: Box<dyn Pull + 'data>,
    /// The bytes decoded.
    end: u64,
    /// Where the data ended, and the failure that ended it, if one did.
    ended: Option<(u64, Option<ErrorCode>)>,
    /// The last bytes decoded, at most [`KEPT`], ending at `end`.
    kept: VecDeque<u8>,
}

/// A member's reads of a [`Replay`].
pub(crate) struct Replaying<'run, 'data>(&'run mut Replay<'data>);

/// The members of one object stream read in index order, each from where the one before ended,
/// with what reading it again from the start gives, so that reading them all decodes it twice,
/// not once a member.
pub(crate) struct Sequence<'data> {
    /// `N`.
    count: u64,
    /// Its data, read on to the next member's object.
    data: Replay<'data>,
    /// `First`.
    first: u64,
    /// The index of the next number and offset pair.
    next: u64,
    /// Its data, read on to the next number and offset pair.
    pairs: Reader<Feeder<'data>>,
    /// Its object number.
    stream: u32,
}

/// The members of a few object streams read in order, as their reads interleave; the least
/// recently opened gives way to the next.
#[derive(Default)]
pub(crate) struct Sequences<'data> {
    /// Those open, the least recently opened first.
    open: Vec<Sequence<'data>>,
    /// Whether the store was rebuilt when they were opened.
    rebuilt: bool,
    /// The object streams whose members are read alone, at most [`REFUSED`].
    refused: BTreeSet<u32>,
}

impl<'data> Sequences<'data> {
    /// The one reading object stream `number`, opened with `open` where none is.
    pub(crate) fn of(
        &mut self,
        number: u32,
        open: impl FnOnce() -> Result<Sequence<'data>, PdfError>,
    ) -> Result<&mut Sequence<'data>, PdfError> {
        let found = self.open.iter().position(|sequence| sequence.reads(number));
        let at = if let Some(at) = found {
            at
        } else {
            if self.open.len() >= SEQUENCES {
                // Reads that thrash read alone: opened again, it would decode it again.
                let gone = self.open.remove(0);
                self.refuse(gone.stream);
            }
            self.open.push(open()?);
            self.open.len().saturating_sub(1)
        };
        self.open
            .get_mut(at)
            .ok_or_else(|| PdfError::new(ErrorCode::Malformed, "object stream"))
    }

    /// Reads object stream `number`'s members alone from now on.
    pub(crate) fn refuse(&mut self, number: u32) {
        self.open.retain(|sequence| !sequence.reads(number));
        if self.refused.len() >= REFUSED {
            self.refused.clear();
        }
        let _new = self.refused.insert(number);
    }

    /// Whether object stream `number`'s members are read alone.
    pub(crate) fn refuses(&self, number: u32) -> bool {
        self.refused.contains(&number)
    }

    /// Gives up all, when the store was rebuilt since they were opened: they read old data.
    pub(crate) fn renew(&mut self, rebuilt: bool) {
        if rebuilt != self.rebuilt {
            self.open.clear();
            self.refused.clear();
            self.rebuilt = rebuilt;
        }
    }
}

/// `First` and each member's number and offset after it.
pub(crate) type Listed = (u64, Vec<(u32, u64)>);

impl Replay<'_> {
    /// The bytes at `offset` into `buffer`, as a reader of the data from its start fills it: a
    /// failure before the buffer's end fails, an end before `offset` gives none.
    fn fill(
        &mut self,
        offset: u64,
        buffer: &mut [u8],
        operation: &'static str,
    ) -> Result<usize, PdfError> {
        let wanted = offset.saturating_add(u64::try_from(buffer.len()).unwrap_or(u64::MAX));
        let mut step = [0_u8; 0x1000];
        while self.end < wanted && self.ended.is_none() {
            match self.data.pull(&mut step) {
                Ok(0) => self.ended = Some((self.end, None)),
                Ok(count) => {
                    self.kept.extend(step.get(..count).unwrap_or_default());
                    while self.kept.len() > KEPT {
                        let _first: Option<u8> = self.kept.pop_front();
                    }
                    self.end = self
                        .end
                        .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
                }
                Err(error) => self.ended = Some((self.end, Some(error.code()))),
            }
        }
        if let Some((at, Some(code))) = self.ended
            && at < wanted
        {
            return Err(PdfError::new(code, operation));
        }
        if offset >= self.end {
            return Ok(0);
        }
        let base = self
            .end
            .saturating_sub(u64::try_from(self.kept.len()).unwrap_or(u64::MAX));
        // Given up already: read anew.
        let skip = usize::try_from(
            offset
                .checked_sub(base)
                .ok_or_else(|| PdfError::new(ErrorCode::Malformed, operation))?,
        )
        .unwrap_or(usize::MAX);
        let mut count = 0_usize;
        for (slot, byte) in buffer.iter_mut().zip(self.kept.iter().skip(skip)) {
            *slot = *byte;
            count = count.saturating_add(1);
        }
        Ok(count)
    }
}

impl Input for Replaying<'_, '_> {
    fn fill(
        &mut self,
        offset: u64,
        buffer: &mut [u8],
        operation: &'static str,
    ) -> Result<usize, PdfError> {
        self.0.fill(offset, buffer, operation)
    }
}

impl<'data> Sequence<'data> {
    /// A reader at object `number`, at `index` in it, that reads what one from its start would;
    /// none where that is before the last read, or its object starts before where the pairs read
    /// so far start.
    pub(crate) fn member(
        &mut self,
        objects: Objects<'_>,
        number: u32,
        index: u32,
    ) -> Result<Option<Reader<Replaying<'_, 'data>>>, PdfError> {
        let malformed = || objects.malformed();
        let wanted = u64::from(index);
        if wanted >= self.count {
            return Err(malformed());
        }
        if wanted < self.next {
            return Ok(None);
        }
        let mut offset = 0;
        while self.next <= wanted {
            let (Token::Integer(found), Token::Integer(at)) =
                (self.pairs.token()?, self.pairs.token()?)
            else {
                return Err(malformed());
            };
            if self.next == wanted {
                if u32::try_from(found) != Ok(number) {
                    return Err(malformed());
                }
                offset = u64::try_from(at).ok().ok_or_else(malformed)?;
            }
            self.next = self.next.saturating_add(1);
        }
        objects.charge(&mut self.pairs)?;
        let start = self.first.checked_add(offset).ok_or_else(malformed)?;
        // A reader from the start would go on from the pairs' window: from it where it holds the
        // object, else from the object, never back.
        let (at, filled) = self.pairs.window();
        if start < at {
            return Ok(None);
        }
        let held = at.saturating_add(u64::try_from(filled).unwrap_or(u64::MAX));
        let mut reader = if start < held {
            let mut primed = Reader::new(Replaying(&mut self.data), at, objects.operation());
            primed.prime()?;
            primed
        } else {
            Reader::new(Replaying(&mut self.data), start, objects.operation())
        };
        reader.seek(start);
        Ok(Some(reader))
    }

    /// Whether it reads object stream `number`.
    pub(crate) const fn reads(&self, number: u32) -> bool {
        self.stream == number
    }
}

/// The members of object stream `stream`, whose dictionary is at `dictionary`, read in order
/// from `pairs` and `data`, each its decoded data.
pub(crate) fn sequence<'data>(
    objects: Objects<'_>,
    (pairs, data): (Box<dyn Pull + 'data>, Box<dyn Pull + 'data>),
    dictionary: At,
    stream: u32,
) -> Result<Sequence<'data>, PdfError> {
    let (first, count) = counts(objects, dictionary)?;
    Ok(Sequence {
        count,
        data: Replay {
            data,
            end: 0,
            ended: None,
            kept: VecDeque::new(),
        },
        first,
        next: 0,
        pairs: Reader::new(Feeder::new(pairs), 0, objects.operation()),
        stream,
    })
}

/// `First` and `N` of the object stream whose dictionary is at `dictionary`.
fn counts(objects: Objects<'_>, dictionary: At) -> Result<(u64, u64), PdfError> {
    let non_negative = |key: &[u8]| {
        objects
            .get(dictionary, key)?
            .as_ref()
            .and_then(Value::integer)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| objects.malformed())
    };
    Ok((non_negative(b"First")?, non_negative(b"N")?))
}

/// The object numbers the object stream whose dictionary is at `dictionary` and whose decoded
/// data is `data` holds, in index order; more than `limit` is [`ErrorCode::LimitExceeded`].
pub(crate) fn numbers(
    objects: Objects<'_>,
    data: Box<dyn Pull + '_>,
    dictionary: At,
    limit: u64,
) -> Result<Vec<u32>, PdfError> {
    let (_, count) = counts(objects, dictionary)?;
    if count > limit {
        return Err(PdfError::new(ErrorCode::LimitExceeded, objects.operation()));
    }
    let mut reader = Reader::new(Feeder::new(data), 0, objects.operation());
    let mut numbers = Vec::new();
    for _ in 0..count {
        let (Token::Integer(number), Token::Integer(_)) = (reader.token()?, reader.token()?) else {
            return Err(objects.malformed());
        };
        numbers.push(
            u32::try_from(number)
                .ok()
                .ok_or_else(|| objects.malformed())?,
        );
    }
    Ok(numbers)
}

/// A reader at object `number`, at `index` in the object stream whose dictionary is at
/// `dictionary` and whose decoded data is `data`.
pub(crate) fn member<'data>(
    objects: Objects<'_>,
    data: Box<dyn Pull + 'data>,
    dictionary: At,
    number: u32,
    index: u32,
) -> Result<Reader<Feeder<'data>>, PdfError> {
    let malformed = || objects.malformed();
    let (first, count) = counts(objects, dictionary)?;
    if u64::from(index) >= count {
        return Err(malformed());
    }
    let mut reader = Reader::new(Feeder::new(data), 0, objects.operation());
    let mut offset = 0;
    for position in 0..=index {
        let (Token::Integer(found), Token::Integer(at)) = (reader.token()?, reader.token()?) else {
            return Err(malformed());
        };
        if position == index {
            if u32::try_from(found) != Ok(number) {
                return Err(malformed());
            }
            offset = u64::try_from(at).ok().ok_or_else(malformed)?;
        }
    }
    objects.charge(&mut reader)?;
    reader.seek(first.checked_add(offset).ok_or_else(malformed)?);
    Ok(reader)
}

/// `First` and each member's number and offset after it, of the object stream whose dictionary is
/// at `dictionary` and whose decoded data is `data`, of `bytes`; none where it lists more than
/// that many bytes could hold, or more than there is room for.
pub(crate) fn members(
    objects: Objects<'_>,
    data: Box<dyn Pull + '_>,
    dictionary: At,
    bytes: usize,
) -> Result<Option<Listed>, PdfError> {
    let malformed = || objects.malformed();
    let (first, count) = counts(objects, dictionary)?;
    // A pair takes at least four bytes: two digits, each with a white-space after it.
    let most = u64::try_from(bytes).unwrap_or(u64::MAX) >> 2_u32;
    let size = count.saturating_mul(u64::try_from(size_of::<(u32, u64)>()).unwrap_or(u64::MAX));
    if count > most || objects.fits_room(size).is_err() {
        return Ok(None);
    }
    let mut members = Vec::new();
    members
        .try_reserve_exact(usize::try_from(count).unwrap_or(usize::MAX))
        .map_err(|_error| PdfError::new(ErrorCode::BudgetExceeded, objects.operation()))?;
    let mut reader = Reader::new(Feeder::new(data), 0, objects.operation());
    for _ in 0..count {
        let (Token::Integer(number), Token::Integer(at)) = (reader.token()?, reader.token()?)
        else {
            return Err(malformed());
        };
        members.push((
            u32::try_from(number).ok().ok_or_else(malformed)?,
            u64::try_from(at).ok().ok_or_else(malformed)?,
        ));
    }
    objects.charge(&mut reader)?;
    Ok(Some((first, members)))
}
