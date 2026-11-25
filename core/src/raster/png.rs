//! PNG files (ISO/IEC 15948:2004): IDAT data kept as `FlateDecode` with PNG predictors, or decoded
//! to unfiltered samples and a soft mask (ISO 32000-2:2020, 7.4.4.4, 8.9.3, 11.6.5.3).

use super::{Cursor, Image, Profile, READ, STEP, exif, malformed};
use crate::inflate::Inflate;
use crate::input::Source;
use crate::predictor::Predictor;
use crate::pull::{Held, Pull, fill, number};
use crate::{ErrorCode, PdfError};

/// The CRC-32 of each byte (Annex D), made once.
const CRCS: [u32; 256] = {
    let mut table = [0_u32; 256];
    let mut rest: &mut [u32] = &mut table;
    let mut index = 0_u32;
    loop {
        let current = rest;
        let Some((slot, tail)) = current.split_first_mut() else {
            break;
        };
        *slot = crc_of(index);
        rest = tail;
        index = index.wrapping_add(1);
    }
    table
};

/// Bytes of an eXIf chunk searched for the orientation; Exif data puts its first directory early.
const EXIF_HEAD: usize = 0x1_0000;

/// The largest chunk length and dimension (5.3, 11.2.2).
const MAX: u32 = 0x7FFF_FFFF;

/// The most pixels a row may hold: a predictor's most columns, so a drawn image reads back.
const MAX_WIDTH: u32 = 0x1_0000;

/// Adam7 passes (8.2): first column, first row, and the log2 of their column and row steps.
const PASSES: [[u32; 4]; 7] = [
    [0, 0, 3, 3],
    [4, 0, 3, 3],
    [0, 4, 2, 3],
    [2, 0, 2, 2],
    [0, 2, 1, 2],
    [1, 0, 1, 1],
    [0, 1, 0, 1],
];

/// The signature (5.2).
pub(super) const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

/// What the chunks after IHDR hold.
#[derive(Default)]
struct Chunks {
    /// Another chunk followed the IDAT chunks.
    ended: bool,
    /// The first IDAT chunk's offset.
    idat: Option<u64>,
    /// Bytes of IDAT data.
    idat_length: u64,
    /// The eXIf orientation.
    orientation: Option<u8>,
    /// The PLTE data.
    palette: Option<Vec<u8>>,
    /// The iCCP profile.
    profile: Option<Profile>,
    /// The tRNS data.
    transparency: Option<Vec<u8>>,
}

/// The IDAT data, read chunk by chunk.
pub(super) struct Idat {
    /// Bytes left in the current chunk.
    left: u64,
    /// The offset of the next chunk.
    next: u64,
    /// The offset of the next byte.
    position: u64,
    /// The source.
    source: Source,
}

/// Takes a decoded row of a pass, with its index in the pass.
type EachRow<'each> = dyn FnMut(Pass, u32, &[u8]) -> Result<(), PdfError> + 'each;

/// A pass of the image's rows.
#[derive(Clone, Copy)]
struct Pass {
    /// The first column.
    column: u32,
    /// The log2 of the column step.
    column_shift: u32,
    /// Rows.
    height: u32,
    /// The first row.
    row: u32,
    /// The log2 of the row step.
    row_shift: u32,
    /// Columns.
    width: u32,
}

/// A PNG's layout.
#[derive(Clone, Debug)]
pub(super) struct Png {
    /// The tRNS alpha per palette entry (11.3.2.1).
    alpha: Option<Vec<u8>>,
    /// The bit depth.
    bits: u8,
    /// The colour type.
    color: u8,
    /// Written decoded, not as the IDAT data.
    decoded: bool,
    /// Rows.
    height: u32,
    /// The offset of the first IDAT chunk.
    idat: u64,
    /// Adam7 interlaced.
    interlaced: bool,
    /// The tRNS colour key: grey, or red, green and blue (11.3.2.1).
    key: Option<[u16; 3]>,
    /// Columns.
    width: u32,
}

/// An image's colour samples or soft mask, unfiltered, read in order.
pub(super) struct Rows {
    /// The row being served.
    converted: Vec<u8>,
    /// The unfiltered data; none before the first pull.
    data: Option<Box<dyn Pull>>,
    /// Rows left.
    left: u32,
    /// The soft mask, not the colour samples.
    mask: bool,
    /// The layout.
    png: Png,
    /// The row as stored, unfiltered.
    raw: Vec<u8>,
    /// How much of `converted` is served.
    served: usize,
    /// The file.
    source: Source,
}

impl Pull for Idat {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        while self.left == 0 {
            let mut header = [0; 8];
            let count = self.source.read(self.next, &mut header, READ)?;
            if header.get(4..count) != Some(b"IDAT") {
                return Ok(0);
            }
            self.left = u64::from(number(header.get(..4).unwrap_or_default()));
            self.position = self.next.saturating_add(8);
            self.next = self.position.saturating_add(self.left).saturating_add(4);
        }
        let wanted = usize::try_from(self.left)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let Some(part) = buf.get_mut(..wanted).filter(|part| !part.is_empty()) else {
            return Ok(0);
        };
        let count = self.source.read(self.position, part, READ)?;
        let step = u64::try_from(count).unwrap_or(0);
        self.position = self.position.saturating_add(step);
        self.left = self.left.saturating_sub(step);
        Ok(count)
    }
}

impl Pull for Rows {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        if self.served >= self.converted.len() && !self.fetch()? {
            return Ok(0);
        }
        let mut rest = self.converted.get(self.served..).unwrap_or_default();
        let count = rest.pull(buf)?;
        self.served = self.served.saturating_add(count);
        Ok(count)
    }
}

impl Chunks {
    /// Takes chunk `kind` at `at`, of `length` bytes, its first bytes `data`, of the image `png`;
    /// `false` at IEND.
    fn take(
        &mut self,
        png: &Png,
        kind: &[u8],
        at: u64,
        length: u32,
        data: Vec<u8>,
    ) -> Result<bool, PdfError> {
        let fits = usize::try_from(length).is_ok_and(|size| size <= keep(kind));
        let before_data = self.idat.is_none();
        self.ended |= !before_data && kind != b"IDAT";
        match kind {
            b"IDAT" => {
                if self.ended || self.palette.is_none() && png.color == 3 {
                    return Err(malformed());
                }
                let _: &mut u64 = self.idat.get_or_insert(at);
                self.idat_length = self.idat_length.saturating_add(u64::from(length));
            }
            b"IEND" => return Ok(false),
            b"PLTE" => {
                let (entries, rest) = data.as_chunks::<3>();
                if !fits
                    || !before_data
                    || self.palette.is_some()
                    || self.transparency.is_some()
                    || matches!(png.color, 0 | 4)
                    || !rest.is_empty()
                    || entries.is_empty()
                    || png.color == 3
                        && 1_usize
                            .checked_shl(u32::from(png.bits))
                            .is_some_and(|most| entries.len() > most)
                {
                    return Err(malformed());
                }
                self.palette = Some(data);
            }
            b"eXIf" => {
                let _: &mut u8 = self
                    .orientation
                    .get_or_insert_with(|| exif::orientation(&data));
            }
            b"iCCP" => {
                let name = data
                    .iter()
                    .position(|&byte| byte == 0)
                    .ok_or_else(malformed)?;
                if !before_data
                    || self.palette.is_some()
                    || self.profile.is_some()
                    || !(1..=79).contains(&name)
                    || data.get(name.wrapping_add(1)) != Some(&0)
                {
                    return Err(malformed());
                }
                let skip = u64::try_from(name.wrapping_add(2)).unwrap_or(u64::MAX);
                let stored = u64::from(length).saturating_sub(skip);
                self.profile = Some(Profile {
                    compressed: true,
                    length: stored,
                    parts: vec![(at.saturating_add(8).saturating_add(skip), stored)],
                });
            }
            b"tRNS" => {
                let allowed = match png.color {
                    0 => data.len() == 2,
                    2 => data.len() == 6,
                    3 => self
                        .palette
                        .as_ref()
                        .is_some_and(|table| data.len() <= table.as_chunks::<3>().0.len()),
                    _ => false,
                };
                if !fits || !before_data || self.transparency.is_some() || !allowed {
                    return Err(malformed());
                }
                self.transparency = Some(data);
            }
            _ => {
                if kind == b"IHDR" {
                    return Err(malformed());
                }
                if kind.first().is_some_and(u8::is_ascii_uppercase) {
                    return Err(PdfError::new(ErrorCode::Unsupported, READ));
                }
            }
        }
        Ok(true)
    }
}

impl Png {
    /// The image has an alpha channel.
    const fn alpha_channel(&self) -> bool {
        matches!(self.color, 4 | 6)
    }

    /// Samples per pixel; `Colors` of the predictor.
    pub(super) const fn channels(&self) -> u8 {
        match self.color {
            2 => 3,
            4 => 2,
            6 => 4,
            _ => 1,
        }
    }

    /// Decodes the rows of each pass in order, passing each with its pass; fails unless the data
    /// holds exactly the image.
    fn decode(&self, source: &Source, each: &mut EachRow<'_>) -> Result<(), PdfError> {
        let mut inflate = Inflate::new(self.idat(source));
        for pass in self.passes() {
            let mut rows = Predictor::with_parameters(
                &mut inflate,
                15,
                i64::from(self.channels()),
                i64::from(self.bits),
                i64::from(pass.width),
            )?;
            let mut row = buffer(self.row_bytes(pass.width))?;
            for index in 0..pass.height {
                if fill(&mut rows, &mut row)? != row.len() {
                    return Err(malformed());
                }
                each(pass, index, &row)?;
            }
        }
        if inflate.pull(&mut [0])? != 0 {
            return Err(malformed());
        }
        Ok(())
    }

    /// Written decoded, not as the IDAT data.
    pub(super) const fn decoded(&self) -> bool {
        self.decoded
    }

    /// The whole image of an interlaced file, unfiltered.
    fn deinterlaced(&self, source: &Source) -> Result<Vec<u8>, PdfError> {
        let stride = usize::try_from(self.row_bytes(self.width))
            .ok()
            .ok_or_else(malformed)?;
        let mut image = buffer(
            self.row_bytes(self.width)
                .saturating_mul(u64::from(self.height)),
        )?;
        let pixel = u32::from(self.channels()).saturating_mul(u32::from(self.bits));
        let bytes = usize::try_from(pixel >> 3_u32).ok().ok_or_else(malformed)?;
        self.decode(source, &mut |pass, index, row| {
            let at = usize::try_from(pass.row.wrapping_add(index.wrapping_shl(pass.row_shift)))
                .ok()
                .ok_or_else(malformed)?
                .wrapping_mul(stride);
            let target = image
                .get_mut(at..at.wrapping_add(stride))
                .ok_or_else(malformed)?;
            for column in 0..pass.width {
                let to = pass
                    .column
                    .wrapping_add(column.wrapping_shl(pass.column_shift));
                if bytes == 0 {
                    let value = sample(row, column, pixel).ok_or_else(malformed)?;
                    put(target, to, pixel, value).ok_or_else(malformed)?;
                } else {
                    let from = usize::try_from(column)
                        .ok()
                        .ok_or_else(malformed)?
                        .wrapping_mul(bytes);
                    let into = usize::try_from(to)
                        .ok()
                        .ok_or_else(malformed)?
                        .wrapping_mul(bytes);
                    target
                        .get_mut(into..into.wrapping_add(bytes))
                        .ok_or_else(malformed)?
                        .copy_from_slice(
                            row.get(from..from.wrapping_add(bytes))
                                .ok_or_else(malformed)?,
                        );
                }
            }
            Ok(())
        })?;
        Ok(image)
    }

    /// Bytes a reader of its samples holds whole: an interlaced image's.
    #[cfg(feature = "rendering")]
    pub(super) fn held(&self) -> u64 {
        if self.interlaced {
            self.row_bytes(self.width)
                .saturating_mul(u64::from(self.height))
        } else {
            0
        }
    }

    /// The IDAT data.
    pub(super) fn idat(&self, source: &Source) -> Idat {
        Idat {
            left: 0,
            next: self.idat,
            position: 0,
            source: source.clone(),
        }
    }

    /// The soft mask's bits per component and bytes.
    pub(super) fn mask(&self) -> Option<(u8, u64)> {
        let bits = if self.alpha_channel() {
            self.bits
        } else if self.alpha.is_some() || self.key.is_some() {
            8
        } else {
            return None;
        };
        let row = u64::from(self.width).saturating_mul(u64::from(bits >> 3_u32));
        Some((bits, row.saturating_mul(u64::from(self.height))))
    }

    /// Appends the soft mask's samples of the unfiltered `row` to `out`.
    fn mask_row(&self, row: &[u8], out: &mut Vec<u8>) -> Result<(), PdfError> {
        let bits = u32::from(self.bits);
        let channels = self.channels();
        if self.alpha_channel() {
            let size = usize::from(self.bits >> 3_u32);
            for pixel in row.chunks_exact(usize::from(channels).wrapping_mul(size)) {
                let alpha = pixel.len().wrapping_sub(size);
                out.extend_from_slice(pixel.get(alpha..).unwrap_or_default());
            }
        } else if let Some(alpha) = self.alpha.as_deref() {
            for column in 0..self.width {
                let index = sample(row, column, bits).ok_or_else(malformed)?;
                out.push(alpha.get(usize::from(index)).copied().unwrap_or(u8::MAX));
            }
        } else {
            let key = self.key.unwrap_or_default();
            for column in 0..self.width {
                let mut matched = true;
                for (channel, wanted) in (0..channels).zip(key) {
                    let index = column
                        .wrapping_mul(u32::from(channels))
                        .wrapping_add(u32::from(channel));
                    matched &= sample(row, index, bits).ok_or_else(malformed)? == wanted;
                }
                out.push(if matched { 0 } else { u8::MAX });
            }
        }
        Ok(())
    }

    /// The soft mask's samples, read in order.
    pub(super) fn mask_samples(&self, source: &Source) -> Rows {
        Rows::new(self, source, true)
    }

    /// The passes with pixels.
    fn passes(&self) -> Vec<Pass> {
        let whole = [[0; 4]];
        let passes: &[[u32; 4]] = if self.interlaced { &PASSES } else { &whole };
        passes
            .iter()
            .map(|&[column, row, column_shift, row_shift]| {
                let count = |size: u32, first: u32, shift: u32| {
                    size.saturating_sub(first)
                        .saturating_add(1_u32.wrapping_shl(shift).saturating_sub(1))
                        .wrapping_shr(shift)
                };
                Pass {
                    column,
                    column_shift,
                    height: count(self.height, row, row_shift),
                    row,
                    row_shift,
                    width: count(self.width, column, column_shift),
                }
            })
            .filter(|pass| pass.width > 0 && pass.height > 0)
            .collect()
    }

    /// Bytes of a row of `columns` pixels, without its filter type.
    fn row_bytes(&self, columns: u32) -> u64 {
        u64::from(columns)
            .saturating_mul(u64::from(self.channels()))
            .saturating_mul(u64::from(self.bits))
            .saturating_add(7)
            >> 3_u32
    }

    /// The colour samples, read in order.
    pub(super) fn samples(&self, source: &Source) -> Rows {
        Rows::new(self, source, false)
    }

    /// Bytes of the colour samples.
    fn samples_length(&self) -> u64 {
        let colors = self
            .channels()
            .saturating_sub(u8::from(self.alpha_channel()));
        let row = u64::from(self.width)
            .saturating_mul(u64::from(colors))
            .saturating_mul(u64::from(self.bits))
            .saturating_add(7)
            >> 3_u32;
        row.saturating_mul(u64::from(self.height))
    }

    /// Appends the colour samples of the unfiltered `row` to `out`.
    fn samples_row(&self, row: &[u8], out: &mut Vec<u8>) {
        if !self.alpha_channel() {
            out.extend_from_slice(row);
            return;
        }
        let size = usize::from(self.bits >> 3_u32);
        let pixel = usize::from(self.channels()).wrapping_mul(size);
        for each in row.chunks_exact(pixel) {
            out.extend_from_slice(each.get(..pixel.wrapping_sub(size)).unwrap_or_default());
        }
    }

    /// The unfiltered data, an interlaced image's held whole.
    fn unfiltered(&self, source: &Source) -> Result<Box<dyn Pull>, PdfError> {
        Ok(if self.interlaced {
            Box::new(Held::new(self.deinterlaced(source)?))
        } else {
            Box::new(Predictor::with_parameters(
                Inflate::new(self.idat(source)),
                15,
                i64::from(self.channels()),
                i64::from(self.bits),
                i64::from(self.width),
            )?)
        })
    }
}

impl Rows {
    /// Makes the next row into `converted`; `false` at the end, which fails unless the data
    /// holds exactly the image.
    fn fetch(&mut self) -> Result<bool, PdfError> {
        if self.data.is_none() {
            self.raw = buffer(self.png.row_bytes(self.png.width))?;
            self.converted = reserved(if self.mask {
                let (bits, _) = self.png.mask().unwrap_or_default();
                u64::from(self.png.width).saturating_mul(u64::from(bits >> 3_u32))
            } else {
                self.png.row_bytes(self.png.width)
            })?;
            self.data = Some(self.png.unfiltered(&self.source)?);
        }
        let data = self.data.as_mut().ok_or_else(malformed)?;
        self.served = 0;
        self.converted.clear();
        if self.left == 0 {
            return if data.pull(&mut [0])? == 0 {
                Ok(false)
            } else {
                Err(malformed())
            };
        }
        if fill(data, &mut self.raw)? != self.raw.len() {
            return Err(malformed());
        }
        self.left = self.left.saturating_sub(1);
        if self.mask {
            self.png.mask_row(&self.raw, &mut self.converted)?;
        } else {
            self.png.samples_row(&self.raw, &mut self.converted);
        }
        Ok(true)
    }

    /// The rows of `png` in `source`, or of its soft mask.
    fn new(png: &Png, source: &Source, mask: bool) -> Self {
        Self {
            converted: Vec::new(),
            data: None,
            left: png.height,
            mask,
            png: png.clone(),
            raw: Vec::new(),
            served: 0,
            source: source.clone(),
        }
    }
}

/// A zeroed buffer of `length` bytes.
fn buffer(length: u64) -> Result<Vec<u8>, PdfError> {
    let mut bytes = reserved(length)?;
    bytes.resize(usize::try_from(length).unwrap_or_default(), 0);
    Ok(bytes)
}

/// An empty buffer with room for `length` bytes.
fn reserved(length: u64) -> Result<Vec<u8>, PdfError> {
    let size = usize::try_from(length)
        .ok()
        .ok_or_else(|| PdfError::new(ErrorCode::LimitExceeded, READ))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .ok()
        .ok_or_else(|| PdfError::new(ErrorCode::BudgetExceeded, READ))?;
    Ok(bytes)
}

/// Reads a chunk's data of `length` bytes after its type `kind`, checking its CRC (5.3); returns
/// its first `keep` bytes.
fn chunk(
    cursor: &mut Cursor<'_>,
    kind: &[u8],
    length: u32,
    keep: usize,
) -> Result<Vec<u8>, PdfError> {
    let mut register = crc(u32::MAX, kind);
    let mut kept = Vec::new();
    let mut left = u64::from(length);
    let mut scratch = vec![0; STEP];
    while left > 0 {
        let step = scratch
            .get_mut(..usize::try_from(left).unwrap_or(STEP).min(STEP))
            .unwrap_or_default();
        if cursor.fill(step)? != step.len() {
            return Err(malformed());
        }
        register = crc(register, step);
        let room = keep.saturating_sub(kept.len()).min(step.len());
        kept.extend_from_slice(step.get(..room).unwrap_or_default());
        left = left.saturating_sub(u64::try_from(step.len()).unwrap_or(u64::MAX));
    }
    let mut stored = [0; 4];
    if cursor.fill(&mut stored)? != stored.len() || number(&stored) != !register {
        return Err(malformed());
    }
    Ok(kept)
}

/// Updates CRC-32 register `register` with `bytes` (ISO 3309; ISO/IEC
/// 15948:2004, Annex D).
pub(crate) fn crc(register: u32, bytes: &[u8]) -> u32 {
    bytes.iter().fold(register, |sum, byte| {
        let index = usize::try_from((sum ^ u32::from(*byte)) & 0xFF).unwrap_or_default();
        CRCS.get(index).copied().unwrap_or_default() ^ sum.wrapping_shr(8)
    })
}

/// The CRC-32 of the byte `index` (Annex D).
const fn crc_of(index: u32) -> u32 {
    let mut value = index;
    let mut bit = 0_u8;
    while bit < 8 {
        value = value.wrapping_shr(1) ^ (0xEDB8_8320 & 0_u32.wrapping_sub(value & 1));
        bit = bit.wrapping_add(1);
    }
    value
}

/// Parses IHDR's data (11.2.2).
fn parse_header(data: &[u8]) -> Result<Png, PdfError> {
    let Some(
        &[
            w0,
            w1,
            w2,
            w3,
            h0,
            h1,
            h2,
            h3,
            bits,
            color,
            compression,
            filter,
            interlace,
        ],
    ) = data.first_chunk::<13>()
    else {
        return Err(malformed());
    };
    let width = number(&[w0, w1, w2, w3]);
    let height = number(&[h0, h1, h2, h3]);
    let allowed = match color {
        0 => matches!(bits, 1 | 2 | 4 | 8 | 16),
        2 | 4 | 6 => matches!(bits, 8 | 16),
        3 => matches!(bits, 1 | 2 | 4 | 8),
        _ => false,
    };
    if !(1..=MAX).contains(&width)
        || !(1..=MAX).contains(&height)
        || !allowed
        || compression != 0
        || filter != 0
        || interlace > 1
    {
        return Err(malformed());
    }
    if width > MAX_WIDTH {
        return Err(PdfError::new(ErrorCode::LimitExceeded, READ));
    }
    Ok(Png {
        alpha: None,
        bits,
        color,
        decoded: false,
        height,
        idat: 0,
        interlaced: interlace == 1,
        key: None,
        width,
    })
}

/// Bytes of chunk `kind` kept to parse.
const fn keep(kind: &[u8]) -> usize {
    match kind {
        b"PLTE" => 768,
        b"tRNS" => 256,
        b"iCCP" => 81,
        b"eXIf" => EXIF_HEAD,
        _ => 0,
    }
}

/// Sets pixel `index` of `bits` bits, packed from the most significant bit, to `value`.
fn put(row: &mut [u8], index: u32, bits: u32, value: u16) -> Option<()> {
    let bit = usize::try_from(index.checked_mul(bits)?).ok()?;
    let shift = 8_u32
        .checked_sub(bits)?
        .checked_sub(u32::try_from(bit & 7).ok()?)?;
    let byte = row.get_mut(bit >> 3_u32)?;
    *byte |= u8::try_from(value).ok()?.checked_shl(shift)?;
    Some(())
}

/// Reads the PNG in `source`, checking all of it; its samples and profile may hold at most `limit`
/// bytes.
pub(super) fn read(source: &Source, limit: u64) -> Result<Image, PdfError> {
    let mut cursor = Cursor::new(source, 0);
    let mut signature = [0; 8];
    if cursor.fill(&mut signature)? != signature.len() || signature != SIGNATURE {
        return Err(malformed());
    }
    let mut header = [0; 8];
    if cursor.fill(&mut header)? != header.len() || header != *b"\0\0\0\x0DIHDR" {
        return Err(malformed());
    }
    let mut png = parse_header(&chunk(&mut cursor, b"IHDR", 13, 13)?)?;
    let mut chunks = Chunks::default();
    loop {
        // A cut file fails at the chunk's CRC.
        let _: usize = cursor.fill(&mut header)?;
        let length = number(header.get(..4).unwrap_or_default());
        let kind = header.get(4..).unwrap_or_default();
        if length > MAX || !kind.iter().all(u8::is_ascii_alphabetic) {
            return Err(malformed());
        }
        let at = cursor.offset().saturating_sub(8);
        let data = chunk(&mut cursor, kind, length, keep(kind))?;
        if !chunks.take(&png, kind, at, length, data)? {
            break;
        }
    }
    let (idat_length, orientation) = (chunks.idat_length, chunks.orientation);
    let (palette, profile, transparency) = (chunks.palette, chunks.profile, chunks.transparency);
    png.idat = chunks.idat.ok_or_else(malformed)?;
    if png
        .row_bytes(png.width)
        .saturating_mul(u64::from(png.height))
        > limit
    {
        return Err(PdfError::new(ErrorCode::LimitExceeded, READ));
    }
    if let Some(data) = transparency {
        if png.color == 3 {
            png.alpha = Some(data);
        } else {
            let mut key = [0; 3];
            for (slot, pair) in key.iter_mut().zip(data.as_chunks::<2>().0) {
                *slot = u16::try_from(number(pair)).unwrap_or(u16::MAX);
            }
            png.key = Some(key);
        }
    }
    let components = if matches!(png.color, 0 | 4) { 1 } else { 3 };
    let embedded = profile.is_some();
    let kept = match profile {
        Some(icc) => icc.check(source, components, limit)?.then_some(icc),
        None => None,
    };
    let indexed = png.color == 3;
    let entries = palette
        .as_ref()
        .map_or(0, |table| table.as_chunks::<3>().0.len());
    let bits = u32::from(png.bits);
    png.decode(source, &mut |pass, _, row| {
        for column in (0..pass.width).filter(|_| indexed) {
            if usize::from(sample(row, column, bits).ok_or_else(malformed)?) >= entries {
                return Err(malformed());
            }
        }
        Ok(())
    })?;
    png.decoded = png.interlaced || png.alpha_channel();
    Ok(Image {
        bits: png.bits,
        components,
        dropped_profile: embedded && kept.is_none(),
        height: png.height,
        inverted: false,
        length: if png.decoded {
            png.samples_length()
        } else {
            idat_length
        },
        orientation: orientation.unwrap_or(1),
        palette: palette.filter(|_| indexed),
        profile: kept,
        source: source.clone(),
        width: png.width,
        png: Some(png),
    })
}

/// Pixel or sample `index` of `bits` bits, packed from the most significant bit.
fn sample(row: &[u8], index: u32, bits: u32) -> Option<u16> {
    let bit = usize::try_from(index.checked_mul(bits)?).ok()?;
    let at = bit >> 3_u32;
    let high = u16::from(*row.get(at)?);
    if bits == 16 {
        return Some(high.wrapping_shl(8) | u16::from(*row.get(at.checked_add(1)?)?));
    }
    let shift = 8_u32
        .checked_sub(bits)?
        .checked_sub(u32::try_from(bit & 7).ok()?)?;
    Some(high.wrapping_shr(shift) & u16::MAX.wrapping_shr(16_u32.wrapping_sub(bits)))
}
