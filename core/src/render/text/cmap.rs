//! Embedded `CMaps` and maps of ranges (ISO 32000-2:2020, 9.7.5, 9.7.6).

use alloc::collections::BTreeMap;

use crate::PdfError;
use crate::overlay::Object;
use crate::pull::number;
use crate::render::OPERATION;
use crate::render::data::{self, Resolved};
use crate::render::interpret::Run;
use crate::render::job::{self, Job};
use crate::stream::Feeder;
use crate::syntax::{Reader, Token};

/// How far `UseCMap` nests.
const DEPTH: u8 = 8;

/// Work of putting a range in a map, which may split others.
const RANGE_WORK: u64 = 64;

/// Work of a token.
const TOKEN_WORK: u64 = 6;

/// The bytes a range takes.
pub(super) const RANGE: u64 = 64;

/// The most ranges of a `CMap` and those it uses.
const RANGES: u64 = 0x4_0000;

/// The most codespace ranges, as each code is held against all.
const SPACES: usize = 256;

/// A `CMap`: from codes to CIDs.
#[derive(Default)]
pub(super) struct Cmap {
    /// The CIDs by code, each with the code its range starts at.
    cids: Ranges<(u64, u32)>,
    /// The ranges read.
    count: u64,
    /// The CIDs of codes without one.
    missing: Ranges<u32>,
    /// The codespace.
    spaces: Vec<Codespace>,
    /// `WMode` 1.
    vertical: bool,
}

/// A codespace range: a span for each byte.
#[derive(Clone, Copy)]
struct Codespace {
    /// The most of each byte.
    high: [u8; 4],
    /// The bytes of a code.
    length: usize,
    /// The least of each byte.
    low: [u8; 4],
}

/// A value for each key, by ranges; the one put last wins.
pub(super) struct Ranges<T> {
    /// Each range's last key and value, by its first: none overlap.
    map: BTreeMap<u64, (u64, T)>,
}

/// A `CMap`'s program being read.
struct Reading {
    /// The dictionary names the `CMap` used.
    based: bool,
    /// What it gives.
    cmap: Cmap,
    /// The codes before a CID or in a codespace range.
    codes: Vec<(u32, usize)>,
    /// The name just read.
    name: Option<Vec<u8>>,
    /// It uses a predefined `CMap` other than Identity.
    predefined: bool,
    /// The section.
    section: Section,
}

/// A section of a `CMap`'s program.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Section {
    /// CID ranges or characters.
    Cid,
    /// Notdef ranges or characters.
    Missing,
    /// None that counts.
    Other,
    /// Codespace ranges.
    Space,
}

impl Cmap {
    /// The bytes it holds.
    pub(super) fn bytes(&self) -> u64 {
        self.cids
            .bytes()
            .saturating_add(self.missing.bytes())
            .saturating_add(
                u64::try_from(self.spaces.len())
                    .unwrap_or(0)
                    .saturating_mul(RANGE),
            )
    }

    /// Adds Identity.
    fn identify(&mut self, vertical: bool) {
        self.vertical = vertical;
        self.spaces.push(Codespace {
            high: [0xFF, 0xFF, 0, 0],
            length: 2,
            low: [0; 4],
        });
        let first = keyed(0, 2);
        self.cids.put(first, keyed(0xFFFF, 2), (first, 0));
    }

    /// Whether it writes downwards.
    pub(super) const fn is_vertical(&self) -> bool {
        self.vertical
    }

    /// The predefined `CMap` `name`, when it is Identity.
    pub(super) fn named(name: &[u8]) -> Option<Self> {
        let vertical = match name {
            b"Identity-H" => false,
            b"Identity-V" => true,
            _ => return None,
        };
        let mut cmap = Self::default();
        cmap.identify(vertical);
        Some(cmap)
    }

    /// The CID and the length of the code that starts `bytes` (9.7.6.2, 9.7.6.3); `None` when it
    /// is cut short.
    pub(super) fn next(
        &self,
        job: &Job<'_>,
        bytes: &[u8],
    ) -> Result<Option<(u32, usize)>, PdfError> {
        job.spend(u64::try_from(self.spaces.len()).unwrap_or(u64::MAX))?;
        // Whole, else the most bytes matched; then the shortest.
        let mut best: Option<(bool, usize, usize)> = None;
        for space in &self.spaces {
            let matched = bytes
                .iter()
                .take(space.length)
                .zip(space.low.iter().zip(&space.high))
                .take_while(|pair| (*pair.1.0..=*pair.1.1).contains(pair.0))
                .count();
            let found = (matched == space.length, matched, space.length);
            let better = best.is_none_or(|(whole, most, length)| {
                if found.0 != whole {
                    found.0
                } else if !whole && found.1 != most {
                    found.1 > most
                } else {
                    found.2 < length
                }
            });
            if better {
                best = Some(found);
            }
        }
        let (valid, _, length) = best.unwrap_or((false, 0, 2));
        let Some(taken) = bytes.get(..length) else {
            return Ok(None);
        };
        let code = keyed(number(taken), length);
        let mapped = self.cids.find(code).filter(|_found| valid);
        let cid = match mapped {
            Some((origin, first)) => {
                let past = u32::try_from(code.saturating_sub(origin)).unwrap_or(u32::MAX);
                first.saturating_add(past)
            }
            None => self.missing.find(code).unwrap_or(0),
        };
        Ok(Some((cid, length)))
    }

    /// The `CMap` in `stream`; `None` when it uses a predefined one other than Identity.
    pub(super) fn read(run: &mut Run<'_, '_>, stream: &Resolved) -> Result<Option<Self>, PdfError> {
        let Some(mut cmap) = Self::used(run, stream, 0)? else {
            return Ok(None);
        };
        if cmap.spaces.is_empty() {
            run.job.warn("9.7.6.2", "a CMap has no codespace")?;
            cmap.spaces.push(Codespace {
                high: [0xFF, 0xFF, 0, 0],
                length: 2,
                low: [0; 4],
            });
        }
        Ok(Some(cmap))
    }

    /// The `CMap` in `stream` over those it uses, `depth` down.
    fn used(run: &mut Run<'_, '_>, stream: &Resolved, depth: u8) -> Result<Option<Self>, PdfError> {
        let job = run.job;
        let owner = stream.value();
        let base = data::entry(job, owner, b"UseCMap")?;
        let mut based = true;
        let below = if let Some(name) = data::name(base.value()) {
            Self::named(name)
        } else if matches!(*base.value(), Object::Stream(..)) && depth.saturating_add(1) < DEPTH {
            Self::used(run, &base, depth.saturating_add(1))?
        } else if matches!(*base.value(), Object::Stream(..)) {
            // A loop: under strict a limit, else the chain ends here.
            if job.objects.strict() {
                return Err(job.limit());
            }
            job.warn("9.7.5.3", "UseCMap nests too deep")?;
            based = false;
            Some(Self::default())
        } else {
            if !matches!(*base.value(), Object::Null) {
                job.warn("9.7.5.3", "UseCMap is neither a name nor a stream")?;
            }
            based = false;
            Some(Self::default())
        };
        let Some(cmap) = below else {
            return Ok(None);
        };
        let mut reading = Reading {
            based,
            cmap,
            codes: Vec::new(),
            name: None,
            predefined: false,
            section: Section::Other,
        };
        let input = data::open(job, &mut run.caches.streams, stream)?;
        let read = reading.all(job, Feeder::new(input));
        let _read: Option<()> = job.tolerate(read, "9.7.5.3", "a CMap is damaged")?;
        if reading.predefined {
            return Ok(None);
        }
        let mode = data::entry(job, owner, b"WMode")?;
        if let Object::Integer(value) = *mode.value() {
            reading.cmap.vertical = value == 1;
        }
        Ok(Some(reading.cmap))
    }
}

impl<T> Default for Ranges<T> {
    fn default() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }
}

impl<T: Copy> Ranges<T> {
    /// The bytes it holds.
    pub(super) fn bytes(&self) -> u64 {
        u64::try_from(self.map.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(RANGE)
    }

    /// The value of `key`.
    pub(super) fn find(&self, key: u64) -> Option<T> {
        let (_, held) = self.map.range(..=key).next_back()?;
        (held.0 >= key).then_some(held.1)
    }

    /// Gives the keys `first` to `last` the value `value`.
    pub(super) fn put(&mut self, first: u64, last: u64, value: T) {
        if first > last {
            return;
        }
        let before = self.map.range(..first).next_back();
        if let Some((start, (end, held))) = before.map(|entry| (*entry.0, *entry.1))
            && end >= first
        {
            let _cut: Option<(u64, T)> = self.map.insert(start, (first.saturating_sub(1), held));
            if end > last {
                let _rest: Option<(u64, T)> = self.map.insert(last.saturating_add(1), (end, held));
            }
        }
        while let Some((start, (end, held))) = self
            .map
            .range(first..=last)
            .next()
            .map(|entry| (*entry.0, *entry.1))
        {
            let _covered: Option<(u64, T)> = self.map.remove(&start);
            if end > last {
                let _rest: Option<(u64, T)> = self.map.insert(last.saturating_add(1), (end, held));
            }
        }
        let _none: Option<(u64, T)> = self.map.insert(first, (last, value));
    }
}

impl Reading {
    /// Reads the program in `input` to its end.
    fn all(&mut self, job: &Job<'_>, input: Feeder<'_>) -> Result<(), PdfError> {
        let mut reader = Reader::new(input, 0, OPERATION);
        reader.set_content();
        reader.set_keep(true);
        reader.set_keep_limit(job::OPERANDS_HELD);
        reader.set_dropping();
        while let Some(token) = reader.next_token()? {
            job.spend(TOKEN_WORK)?;
            if reader.dropped() {
                let _dropped: Option<()> = job.tolerate(Err(job.capped()), "7.8.2", job::LONG)?;
                continue;
            }
            let name = self.name.take();
            match token {
                Token::Keyword(_) => self.keyword(job, reader.word(), name.as_deref())?,
                Token::Name(found) => self.name = Some(found),
                Token::Integer(value) => {
                    if name.as_deref() == Some(b"WMode".as_slice()) {
                        self.cmap.vertical = value == 1;
                    } else {
                        self.number(job, value)?;
                    }
                }
                Token::String(bytes) => self.string(job, &bytes)?,
                Token::ArrayEnd
                | Token::ArrayStart
                | Token::DictionaryEnd
                | Token::DictionaryStart
                | Token::Real(_)
                | Token::Reference(_) => {}
            }
        }
        Ok(())
    }

    /// Counts one range more.
    fn counted(&mut self, job: &Job<'_>) -> Result<(), PdfError> {
        self.cmap.count = self.cmap.count.saturating_add(1);
        if self.cmap.count > RANGES {
            return Err(job.capped());
        }
        Ok(())
    }

    /// Takes the keyword that starts with `word`, after `name`.
    fn keyword(&mut self, job: &Job<'_>, word: &[u8], name: Option<&[u8]>) -> Result<(), PdfError> {
        // The reader keeps eight bytes of it; the operands tell a range from a character.
        self.section = match word {
            b"begincid" => Section::Cid,
            b"begincod" => Section::Space,
            b"beginnot" => Section::Missing,
            b"usecmap" => {
                if !self.based {
                    self.used(job, name)?;
                }
                Section::Other
            }
            _ if word.starts_with(b"begin") || word.starts_with(b"end") => Section::Other,
            _ => return Ok(()),
        };
        self.codes.clear();
        Ok(())
    }

    /// Takes the CID `value` of the codes before it.
    fn number(&mut self, job: &Job<'_>, value: i64) -> Result<(), PdfError> {
        if !matches!(self.section, Section::Cid | Section::Missing) {
            return Ok(());
        }
        let ends = self.codes.first().copied().zip(self.codes.last().copied());
        self.codes.clear();
        let cid = u32::try_from(value).ok();
        let (Some(((first, length), (last, width))), Some(target)) = (ends, cid) else {
            return job.warn("9.7.5.4", "a range of a CMap is malformed");
        };
        if length != width || first > last {
            return job.warn("9.7.5.4", "a range of a CMap is malformed");
        }
        self.counted(job)?;
        self.room(job)?;
        job.spend(RANGE_WORK)?;
        let (from, until) = (keyed(first, length), keyed(last, width));
        if self.section == Section::Cid {
            self.cmap.cids.put(from, until, (from, target));
        } else {
            self.cmap.missing.put(from, until, target);
        }
        Ok(())
    }

    /// Fails unless two map entries more fit beside all that is held: a range may split another.
    fn room(&self, job: &Job<'_>) -> Result<(), PdfError> {
        job.fits(self.cmap.bytes().saturating_add(RANGE.saturating_mul(2)))
    }

    /// Takes the code `bytes`.
    fn string(&mut self, job: &Job<'_>, bytes: &[u8]) -> Result<(), PdfError> {
        if self.section == Section::Other {
            return Ok(());
        }
        if !(1..=4).contains(&bytes.len()) || self.codes.len() >= 2 {
            self.codes.clear();
            return job.warn("9.7.5.4", "a range of a CMap is malformed");
        }
        self.codes.push((number(bytes), bytes.len()));
        if self.section != Section::Space || self.codes.len() < 2 {
            return Ok(());
        }
        let ends = self.codes.first().copied().zip(self.codes.last().copied());
        self.codes.clear();
        let Some(((first, length), (last, width))) = ends else {
            return Ok(());
        };
        if length != width {
            return job.warn("9.7.5.4", "a range of a CMap is malformed");
        }
        if self.cmap.spaces.len() >= SPACES {
            return Err(job.capped());
        }
        self.cmap.spaces.push(Codespace {
            high: aligned(last, width),
            length,
            low: aligned(first, length),
        });
        Ok(())
    }

    /// Uses the `CMap` `name` (9.7.5.3).
    fn used(&mut self, job: &Job<'_>, name: Option<&[u8]>) -> Result<(), PdfError> {
        let vertical = match name {
            Some(b"Identity-H") => false,
            Some(b"Identity-V") => true,
            Some(_) | None => {
                self.predefined = true;
                return Ok(());
            }
        };
        if self.cmap.spaces.len() >= SPACES {
            return Err(job.capped());
        }
        self.counted(job)?;
        self.room(job)?;
        self.cmap.identify(vertical);
        Ok(())
    }
}

/// The bytes of a code of `length` from its first.
fn aligned(code: u32, length: usize) -> [u8; 4] {
    let mut bytes = [0; 4];
    for (slot, place) in bytes.iter_mut().zip((0..length).rev()) {
        let shift = u32::try_from(place).unwrap_or(0).saturating_mul(8);
        *slot = u8::try_from(code.wrapping_shr(shift) & 0xFF).unwrap_or(0);
    }
    bytes
}

/// The key of a code of `length` bytes: lengths never meet.
fn keyed(code: u32, length: usize) -> u64 {
    u64::try_from(length).unwrap_or(0).wrapping_shl(32) | u64::from(code)
}
