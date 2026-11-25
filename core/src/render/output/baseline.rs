//! Baseline JPEG output (ITU-T T.81, F.1.2 and Annex K) in a JFIF file (ITU-T T.871).

use crate::dct::ZIGZAG;
use crate::output::Sink;
use crate::render::OPERATION;
use crate::render::output::{BLOCK, Format, Settings, buffer, channels, low, lowest};
use crate::{ErrorCode, PdfError};

/// The BITS of luminance's AC codes (Table K.5).
const AC_COUNTS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 125];

/// The HUFFVAL of luminance's AC codes (Table K.5).
const AC_VALUES: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7,
    0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5,
    0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2,
    0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8,
    0xF9, 0xFA,
];

/// The JFIF segment: version 1.02, square pixels, no thumbnail (T.871, 10.1).
const APP0: &[u8] = b"JFIF\0\x01\x02\0\0\x01\0\x01\0\0";

/// The BITS of chrominance's AC codes (Table K.6).
const CHROMA_AC_COUNTS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 119];

/// The HUFFVAL of chrominance's AC codes (Table K.6).
const CHROMA_AC_VALUES: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52, 0xF0,
    0x15, 0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34, 0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26,
    0x27, 0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5,
    0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3,
    0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA,
    0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8,
    0xF9, 0xFA,
];

/// The BITS of chrominance's DC codes (Table K.4).
const CHROMA_DC_COUNTS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];

/// Chrominance's quantisation (Table K.2), zig-zag.
const CHROMA_STEPS: [u8; 64] = [
    17, 18, 18, 24, 21, 24, 47, 26, 26, 47, 99, 66, 56, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

/// C(u) cos((2x + 1)u pi / 16) of A.3.3 over root 2 for x to 3, with 13 fraction bits: exact
/// for the DC term, so that flat blocks round right.
const COSINES: [[i32; 4]; 8] = [
    [4096, 4096, 4096, 4096],
    [5681, 4816, 3218, 1130],
    [5352, 2217, -2217, -5352],
    [4816, -1130, -5681, -3218],
    [4096, -4096, -4096, 4096],
    [3218, -5681, 1130, 4816],
    [2217, -5352, 5352, -2217],
    [1130, -3218, 4816, -5681],
];

/// The BITS of luminance's DC codes (Table K.3).
const DC_COUNTS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];

/// The HUFFVAL of both DC codes (Tables K.3 and K.4).
const DC_VALUES: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

/// The fraction bits dropped after the first pass, leaving 4.
const FIRST_PASS: u32 = 9;

/// The fraction bits of a transformed sample.
const FRACTION: u32 = 18;

/// Luminance's quantisation (Table K.1), zig-zag.
const STEPS: [u8; 64] = [
    16, 11, 12, 14, 12, 10, 16, 14, 13, 14, 18, 17, 16, 19, 24, 40, 26, 24, 22, 22, 24, 49, 35, 37,
    29, 40, 58, 51, 61, 60, 57, 51, 56, 55, 64, 72, 92, 78, 64, 68, 87, 69, 55, 56, 80, 109, 81,
    87, 95, 98, 103, 104, 103, 62, 77, 113, 121, 112, 100, 120, 92, 101, 103, 99,
];

/// Y, Cb and Cr of red, green and blue, then the offset, with 16 fraction bits (T.871, 7).
const YCC: [[i32; 4]; 3] = [
    [19_595, 38_470, 7471, 0x8000],
    [-11_059, -21_709, 0x8000, 0x80_8000],
    [0x8000, -27_439, -5329, 0x80_8000],
];

/// A JPEG being written.
pub(super) struct Baseline<'sink> {
    /// Samples a pixel.
    channels: usize,
    /// Each component's last DC term.
    previous: [i32; 3],
    /// Up to 8 rows, interleaved.
    rows: Vec<u8>,
    /// Where to.
    sink: &'sink Sink,
    /// The bytes of a row.
    stride: usize,
    /// Luminance's tables, then chrominance's.
    tables: Vec<Table>,
    /// The file's bytes not yet written.
    writer: Writer,
}

/// The codes of one Huffman table, by symbol: code and width.
type Codes = Vec<(u32, u32)>;

/// What a kind of component is coded with, as printed in Annex K.
struct Printed {
    /// The BITS of the AC codes.
    ac_counts: &'static [u8; 16],
    /// Their HUFFVAL.
    ac_values: &'static [u8; 162],
    /// The BITS of the DC codes.
    dc_counts: &'static [u8; 16],
    /// The quantisation, zig-zag.
    steps: &'static [u8; 64],
}

/// What a kind of component is coded with, ready for use.
struct Table {
    /// The AC codes.
    ac: Codes,
    /// The DC codes.
    dc: Codes,
    /// The quantisation, zig-zag.
    steps: [u8; 64],
}

/// Bytes gathered, and the bits after them.
struct Writer {
    /// Bits not yet in `out`, the last lowest.
    bits: u32,
    /// How many `bits` holds.
    count: u32,
    /// The bytes.
    out: Vec<u8>,
}

impl Format for Baseline<'_> {
    fn finish(&mut self) -> Result<(), PdfError> {
        if !self.rows.is_empty() {
            self.blocks()?;
        }
        // The last byte is padded with ones (F.1.2.3).
        let padding = self.writer.count.wrapping_neg() & 7;
        self.writer.push(lowest(u32::MAX, padding), padding);
        self.writer.out.extend_from_slice(&[0xFF, 0xD9]);
        self.sink.write(&self.writer.out, OPERATION)
    }

    fn row(&mut self, samples: &mut [u8]) -> Result<(), PdfError> {
        if self.channels == 3 {
            for pixel in samples.as_chunks_mut::<3>().0 {
                *pixel = luma_chroma(*pixel);
            }
        }
        self.rows.extend_from_slice(samples);
        if self.rows.len() >= self.stride.saturating_mul(8) {
            self.blocks()?;
        }
        Ok(())
    }
}

impl<'sink> Baseline<'sink> {
    /// Codes the rows held, padded to 8 by the last column and row (A.2.4).
    fn blocks(&mut self) -> Result<(), PdfError> {
        let last = self.rows.len().saturating_sub(self.stride);
        let step = self.channels.saturating_mul(8);
        let mut left = 0_usize;
        while left < self.stride {
            for (component, previous) in self.previous.iter_mut().take(self.channels).enumerate() {
                let Some(table) = self.tables.get(component.min(1)) else {
                    continue;
                };
                let mut block = [0_i32; 64];
                let first = left.saturating_add(component);
                for (index, line) in block.as_chunks_mut::<8>().0.iter_mut().enumerate() {
                    let from = index.saturating_mul(self.stride).min(last);
                    let row = self.rows.get(from..).unwrap_or_default();
                    let cells = row.get(..self.stride).unwrap_or(row);
                    let near = cells.get(first..).unwrap_or_default();
                    let mut source = near.iter().step_by(self.channels);
                    let mut sample = 0_u8;
                    for level in line {
                        sample = source.next().copied().unwrap_or(sample);
                        // The level shift (A.3.1).
                        *level = i32::from(sample).saturating_sub(128);
                    }
                }
                let terms = quantised(&transformed(&block), &table.steps);
                *previous = self.writer.block(table, *previous, &terms);
            }
            if self.writer.out.len() >= BLOCK {
                self.sink.write(&self.writer.out, OPERATION)?;
                self.writer.out.clear();
            }
            left = left.saturating_add(step);
        }
        self.rows.clear();
        Ok(())
    }

    /// Writes the segments before the scan (B.2).
    pub(super) fn new(sink: &'sink Sink, settings: Settings) -> Result<Self, PdfError> {
        let limit = |_error| PdfError::new(ErrorCode::LimitExceeded, OPERATION);
        let height = u16::try_from(settings.height).map_err(limit)?;
        let width = u16::try_from(settings.width).map_err(limit)?;
        let samples = channels(settings.gray);
        let luminance = Printed {
            ac_counts: &AC_COUNTS,
            ac_values: &AC_VALUES,
            dc_counts: &DC_COUNTS,
            steps: &STEPS,
        };
        let chrominance = Printed {
            ac_counts: &CHROMA_AC_COUNTS,
            ac_values: &CHROMA_AC_VALUES,
            dc_counts: &CHROMA_DC_COUNTS,
            steps: &CHROMA_STEPS,
        };
        let mut out = vec![0xFF, 0xD8];
        segment(&mut out, 0xE0, APP0);
        let mut frame = vec![8];
        frame.extend([height, width].map(pair).as_flattened());
        let mut scan = Vec::new();
        let (count, selectors): (u8, &[u8]) = if settings.gray {
            (1, &[0])
        } else {
            (3, &[0, 1, 1])
        };
        frame.push(count);
        scan.push(count);
        for (id, &selector) in (1_u8..).zip(selectors) {
            frame.extend_from_slice(&[id, 0x11, selector]);
            scan.extend_from_slice(&[id, selector.wrapping_mul(0x11)]);
        }
        // Spectral selection and approximation of a sequential scan (B.2.3).
        scan.extend_from_slice(&[0, 63, 0]);
        let mut tables = Vec::new();
        let mut codes = Vec::new();
        let kinds = [luminance, chrominance];
        let used = if settings.gray { 1 } else { 2 };
        for (id, printed) in (0_u8..).zip(kinds.iter().take(used)) {
            let steps = scaled(printed.steps, settings.quality);
            segment(&mut out, 0xDB, &[&[id][..], &steps].concat());
            let dc = [&[id][..], printed.dc_counts, &DC_VALUES].concat();
            let ac = [&[id | 0x10][..], printed.ac_counts, printed.ac_values].concat();
            codes.extend([dc, ac]);
            tables.push(Table {
                ac: canonical(printed.ac_counts, printed.ac_values),
                dc: canonical(printed.dc_counts, &DC_VALUES),
                steps,
            });
        }
        segment(&mut out, 0xC0, &frame);
        for body in &codes {
            segment(&mut out, 0xC4, body);
        }
        segment(&mut out, 0xDA, &scan);
        let mut rows = buffer(settings.width, samples.saturating_mul(8))?;
        rows.clear();
        Ok(Self {
            channels: samples,
            previous: [0; 3],
            rows,
            sink,
            stride: usize::from(width).saturating_mul(samples),
            tables,
            writer: Writer {
                bits: 0,
                count: 0,
                out,
            },
        })
    }
}

impl Writer {
    /// Codes a block's `terms` (F.1.2); the DC term, which the next difference is from.
    fn block(&mut self, table: &Table, previous: i32, terms: &[i32; 64]) -> i32 {
        let [dc, rest @ ..] = *terms;
        self.term(&table.dc, 0, dc.saturating_sub(previous));
        let mut zeros = 0_u32;
        for term in rest {
            if term == 0_i32 {
                zeros = zeros.saturating_add(1);
                continue;
            }
            while zeros > 15 {
                // ZRL: 16 zeros.
                self.code(&table.ac, 0xF0);
                zeros = zeros.saturating_sub(16);
            }
            self.term(&table.ac, zeros, term);
            zeros = 0;
        }
        if zeros > 0 {
            // EOB.
            self.code(&table.ac, 0);
        }
        dc
    }

    /// Writes `symbol`'s code.
    fn code(&mut self, codes: &Codes, symbol: u32) {
        let entry = usize::try_from(symbol).ok().and_then(|at| codes.get(at));
        if let Some(&(code, width)) = entry {
            self.push(code, width);
        }
    }

    /// Writes the `width` lowest bits of `value`, highest first, with FF stuffed (F.1.2.3).
    fn push(&mut self, value: u32, width: u32) {
        self.bits = self.bits.wrapping_shl(width) | value;
        self.count = self.count.saturating_add(width);
        while self.count >= 8 {
            self.count = self.count.saturating_sub(8);
            let byte = low(self.bits.wrapping_shr(self.count));
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0);
            }
        }
    }

    /// Writes a value after `zeros` zeros: its category's code, then its bits (F.1.2.1, F.1.2.2).
    fn term(&mut self, codes: &Codes, zeros: u32, value: i32) {
        let magnitude = value.unsigned_abs();
        let width = u32::BITS.saturating_sub(magnitude.leading_zeros());
        self.code(codes, zeros.wrapping_shl(4) | width);
        let bits = if value < 0_i32 {
            lowest(u32::MAX, width).saturating_sub(magnitude)
        } else {
            magnitude
        };
        self.push(bits, width);
    }
}

/// The codes of BITS and HUFFVAL (Annex C).
fn canonical(counts: &[u8; 16], values: &[u8]) -> Codes {
    let mut codes = vec![(0, 0); 256];
    let mut symbols = values.iter();
    let mut code = 0_u32;
    for (width, &count) in (1_u32..).zip(counts) {
        for &symbol in symbols.by_ref().take(usize::from(count)) {
            if let Some(slot) = codes.get_mut(usize::from(symbol)) {
                *slot = (code, width);
            }
            code = code.saturating_add(1);
        }
        code = code.wrapping_shl(1);
    }
    codes
}

/// Y, Cb and Cr of red, green and blue (T.871, 7).
fn luma_chroma([red, green, blue]: [u8; 3]) -> [u8; 3] {
    YCC.map(|[by_red, by_green, by_blue, offset]| {
        let wide = i32::from(red)
            .wrapping_mul(by_red)
            .wrapping_add(i32::from(green).wrapping_mul(by_green))
            .wrapping_add(i32::from(blue).wrapping_mul(by_blue))
            .wrapping_add(offset);
        u8::try_from(wide >> 16_u32).unwrap_or(u8::MAX)
    })
}

/// The bytes of `value`, highest first.
fn pair(value: u16) -> [u8; 2] {
    let wide = u32::from(value);
    [low(wide.wrapping_shr(8)), low(wide)]
}

/// The terms in zig-zag order, quantised to the nearest (A.3.4), in baseline's range (F.1.2).
fn quantised(transform: &[i32; 64], steps: &[u8; 64]) -> [i32; 64] {
    let mut terms = [0_i32; 64];
    for ((term, &natural), &step) in terms.iter_mut().zip(&ZIGZAG).zip(steps) {
        let value = transform
            .get(usize::from(natural))
            .copied()
            .unwrap_or(0_i32);
        let divisor = u32::from(step).wrapping_shl(FRACTION);
        let rounded = value.unsigned_abs().saturating_add(divisor.wrapping_shr(1));
        let quotient = rounded.checked_div(divisor).unwrap_or_default();
        let magnitude = i32::try_from(quotient).unwrap_or(i32::MAX);
        let signed = if value < 0_i32 {
            magnitude.saturating_neg()
        } else {
            magnitude
        };
        let least = if natural == 0 { -1024_i32 } else { -1023_i32 };
        *term = signed.clamp(least, 1023_i32);
    }
    terms
}

/// The quantisation at `quality`, 50 being Annex K's, by the Independent JPEG Group's scale.
fn scaled(printed: &[u8; 64], quality: u8) -> [u8; 64] {
    let asked = u32::from(quality);
    let percent = if asked < 50 {
        5000_u32.checked_div(asked).unwrap_or_default()
    } else {
        200_u32.saturating_sub(asked.saturating_mul(2))
    };
    printed.map(|step| {
        let hundredths = u32::from(step).saturating_mul(percent).saturating_add(50);
        let nearest = hundredths.checked_div(100).unwrap_or_default();
        u8::try_from(nearest.max(1)).unwrap_or(u8::MAX)
    })
}

/// Appends a marker segment (B.1.1.4).
fn segment(out: &mut Vec<u8>, marker: u8, body: &[u8]) {
    let length = body.len().saturating_add(2);
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&pair(u16::try_from(length).unwrap_or(u16::MAX)));
    out.extend_from_slice(body);
}

/// The forward transform of A.3.3 with `FRACTION` fraction bits.
fn transformed(levels: &[i32; 64]) -> [i32; 64] {
    let mut once = turned(levels);
    for value in &mut once {
        *value = value.wrapping_add(1_i32 << FIRST_PASS.saturating_sub(1)) >> FIRST_PASS;
    }
    turned(&once)
}

/// Each row's transform, as a column.
fn turned(block: &[i32; 64]) -> [i32; 64] {
    let mut result = [0_i32; 64];
    let (lines, _) = block.as_chunks::<8>();
    let (columns, _) = result.as_chunks_mut::<8>();
    for (frequency, (column, weights)) in columns.iter_mut().zip(&COSINES).enumerate() {
        let odd = frequency & 1 == 1;
        for (slot, &[zero, one, two, three, four, five, six, seven]) in column.iter_mut().zip(lines)
        {
            // The cosines mirror about the middle.
            let pairs = [(zero, seven), (one, six), (two, five), (three, four)];
            for ((near, far), weight) in pairs.into_iter().zip(weights) {
                let folded = if odd {
                    near.wrapping_sub(far)
                } else {
                    near.wrapping_add(far)
                };
                *slot = slot.wrapping_add(folded.wrapping_mul(*weight));
            }
        }
    }
    result
}
