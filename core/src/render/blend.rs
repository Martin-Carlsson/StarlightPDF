//! The blend modes (ISO 32000-2:2020, 11.3.5).

use crate::render::fixed::unit;

/// One, in unit values.
const ONE: i64 = 0xFFFF;

/// A blend mode.
#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(crate) enum Mode {
    /// `Color`.
    Color,
    /// `ColorBurn`.
    ColorBurn,
    /// `ColorDodge`.
    ColorDodge,
    /// `Darken`.
    Darken,
    /// `Difference`.
    Difference,
    /// `Exclusion`.
    Exclusion,
    /// `HardLight`.
    HardLight,
    /// `Hue`.
    Hue,
    /// `Lighten`.
    Lighten,
    /// `Luminosity`.
    Luminosity,
    /// `Multiply`.
    Multiply,
    /// `Normal`, also `Compatible`.
    #[default]
    Normal,
    /// `Overlay`.
    Overlay,
    /// `Saturation`.
    Saturation,
    /// `Screen`.
    Screen,
    /// `SoftLight`.
    SoftLight,
}

/// What is left of one beside `value`.
const fn complement(value: u16) -> u16 {
    u16::MAX.saturating_sub(value)
}

/// `ColorBurn` of one component, as ISO 32000-2 defines it.
fn burn(below: u16, above: u16) -> u16 {
    let rest = complement(below);
    if below == u16::MAX {
        u16::MAX
    } else if rest >= above {
        0
    } else {
        complement(ratio(rest, above))
    }
}

/// `ColorDodge` of one component, as ISO 32000-2 defines it.
fn dodge(below: u16, above: u16) -> u16 {
    let rest = complement(above);
    if below == 0 {
        0
    } else if below >= rest {
        u16::MAX
    } else {
        ratio(below, rest)
    }
}

/// A separable mode, by component.
fn each(backdrop: [u16; 3], source: [u16; 3], function: fn(u16, u16) -> u16) -> [u16; 3] {
    let [red, green, blue] = backdrop;
    let [over_red, over_green, over_blue] = source;
    [
        function(red, over_red),
        function(green, over_green),
        function(blue, over_blue),
    ]
}

/// `HardLight` of one component.
fn hard(below: u16, above: u16) -> u16 {
    let double = u32::from(above).saturating_mul(2);
    u16::try_from(double).map_or_else(
        |_error| screen(below, above.saturating_sub(complement(above))),
        |low| unit(below, low),
    )
}

/// The luminosity of `color`: 0.30 R + 0.59 G + 0.11 B (11.3.5.3).
pub(crate) fn luminosity(color: [u16; 3]) -> u16 {
    let [red, green, blue] = color.map(u32::from);
    let sum = red
        .wrapping_mul(19_661)
        .wrapping_add(green.wrapping_mul(38_666))
        .wrapping_add(blue.wrapping_mul(7209))
        .wrapping_add(0x8000);
    u16::try_from(sum >> 16_u32).unwrap_or(u16::MAX)
}

/// `source` blended with `backdrop` by `mode`.
pub(crate) fn mix(mode: Mode, backdrop: [u16; 3], source: [u16; 3]) -> [u16; 3] {
    match mode {
        Mode::Color => with_luminosity(source, luminosity(backdrop)),
        Mode::ColorBurn => each(backdrop, source, burn),
        Mode::ColorDodge => each(backdrop, source, dodge),
        Mode::Darken => each(backdrop, source, u16::min),
        Mode::Difference => each(backdrop, source, u16::abs_diff),
        Mode::Exclusion => each(backdrop, source, |below, above| {
            unit(below, complement(above)).saturating_add(unit(above, complement(below)))
        }),
        Mode::HardLight => each(backdrop, source, hard),
        Mode::Hue => with_luminosity(
            with_saturation(source, saturation(backdrop)),
            luminosity(backdrop),
        ),
        Mode::Lighten => each(backdrop, source, u16::max),
        Mode::Luminosity => with_luminosity(backdrop, luminosity(source)),
        Mode::Multiply => each(backdrop, source, unit),
        Mode::Normal => source,
        Mode::Overlay => each(backdrop, source, |below, above| hard(above, below)),
        Mode::Saturation => with_luminosity(
            with_saturation(backdrop, saturation(source)),
            luminosity(backdrop),
        ),
        Mode::Screen => each(backdrop, source, screen),
        Mode::SoftLight => each(backdrop, source, soft),
    }
}

/// The mode called `name`.
pub(crate) const fn named(name: &[u8]) -> Option<Mode> {
    Some(match name {
        b"Color" => Mode::Color,
        b"ColorBurn" => Mode::ColorBurn,
        b"ColorDodge" => Mode::ColorDodge,
        b"Compatible" | b"Normal" => Mode::Normal,
        b"Darken" => Mode::Darken,
        b"Difference" => Mode::Difference,
        b"Exclusion" => Mode::Exclusion,
        b"HardLight" => Mode::HardLight,
        b"Hue" => Mode::Hue,
        b"Lighten" => Mode::Lighten,
        b"Luminosity" => Mode::Luminosity,
        b"Multiply" => Mode::Multiply,
        b"Overlay" => Mode::Overlay,
        b"Saturation" => Mode::Saturation,
        b"Screen" => Mode::Screen,
        b"SoftLight" => Mode::SoftLight,
        _ => return None,
    })
}

/// `part` as a share of `whole`, rounded to the nearest, halves up; 0 of none.
pub(crate) fn ratio(part: u16, whole: u16) -> u16 {
    let share = u32::from(part)
        .wrapping_mul(0xFFFF)
        .wrapping_add(u32::from(whole >> 1_u32))
        .checked_div(u32::from(whole))
        .unwrap_or(0);
    u16::try_from(share).unwrap_or(u16::MAX)
}

/// The range of the components.
fn saturation(color: [u16; 3]) -> u16 {
    let [red, green, blue] = color;
    red.max(green)
        .max(blue)
        .saturating_sub(red.min(green).min(blue))
}

/// `value` times `factor` over a positive `divisor`, rounded to the nearest, halves up.
fn scaled(value: i64, factor: i64, divisor: i64) -> i64 {
    value
        .saturating_mul(factor)
        .saturating_mul(2)
        .saturating_add(divisor)
        .checked_div_euclid(divisor.saturating_mul(2))
        .unwrap_or(0)
}

/// `Screen` of one component.
fn screen(below: u16, above: u16) -> u16 {
    above.saturating_add(unit(below, complement(above)))
}

/// `SoftLight` of one component.
fn soft(below: u16, above: u16) -> u16 {
    let rest = complement(above);
    if above <= rest {
        let less = unit(rest.saturating_sub(above), unit(below, complement(below)));
        return below.saturating_sub(less);
    }
    let wide = i64::from(below);
    let lightest = if wide.saturating_mul(4) <= ONE {
        // ((16b - 12) b + 4) b
        let squared = ONE.saturating_mul(ONE);
        let inner = wide
            .saturating_mul(16)
            .saturating_sub(ONE.saturating_mul(12))
            .saturating_mul(wide)
            .saturating_add(squared.saturating_mul(4));
        unit_of(scaled(inner, wide, squared))
    } else {
        let square = u32::from(below).wrapping_mul(0xFFFF);
        let root = square.isqrt();
        let up = square.saturating_sub(root.wrapping_mul(root)) > root;
        u16::try_from(root.saturating_add(u32::from(up))).unwrap_or(u16::MAX)
    };
    let more = unit(above.saturating_sub(rest), lightest.saturating_sub(below));
    below.saturating_add(more)
}

/// `value` held to 0 to one.
fn unit_of(value: i64) -> u16 {
    u16::try_from(value.clamp(0, ONE)).unwrap_or(u16::MAX)
}

/// `color` with the luminosity `wanted`, clipped into range (`SetLum` and `ClipColor`).
fn with_luminosity(color: [u16; 3], wanted: u16) -> [u16; 3] {
    let light = i64::from(wanted);
    let shift = light.saturating_sub(i64::from(luminosity(color)));
    let [red, green, blue] = color.map(|part| i64::from(part).saturating_add(shift));
    let (low, high) = (red.min(green).min(blue), red.max(green).max(blue));
    let (factor, divisor) = if low < 0 {
        (light, light.saturating_sub(low))
    } else if high > ONE {
        (ONE.saturating_sub(light), high.saturating_sub(light))
    } else {
        return [red, green, blue].map(unit_of);
    };
    [red, green, blue].map(|part| {
        let away = part.saturating_sub(light);
        unit_of(light.saturating_add(scaled(away, factor, divisor)))
    })
}

/// `color` with the saturation `wanted` (`SetSat`).
fn with_saturation(color: [u16; 3], wanted: u16) -> [u16; 3] {
    let [red, green, blue] = color;
    let low = red.min(green).min(blue);
    let range = saturation(color);
    color.map(|part| unit(ratio(part.saturating_sub(low), range), wanted))
}
