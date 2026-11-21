//! Predictors (ISO 32000-2:2020, 7.4.4.4): TIFF 2 (TIFF 6.0, 14) and PNG (ISO/IEC 15948:2004, 9).

use core::mem::swap;

use crate::pull::Pull;
use crate::{ErrorCode, PdfError};

/// The error operation name.
const OPERATION: &str = "predictor";

/// The most pixels a row may hold.
const MAX_COLUMNS: u64 = 0x1_0000;

/// The most components a pixel may hold: a `DeviceN` space's most (ISO 32000-2:2020, Annex C).
const MAX_COLORS: u64 = 32;

/// Bytes a row grows by as its data arrives.
const GROWTH: usize = 4096;

/// Undoes a predictor.
#[derive(Debug)]
pub(crate) struct Predictor<P> {
    /// Bits per component.
    bits: u32,
    /// Components per pixel.
    colors: usize,
    /// Components per row.
    components: usize,
    /// The row being served, grown as its data arrives.
    current: Vec<u8>,
    /// The input.
    inner: P,
    /// Bytes per pixel, at least 1, for PNG.
    pixel: usize,
    /// PNG, else TIFF.
    png: bool,
    /// The previous row; empty before the first.
    previous: Vec<u8>,
    /// How much of `current` is served.
    served: usize,
    /// Bytes per row.
    width: usize,
}

impl<P: Pull> Predictor<P> {
    /// Reads the next row into `current`; `false` at the end of the data.
    fn fetch(&mut self) -> Result<bool, PdfError> {
        swap(&mut self.previous, &mut self.current);
        let mut kind = [0_u8; 1];
        if self.png && self.inner.pull(&mut kind)? == 0 {
            return Ok(false);
        }
        let mut filled = 0_usize;
        while filled < self.width {
            if filled == self.current.len() {
                self.current
                    .resize(filled.saturating_add(GROWTH).min(self.width), 0);
            }
            let rest = self.current.get_mut(filled..).ok_or_else(malformed)?;
            let read = self.inner.pull(rest)?;
            if read == 0 {
                return if filled == 0 && !self.png {
                    self.current.clear();
                    Ok(false)
                } else {
                    Err(malformed())
                };
            }
            filled = filled.wrapping_add(read);
        }
        if self.png {
            self.png_row(kind.first().copied().ok_or_else(malformed)?)?;
        } else {
            self.tiff_row()?;
        }
        Ok(true)
    }

    /// Undoes the PNG filter of row type `kind` on `current` (ISO/IEC 15948:2004, 9.2).
    fn png_row(&mut self, kind: u8) -> Result<(), PdfError> {
        let pixel = self.pixel;
        let row = &mut self.current;
        let above = &self.previous;
        let at = |bytes: &[u8], index: Option<usize>| {
            index.and_then(|at| bytes.get(at)).copied().unwrap_or(0)
        };
        for index in 0..row.len() {
            let before = index.checked_sub(pixel);
            let left = at(row, before);
            let upper = at(above, Some(index));
            let predicted = match kind {
                0 => 0,
                1 => left,
                2 => upper,
                3 => u8::try_from(
                    u16::from(left)
                        .wrapping_add(u16::from(upper))
                        .wrapping_shr(1),
                )
                .ok()
                .ok_or_else(malformed)?,
                4 => paeth(left, upper, at(above, before)),
                _ => return Err(malformed()),
            };
            if let Some(byte) = row.get_mut(index) {
                *byte = byte.wrapping_add(predicted);
            }
        }
        Ok(())
    }

    /// Undoes TIFF predictor 2 on `current`: each component adds the one a pixel before it.
    fn tiff_row(&mut self) -> Result<(), PdfError> {
        let width = self.bits;
        let mask = u16::MAX.wrapping_shr(16_u32.wrapping_sub(width));
        for index in self.colors..self.components {
            let left = component(&self.current, index.wrapping_sub(self.colors), width)?;
            let own = component(&self.current, index, width)?;
            set_component(
                &mut self.current,
                index,
                width,
                own.wrapping_add(left) & mask,
            )?;
        }
        Ok(())
    }

    /// Undoes `predictor` (2 for TIFF, 10 to 15 for PNG) on rows of `columns` pixels of `colors`
    /// components of `bits_per_component` bits.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Malformed`] for a parameter out of range, [`ErrorCode::LimitExceeded`] for
    /// more than [`MAX_COLUMNS`] columns or [`MAX_COLORS`] colors.
    pub(crate) fn with_parameters(
        inner: P,
        predictor: i64,
        colors: i64,
        bits_per_component: i64,
        columns: i64,
    ) -> Result<Self, PdfError> {
        let png = match predictor {
            2 => false,
            10..=15 => true,
            _ => return Err(malformed()),
        };
        if !matches!(bits_per_component, 1 | 2 | 4 | 8 | 16) {
            return Err(malformed());
        }
        let positive = |value: i64| u64::try_from(value).ok().filter(|&count| count > 0);
        let colors_count = positive(colors).ok_or_else(malformed)?;
        let columns_count = positive(columns).ok_or_else(malformed)?;
        let bits = positive(bits_per_component).ok_or_else(malformed)?;
        let limit = || PdfError::new(ErrorCode::LimitExceeded, OPERATION);
        if columns_count > MAX_COLUMNS || colors_count > MAX_COLORS {
            return Err(limit());
        }
        let pixel_bits = colors_count.checked_mul(bits).ok_or_else(limit)?;
        let row = pixel_bits
            .checked_mul(columns_count)
            .and_then(|row_bits| row_bits.checked_add(7))
            .ok_or_else(limit)?
            >> 3_u32;
        let width = usize::try_from(row).ok().ok_or_else(limit)?;
        Ok(Self {
            bits: u32::try_from(bits).ok().ok_or_else(malformed)?,
            colors: usize::try_from(colors_count).ok().ok_or_else(limit)?,
            components: usize::try_from(colors_count.saturating_mul(columns_count))
                .ok()
                .ok_or_else(limit)?,
            current: Vec::new(),
            inner,
            pixel: usize::try_from(pixel_bits.wrapping_add(7) >> 3_u32)
                .ok()
                .ok_or_else(limit)?,
            png,
            previous: Vec::new(),
            served: 0,
            width,
        })
    }
}

impl<P: Pull> Pull for Predictor<P> {
    fn pull(&mut self, buf: &mut [u8]) -> Result<usize, PdfError> {
        let mut filled = 0_usize;
        while filled < buf.len() {
            if self.served >= self.current.len() {
                if !self.fetch()? {
                    break;
                }
                self.served = 0;
            }
            let source = self.current.get(self.served..).ok_or_else(malformed)?;
            let target = buf.get_mut(filled..).ok_or_else(malformed)?;
            let count = source.len().min(target.len());
            let from = source.get(..count).ok_or_else(malformed)?;
            target
                .get_mut(..count)
                .ok_or_else(malformed)?
                .copy_from_slice(from);
            filled = filled.wrapping_add(count);
            self.served = self.served.wrapping_add(count);
        }
        Ok(filled)
    }
}

/// Component `index` of `row`, of `width` bits, packed from the most significant bit.
fn component(row: &[u8], index: usize, width: u32) -> Result<u16, PdfError> {
    let (at, shift) = place(index, width)?;
    let high = u16::from(*row.get(at).ok_or_else(malformed)?);
    if width == 16 {
        let low = u16::from(*row.get(at.wrapping_add(1)).ok_or_else(malformed)?);
        return Ok(high.wrapping_shl(8) | low);
    }
    Ok(high.wrapping_shr(shift) & u16::MAX.wrapping_shr(16_u32.wrapping_sub(width)))
}

/// The error for any defect in the data.
fn malformed() -> PdfError {
    PdfError::new(ErrorCode::Malformed, OPERATION)
}

/// The Paeth predictor (ISO/IEC 15948:2004, 9.4).
fn paeth(left: u8, upper: u8, upper_left: u8) -> u8 {
    let pa = u16::from(upper.abs_diff(upper_left));
    let pb = u16::from(left.abs_diff(upper_left));
    let pc = u16::from(left)
        .wrapping_add(u16::from(upper))
        .abs_diff(u16::from(upper_left).wrapping_mul(2));
    if pa <= pb && pa <= pc {
        left
    } else if pb <= pc {
        upper
    } else {
        upper_left
    }
}

/// The byte holding component `index` of `width` bits and its shift from the byte's low end.
fn place(index: usize, width: u32) -> Result<(usize, u32), PdfError> {
    let bit = index
        .checked_mul(usize::try_from(width).ok().ok_or_else(malformed)?)
        .ok_or_else(malformed)?;
    let offset = u32::try_from(bit & 7).ok().ok_or_else(malformed)?;
    Ok((
        bit >> 3_u32,
        8_u32.saturating_sub(width).saturating_sub(offset),
    ))
}

/// Sets component `index` of `row`, of `width` bits, to `value`.
fn set_component(row: &mut [u8], index: usize, width: u32, value: u16) -> Result<(), PdfError> {
    let (at, shift) = place(index, width)?;
    if width == 16 {
        *row.get_mut(at).ok_or_else(malformed)? = u8::try_from(value.wrapping_shr(8))
            .ok()
            .ok_or_else(malformed)?;
        *row.get_mut(at.wrapping_add(1)).ok_or_else(malformed)? =
            u8::try_from(value & 0xFF).ok().ok_or_else(malformed)?;
        return Ok(());
    }
    let mask = u16::MAX
        .wrapping_shr(16_u32.wrapping_sub(width))
        .wrapping_shl(shift);
    let byte = row.get_mut(at).ok_or_else(malformed)?;
    let merged = (u16::from(*byte) & !mask) | (value.wrapping_shl(shift) & mask);
    *byte = u8::try_from(merged & 0xFF).ok().ok_or_else(malformed)?;
    Ok(())
}
