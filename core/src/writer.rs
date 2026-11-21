//! A rewrite of the objects reachable from the trailer, renumbered from 1 in the order reached
//! (ISO 32000-2:2020, 7.5).

use alloc::collections::BTreeSet;

use crate::crypto::md5::Md5;
use crate::metadata::text;
use crate::objects::{ObjectId, XrefKind};
use crate::output::Sink;
use crate::overlay::{Data, Object, Overlay};
use crate::progress::Progress;
use crate::pull::Pull;
use crate::store::Objects;
use crate::stream::stored_data;
use crate::syntax::is_regular;
use crate::{ErrorCode, PdfError, Version};

/// Bytes gathered before they go to the sink.
const CHUNK: usize = 0x1_0000;

/// The largest offset a cross-reference table entry holds: ten digits (ISO 32000-2:2020, 7.5.4).
const OFFSET_LIMIT: u64 = 9_999_999_999;

/// The operation errors name.
const OPERATION: &str = "Document::save";

/// Objects written between progress reports.
const STEP: usize = 1024;

/// The new number of each source or overlay object reached, and, per new number, the old number
/// until written, then its offset: 9 bytes per object.
struct Numbers {
    /// New numbers by old number; 0 when not reached.
    new: Vec<u32>,
    /// Per new number from 1: the old number, then, once written, the offset, 40 bits each.
    slots: Vec<[u8; 5]>,
}

/// The output, counted, hashed until the trailer (ISO 32000-2:2020, 14.4).
struct Out<'sink> {
    /// Bytes not yet given to the sink.
    buffer: Vec<u8>,
    /// The hash of the bytes given so far, until taken.
    hash: Option<Md5>,
    /// The sink.
    sink: &'sink Sink,
    /// Bytes given to the sink.
    written: u64,
}

/// What a rewrite needs and gives.
pub(crate) struct Rewrite<'run> {
    /// Allow invalidating signatures.
    pub break_signatures: bool,
    /// The source's objects.
    pub objects: Objects<'run>,
    /// The changes.
    pub overlay: &'run Overlay,
    /// Told of progress, and may cancel.
    pub progress: Option<&'run dyn Progress>,
    /// The header's version.
    pub version: Version,
}

impl Numbers {
    /// The new number of `id`, given when first reached; `None` for one that no object can have.
    fn number(&mut self, id: ObjectId) -> Result<Option<u32>, PdfError> {
        let Some(slot) = usize::try_from(id.number)
            .ok()
            .filter(|&number| number > 0)
            .and_then(|number| self.new.get_mut(number))
        else {
            return Ok(None);
        };
        if *slot == 0 {
            self.slots.push(pack(u64::from(id.number)));
            *slot = u32::try_from(self.slots.len())
                .map_err(|_error| PdfError::new(ErrorCode::LimitExceeded, OPERATION))?;
        }
        Ok(Some(*slot))
    }
}

impl Out<'_> {
    /// Gives the gathered bytes to the sink.
    fn flush(&mut self) -> Result<(), PdfError> {
        if let Some(hash) = self.hash.as_mut() {
            hash.update(&self.buffer);
        }
        self.sink.write(&self.buffer, OPERATION)?;
        self.written = self
            .written
            .saturating_add(u64::try_from(self.buffer.len()).unwrap_or(u64::MAX));
        self.buffer.clear();
        Ok(())
    }

    /// The offset of the next byte.
    fn position(&self) -> u64 {
        self.written
            .saturating_add(u64::try_from(self.buffer.len()).unwrap_or(u64::MAX))
    }

    /// Appends `bytes`.
    fn put(&mut self, bytes: &[u8]) -> Result<(), PdfError> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() >= CHUNK {
            self.flush()?;
        }
        Ok(())
    }
}

/// Fails unless the document of `run` has pages and, unless allowed, no signature a rewrite
/// breaks; the names of the signature fields it breaks (ISO 32000-2:2020, 7.7.3.2, 12.8).
fn check(run: &Rewrite<'_>, root: ObjectId) -> Result<Vec<String>, PdfError> {
    let (objects, overlay) = (run.objects, run.overlay);
    let catalog = overlay.resolve(objects, root)?;
    let tree = overlay.entry(objects, &catalog, b"Pages")?;
    match overlay.entry(objects, &tree, b"Count")? {
        Object::Integer(count) if count > 0 => {}
        Object::Integer(_) => return Err(PdfError::new(ErrorCode::InvalidArgument, OPERATION)),
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Name(_)
        | Object::Null
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => return Err(objects.malformed()),
    }
    let broken = signed(objects, overlay, &catalog)?;
    if !broken.is_empty() && !run.break_signatures {
        return Err(PdfError::new(ErrorCode::SignaturesWouldBreak, OPERATION));
    }
    Ok(broken)
}

/// Writes the cross-reference table and the trailer, after `entries`, the trailer's entries
/// written; the file identifier keeps the source's first element (ISO 32000-2:2020, 7.5.4,
/// 7.5.5, 14.4).
fn finish(
    out: &mut Out<'_>,
    objects: Objects<'_>,
    numbers: &Numbers,
    entries: &[u8],
) -> Result<(), PdfError> {
    let table = out.position();
    if table > OFFSET_LIMIT {
        return Err(PdfError::new(ErrorCode::LimitExceeded, OPERATION));
    }
    let count = numbers.slots.len().saturating_add(1);
    out.put(format!("xref\n0 {count}\n0000000000 65535 f\r\n").as_bytes())?;
    for &slot in &numbers.slots {
        out.put(format!("{:010} 00000 n\r\n", unpack(slot)).as_bytes())?;
    }
    out.flush()?;
    let digest = out.hash.take().map_or_default(Md5::finish);
    let first = match objects.owned_trailer()?.take(b"ID") {
        Some(Object::Array(elements)) => match elements.into_iter().next() {
            Some(Object::String(kept)) => kept,
            Some(_) | None => digest.to_vec(),
        },
        Some(_) | None => digest.to_vec(),
    };
    let mut text = format!("trailer\n<< /Size {count}").into_bytes();
    text.extend_from_slice(entries);
    text.extend_from_slice(b" /ID [");
    hex(&mut text, &first);
    text.push(b' ');
    hex(&mut text, &digest);
    text.extend_from_slice(format!("] >>\nstartxref\n{table}\n%%EOF\n").as_bytes());
    out.put(&text)?;
    out.flush()
}

/// Writes the document `run` describes to `sink`: header, objects, table, trailer; the names of
/// the signature fields it invalidates and the bytes written (ISO 32000-2:2020, 7.5, 12.8, 14.4).
pub(crate) fn rewrite(run: &Rewrite<'_>, sink: &Sink) -> Result<(Vec<String>, u64), PdfError> {
    let (objects, overlay) = (run.objects, run.overlay);
    let trailer = Object::Dictionary(overlay.trailer(objects)?);
    let Some(Object::Reference(root)) = trailer.get(b"Root") else {
        return Err(objects.malformed());
    };
    let broken = check(run, root)?;
    let total = u64::from(overlay.next().saturating_sub(1));
    report(run.progress, 0, total)?;
    // Reserved whole, so no growth holds two copies.
    let size = usize::try_from(overlay.next()).unwrap_or(usize::MAX);
    let limit = || PdfError::new(ErrorCode::LimitExceeded, OPERATION);
    let (mut new, mut slots) = (Vec::new(), Vec::new());
    new.try_reserve_exact(size).map_err(|_error| limit())?;
    slots.try_reserve_exact(size).map_err(|_error| limit())?;
    new.resize(size, 0);
    let mut numbers = Numbers { new, slots };
    let mut entries = Vec::new();
    for key in [b"Root".as_slice(), b"Info"] {
        if let Some(found) = trailer.get(key) {
            entries.extend_from_slice(b" /");
            entries.extend_from_slice(key);
            entries.push(b' ');
            value(&mut entries, found, &mut numbers)?;
        }
    }
    let mut out = Out {
        buffer: Vec::with_capacity(CHUNK),
        hash: Some(Md5::default()),
        sink,
        written: 0,
    };
    let mut header = format!("%PDF-{}.{}\n%", run.version.major, run.version.minor).into_bytes();
    header.extend_from_slice(&[0xE2, 0xE3, 0xCF, 0xD3, b'\n']);
    out.put(&header)?;
    let mut index = 0_usize;
    while let Some(&slot) = numbers.slots.get(index) {
        let offset = out.position();
        if offset > OFFSET_LIMIT {
            return Err(PdfError::new(ErrorCode::LimitExceeded, OPERATION));
        }
        if let Some(written) = numbers.slots.get_mut(index) {
            *written = pack(offset);
        }
        let old = u32::try_from(unpack(slot)).unwrap_or_default();
        let object = match overlay.get(old) {
            Some((_, found)) => found,
            None => source(objects, old)?,
        };
        index = index.saturating_add(1);
        write(&mut out, objects, &mut numbers, index, object)?;
        if index.is_multiple_of(STEP) {
            report(run.progress, u64::try_from(index).unwrap_or(total), total)?;
        }
    }
    finish(&mut out, objects, &numbers, &entries)?;
    report(run.progress, total, total)?;
    Ok((broken, out.written))
}

/// Writes `bytes` as a hexadecimal string (ISO 32000-2:2020, 7.3.4.3).
fn hex(text: &mut Vec<u8>, bytes: &[u8]) {
    text.push(b'<');
    for &byte in bytes {
        for nibble in [byte.wrapping_shr(4), byte & 0x0F] {
            text.push(
                char::from_digit(u32::from(nibble), 16)
                    .and_then(|digit| u8::try_from(digit).ok())
                    .unwrap_or(b'0'),
            );
        }
    }
    text.push(b'>');
}

/// `value`'s low 40 bits.
fn pack(value: u64) -> [u8; 5] {
    [32_u32, 24, 16, 8, 0]
        .map(|shift| u8::try_from(value.wrapping_shr(shift) & 0xFF).unwrap_or_default())
}

/// Tells `progress` of `done` of `total`; [`ErrorCode::Cancelled`] when it cancels.
fn report(progress: Option<&dyn Progress>, done: u64, total: u64) -> Result<(), PdfError> {
    match progress {
        Some(found) if !found.update(done, total) => {
            Err(PdfError::new(ErrorCode::Cancelled, OPERATION))
        }
        Some(_) | None => Ok(()),
    }
}

/// The fully qualified names of the signed signature fields under `catalog`'s form, in order: a
/// field whose type, inherited, is `Sig` and that has a value (ISO 32000-2:2020, 12.7.4.1,
/// 12.7.5.5, 12.8).
fn signed(
    objects: Objects<'_>,
    overlay: &Overlay,
    catalog: &Object,
) -> Result<Vec<String>, PdfError> {
    let form = overlay.entry(objects, catalog, b"AcroForm")?;
    let mut found = Vec::new();
    if let Object::Array(fields) = overlay.entry(objects, &form, b"Fields")? {
        let mut seen = BTreeSet::new();
        for field in fields {
            visit(
                objects,
                overlay,
                (field, "", None),
                objects.depth_limit(),
                (&mut seen, &mut found),
            )?;
        }
    }
    Ok(found)
}

/// Object `number` of the source, the generation its entry gives; null when free or missing.
fn source(objects: Objects<'_>, number: u32) -> Result<Object, PdfError> {
    let generation = match objects.entry(number)? {
        Some(entry) if entry.kind == XrefKind::InUse => entry.generation,
        Some(entry) if entry.kind == XrefKind::Compressed => 0,
        Some(_) | None => return Ok(Object::Null),
    };
    objects.owned(ObjectId { generation, number })
}

/// `value`'s 40 bits.
fn unpack(slot: [u8; 5]) -> u64 {
    slot.iter()
        .fold(0, |value, &byte| value.wrapping_shl(8) | u64::from(byte))
}

/// Writes `object` in PDF syntax, references renumbered (ISO 32000-2:2020, 7.3).
fn value(text: &mut Vec<u8>, object: Object, numbers: &mut Numbers) -> Result<(), PdfError> {
    match object {
        Object::Array(items) => {
            text.push(b'[');
            for (at, item) in items.into_iter().enumerate() {
                if at > 0 {
                    text.push(b' ');
                }
                value(text, item, numbers)?;
            }
            text.push(b']');
        }
        Object::Boolean(flag) => text.extend_from_slice(if flag { b"true" } else { b"false" }),
        Object::Dictionary(entries) => {
            text.extend_from_slice(b"<<");
            for (key, item) in entries {
                text.push(b'/');
                name(text, &key);
                text.push(b' ');
                value(text, item, numbers)?;
            }
            text.extend_from_slice(b">>");
        }
        Object::Integer(number) => text.extend_from_slice(number.to_string().as_bytes()),
        Object::Name(bytes) => {
            text.push(b'/');
            name(text, &bytes);
        }
        Object::Null | Object::Stream(..) => text.extend_from_slice(b"null"),
        Object::Real(number) if number.is_finite() => {
            text.extend_from_slice(number.to_string().as_bytes());
        }
        Object::Real(_) => text.push(b'0'),
        Object::Reference(id) => match numbers.number(id)? {
            Some(number) => text.extend_from_slice(format!("{number} 0 R").as_bytes()),
            None => text.extend_from_slice(b"null"),
        },
        Object::String(bytes) => {
            if bytes.iter().all(|byte| (0x20..=0x7E).contains(byte)) {
                text.push(b'(');
                for byte in bytes {
                    if matches!(byte, b'\\' | b'(' | b')') {
                        text.push(b'\\');
                    }
                    text.push(byte);
                }
                text.push(b')');
            } else {
                hex(text, &bytes);
            }
        }
    }
    Ok(())
}

/// Visits the field `node`, under its parent's fully qualified name and inherited type, and its
/// kids, nesting at most `depth` deep, adding the signed ones to `found`; an object in `seen` is
/// not visited again.
fn visit(
    objects: Objects<'_>,
    overlay: &Overlay,
    node: (Object, &str, Option<Vec<u8>>),
    depth: u32,
    (seen, found): (&mut BTreeSet<u32>, &mut Vec<String>),
) -> Result<(), PdfError> {
    let (item, parent, inherited) = node;
    let below = depth
        .checked_sub(1)
        .ok_or_else(|| PdfError::new(ErrorCode::LimitExceeded, OPERATION))?;
    if let Object::Reference(id) = item
        && !seen.insert(id.number)
    {
        return Ok(());
    }
    let field = overlay.direct(objects, item)?;
    let partial = match overlay.entry(objects, &field, b"T")? {
        Object::String(bytes) => text(&bytes),
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Null
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..) => String::new(),
    };
    let full = match (parent.is_empty(), partial.is_empty()) {
        (true, _) => partial,
        (false, true) => String::from(parent),
        (false, false) => format!("{parent}.{partial}"),
    };
    let kind = match field.get(b"FT") {
        Some(Object::Name(own)) => Some(own),
        Some(_) | None => inherited,
    };
    if kind.as_deref() == Some(b"Sig".as_slice())
        && field
            .get(b"V")
            .is_some_and(|signature| !matches!(signature, Object::Null))
    {
        found.push(full.clone());
    }
    if let Object::Array(kids) = overlay.entry(objects, &field, b"Kids")? {
        for kid in kids {
            visit(
                objects,
                overlay,
                (kid, &full, kind.clone()),
                below,
                (&mut *seen, &mut *found),
            )?;
        }
    }
    Ok(())
}

/// Writes object `number`: `object`, and a stream's data with its `Length`
/// (ISO 32000-2:2020, 7.3.8, 7.3.10).
fn write(
    out: &mut Out<'_>,
    objects: Objects<'_>,
    numbers: &mut Numbers,
    number: usize,
    object: Object,
) -> Result<(), PdfError> {
    let mut text = format!("{number} 0 obj\n").into_bytes();
    let Object::Stream(mut entries, data) = object else {
        value(&mut text, object, numbers)?;
        text.extend_from_slice(b"\nendobj\n");
        return out.put(&text);
    };
    entries.retain(|entry| entry.0 != b"Length");
    let malformed = || PdfError::new(ErrorCode::Malformed, OPERATION);
    let mut head = |target: &mut Out<'_>, length: u64| {
        entries.push((
            b"Length".to_vec(),
            Object::Integer(i64::try_from(length).map_err(|_error| malformed())?),
        ));
        value(&mut text, Object::Dictionary(entries.clone()), numbers)?;
        text.extend_from_slice(b"\nstream\n");
        target.put(&text)
    };
    let mut given = 0_u64;
    let length = match data {
        Data::Bytes(bytes) => {
            let length = u64::try_from(bytes.len()).map_err(|_error| malformed())?;
            head(out, length)?;
            out.put(&bytes)?;
            given = length;
            length
        }
        Data::Produced(producer) => {
            let length = producer.length();
            head(out, length)?;
            producer.write(&mut |bytes| {
                given = given.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                out.put(bytes)
            })?;
            length
        }
        Data::Stored(id) => {
            let (mut stored, known) = stored_data(objects, id)?.ok_or_else(malformed)?;
            let length = if let Some(found) = known {
                found
            } else {
                let counted = count(&mut *stored)?;
                stored = stored_data(objects, id)?.ok_or_else(malformed)?.0;
                counted
            };
            head(out, length)?;
            let mut buffer = vec![0; CHUNK];
            loop {
                let pulled = stored.pull(&mut buffer)?;
                let Some(bytes) = buffer.get(..pulled).filter(|bytes| !bytes.is_empty()) else {
                    break;
                };
                given = given.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                out.put(bytes)?;
            }
            length
        }
    };
    if given != length {
        return Err(malformed());
    }
    out.put(b"\nendstream\nendobj\n")
}

/// The bytes left in `data`, read through.
fn count(data: &mut dyn Pull) -> Result<u64, PdfError> {
    let mut buffer = vec![0; CHUNK];
    let mut total = 0_u64;
    loop {
        match data.pull(&mut buffer)? {
            0 => return Ok(total),
            pulled => total = total.saturating_add(u64::try_from(pulled).unwrap_or(u64::MAX)),
        }
    }
}

/// Writes name `bytes` after its `/`: `#xx` for bytes outside `!` to `~`, delimiters and `#`
/// (ISO 32000-2:2020, 7.3.5).
fn name(text: &mut Vec<u8>, bytes: &[u8]) {
    for &byte in bytes {
        if byte.is_ascii_graphic() && is_regular(byte) && byte != b'#' {
            text.push(byte);
        } else {
            text.push(b'#');
            for nibble in [byte.wrapping_shr(4), byte & 0x0F] {
                text.push(
                    char::from_digit(u32::from(nibble), 16)
                        .map(|digit| digit.to_ascii_uppercase())
                        .and_then(|digit| u8::try_from(digit).ok())
                        .unwrap_or(b'0'),
                );
            }
        }
    }
}
