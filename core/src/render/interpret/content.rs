//! Content streams as operations (ISO 32000-2:2020, 7.7.3.3, 7.8.2).

use crate::PdfError;
use crate::overlay::{Dictionary, Object};
use crate::pull::Pull;
use crate::render::OPERATION;
use crate::render::data::{self, Resolved};
use crate::render::job::{self, Job};
use crate::stream::Feeder;
use crate::syntax::{Keyword, Reader, Token};

/// The room past a string or name it may keep beside twice its bytes.
const SPARE: usize = 0x1000;

/// How deep operands nest, the policy permitting.
const DEPTH: u32 = 64;

/// Work of an operand, an item or a key: its token and holding it.
const OPERAND_WORK: u64 = 5;

/// A page's content streams as one, white space between them.
pub(crate) struct Joined<'run, 'doc> {
    /// The stream being read.
    current: Option<Box<dyn Pull + 'doc>>,
    /// The streams.
    items: Resolved,
    /// The render.
    job: &'run Job<'doc>,
    /// The stream after it.
    next: usize,
}

/// A stream's data that ends at damage, warned, so that what comes before it runs (7.8.2).
pub(crate) struct Ending<'run, 'doc, 'data> {
    /// The data.
    data: Box<dyn Pull + 'data>,
    /// Whether damage ended it.
    ended: bool,
    /// The render.
    job: &'run Job<'doc>,
}

/// An operator's name, as much as tells it apart.
#[derive(Clone, Copy)]
pub(crate) struct Name {
    /// Its first bytes.
    bytes: [u8; 8],
    /// How many count.
    length: usize,
}

/// One stream's operations.
pub(crate) struct Operations<'data> {
    /// Where the last operation starts.
    begun: u64,
    /// The bytes of the operands held.
    held: usize,
    /// The tokens.
    reader: Reader<Feeder<'data>>,
    /// The bytes each operand counts.
    weights: Vec<usize>,
}

impl<'run, 'doc> Joined<'run, 'doc> {
    /// The streams `items`, in order.
    pub(crate) const fn new(job: &'run Job<'doc>, items: Resolved) -> Self {
        Self {
            current: None,
            items,
            job,
            next: 0,
        }
    }
}

impl<'run, 'doc, 'data> Ending<'run, 'doc, 'data> {
    /// `data`, ending at damage.
    pub(crate) const fn new(job: &'run Job<'doc>, data: Box<dyn Pull + 'data>) -> Self {
        Self {
            data,
            ended: false,
            job,
        }
    }
}

impl Pull for Ending<'_, '_, '_> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        if self.ended {
            return Ok(0);
        }
        let read =
            self.job
                .tolerate(self.data.pull(buf), "7.8.2", "a content stream is damaged")?;
        self.ended = read.is_none();
        if self.ended {
            self.job.damaged.set(true);
        }
        Ok(read.unwrap_or(0))
    }
}

impl Pull for Joined<'_, '_> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        if buf.is_empty() {
            return Ok(0);
        }
        let job = self.job;
        let damaged = "a content stream is damaged";
        loop {
            if let Some(data) = self.current.as_mut() {
                // Damage ends that stream, not those after it.
                let read = job.tolerate(data.pull(buf), "7.8.2", damaged)?;
                if let Some(count @ 1..) = read {
                    return Ok(count);
                }
                if read.is_none() {
                    job.damaged.set(true);
                }
                self.current = None;
                if let Some(slot) = buf.first_mut() {
                    *slot = b'\n';
                    return Ok(1);
                }
            }
            let listed = data::items(self.items.value()).unwrap_or_default();
            let Some(item) = listed.get(self.next).cloned() else {
                return Ok(0);
            };
            self.next = self.next.saturating_add(1);
            job.spend(1)?;
            let opened = job.tolerate(begun(job, item), "7.8.2", damaged)?;
            if opened.is_none() {
                job.damaged.set(true);
            }
            self.current = opened.flatten();
        }
    }
}

impl Name {
    /// Its bytes.
    pub(crate) fn bytes(&self) -> &[u8] {
        self.bytes.get(..self.length).unwrap_or_default()
    }

    /// The name that starts with `word`.
    fn new(word: &[u8]) -> Self {
        let mut bytes = [0; 8];
        for (slot, byte) in bytes.iter_mut().zip(word) {
            *slot = *byte;
        }
        Self {
            bytes,
            length: word.len().min(8),
        }
    }
}

impl<'data> Operations<'data> {
    /// Where the last operation starts: its first operand, or else its operator.
    pub(crate) const fn begun(&self) -> u64 {
        self.begun
    }

    /// The bytes the operator's operands hold.
    pub(crate) const fn held(&self) -> usize {
        self.held
    }

    /// Counts `payload` and an object to the operator's operands.
    fn hold(&mut self, job: &Job<'_>, payload: usize) -> Result<(), PdfError> {
        self.held = self
            .held
            .saturating_add(payload)
            .saturating_add(size_of::<Object>());
        if self.held > job::OPERANDS_HELD {
            return Err(job.capped());
        }
        job.spend(OPERAND_WORK)
    }

    /// An inline image's entries, read up to `ID` (8.9.7).
    pub(crate) fn image_entries(&mut self, job: &Job<'_>) -> Result<Dictionary, PdfError> {
        self.held = 0;
        let depth = job.objects.depth_limit().min(DEPTH);
        let mut entries = Vec::new();
        loop {
            let token = self.reader.token()?;
            if matches!(token, Token::Keyword(Keyword::Other)) && self.reader.word() == b"ID" {
                return Ok(entries);
            }
            let Token::Name(key) = token else {
                return Err(job.objects.malformed());
            };
            self.hold(job, key.len())?;
            let first = self.reader.token()?;
            let value = self.value(job, first, depth)?;
            if self.reader.dropped() {
                return Err(job.capped());
            }
            entries.push((key, value));
        }
    }

    /// The tokens, for an inline image.
    pub(crate) const fn input(&mut self) -> &mut Reader<Feeder<'data>> {
        &mut self.reader
    }

    /// The operations in `data`.
    pub(crate) fn new(data: Box<dyn Pull + 'data>) -> Self {
        let mut reader = Reader::new(Feeder::new(data), 0, OPERATION);
        reader.set_content();
        reader.set_keep(true);
        reader.set_keep_limit(job::OPERANDS_HELD);
        reader.set_dropping();
        Self {
            begun: 0,
            held: 0,
            reader,
            weights: Vec::new(),
        }
    }

    /// The next operator, its operands put in `operands`; `None` at the end.
    pub(crate) fn next(
        &mut self,
        job: &Job<'_>,
        operands: &mut Vec<Object>,
    ) -> Result<Option<Name>, PdfError> {
        operands.clear();
        self.weights.clear();
        self.held = 0;
        let depth = job.objects.depth_limit().min(DEPTH);
        // Past the most operands, each replaces the oldest, here.
        let mut oldest = 0_usize;
        let mut warned = false;
        self.reader.skip_space()?;
        self.begun = self.reader.position();
        while let Some(token) = self.reader.next_token()? {
            match token {
                Token::Keyword(
                    Keyword::EndObj
                    | Keyword::EndStream
                    | Keyword::Obj
                    | Keyword::Other
                    | Keyword::Reference
                    | Keyword::Stream
                    | Keyword::Trailer,
                ) => {
                    operands.rotate_left(oldest);
                    return Ok(Some(Name::new(self.reader.word())));
                }
                Token::ArrayEnd | Token::DictionaryEnd => {
                    // Charged as the operator it stands for.
                    job.spend(job::OPERATOR_WORK)?;
                    job.warn("7.8.2", "a delimiter closes nothing")?;
                }
                Token::ArrayStart
                | Token::DictionaryStart
                | Token::Integer(_)
                | Token::Keyword(Keyword::False | Keyword::Null | Keyword::True)
                | Token::Name(_)
                | Token::Real(_)
                | Token::Reference(_)
                | Token::String(_) => {
                    let before = self.held;
                    let value = self.value(job, token, depth)?;
                    if self.reader.dropped() {
                        self.held = before;
                        let _dropped: Option<()> =
                            job.tolerate(Err(job.capped()), "7.8.2", job::LONG)?;
                        continue;
                    }
                    let weight = self.held.saturating_sub(before);
                    let full = operands.len() >= job::OPERANDS;
                    if full && !warned {
                        job.warn("7.8.2", "an operator has too many operands")?;
                        warned = true;
                    }
                    if full {
                        let old = self.weights.get(oldest).copied().unwrap_or(0);
                        self.held = self.held.saturating_sub(old);
                    }
                    if full
                        && let (Some(slot), Some(kept)) =
                            (operands.get_mut(oldest), self.weights.get_mut(oldest))
                    {
                        *slot = value;
                        *kept = weight;
                        let next = oldest.saturating_add(1);
                        oldest = if next < job::OPERANDS { next } else { 0 };
                    } else {
                        self.weights.push(weight);
                        operands.push(value);
                    }
                }
            }
        }
        Ok(None)
    }

    /// The value that starts with `token`, nesting at most `depth` deep.
    fn value(&mut self, job: &Job<'_>, token: Token, depth: u32) -> Result<Object, PdfError> {
        self.hold(job, 0)?;
        let below = || depth.checked_sub(1).ok_or_else(|| job.capped());
        Ok(match token {
            Token::ArrayStart => {
                let inner = below()?;
                let mut items = Vec::new();
                loop {
                    let item = self.reader.token()?;
                    if item == Token::ArrayEnd {
                        break Object::Array(items);
                    }
                    items.push(self.value(job, item, inner)?);
                }
            }
            Token::DictionaryStart => {
                let inner = below()?;
                let mut entries = Vec::new();
                loop {
                    match self.reader.token()? {
                        Token::DictionaryEnd => break Object::Dictionary(entries),
                        Token::Name(read) => {
                            let key = trimmed(read);
                            self.hold(job, key.capacity())?;
                            let item = self.reader.token()?;
                            entries.push((key, self.value(job, item, inner)?));
                        }
                        Token::ArrayEnd
                        | Token::ArrayStart
                        | Token::DictionaryStart
                        | Token::Integer(_)
                        | Token::Keyword(_)
                        | Token::Real(_)
                        | Token::Reference(_)
                        | Token::String(_) => return Err(job.objects.malformed()),
                    }
                }
            }
            Token::Integer(value) => Object::Integer(value),
            Token::Keyword(Keyword::False) => Object::Boolean(false),
            Token::Keyword(Keyword::Null) => Object::Null,
            Token::Keyword(Keyword::True) => Object::Boolean(true),
            Token::Name(bytes) => {
                let name = trimmed(bytes);
                self.hold(job, name.capacity())?;
                Object::Name(name)
            }
            Token::Real(value) => Object::Real(value),
            Token::String(bytes) => {
                let text = trimmed(bytes);
                self.hold(job, text.capacity())?;
                Object::String(text)
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
            )
            | Token::Reference(_) => return Err(job.objects.malformed()),
        })
    }
}

/// The data of the content stream `item`; none when null.
fn begun<'doc>(job: &Job<'doc>, item: Object) -> Result<Option<Box<dyn Pull + 'doc>>, PdfError> {
    let stream = data::resolved(job, item)?;
    if matches!(*stream.value(), Object::Null) {
        return Ok(None);
    }
    let opened = data::coded(job, &stream)?;
    if opened.image.is_some() {
        return Err(job.objects.malformed());
    }
    Ok(Some(opened.data))
}

/// `bytes`, given up the room past them where it is more than they take: so one dropped holds
/// nothing.
fn trimmed(mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.capacity() > bytes.len().saturating_mul(2).saturating_add(SPARE) {
        bytes.shrink_to_fit();
    }
    bytes
}
