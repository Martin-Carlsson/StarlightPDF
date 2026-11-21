//! The standard security handler: authenticating a password and decrypting strings and streams
//! (ISO 32000-2:2020, 7.6.2 to 7.6.4).

use core::fmt::{self, Debug, Formatter};
use core::slice;

use crate::crypto::aes::Aes;
use crate::crypto::md5::Md5;
use crate::crypto::rc4::Rc4;
use crate::crypto::sha2::{sha256, sha384, sha512};
use crate::objects::ObjectId;
use crate::pull::Pull;
use crate::security::{Cipher, Encryption, Permission};
use crate::store::Objects;
use crate::syntax::{At, Value};
use crate::{ErrorCode, PdfError};

/// The AES block size.
const BLOCK: usize = 16;

/// The padding string (ISO 32000-2:2020, 7.6.4.3.2, Algorithm 2 step a).
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// Bytes of a revision 5 or 6 password used (ISO 32000-2:2020, 7.6.4.3.3).
const PASSWORD_LIMIT: usize = 127;

/// The characters of `PDFDocEncoding` codes 0x18 to 0x1F and 0x80 to 0xA0 other than Latin-1's
/// (ISO 32000-2:2020, Annex D, Table D.2).
const PDF_DOC: [(char, u8); 40] = [
    ('\u{2D8}', 0x18),
    ('\u{2C7}', 0x19),
    ('\u{2C6}', 0x1A),
    ('\u{2D9}', 0x1B),
    ('\u{2DD}', 0x1C),
    ('\u{2DB}', 0x1D),
    ('\u{2DA}', 0x1E),
    ('\u{2DC}', 0x1F),
    ('\u{2022}', 0x80),
    ('\u{2020}', 0x81),
    ('\u{2021}', 0x82),
    ('\u{2026}', 0x83),
    ('\u{2014}', 0x84),
    ('\u{2013}', 0x85),
    ('\u{192}', 0x86),
    ('\u{2044}', 0x87),
    ('\u{2039}', 0x88),
    ('\u{203A}', 0x89),
    ('\u{2212}', 0x8A),
    ('\u{2030}', 0x8B),
    ('\u{201E}', 0x8C),
    ('\u{201C}', 0x8D),
    ('\u{201D}', 0x8E),
    ('\u{2018}', 0x8F),
    ('\u{2019}', 0x90),
    ('\u{201A}', 0x91),
    ('\u{2122}', 0x92),
    ('\u{FB01}', 0x93),
    ('\u{FB02}', 0x94),
    ('\u{141}', 0x95),
    ('\u{152}', 0x96),
    ('\u{160}', 0x97),
    ('\u{178}', 0x98),
    ('\u{17D}', 0x99),
    ('\u{131}', 0x9A),
    ('\u{142}', 0x9B),
    ('\u{153}', 0x9C),
    ('\u{161}', 0x9D),
    ('\u{17E}', 0x9E),
    ('\u{20AC}', 0xA0),
];

/// The permissions, in declaration order.
const PERMISSIONS: [Permission; 7] = [
    Permission::Annotate,
    Permission::Assemble,
    Permission::Copy,
    Permission::Edit,
    Permission::FillForms,
    Permission::Print,
    Permission::PrintHighQuality,
];

/// AES-CBC data decrypted a block at a time, the IV first and the last block's padding removed
/// (ISO 32000-2:2020, 7.6.3.1).
struct AesData<'data> {
    /// The cipher.
    aes: Aes,
    /// The ciphertext.
    data: Box<dyn Pull + 'data>,
    /// The end of the bytes in `ready`.
    end: usize,
    /// The last block decrypted, released once another follows.
    held: Option<[u8; BLOCK]>,
    /// The previous ciphertext block; none before the IV is read.
    previous: Option<[u8; BLOCK]>,
    /// Plaintext ready.
    ready: [u8; BLOCK],
    /// The next byte in `ready`.
    start: usize,
}

/// The entries of an `Encrypt` dictionary that authenticate a password, and the file identifier.
struct Entries {
    /// The first element of the trailer's `ID`.
    id: Vec<u8>,
    /// Bytes of the file encryption key, revisions 2 to 4.
    length: usize,
    /// `EncryptMetadata`.
    metadata: bool,
    /// `O`.
    owner: Vec<u8>,
    /// `OE`.
    owner_key: Vec<u8>,
    /// `P` as 32 bits.
    permissions: u32,
    /// `R`.
    revision: i64,
    /// `U`.
    user: Vec<u8>,
    /// `UE`.
    user_key: Vec<u8>,
}

/// A document's standard security handler, authenticated.
pub(crate) struct Handler {
    /// The `Encrypt` dictionary's id, when indirect; its strings are not encrypted.
    dictionary: Option<ObjectId>,
    /// The method of embedded file streams.
    embedded: Method,
    /// What is reported.
    encryption: Encryption,
    /// The `CF` dictionary.
    filters: Option<At>,
    /// The file encryption key.
    key: Vec<u8>,
    /// Whether metadata streams are encrypted.
    metadata: bool,
    /// The method of streams.
    stream: Method,
    /// The method of strings.
    string: Method,
}

/// How a crypt filter decrypts (ISO 32000-2:2020, 7.6.6, Table 25).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Method {
    /// AES-128 (`AESV2`).
    Aes128,
    /// AES-256 (`AESV3`).
    Aes256,
    /// None.
    Identity,
    /// RC4 (`V2`).
    Rc4,
}

/// RC4 data decrypted.
struct Rc4Data<'data> {
    /// The ciphertext.
    data: Box<dyn Pull + 'data>,
    /// The keystream.
    rc4: Rc4,
}

impl<'data> AesData<'data> {
    /// Decrypts `data` with `aes`.
    const fn new(aes: Aes, data: Box<dyn Pull + 'data>) -> Self {
        Self {
            aes,
            data,
            end: 0,
            held: None,
            previous: None,
            ready: [0; BLOCK],
            start: 0,
        }
    }

    /// Readies the next plaintext; false at the end. Invalid padding is kept.
    fn refill(&mut self) -> Result<bool, PdfError> {
        loop {
            let mut block = [0; BLOCK];
            if !read_block(&mut *self.data, &mut block)? {
                let Some(last) = self.held.take() else {
                    return Ok(false);
                };
                let pad = usize::from(last.last().copied().unwrap_or_default());
                self.ready = last;
                self.start = 0;
                self.end = if (1..=BLOCK).contains(&pad) {
                    BLOCK.saturating_sub(pad)
                } else {
                    BLOCK
                };
                return Ok(self.end > 0);
            }
            let Some(previous) = self.previous.as_mut() else {
                self.previous = Some(block);
                continue;
            };
            self.aes.decrypt_cbc(previous, slice::from_mut(&mut block));
            if let Some(released) = self.held.replace(block) {
                self.ready = released;
                self.start = 0;
                self.end = BLOCK;
                return Ok(true);
            }
        }
    }
}

impl Entries {
    /// Whether `key` gives `U` (ISO 32000-2:2020, 7.6.4.4, Algorithms 4, 5 and 6).
    fn authenticates(&self, key: &[u8]) -> bool {
        if self.revision == 2 {
            let mut value = PAD;
            apply(key, &mut value);
            return self.user.get(..32) == Some(value.as_slice());
        }
        let mut md5 = Md5::default();
        md5.update(&PAD);
        md5.update(&self.id);
        let mut value = md5.finish();
        for round in 0..=19 {
            apply(&xored(key, round), &mut value);
        }
        self.user.get(..BLOCK) == Some(value.as_slice())
    }

    /// The file encryption key from a padded password (ISO 32000-2:2020, 7.6.4.3.2,
    /// Algorithm 2).
    fn file_key(&self, padded: &[u8; 32]) -> Vec<u8> {
        let mut md5 = Md5::default();
        md5.update(padded);
        md5.update(self.owner.get(..32).unwrap_or(&self.owner));
        md5.update(&low_first(self.permissions));
        md5.update(&self.id);
        if self.revision >= 4 && !self.metadata {
            md5.update(&[0xFF; 4]);
        }
        let mut hash = md5.finish();
        if self.revision >= 3 {
            for _ in 0..50_u8 {
                hash = digest(hash.get(..self.length).unwrap_or_default());
            }
        }
        hash.get(..self.length).unwrap_or_default().to_vec()
    }

    /// The hash of a revision 5 or 6 password with `salt` and `user` (ISO 32000-2:2020,
    /// 7.6.4.3.4, Algorithm 2.B; revision 5 hashes once).
    fn hash(&self, password: &[u8], salt: &[u8], user: &[u8]) -> Vec<u8> {
        let mut hash = sha256(&[password, salt, user].concat()).to_vec();
        if self.revision == 6 {
            let mut round = 0_u32;
            loop {
                let mut data = [password, &hash, user].concat().repeat(64);
                encrypt_cbc(&hash, &mut data);
                let sum = data
                    .get(..BLOCK)
                    .unwrap_or_default()
                    .iter()
                    .fold(0_u32, |total, &byte| total.wrapping_add(u32::from(byte)));
                hash = match sum.checked_rem(3) {
                    Some(0) => sha256(&data).to_vec(),
                    Some(1) => sha384(&data).to_vec(),
                    Some(_) | None => sha512(&data).to_vec(),
                };
                round = round.wrapping_add(1);
                if round >= 64
                    && data
                        .last()
                        .is_some_and(|&last| u32::from(last) <= round.wrapping_sub(32))
                {
                    break;
                }
            }
        }
        hash.truncate(32);
        hash
    }

    /// The file encryption key for `password`, user or owner, revisions 2 to 4
    /// (ISO 32000-2:2020, 7.6.4.4, Algorithms 6 and 7).
    fn legacy_key(&self, password: &[u8]) -> Option<Vec<u8>> {
        let user = self.file_key(&padded(password));
        if self.authenticates(&user) {
            return Some(user);
        }
        let owner = self.file_key(&self.owner_to_user(password));
        self.authenticates(&owner).then_some(owner)
    }

    /// The file encryption key for `password`, owner or user, revisions 5 and 6
    /// (ISO 32000-2:2020, 7.6.4.3.3, Algorithm 2.A).
    fn modern_key(&self, password: &[u8]) -> Option<Vec<u8>> {
        let truncated = password.get(..PASSWORD_LIMIT).unwrap_or(password);
        let user = self.user.get(..48)?;
        if let Some((owner_hash, salts)) = self
            .owner
            .get(..48)
            .and_then(|owner| owner.split_at_checked(32))
            && let Some((validation, key_salt)) = salts.split_at_checked(8)
            && self.hash(truncated, validation, user) == owner_hash
        {
            return unwrap_key(&self.hash(truncated, key_salt, user), &self.owner_key);
        }
        let (user_hash, user_salts) = user.split_at_checked(32)?;
        if self.hash(truncated, user_salts.get(..8)?, &[]) == user_hash {
            let key = self.hash(truncated, user_salts.get(8..BLOCK)?, &[]);
            return unwrap_key(&key, &self.user_key);
        }
        None
    }

    /// The padded user password `O` holds for owner password `password` (ISO 32000-2:2020,
    /// 7.6.4.4, Algorithm 7 with Algorithm 3 steps a to d).
    fn owner_to_user(&self, password: &[u8]) -> [u8; 32] {
        let mut hash = digest(&padded(password));
        if self.revision >= 3 {
            for _ in 0..50_u8 {
                hash = digest(&hash);
            }
        }
        let key = hash.get(..self.length).unwrap_or_default();
        let mut value = [0; 32];
        for (slot, byte) in value.iter_mut().zip(&self.owner) {
            *slot = *byte;
        }
        if self.revision == 2 {
            apply(key, &mut value);
        } else {
            for round in (0..=19).rev() {
                apply(&xored(key, round), &mut value);
            }
        }
        value
    }
}

impl Debug for Handler {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handler").finish_non_exhaustive()
    }
}

impl Handler {
    /// `data` of object `id` decrypted with `method` (ISO 32000-2:2020, 7.6.3.2 and 7.6.3.3,
    /// Algorithms 1 and 1.A).
    fn decrypt<'data>(
        &self,
        id: ObjectId,
        method: Method,
        data: Box<dyn Pull + 'data>,
        operation: &'static str,
    ) -> Result<Box<dyn Pull + 'data>, PdfError> {
        let malformed = || PdfError::new(ErrorCode::Malformed, operation);
        Ok(match method {
            Method::Identity => data,
            Method::Rc4 => Box::new(Rc4Data {
                data,
                rc4: Rc4::new(&self.object_key(id, false)).ok_or_else(malformed)?,
            }),
            Method::Aes128 => Box::new(AesData::new(
                Aes::new(&self.object_key(id, true)).ok_or_else(malformed)?,
                data,
            )),
            Method::Aes256 => Box::new(AesData::new(
                Aes::new(&self.key).ok_or_else(malformed)?,
                data,
            )),
        })
    }

    /// How the document is protected.
    pub(crate) fn encryption(&self) -> Encryption {
        self.encryption.clone()
    }

    /// Whether the strings of object `id` are encrypted: all but the `Encrypt` dictionary's.
    pub(crate) fn encrypts(&self, id: ObjectId) -> bool {
        self.dictionary != Some(id)
    }

    /// The key of object `id`, salted for AES (ISO 32000-2:2020, 7.6.3.2, Algorithm 1).
    fn object_key(&self, id: ObjectId, aes: bool) -> Vec<u8> {
        let mut md5 = Md5::default();
        md5.update(&self.key);
        md5.update(low_first(id.number).get(..3).unwrap_or_default());
        md5.update(
            low_first(u32::from(id.generation))
                .get(..2)
                .unwrap_or_default(),
        );
        if aes {
            md5.update(b"sAlT");
        }
        let hash = md5.finish();
        hash.get(..self.key.len().saturating_add(5).min(BLOCK))
            .unwrap_or_default()
            .to_vec()
    }

    /// Stream `id`'s stored data, dictionary at `dictionary`, decrypted with crypt filter `crypt`,
    /// else `EFF` for an embedded file, else `StmF`; cross-reference streams, and metadata ones
    /// if `EncryptMetadata` is false, are not (ISO 32000-2:2020, 7.5.8.2, 7.6.2, 7.6.6).
    pub(crate) fn stream<'data>(
        &self,
        objects: Objects<'_>,
        id: ObjectId,
        dictionary: At,
        crypt: Option<&[u8]>,
        data: Box<dyn Pull + 'data>,
    ) -> Result<Box<dyn Pull + 'data>, PdfError> {
        let kind = objects.get(dictionary, b"Type")?;
        let is = |name: &[u8]| kind.as_ref().is_some_and(|found| found.is_name(name));
        if is(b"XRef") || (!self.metadata && is(b"Metadata")) {
            return Ok(data);
        }
        let used = match crypt {
            Some(name) => method(objects, self.filters, name)?,
            None if is(b"EmbeddedFile") => self.embedded,
            None => self.stream,
        };
        self.decrypt(id, used, data, objects.operation())
    }

    /// String `bytes` of object `id`, decrypted with `StrF` (ISO 32000-2:2020, 7.6.3.1).
    pub(crate) fn string(
        &self,
        id: ObjectId,
        bytes: &[u8],
        operation: &'static str,
    ) -> Result<Vec<u8>, PdfError> {
        let mut data = self.decrypt(id, self.string, Box::new(bytes), operation)?;
        let mut plain = Vec::new();
        let mut buffer = [0; 256];
        loop {
            let count = data.pull(&mut buffer)?;
            let Some(part) = buffer.get(..count).filter(|part| !part.is_empty()) else {
                return Ok(plain);
            };
            plain.extend_from_slice(part);
        }
    }
}

impl Pull for AesData<'_> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let mut written: usize = 0;
        while let Some(rest) = buf.get_mut(written..).filter(|rest| !rest.is_empty()) {
            if self.start >= self.end && !self.refill()? {
                break;
            }
            let ready = self.ready.get(self.start..self.end).unwrap_or_default();
            let count = ready.len().min(rest.len());
            for (slot, byte) in rest.iter_mut().zip(ready) {
                *slot = *byte;
            }
            self.start = self.start.saturating_add(count);
            written = written.saturating_add(count);
        }
        Ok(written)
    }
}

impl Pull for Rc4Data<'_> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let count = self.data.pull(buf)?;
        self.rc4.apply(buf.get_mut(..count).unwrap_or_default());
        Ok(count)
    }
}

/// Whether `P` value `bits` grants `permission` in `revision` (ISO 32000-2:2020, Table 22).
fn allows(bits: u32, revision: i64, permission: Permission) -> bool {
    let set = |bit: u32| bits & 1_u32.wrapping_shl(bit.wrapping_sub(1)) != 0;
    let modern = revision >= 3;
    match permission {
        Permission::Annotate => set(6),
        Permission::Assemble => set(if modern { 11 } else { 4 }),
        Permission::Copy => set(5),
        Permission::Edit => set(4),
        Permission::FillForms => set(6) || (modern && set(9)),
        Permission::Print => set(3),
        Permission::PrintHighQuality => set(3) && (!modern || set(12)),
    }
}

/// XORs the RC4 keystream of `key` into `data`.
fn apply(key: &[u8], data: &mut [u8]) {
    if let Some(mut rc4) = Rc4::new(key) {
        rc4.apply(data);
    }
}

/// The cipher of the first of `methods` that encrypts.
fn cipher(methods: [Method; 2]) -> Cipher {
    match methods.into_iter().find(|used| *used != Method::Identity) {
        Some(Method::Aes128) => Cipher::Aes128,
        Some(Method::Aes256) => Cipher::Aes256,
        Some(Method::Rc4) => Cipher::Rc4,
        Some(Method::Identity) | None => Cipher::Unencrypted,
    }
}

/// The MD5 digest of `bytes`.
fn digest(bytes: &[u8]) -> [u8; 16] {
    let mut md5 = Md5::default();
    md5.update(bytes);
    md5.finish()
}

/// Encrypts `data` in place with AES-128-CBC, unpadded, the key and IV the first 32 bytes of
/// `key_iv` (ISO 32000-2:2020, 7.6.4.3.4, Algorithm 2.B step b).
fn encrypt_cbc(key_iv: &[u8], data: &mut [u8]) {
    let (Some(aes), Some(iv)) = (
        key_iv.get(..BLOCK).and_then(Aes::new),
        key_iv
            .get(BLOCK..)
            .and_then(|rest| rest.get(..BLOCK))
            .and_then(|iv| <[u8; BLOCK]>::try_from(iv).ok()),
    ) else {
        return;
    };
    aes.encrypt_cbc(&iv, data);
}

/// The first element of the file identifier in the trailer at `trailer`; empty when missing
/// (ISO 32000-2:2020, 14.4).
fn file_id(objects: Objects<'_>, trailer: At) -> Result<Vec<u8>, PdfError> {
    let Some(Value::Array(at)) = objects.get(trailer, b"ID")? else {
        return Ok(Vec::new());
    };
    let mut reader = objects.reader(at)?;
    reader.set_keep(true);
    drop(reader.token()?);
    Ok(match reader.head(objects.depth_limit())? {
        Value::String(bytes) => bytes,
        Value::Array(_)
        | Value::Boolean(_)
        | Value::Dictionary(_)
        | Value::Integer(_)
        | Value::Name(_)
        | Value::Null
        | Value::Real(_)
        | Value::Reference(_) => Vec::new(),
    })
}

/// `value`'s bytes, least significant first.
fn low_first(value: u32) -> [u8; 4] {
    let mut bytes = [0; 4];
    let mut rest = value;
    for byte in &mut bytes {
        *byte = u8::try_from(rest & 0xFF).unwrap_or_default();
        rest >>= 8_u8;
    }
    bytes
}

/// The method of the crypt filter named `name` in `filters`; one the library cannot use is
/// [`ErrorCode::Unsupported`] (ISO 32000-2:2020, 7.6.6).
fn method(objects: Objects<'_>, filters: Option<At>, name: &[u8]) -> Result<Method, PdfError> {
    if name == b"Identity" {
        return Ok(Method::Identity);
    }
    let found = filters.map(|at| objects.get(at, name)).transpose()?;
    let filter = objects.dictionary(found.flatten())?;
    if let Some(Value::Name(cfm)) = objects.get(filter, b"CFM")? {
        match cfm.as_slice() {
            b"V2" => return Ok(Method::Rc4),
            b"AESV2" => return Ok(Method::Aes128),
            b"AESV3" => return Ok(Method::Aes256),
            _ => {}
        }
    }
    Err(PdfError::new(ErrorCode::Unsupported, objects.operation()))
}

/// The standard security handler of the document whose trailer is at `trailer`, authenticated
/// with `password`, user or owner; none when unencrypted. Another handler is
/// [`ErrorCode::Unsupported`], a wrong password [`ErrorCode::InvalidPassword`]
/// (ISO 32000-2:2020, 7.6.2 to 7.6.4).
pub(crate) fn open(
    objects: Objects<'_>,
    trailer: At,
    password: &str,
) -> Result<Option<Handler>, PdfError> {
    let Some(entry) = objects.get(trailer, b"Encrypt")? else {
        return Ok(None);
    };
    let operation = objects.operation();
    let unsupported = || PdfError::new(ErrorCode::Unsupported, operation);
    let dictionary = entry.reference();
    let encrypt = objects.dictionary(Some(entry))?;
    if !objects
        .get(encrypt, b"Filter")?
        .is_some_and(|filter| filter.is_name(b"Standard"))
    {
        return Err(unsupported());
    }
    let integer = |key: &[u8]| -> Result<Option<i64>, PdfError> {
        Ok(objects.get(encrypt, key)?.as_ref().and_then(Value::integer))
    };
    let string = |key: &[u8]| -> Result<Vec<u8>, PdfError> {
        Ok(match objects.get(encrypt, key)? {
            Some(Value::String(bytes)) => bytes,
            Some(_) | None => Vec::new(),
        })
    };
    let version = integer(b"V")?.unwrap_or_default();
    let filters = objects
        .get(encrypt, b"CF")?
        .map(|found| objects.dictionary(Some(found)))
        .transpose()?;
    let named = |key: &[u8], default: Method| -> Result<Method, PdfError> {
        match objects.get(encrypt, key)? {
            Some(Value::Name(name)) => method(objects, filters, &name),
            Some(_) | None => Ok(default),
        }
    };
    let (stream, strings, embedded) = match version {
        1 | 2 => (Method::Rc4, Method::Rc4, Method::Rc4),
        4 | 5 => {
            let stream = named(b"StmF", Method::Identity)?;
            (
                stream,
                named(b"StrF", Method::Identity)?,
                named(b"EFF", stream)?,
            )
        }
        _ => return Err(unsupported()),
    };
    let revision = integer(b"R")?.unwrap_or_default();
    let bits = integer(b"Length")?.unwrap_or(if version == 4 { 128 } else { 40 });
    let length = match revision {
        2 => 5,
        3 | 4 if (40..=128).contains(&bits) && bits.trailing_zeros() >= 3 => {
            usize::try_from(bits >> 3_u8).unwrap_or_default()
        }
        3 | 4 => return Err(objects.malformed()),
        _ => 0,
    };
    let entries = Entries {
        id: file_id(objects, trailer)?,
        length,
        metadata: version < 4
            || !matches!(
                objects.get(encrypt, b"EncryptMetadata")?,
                Some(Value::Boolean(false))
            ),
        owner: string(b"O")?,
        owner_key: string(b"OE")?,
        permissions: u32::try_from(integer(b"P")?.unwrap_or_default() & 0xFFFF_FFFF)
            .unwrap_or_default(),
        revision,
        user: string(b"U")?,
        user_key: string(b"UE")?,
    };
    let key = match revision {
        2..=4 => pdf_doc(password).and_then(|bytes| entries.legacy_key(&bytes)),
        5 | 6 => entries.modern_key(password.as_bytes()),
        _ => return Err(unsupported()),
    }
    .ok_or_else(|| PdfError::new(ErrorCode::InvalidPassword, operation))?;
    let restrictions = PERMISSIONS
        .into_iter()
        .filter(|&permission| !allows(entries.permissions, revision, permission))
        .collect();
    Ok(Some(Handler {
        dictionary,
        encryption: Encryption {
            certificate: false,
            cipher: cipher([stream, strings]),
            restrictions,
        },
        embedded,
        filters,
        key,
        metadata: entries.metadata,
        stream,
        string: strings,
    }))
}

/// `password` padded or cut to 32 bytes (ISO 32000-2:2020, 7.6.4.3.2, Algorithm 2 step a).
fn padded(password: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    for (slot, byte) in out.iter_mut().zip(password.iter().chain(&PAD)) {
        *slot = *byte;
    }
    out
}

/// `password` in `PDFDocEncoding`; none if a character has no code (ISO 32000-2:2020, Annex D).
fn pdf_doc(password: &str) -> Option<Vec<u8>> {
    password
        .chars()
        .map(|character| match u32::from(character) {
            code @ (0x09 | 0x0A | 0x0D | 0x20..=0x7E | 0xA1..=0xAC | 0xAE..=0xFF) => {
                u8::try_from(code).ok()
            }
            _ => PDF_DOC
                .iter()
                .find(|&&(found, _)| found == character)
                .map(|&(_, byte)| byte),
        })
        .collect()
}

/// Fills `block` from `data`; whether it filled.
fn read_block(data: &mut dyn Pull, block: &mut [u8; BLOCK]) -> Result<bool, PdfError> {
    let mut filled: usize = 0;
    while let Some(rest) = block.get_mut(filled..).filter(|rest| !rest.is_empty()) {
        match data.pull(rest)? {
            0 => return Ok(false),
            count => filled = filled.saturating_add(count),
        }
    }
    Ok(true)
}

/// The file encryption key `wrapped` (`OE` or `UE`) holds, decrypted with `key` by AES-256-CBC
/// with a zero IV and no padding (ISO 32000-2:2020, 7.6.4.3.3, Algorithm 2.A steps d and e).
fn unwrap_key(key: &[u8], wrapped: &[u8]) -> Option<Vec<u8>> {
    let aes = Aes::new(key)?;
    let mut plain = wrapped.get(..32)?.to_vec();
    aes.decrypt_cbc(&mut [0; BLOCK], plain.as_chunks_mut::<BLOCK>().0);
    Some(plain)
}

/// `key`, each byte combined with `round` by exclusive or.
fn xored(key: &[u8], round: u8) -> Vec<u8> {
    key.iter().map(|byte| byte ^ round).collect()
}
