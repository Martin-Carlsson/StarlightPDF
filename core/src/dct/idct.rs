//! The inverse DCT, upsampling and colour transform (ITU-T T.81, A.3.3 and A.1.1; T.871, 7).

/// The cosine of pi/16, with [`BITS`] fraction bits.
const C1: i32 = 8035;

/// The cosine of 2 pi/16.
const C2: i32 = 7568;

/// The cosine of 3 pi/16.
const C3: i32 = 6811;

/// The cosine of 4 pi/16, which is also the scale of the DC term.
const C4: i32 = 5793;

/// The cosine of 5 pi/16.
const C5: i32 = 4551;

/// The cosine of 6 pi/16.
const C6: i32 = 3135;

/// The cosine of 7 pi/16.
const C7: i32 = 1598;

/// Fraction bits of the factors here.
const BITS: u32 = 13;

/// The most a value between the two passes may be: past it, no image's, the second could overflow.
const FIRST: i32 = 0x7FFF;

/// Fraction bits kept between the two passes.
const KEPT: u32 = 3;

/// The level shift of 8-bit samples (A.3.1).
const LEVEL: i32 = 128;

/// The factors of T.871's conversion: 1.402, 0.344136, 0.714136 and 1.772.
const YCC: [i32; 4] = [11_485, 2819, 5850, 14_516];

/// Turns interleaved Y, Cb, Cr into R, G, B or, with a fourth component, into C, M, Y beside the
/// K as it is.
pub(super) fn coloured(out: &mut [u8], components: usize) {
    if components == 3 {
        for pixel in out.as_chunks_mut::<3>().0 {
            *pixel = rgb(*pixel);
        }
    } else if components == 4 {
        for pixel in out.as_chunks_mut::<4>().0 {
            let [luma, blue, red, black] = *pixel;
            let [cyan, magenta, yellow] = rgb([luma, blue, red]);
            *pixel = [!cyan, !magenta, !yellow, black];
        }
    } else {
        // T.871 and 7.4.8 define no transform for other counts.
    }
}

/// The samples of the block `coefficients`, scaled by `quant`, into the eight lines of `samples`
/// that are `width` long, from `left` on.
pub(super) fn inverse(
    coefficients: &[i16; 64],
    quant: &[u16; 64],
    samples: &mut [u8],
    (width, left): (usize, usize),
) {
    if coefficients.iter().skip(1).all(|value| *value == 0) {
        let dc = coefficients
            .first()
            .zip(quant.first())
            .map_or(0_i32, |(value, factor)| scaled(*value, *factor));
        let level = [sample(descaled(dc, KEPT)); 8];
        for line in samples.chunks_exact_mut(width) {
            put(line, left, level);
        }
        return;
    }
    let mut rows = [[0_i32; 8]; 8];
    let sources = coefficients.as_chunks::<8>().0;
    let factors = quant.as_chunks::<8>().0;
    for (row, (values, by)) in rows.iter_mut().zip(sources.iter().zip(factors)) {
        let mut input = [0_i32; 8];
        for (slot, (value, factor)) in input.iter_mut().zip(values.iter().zip(by)) {
            *slot = scaled(*value, *factor);
        }
        let shift = BITS.saturating_add(1).saturating_sub(KEPT);
        // Past what any image reaches, kept within it so that the second pass cannot overflow.
        *row = if input.iter().skip(1).all(|value| *value == 0_i32) {
            let [first, ..] = input;
            [descaled(first.wrapping_mul(C4), shift).clamp(-FIRST, FIRST); 8]
        } else {
            transform(input).map(|value| descaled(value, shift).clamp(-FIRST, FIRST))
        };
    }
    let mut lines = [[0_u8; 8]; 8];
    for column in 0..8_usize {
        let input = rows.map(|row| row.get(column).copied().unwrap_or_default());
        let shift = BITS.saturating_add(1).saturating_add(KEPT);
        for (line, value) in lines.iter_mut().zip(transform(input)) {
            if let Some(slot) = line.get_mut(column) {
                *slot = sample(descaled(value, shift));
            }
        }
    }
    for (line, values) in samples.chunks_exact_mut(width).zip(lines) {
        put(line, left, values);
    }
}

/// Copies `source` into every `step`th byte of `out`, each output column taking the sample whose
/// centre is nearest when the component has `part` of `whole` columns.
pub(super) fn upsampled(source: &[u8], out: &mut [u8], step: usize, (part, whole): (usize, usize)) {
    if part == whole {
        for (slot, value) in out.iter_mut().step_by(step).zip(source) {
            *slot = *value;
        }
        return;
    }
    let period = whole.saturating_mul(2);
    let advance = part.saturating_mul(2);
    let mut position = part;
    let mut rest = source.iter().copied();
    let mut current = rest.next().unwrap_or_default();
    for slot in out.iter_mut().step_by(step) {
        if position >= period {
            position = position.saturating_sub(period);
            current = rest.next().unwrap_or(current);
        }
        *slot = current;
        position = position.saturating_add(advance);
    }
}

/// `value` without its lowest `bits`, rounded to nearest, halves up.
const fn descaled(value: i32, bits: u32) -> i32 {
    value
        .wrapping_add(1_i32.wrapping_shl(bits.saturating_sub(1)))
        .wrapping_shr(bits)
}

/// Writes one block's line.
fn put(line: &mut [u8], left: usize, values: [u8; 8]) {
    if let Some(slots) = line.get_mut(left..).and_then(<[u8]>::first_chunk_mut::<8>) {
        *slots = values;
    }
}

/// R, G, B of Y, Cb, Cr.
fn rgb([luma, blue, red]: [u8; 3]) -> [u8; 3] {
    let [red_red, green_blue, green_red, blue_blue] = YCC;
    let base = i32::from(luma).wrapping_shl(BITS);
    let blue_off = i32::from(blue).wrapping_sub(LEVEL);
    let red_off = i32::from(red).wrapping_sub(LEVEL);
    [
        base.wrapping_add(red_off.wrapping_mul(red_red)),
        base.wrapping_sub(blue_off.wrapping_mul(green_blue))
            .wrapping_sub(red_off.wrapping_mul(green_red)),
        base.wrapping_add(blue_off.wrapping_mul(blue_blue)),
    ]
    .map(|value| clamped(descaled(value, BITS)))
}

/// A level-shifted sample.
fn sample(value: i32) -> u8 {
    clamped(value.wrapping_add(LEVEL))
}

/// A coefficient times its quantiser, kept within 16 bits.
fn scaled(value: i16, factor: u16) -> i32 {
    i32::from(value)
        .wrapping_mul(i32::from(factor))
        .clamp(i32::from(i16::MIN), i32::from(i16::MAX))
}

/// A value held within 0 to 255.
fn clamped(value: i32) -> u8 {
    u8::try_from(value.clamp(0, 255)).unwrap_or_default()
}

/// Eight terms of the one-dimensional inverse DCT, times 2 to the power of [`BITS`] plus one.
const fn transform([f0, f1, f2, f3, f4, f5, f6, f7]: [i32; 8]) -> [i32; 8] {
    let sum = f0.wrapping_add(f4).wrapping_mul(C4);
    let difference = f0.wrapping_sub(f4).wrapping_mul(C4);
    let outer = f2.wrapping_mul(C2).wrapping_add(f6.wrapping_mul(C6));
    let inner = f2.wrapping_mul(C6).wrapping_sub(f6.wrapping_mul(C2));
    let even = [
        sum.wrapping_add(outer),
        difference.wrapping_add(inner),
        difference.wrapping_sub(inner),
        sum.wrapping_sub(outer),
    ];
    let odd = [
        f1.wrapping_mul(C1)
            .wrapping_add(f3.wrapping_mul(C3))
            .wrapping_add(f5.wrapping_mul(C5))
            .wrapping_add(f7.wrapping_mul(C7)),
        f1.wrapping_mul(C3)
            .wrapping_sub(f3.wrapping_mul(C7))
            .wrapping_sub(f5.wrapping_mul(C1))
            .wrapping_sub(f7.wrapping_mul(C5)),
        f1.wrapping_mul(C5)
            .wrapping_sub(f3.wrapping_mul(C1))
            .wrapping_add(f5.wrapping_mul(C7))
            .wrapping_add(f7.wrapping_mul(C3)),
        f1.wrapping_mul(C7)
            .wrapping_sub(f3.wrapping_mul(C5))
            .wrapping_add(f5.wrapping_mul(C3))
            .wrapping_sub(f7.wrapping_mul(C1)),
    ];
    let [even0, even1, even2, even3] = even;
    let [odd0, odd1, odd2, odd3] = odd;
    [
        even0.wrapping_add(odd0),
        even1.wrapping_add(odd1),
        even2.wrapping_add(odd2),
        even3.wrapping_add(odd3),
        even3.wrapping_sub(odd3),
        even2.wrapping_sub(odd2),
        even1.wrapping_sub(odd1),
        even0.wrapping_sub(odd0),
    ]
}
