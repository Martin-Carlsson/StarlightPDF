//! PNG output (PNG, W3C, third edition).

use crate::PdfError;
use crate::output::Sink;
use crate::raster::png::crc;
use crate::render::OPERATION;
use crate::render::output::deflate::Deflate;
use crate::render::output::{BLOCK, Format, Settings, buffer, channels, word};

/// An IDAT chunk before its data, the length still to be set.
const IDAT: [u8; 8] = *b"\0\0\0\0IDAT";

/// The signature (5.2).
const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1A\n";

/// The Up filter type (9.2).
const UP: u8 = 2;

/// A PNG being written.
pub(super) struct Png<'sink> {
    /// The compressor.
    deflate: Deflate,
    /// The IDAT chunk being gathered.
    out: Vec<u8>,
    /// The row before, unfiltered.
    previous: Vec<u8>,
    /// Where to.
    sink: &'sink Sink,
}

impl Format for Png<'_> {
    fn finish(&mut self) -> Result<(), PdfError> {
        self.deflate.finish(&mut self.out);
        self.idat()?;
        let mut end = Vec::new();
        chunk(*b"IEND", &[], &mut end);
        self.sink.write(&end, OPERATION)
    }

    fn row(&mut self, samples: &mut [u8]) -> Result<(), PdfError> {
        for (sample, above) in samples.iter_mut().zip(&mut self.previous) {
            let raw = *sample;
            *sample = raw.wrapping_sub(*above);
            *above = raw;
        }
        self.deflate.write(&[UP], &mut self.out);
        for piece in samples.chunks(BLOCK) {
            self.deflate.write(piece, &mut self.out);
            if self.out.len() >= BLOCK {
                self.idat()?;
            }
        }
        Ok(())
    }
}

impl<'sink> Png<'sink> {
    /// Writes the IDAT chunk gathered and starts the next.
    fn idat(&mut self) -> Result<(), PdfError> {
        let length = self.out.len().saturating_sub(IDAT.len());
        if let Some(stored) = self.out.first_chunk_mut::<4>() {
            *stored = word(u32::try_from(length).unwrap_or(u32::MAX));
        }
        let sum = !crc(u32::MAX, self.out.get(4..).unwrap_or_default());
        self.out.extend_from_slice(&word(sum));
        self.sink.write(&self.out, OPERATION)?;
        self.out.clear();
        self.out.extend_from_slice(&IDAT);
        Ok(())
    }

    /// Writes the signature and IHDR.
    pub(super) fn new(sink: &'sink Sink, settings: Settings) -> Result<Self, PdfError> {
        let previous = buffer(settings.width, channels(settings.gray))?;
        let colour = if settings.gray { 0 } else { 2 };
        let mut data = [word(settings.width), word(settings.height)].concat();
        // 8 bits, the colour type, the standard methods, not interlaced (11.2.2).
        data.extend_from_slice(&[8, colour, 0, 0, 0]);
        let mut start = SIGNATURE.to_vec();
        chunk(*b"IHDR", &data, &mut start);
        sink.write(&start, OPERATION)?;
        let mut out = IDAT.to_vec();
        let deflate = Deflate::new(&mut out);
        Ok(Self {
            deflate,
            out,
            previous,
            sink,
        })
    }
}

/// Appends a chunk of type `kind` (5.3).
fn chunk(kind: [u8; 4], data: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&word(u32::try_from(data.len()).unwrap_or(u32::MAX)));
    let from = out.len();
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let sum = !crc(u32::MAX, out.get(from..).unwrap_or_default());
    out.extend_from_slice(&word(sum));
}
