//! Peak allocation of saving.

extern crate alloc;

use alloc::sync::Arc;
use core::error::Error;

use starlightpdf::ErrorCode;
use starlightpdf::document::{Document, OpenOptions, SaveOptions};
use starlightpdf::input::{BytesSourceOptions, Source};
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf_memory::{adler_run, adler_value, peak};

/// Most bytes saving the file of four object streams, each two 4 MiB predictor rows, may hold at once: the 10,592,644
/// main holds, one stream's rows at a time, and 1,990,268 bytes more.
const PREDICTED_BOUND: usize = 0xC0_0000;

/// The bytes of a row of the PNG predictor of 65,536 columns of 32 colours of 16 bits (ISO 32000-2:2020, 7.4.4.4).
const ROW: usize = 0x40_0000;

/// What the steps of the test return.
type Made<T> = Result<T, Box<dyn Error>>;

/// Bits gathered into bytes.
#[derive(Default)]
struct Bits {
    /// The bits of the byte begun.
    byte: u8,
    /// Whole bytes.
    bytes: Vec<u8>,
    /// Bits in the byte begun.
    count: u32,
}

impl Bits {
    /// Appends a Huffman code of `width` bits, its highest first (RFC 1951, 3.1.1).
    fn code(&mut self, code: u16, width: u32) {
        for place in (0..width).rev() {
            self.push(code.wrapping_shr(place) & 1 == 1);
        }
    }

    /// Appends a bit, into each byte's lowest first.
    fn push(&mut self, one: bool) {
        if one {
            self.byte |= 1_u8.wrapping_shl(self.count);
        }
        self.count = self.count.saturating_add(1);
        if self.count == 8 {
            self.bytes.push(self.byte);
            self.byte = 0;
            self.count = 0;
        }
    }
}

/// A zlib stream of `data` in one fixed-Huffman block: each run's first byte a literal, then repeats of the byte before
/// (RFC 1951, 3.2.6).
fn zlib(data: &[u8]) -> Made<Vec<u8>> {
    let mut bits = Bits::default();
    bits.bytes.extend_from_slice(&[0x78, 0x01]);
    for bit in [true, true, false] {
        bits.push(bit);
    }
    let mut sum = (1, 0);
    for run in data.chunk_by(|one, other| one == other) {
        let byte = *run.first().ok_or("a run")?;
        let literal = |out: &mut Bits| {
            if byte < 144 {
                out.code(u16::from(byte).saturating_add(0x30), 8);
            } else {
                out.code(u16::from(byte).saturating_add(0x100), 9);
            }
        };
        literal(&mut bits);
        let mut rest = run.len().saturating_sub(1);
        while rest >= 258 {
            // Length 258 is code 285; distance 1 is code 0.
            bits.code(0xC5, 8);
            bits.code(0, 5);
            rest = rest.saturating_sub(258);
        }
        for _ in 0..rest {
            literal(&mut bits);
        }
        sum = adler_run(sum, byte, u64::try_from(run.len())?).ok_or("Adler-32")?;
    }
    bits.code(0, 7);
    while bits.count > 0 {
        bits.push(false);
    }
    let value = adler_value(sum).ok_or("Adler-32")?;
    for shift in [24_u32, 16, 8, 0] {
        bits.bytes
            .push(u8::try_from(value.wrapping_shr(shift) & 0xFF)?);
    }
    Ok(bits.bytes)
}

/// An object stream of objects `first` and `first` + 1, the second in its second row, behind a Flate stage with the
/// predictor of [`ROW`]s (ISO 32000-2:2020, 7.4.4.4, 7.5.7).
fn container(first: u32) -> Made<Vec<u8>> {
    let start = 64;
    let at = ROW.saturating_add(100).saturating_sub(start);
    let mut text = vec![b' '; ROW.saturating_mul(2)];
    let mut put = |offset: usize, bytes: &[u8]| -> Made<()> {
        text.get_mut(offset..offset.saturating_add(bytes.len()))
            .ok_or("a place")?
            .copy_from_slice(bytes);
        Ok(())
    };
    put(
        0,
        format!("{first} 0 {} {at}\n", first.saturating_add(1)).as_bytes(),
    )?;
    put(start, b"(m0aa)")?;
    put(start.saturating_add(at), b"(m1bb)")?;
    // Each row's filter type, None (PNG, 9.2).
    let mut rows = Vec::new();
    for row in text.chunks(ROW) {
        rows.push(0);
        rows.extend_from_slice(row);
    }
    let data = zlib(&rows)?;
    Ok([
        format!(
            "<< /Type /ObjStm /N 2 /First {start} /Length {} /Filter /FlateDecode /DecodeParms << /Predictor 12 \
             /Columns 65536 /Colors 32 /BitsPerComponent 16 >> >>\nstream\n",
            data.len()
        )
        .as_bytes(),
        &data,
        b"\nendstream",
    ]
    .concat())
}

/// A file of one page whose catalog lists the first member of each of four object streams, then each second member, by a
/// cross-reference stream (ISO 32000-2:2020, 7.5.7, 7.5.8).
fn predicted_file() -> Made<Vec<u8>> {
    let members = [100_u32, 102, 104, 106];
    let listed = members
        .iter()
        .chain(&members)
        .zip((0_u8..8).map(|place| u32::from(place >= 4)))
        .map(|(first, second)| format!("{} 0 R", first.saturating_add(second)))
        .collect::<Vec<_>>()
        .join(" ");
    let mut plain = vec![
        (
            1_u32,
            format!("<< /Type /Catalog /Pages 2 0 R /Arr [{listed}] >>").into_bytes(),
        ),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
        (
            3,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>".to_vec(),
        ),
    ];
    for (number, first) in (10_u32..).zip(members) {
        plain.push((number, container(first)?));
    }
    let size = 108_usize;
    let mut file = b"%PDF-1.7\n".to_vec();
    // Each entry: type, offset or container, generation or index (7.5.8.3).
    let mut rows = vec![[0_u8; 7]; size];
    let entry = |kind: u8, field: usize, index: u8| -> Made<[u8; 7]> {
        let value = u32::try_from(field)?;
        let byte = |shift: u32| u8::try_from(value.wrapping_shr(shift) & 0xFF);
        Ok([kind, byte(24)?, byte(16)?, byte(8)?, byte(0)?, 0, index])
    };
    for (number, body) in plain {
        *rows.get_mut(usize::try_from(number)?).ok_or("an entry")? = entry(1, file.len(), 0)?;
        file.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        file.extend(body);
        file.extend_from_slice(b"\nendobj\n");
    }
    for (container, first) in (10_usize..).zip(members) {
        for index in 0..2_u8 {
            let number = usize::try_from(first)?.saturating_add(usize::from(index));
            *rows.get_mut(number).ok_or("an entry")? = entry(2, container, index)?;
        }
    }
    let start = file.len();
    *rows.get_mut(size.saturating_sub(1)).ok_or("an entry")? = entry(1, start, 0)?;
    let table = rows.concat();
    file.extend_from_slice(
        format!(
            "{} 0 obj\n<< /Type /XRef /Size {size} /W [1 4 2] /Root 1 0 R /Length {} >>\nstream\n",
            size.saturating_sub(1),
            table.len()
        )
        .as_bytes(),
    );
    file.extend(table);
    file.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{start}\n%%EOF\n").as_bytes());
    Ok(file)
}

/// Saving a file whose members are read from four object streams in turn, each stream's two predictor rows 4 MiB, holds
/// no more than [`PREDICTED_BOUND`] (ISO 32000-2:2020, 7.4.4.4, 7.5.7).
#[test]
fn save_of_members_behind_predictors_is_bounded() {
    let found = predicted_file()
        .map_err(|error| error.to_string())
        .map(|file| {
            let mut source = BytesSourceOptions::default();
            source.bytes = Arc::new(file);
            peak(|| -> Result<(), ErrorCode> {
                let attempt = || -> Result<(), starlightpdf::PdfError> {
                    let document = Document::open(
                        &Source::from_bytes(&source)?,
                        None,
                        &OpenOptions::default(),
                    )?;
                    let sink = Sink::in_memory(&MemorySinkOptions::default())?;
                    drop(document.save(&sink, None, &SaveOptions::default())?);
                    Ok(())
                };
                attempt().map_err(|error| error.code())
            })
        });
    assert!(matches!(&found, Ok((Ok(()), _))), "outcome {found:?}");
    let top = found.map_or(0, |(_, bytes)| bytes);
    assert!((1..=PREDICTED_BOUND).contains(&top), "peak {top} bytes");
}
