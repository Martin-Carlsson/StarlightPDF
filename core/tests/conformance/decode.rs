//! Reads the PNG and the JPEG files a render writes, sharing no code with the library.
//!
//! Only what the writers make, so that a change to them fails here: PNG (W3C, third edition) of 8-bit grey or RGB
//! without alpha, deflated in fixed-code blocks (RFC 1950, RFC 1951, 3.2.6); baseline JPEG (ITU-T T.81) of 1x1
//! sampling without restarts, read by its DC terms alone, which are each block's mean (A.3.3): right where a block
//! is flat.

use alloc::collections::BTreeMap;

/// Each length code's shortest length (RFC 1951, 3.2.5).
const LENGTHS: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];

/// Each distance code's shortest distance (RFC 1951, 3.2.5).
const DISTANCES: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 0x3001, 0x4001, 0x6001,
];

/// The modulus of Adler-32 (RFC 1950, 8.2).
const ADLER: u32 = 0xFFF1;

/// The CRC-32 polynomial, reversed (PNG, 5.5).
const CRC: u32 = 0xEDB8_8320;

/// The first bytes of a PNG file (PNG, 5.2).
const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1A\n";

/// Most bytes an image here may decode to.
const MOST: usize = 0x1000_0000;

/// A fault in the file.
type Fault = &'static str;

/// A canonical Huffman code (RFC 1951, 3.2.2; ITU-T T.81, Annex C).
struct Code {
    /// Codes of each length from 1.
    counts: Vec<usize>,
    /// The symbols by code.
    symbols: Vec<u16>,
}

/// One component of a JPEG frame and its DC terms.
struct Component {
    /// The AC table's identifier.
    ac_table: u8,
    /// The DC table's identifier.
    dc_table: u8,
    /// The component's identifier.
    id: u8,
    /// Its blocks' DC terms, dequantised, a row after another.
    levels: Vec<i64>,
    /// The quantisation table's identifier.
    quantiser: u8,
}

/// A JPEG scan's entropy-coded data.
struct Entropy<'data> {
    /// The index of the next byte.
    at: usize,
    /// The byte being read.
    byte: u8,
    /// The file from the scan's data on.
    data: &'data [u8],
    /// Bits of `byte` not yet read.
    left: u32,
}

/// A JPEG file's segments before its scan's data.
#[derive(Default)]
struct Header {
    /// The components.
    components: Vec<Component>,
    /// The height.
    height: usize,
    /// Each table's DC quantiser.
    quantisers: BTreeMap<u8, i64>,
    /// The Huffman codes, by class and identifier.
    tables: BTreeMap<u8, Code>,
    /// The width.
    width: usize,
}

/// A decoded file.
pub(crate) struct Image {
    /// 1 for grey, 3 for RGB.
    pub channels: usize,
    /// The height.
    pub height: usize,
    /// Whether the file is a JPEG.
    pub jpeg: bool,
    /// Every sample, interleaved, a row after another.
    pub samples: Vec<u8>,
    /// The width.
    pub width: usize,
}

/// RFC 1951's bits, each byte's lowest first.
struct Inflate<'data> {
    /// The index of the next bit.
    at: usize,
    /// The compressed data.
    data: &'data [u8],
    /// What is decoded so far.
    out: Vec<u8>,
}

impl Code {
    /// The code of `counts` and `symbols`; fails when a length has more codes than fit.
    fn checked(counts: Vec<usize>, symbols: Vec<u16>) -> Result<Self, Fault> {
        let mut room = 1_usize;
        for &count in &counts {
            room = room
                .saturating_mul(2)
                .checked_sub(count)
                .ok_or("a Huffman code with too many codes")?;
        }
        Ok(Self { counts, symbols })
    }

    /// The code of each symbol's length in `lengths`; 0 gives it none.
    fn of_lengths(lengths: &[u8]) -> Result<Self, Fault> {
        let mut counts = Vec::new();
        let mut symbols = Vec::new();
        for length in 1..=15_u8 {
            let before = symbols.len();
            for (&given, symbol) in lengths.iter().zip(0_u16..) {
                if given == length {
                    symbols.push(symbol);
                }
            }
            counts.push(symbols.len().saturating_sub(before));
        }
        Self::checked(counts, symbols)
    }

    /// The next symbol of the bits `next` gives.
    fn read<F>(&self, mut next: F) -> Result<u16, Fault>
    where
        F: FnMut() -> Result<usize, Fault>,
    {
        let (mut code, mut first, mut index) = (0_usize, 0_usize, 0_usize);
        for &count in &self.counts {
            code |= next()?;
            let rank = code.checked_sub(first).ok_or("no such Huffman code")?;
            if rank < count {
                return index
                    .checked_add(rank)
                    .and_then(|at| self.symbols.get(at))
                    .copied()
                    .ok_or("no such symbol");
            }
            index = index.saturating_add(count);
            first = first.saturating_add(count).saturating_mul(2);
            code = code.saturating_mul(2);
        }
        Err("no such Huffman code")
    }
}

impl Entropy<'_> {
    /// The next bit, past stuffed bytes (ITU-T T.81, F.2.2.5).
    fn bit(&mut self) -> Result<usize, Fault> {
        if self.left == 0 {
            self.byte = self.take()?;
            if self.byte == 0xFF && self.take()? != 0 {
                return Err("a marker within the scan's data");
            }
            self.left = 8;
        }
        self.left = self.left.saturating_sub(1);
        Ok(usize::from(self.byte.wrapping_shr(self.left) & 1))
    }

    /// Passes the padding of ones and the marker `code` (ITU-T T.81, B.1.1.5, F.1.2.3).
    fn marker(&mut self, code: u8) -> Result<(), Fault> {
        while self.left > 0 {
            if self.bit()? != 1 {
                return Err("padding other than ones");
            }
        }
        if self.take()? != 0xFF || self.take()? != code {
            return Err("a marker missing after the scan's data");
        }
        Ok(())
    }

    /// The next signed number of `size` bits (ITU-T T.81, F.2.2.1).
    fn number(&mut self, size: u32) -> Result<i64, Fault> {
        let mut value = 0_i64;
        for _ in 0..size {
            let bit = i64::try_from(self.bit()?).map_err(|_error| "a bit")?;
            value = value.saturating_mul(2).saturating_add(bit);
        }
        let range = 1_i64.wrapping_shl(size);
        Ok(if size > 0 && value.saturating_mul(2) < range {
            value.saturating_sub(range).saturating_add(1)
        } else {
            value
        })
    }

    /// The next byte.
    fn take(&mut self) -> Result<u8, Fault> {
        let byte = *self.data.get(self.at).ok_or("the scan's data ends early")?;
        self.at = self.at.saturating_add(1);
        Ok(byte)
    }
}

impl Header {
    /// Takes a SOF0 segment (ITU-T T.81, B.2.2).
    fn frame(&mut self, body: &[u8]) -> Result<(), Fault> {
        let Some((&[8, tall_high, tall_low, wide_high, wide_low, count], specs)) =
            body.split_first_chunk()
        else {
            return Err("a frame without 8-bit samples");
        };
        self.height = number(&[tall_high, tall_low]);
        self.width = number(&[wide_high, wide_low]);
        let (triples, spare) = specs.as_chunks::<3>();
        if !matches!(count, 1 | 3)
            || triples.len() != usize::from(count)
            || !spare.is_empty()
            || !self.components.is_empty()
        {
            return Err("a frame without 1 or 3 components");
        }
        for &[id, factors, quantiser] in triples {
            if factors != 0x11 {
                return Err("sampling other than 1x1");
            }
            self.components.push(Component {
                ac_table: 0,
                dc_table: 0,
                id,
                levels: Vec::new(),
                quantiser,
            });
        }
        Ok(())
    }

    /// Takes the Huffman tables of a DHT segment (ITU-T T.81, B.2.4.2).
    fn huffman(&mut self, body: &[u8]) -> Result<(), Fault> {
        let mut rest = body;
        while let Some((&key, after)) = rest.split_first() {
            if !matches!(key, 0 | 1 | 0x10 | 0x11) {
                return Err("a table baseline JPEG lacks");
            }
            let (lengths, values) = after.split_first_chunk::<16>().ok_or("a short DHT")?;
            let counts: Vec<usize> = lengths.iter().map(|&count| usize::from(count)).collect();
            let total = counts
                .iter()
                .fold(0_usize, |sum, &count| sum.saturating_add(count));
            let (symbols, left) = values.split_at_checked(total).ok_or("a short DHT")?;
            let code = Code::checked(
                counts,
                symbols.iter().map(|&value| u16::from(value)).collect(),
            )?;
            drop(self.tables.insert(key, code));
            rest = left;
        }
        Ok(())
    }

    /// Takes the DC quantisers of a DQT segment (ITU-T T.81, B.2.4.1).
    fn quantise(&mut self, body: &[u8]) -> Result<(), Fault> {
        let mut rest = body;
        while let Some((&key, after)) = rest.split_first() {
            let (table, left) = after.split_first_chunk::<64>().ok_or("a short DQT")?;
            if key > 3 || table.contains(&0) {
                return Err("a quantisation table baseline JPEG lacks");
            }
            let first = table.first().copied().unwrap_or(0);
            let _previous: Option<i64> = self.quantisers.insert(key, i64::from(first));
            rest = left;
        }
        Ok(())
    }

    /// Takes the one interleaved scan's segment (ITU-T T.81, B.2.3).
    fn scan(&mut self, body: &[u8]) -> Result<(), Fault> {
        let (&count, specs) = body.split_first().ok_or("a short SOS")?;
        let (pairs, tail) = specs
            .split_at_checked(usize::from(count).saturating_mul(2))
            .ok_or("a short SOS")?;
        if usize::from(count) != self.components.len() || tail != [0, 63, 0] {
            return Err("not one baseline scan of every component");
        }
        for (component, &[id, selected]) in self.components.iter_mut().zip(pairs.as_chunks::<2>().0)
        {
            if component.id != id {
                return Err("a scan out of the frame's order");
            }
            component.dc_table = selected.wrapping_shr(4);
            component.ac_table = 0x10 | (selected & 15);
        }
        Ok(())
    }
}

impl Image {
    /// The samples of the pixel at `column` and `row`, if the image has one.
    pub(crate) fn pixel(&self, column: usize, row: usize) -> Option<&[u8]> {
        if column >= self.width || row >= self.height {
            return None;
        }
        let start = row
            .checked_mul(self.width)?
            .checked_add(column)?
            .checked_mul(self.channels)?;
        self.samples.get(start..start.checked_add(self.channels)?)
    }

    /// The file `bytes`: a PNG or a baseline JPEG.
    pub(crate) fn read(bytes: &[u8]) -> Result<Self, Fault> {
        match (
            bytes.strip_prefix(SIGNATURE),
            bytes.strip_prefix(&[0xFF, 0xD8]),
        ) {
            (Some(chunks), _) => png(chunks),
            (None, Some(segments)) => jpeg(segments),
            (None, None) => Err("neither PNG nor JPEG"),
        }
    }
}

impl Inflate<'_> {
    /// The next bit.
    fn bit(&mut self) -> Result<usize, Fault> {
        let byte = *self
            .data
            .get(self.at.wrapping_shr(3))
            .ok_or("the compressed data ends early")?;
        let shift = u32::try_from(self.at & 7).map_err(|_error| "a bit")?;
        self.at = self.at.saturating_add(1);
        Ok(usize::from(byte.wrapping_shr(shift) & 1))
    }

    /// The next `count` bits as a number, its lowest first.
    fn bits(&mut self, count: usize) -> Result<usize, Fault> {
        let (mut value, mut place) = (0_usize, 1_usize);
        for _ in 0..count {
            value = value.saturating_add(self.bit()?.saturating_mul(place));
            place = place.saturating_mul(2);
        }
        Ok(value)
    }

    /// Decodes a block of symbols (RFC 1951, 3.2.5).
    fn block(&mut self, literal: &Code, distance: &Code) -> Result<(), Fault> {
        loop {
            if self.out.len() > MOST {
                return Err("too much data");
            }
            let symbol = self.symbol(literal)?;
            let Some(index) = symbol.checked_sub(257) else {
                if symbol == 256 {
                    return Ok(());
                }
                self.out
                    .push(u8::try_from(symbol).map_err(|_error| "a literal")?);
                continue;
            };
            let base = *LENGTHS.get(usize::from(index)).ok_or("no such length")?;
            let extra = if index < 8 || index == 28 {
                0
            } else {
                usize::from(index.wrapping_shr(2).saturating_sub(1))
            };
            let length = usize::from(base).saturating_add(self.bits(extra)?);
            let code = self.symbol(distance)?;
            let near = *DISTANCES.get(usize::from(code)).ok_or("no such distance")?;
            let more = usize::from(code.wrapping_shr(1).saturating_sub(1));
            let back = usize::from(near).saturating_add(self.bits(more)?);
            let start = self
                .out
                .len()
                .checked_sub(back)
                .ok_or("a distance before the start")?;
            for at in (start..).take(length) {
                let byte = *self.out.get(at).ok_or("a distance before the start")?;
                self.out.push(byte);
            }
        }
    }

    /// The next symbol of `code`.
    fn symbol(&mut self, code: &Code) -> Result<u16, Fault> {
        code.read(|| self.bit())
    }
}

/// The big-endian number `bytes` hold.
fn number(bytes: &[u8]) -> usize {
    bytes.iter().fold(0, |value: usize, &byte| {
        value.saturating_mul(256).saturating_add(usize::from(byte))
    })
}

/// The zlib stream `data`, decoded and checked (RFC 1950, 2.2).
fn inflate(data: &[u8]) -> Result<Vec<u8>, Fault> {
    let (&[method, flags], body) = data.split_first_chunk().ok_or("a short zlib stream")?;
    if method & 15 != 8 || flags & 0x20 != 0 || number(&[method, flags]).checked_rem(31) != Some(0)
    {
        return Err("not a zlib stream");
    }
    let mut state = Inflate {
        at: 0,
        data: body,
        out: Vec::new(),
    };
    loop {
        let last = state.bit()? == 1;
        if state.bits(2)? != 1 {
            return Err("a block not of fixed codes");
        }
        let literal = [[8_u8; 144].as_slice(), &[9; 112], &[7; 24], &[8; 8]].concat();
        state.block(&Code::of_lengths(&literal)?, &Code::of_lengths(&[5; 30])?)?;
        if last {
            break;
        }
    }
    let end = state.at.saturating_add(7).wrapping_shr(3);
    let sum = body.get(end..).ok_or("no Adler-32")?;
    let (mut low, mut high) = (1_u32, 0_u32);
    for &byte in &state.out {
        low = low
            .saturating_add(u32::from(byte))
            .checked_rem(ADLER)
            .ok_or("Adler-32")?;
        high = high
            .saturating_add(low)
            .checked_rem(ADLER)
            .ok_or("Adler-32")?;
    }
    if sum.len() != 4 || u32::try_from(number(sum)).ok() != Some(high.wrapping_shl(16) | low) {
        return Err("a wrong Adler-32");
    }
    Ok(state.out)
}

/// The CRC-32 of `kind` and `body` (PNG, 5.5).
fn crc(kind: &[u8], body: &[u8]) -> u32 {
    let mut value = u32::MAX;
    for &byte in kind.iter().chain(body) {
        value ^= u32::from(byte);
        for _ in 0_u8..8 {
            let low = value & 1;
            value = value.wrapping_shr(1) ^ if low == 1 { CRC } else { 0 };
        }
    }
    !value
}

/// The PNG of `chunks`: IHDR first, IEND last, no transparency (PNG, 5.6, 11.2.1).
fn png(chunks: &[u8]) -> Result<Image, Fault> {
    let mut rest = chunks;
    let mut header = None;
    let mut data = Vec::new();
    let mut after_data = false;
    loop {
        let (length, after) = rest.split_first_chunk::<4>().ok_or("no IEND")?;
        let (kind, tail) = after.split_first_chunk::<4>().ok_or("a short chunk")?;
        let (body, end) = tail
            .split_at_checked(number(length))
            .ok_or("a short chunk")?;
        let (sum, next) = end.split_first_chunk::<4>().ok_or("a chunk without CRC")?;
        if usize::try_from(crc(kind, body)).ok() != Some(number(sum)) {
            return Err("a wrong CRC");
        }
        match kind {
            b"IHDR" if header.is_none() => header = Some(body),
            _ if header.is_none() => return Err("IHDR is not first"),
            b"IDAT" if !after_data => data.extend_from_slice(body),
            b"IEND" if next.is_empty() && body.is_empty() => break,
            b"tRNS" | b"acTL" => return Err("transparency or animation"),
            &[first, ..] if first.is_ascii_lowercase() => after_data = !data.is_empty(),
            _ => return Err("a chunk out of place"),
        }
        rest = next;
    }
    let Some(
        &[
            wide0,
            wide1,
            wide2,
            wide3,
            tall0,
            tall1,
            tall2,
            tall3,
            8,
            colour,
            0,
            0,
            0,
        ],
    ) = header
    else {
        return Err("not 8 bits without interlace");
    };
    let channels = match colour {
        0 => 1,
        2 => 3,
        _ => return Err("not opaque grey or RGB"),
    };
    let width = number(&[wide0, wide1, wide2, wide3]);
    let height = number(&[tall0, tall1, tall2, tall3]);
    if width == 0 || height == 0 {
        return Err("no pixels");
    }
    let samples = unfilter(&inflate(&data)?, width, height, channels)?;
    Ok(Image {
        channels,
        height,
        jpeg: false,
        samples,
        width,
    })
}

/// The samples of the filtered rows `raw` (PNG, 9.2).
fn unfilter(raw: &[u8], width: usize, height: usize, channels: usize) -> Result<Vec<u8>, Fault> {
    let stride = width.checked_mul(channels).ok_or("too wide")?;
    let line = stride.checked_add(1).ok_or("too wide")?;
    if line.checked_mul(height) != Some(raw.len()) {
        return Err("rows of another size");
    }
    let mut samples = Vec::new();
    let mut above = vec![0_u8; stride];
    for packed in raw.chunks_exact(line) {
        let (&filter, filtered) = packed.split_first().ok_or("a short row")?;
        let mut row: Vec<u8> = Vec::with_capacity(stride);
        for (&value, (&over, at)) in filtered.iter().zip(above.iter().zip(0_usize..)) {
            let before = at.checked_sub(channels);
            let left = before
                .and_then(|index| row.get(index))
                .copied()
                .unwrap_or(0);
            let corner = before
                .and_then(|index| above.get(index))
                .copied()
                .unwrap_or(0);
            let predicted = match filter {
                0 => 0,
                1 => left,
                2 => over,
                3 => left.midpoint(over),
                4 => paeth(left, over, corner),
                _ => return Err("an unknown filter"),
            };
            row.push(value.wrapping_add(predicted));
        }
        samples.extend_from_slice(&row);
        above = row;
    }
    Ok(samples)
}

/// The Paeth predictor (PNG, 9.4).
fn paeth(left: u8, over: u8, corner: u8) -> u8 {
    let to_left = over.abs_diff(corner);
    let to_over = left.abs_diff(corner);
    let to_corner = u16::from(left)
        .saturating_add(u16::from(over))
        .abs_diff(u16::from(corner).saturating_mul(2));
    if to_left <= to_over && u16::from(to_left) <= to_corner {
        left
    } else if u16::from(to_over) <= to_corner {
        over
    } else {
        corner
    }
}

/// The baseline JPEG of `segments`, those after SOI (ITU-T T.81, B.2).
fn jpeg(segments: &[u8]) -> Result<Image, Fault> {
    let mut header = Header::default();
    let mut rest = segments;
    loop {
        let (&[lead, marker, high, low], after) = rest.split_first_chunk().ok_or("no scan")?;
        let length = number(&[high, low])
            .checked_sub(2)
            .ok_or("a short segment")?;
        let (body, next) = after.split_at_checked(length).ok_or("a short segment")?;
        if lead != 0xFF {
            return Err("no marker where one is due");
        }
        match marker {
            0xC0 => header.frame(body)?,
            0xC4 => header.huffman(body)?,
            0xDA => {
                header.scan(body)?;
                return pixels(&scanned(header, next)?);
            }
            0xDB => header.quantise(body)?,
            0xE0..=0xEF | 0xFE => {}
            _ => return Err("not baseline JPEG"),
        }
        rest = next;
    }
}

/// How many blocks cover `size` samples (ITU-T T.81, A.2.4).
fn across(size: usize) -> Result<usize, Fault> {
    Some(size.div_ceil(8))
        .filter(|&count| count > 0)
        .ok_or("no samples")
}

/// `header` with every block's DC term, read from the scan's `data` up to EOI (ITU-T T.81, F.2.2).
fn scanned(mut header: Header, data: &[u8]) -> Result<Header, Fault> {
    let total = across(header.width)?
        .checked_mul(across(header.height)?)
        .filter(|&count| count <= MOST)
        .ok_or("too many blocks")?;
    let mut entropy = Entropy {
        at: 0,
        byte: 0,
        data,
        left: 0,
    };
    for component in &mut header.components {
        component.levels = vec![0; total];
    }
    let mut predicted = [0_i64; 3];
    for mcu in 0..total {
        for (component, previous) in header.components.iter_mut().zip(&mut predicted) {
            let direct = header
                .tables
                .get(&component.dc_table)
                .ok_or("no DC table")?;
            let others = header
                .tables
                .get(&component.ac_table)
                .ok_or("no AC table")?;
            let step = *header
                .quantisers
                .get(&component.quantiser)
                .ok_or("no quantiser")?;
            let change = block(&mut entropy, direct, others)?;
            *previous = previous.saturating_add(change);
            *component.levels.get_mut(mcu).ok_or("a block outside")? =
                previous.saturating_mul(step);
        }
    }
    entropy.marker(0xD9)?;
    if entropy.at != data.len() {
        return Err("bytes after EOI");
    }
    Ok(header)
}

/// A block's DC difference; its AC symbols are passed.
fn block(entropy: &mut Entropy<'_>, direct: &Code, others: &Code) -> Result<i64, Fault> {
    let category = direct.read(|| entropy.bit())?;
    if category > 11 {
        return Err("a DC category baseline JPEG lacks");
    }
    let change = entropy.number(u32::from(category))?;
    let mut index = 1_u32;
    while index < 64 {
        let symbol = others.read(|| entropy.bit())?;
        let (run, size) = (symbol.wrapping_shr(4), symbol & 15);
        if symbol == 0 {
            break;
        }
        if size > 10 || size == 0 && run != 15 {
            return Err("an AC symbol baseline JPEG lacks");
        }
        index = index.saturating_add(u32::from(run)).saturating_add(1);
        if index > 64 {
            return Err("a block of over 64 coefficients");
        }
        let _: i64 = entropy.number(u32::from(size))?;
    }
    Ok(change)
}

/// The image of each block's mean, in RGB by ITU-T T.871, 7.
fn pixels(header: &Header) -> Result<Image, Fault> {
    let columns = across(header.width)?;
    let mut samples = Vec::new();
    for row in 0..header.height {
        for column in 0..header.width {
            let at = row
                .wrapping_shr(3)
                .saturating_mul(columns)
                .saturating_add(column.wrapping_shr(3));
            let mut mean = [0_i64; 3];
            for (component, value) in header.components.iter().zip(&mut mean) {
                *value = *component.levels.get(at).ok_or("a pixel outside")?;
            }
            // In eighths: the DC term, less the level shift of 128.
            let [luma, blue, red] = mean;
            let base = luma.saturating_add(1024).saturating_mul(0x1_0000);
            let shifted: [i64; 3] = if header.components.len() == 1 {
                [base; 3]
            } else {
                [
                    base.saturating_add(red.saturating_mul(91_881)),
                    base.saturating_sub(blue.saturating_mul(22_554))
                        .saturating_sub(red.saturating_mul(46_802)),
                    base.saturating_add(blue.saturating_mul(0x1_C5A2)),
                ]
            };
            for value in shifted.iter().take(header.components.len()) {
                let rounded = value
                    .saturating_add(0x4_0000)
                    .wrapping_shr(19)
                    .clamp(0, 255);
                samples.push(u8::try_from(rounded).map_err(|_error| "a sample")?);
            }
        }
    }
    Ok(Image {
        channels: header.components.len(),
        height: header.height,
        jpeg: true,
        samples,
        width: header.width,
    })
}
