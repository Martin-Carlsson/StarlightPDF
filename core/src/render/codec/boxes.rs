//! The boxes of a JP2 file, which the decoder is never shown (ISO/IEC 15444-1, Annex I).

use crate::PdfError;
use crate::render::codec::{self, Implied, Reader};
use crate::render::job::Job;

/// The most channels: CMYK and opacity.
pub(super) const CHANNELS: usize = 5;

/// The first bytes of a bare codestream: SOC and SIZ.
pub(super) const START: &[u8] = &[0xFF, 0x4F, 0xFF, 0x51];

/// One channel of the image.
pub(super) struct Channel {
    /// The component it is read from.
    pub component: usize,
    /// The palette's column in 8 bits, by index.
    pub table: Option<Vec<u8>>,
}

/// What the boxes say.
#[derive(Default)]
pub(super) struct File<'data> {
    /// Where the codestream starts in the data.
    pub at: usize,
    /// The codestream.
    pub codestream: &'data [u8],
    /// `cdef`.
    pub definitions: Option<&'data [u8]>,
    /// The colour space of the first `colr` that names one known here.
    pub implied: Option<Implied>,
    /// `cmap`.
    pub mapping: Option<&'data [u8]>,
    /// `pclr`.
    pub palette: Option<&'data [u8]>,
    /// The bytes of the profile held.
    pub profile: u64,
}

/// The channels, colour first, then opacity.
pub(super) struct Layout {
    /// The channel that is opacity.
    pub alpha: Option<usize>,
    /// The channels.
    pub channels: Vec<Channel>,
    /// Colour is premultiplied by it.
    pub premultiplied: bool,
}

/// A palette (I.5.3.4).
struct Palette<'data> {
    /// Its columns' depths, less 1.
    depths: &'data [u8],
    /// The rows.
    entries: usize,
    /// Their values.
    values: &'data [u8],
    /// The bytes of a row.
    width: usize,
}

impl Palette<'_> {
    /// Column `column` in 8 bits.
    fn table(&self, job: &Job<'_>, column: usize) -> Result<Vec<u8>, PdfError> {
        let depth = self
            .depths
            .get(column)
            .ok_or_else(|| job.objects.malformed())?;
        let bits = u32::from(*depth).saturating_add(1);
        let skipped: usize = self
            .depths
            .iter()
            .take(column)
            .map(|before| usize::from(*before).saturating_add(1).div_ceil(8))
            .sum();
        let end = skipped.saturating_add(usize::from(*depth).saturating_add(1).div_ceil(8));
        let top = 1_u64.wrapping_shl(bits).saturating_sub(1);
        let mut table = codec::reserved(job, codec::wide(self.entries))?;
        for row in self.values.chunks_exact(self.width) {
            let raw = row
                .get(skipped..end)
                .unwrap_or_default()
                .iter()
                .fold(0_u64, |value, byte| {
                    value.wrapping_shl(8) | u64::from(*byte)
                })
                .min(top);
            let scaled = raw
                .saturating_mul(255)
                .saturating_add(top >> 1_u32)
                .checked_div(top)
                .unwrap_or(0);
            table.push(u8::try_from(scaled).unwrap_or(u8::MAX));
        }
        Ok(table)
    }
}

/// The colour space of a `colr` box (I.5.3.3); none where no enumerated or ICC space is known.
fn colour(job: &Job<'_>, body: &[u8]) -> Result<Option<Implied>, PdfError> {
    let mut reader = Reader::new(body);
    let method = reader.byte();
    if reader.take(2).is_none() {
        return Ok(None);
    }
    Ok(match method {
        Some(1) => reader.long().and_then(enumerated),
        Some(2 | 3) if reader.left() > 0 => {
            let profile = reader.rest();
            let mut held = codec::reserved(job, codec::wide(profile.len()))?;
            held.extend_from_slice(profile);
            Some(Implied::Profile(held))
        }
        Some(_) | None => None,
    })
}

/// Orders `channels` by `cdef` (I.5.3.6): colour by association, then one opacity.
fn defined(job: &Job<'_>, layout: &mut Layout, body: &[u8]) -> Result<(), PdfError> {
    let count = layout.channels.len();
    let Some(kinds) = definitions(body, count) else {
        return job.warn("7.4.9", "JPX channel definitions invalid");
    };
    let mut colours = Vec::with_capacity(count);
    let mut opacities = Vec::with_capacity(count);
    for (index, channel) in layout.channels.drain(..).enumerate() {
        let [kind, association] = kinds.get(index).copied().unwrap_or_default();
        // 0 is the whole image and 65,535 none: neither is a colour's place.
        let place = if association == 0 {
            u16::MAX
        } else {
            association
        };
        if kind == 1 || kind == 2 {
            opacities.push((association != 0, kind, channel));
        } else {
            colours.push((place, channel));
        }
    }
    colours.sort_by_key(|entry| entry.0);
    opacities.sort_by_key(|entry| entry.0);
    layout
        .channels
        .extend(colours.into_iter().map(|entry| entry.1));
    let mut rest = opacities.into_iter();
    if let Some((_, kind, channel)) = rest.next() {
        layout.alpha = Some(layout.channels.len());
        layout.premultiplied = kind == 2;
        layout.channels.push(channel);
    }
    if rest.next().is_some() {
        job.warn("7.4.9", "JPX has several opacity channels: one is used")?;
    }
    Ok(())
}

/// The type and association of each of `count` channels; none when the box is invalid or leaves
/// no colour.
fn definitions(body: &[u8], count: usize) -> Option<Vec<[u16; 2]>> {
    let mut reader = Reader::new(body);
    let listed = reader.short()?;
    if listed == 0 || usize::from(listed) > count {
        return None;
    }
    let mut found: Vec<Option<[u16; 2]>> = vec![None; count];
    for _ in 0..listed {
        let channel = usize::from(reader.short()?);
        let kind = [reader.short()?, reader.short()?];
        let slot = found.get_mut(channel)?;
        if slot.is_some() {
            return None;
        }
        *slot = Some(kind);
    }
    let kinds: Vec<[u16; 2]> = found
        .into_iter()
        .zip(1_u16..)
        .map(|(kind, place)| kind.unwrap_or([0, place]))
        .collect();
    kinds
        .iter()
        .any(|kind| !matches!(kind.first(), Some(&(1 | 2))))
        .then_some(kinds)
}

/// The colour spaces of an enumerated `code` that have a place in [`Implied`].
const fn enumerated(code: u32) -> Option<Implied> {
    match code {
        12 => Some(Implied::Cmyk),
        16 => Some(Implied::Rgb),
        17 => Some(Implied::Gray),
        18 => Some(Implied::Ycc),
        _ => None,
    }
}

/// The channels of an image of `components`.
pub(super) fn layout(
    job: &Job<'_>,
    file: &File<'_>,
    components: usize,
) -> Result<Layout, PdfError> {
    let mut layout = Layout {
        alpha: None,
        channels: Vec::with_capacity(CHANNELS),
        premultiplied: false,
    };
    if let Some(body) = file.palette {
        mapped(job, &mut layout.channels, body, file.mapping, components)?;
    } else {
        layout
            .channels
            .extend((0..components).map(|component| Channel {
                component,
                table: None,
            }));
    }
    if let Some(body) = file.definitions {
        defined(job, &mut layout, body)?;
    }
    Ok(layout)
}

/// The channels `cmap` makes of the components and the palette `body` (I.5.3.5); without it each
/// column is one, indexed by component 0.
fn mapped(
    job: &Job<'_>,
    channels: &mut Vec<Channel>,
    body: &[u8],
    mapping: Option<&[u8]>,
    components: usize,
) -> Result<(), PdfError> {
    let malformed = || job.objects.malformed();
    let found = palette(body).ok_or_else(malformed)?;
    let count = mapping.map_or(found.depths.len(), |map| map.len() >> 2_u32);
    if count > CHANNELS {
        return Err(job.unsupported());
    }
    if count == 0 || mapping.is_some_and(|map| map.len() & 3 != 0) {
        return Err(malformed());
    }
    let mut entries = mapping.map(Reader::new);
    for index in 0..count {
        let (component, column) = match entries.as_mut() {
            None => (0, Some(index)),
            Some(reader) => {
                let component = usize::from(reader.short().ok_or_else(malformed)?);
                let kind = reader.byte().ok_or_else(malformed)?;
                let column = usize::from(reader.byte().ok_or_else(malformed)?);
                match kind {
                    0 => (component, None),
                    1 => (component, Some(column)),
                    _ => return Err(malformed()),
                }
            }
        };
        if component >= components {
            return Err(malformed());
        }
        channels.push(Channel {
            component,
            table: column.map(|place| found.table(job, place)).transpose()?,
        });
    }
    Ok(())
}

/// The next box, its type and body; one longer than the data is what there is of it.
fn next<'data>(reader: &mut Reader<'data>) -> Option<(&'data [u8], &'data [u8])> {
    let length = reader.long()?;
    let kind = reader.take(4)?;
    let size = match length {
        0 => reader.left(),
        1 => {
            let (high, low) = (reader.long()?, reader.long()?);
            let whole = (u64::from(high) << 32_u32) | u64::from(low);
            usize::try_from(whole.checked_sub(16)?).unwrap_or(usize::MAX)
        }
        _ => usize::try_from(length.checked_sub(8)?).unwrap_or(usize::MAX),
    };
    let body = reader.take(size.min(reader.left()))?;
    Some((kind, body))
}

/// The palette in `body`.
fn palette(body: &[u8]) -> Option<Palette<'_>> {
    let mut reader = Reader::new(body);
    let entries = usize::from(reader.short()?);
    let columns = usize::from(reader.byte()?);
    let depths = reader.take(columns)?;
    // Signed columns and depths over 38 bits are no colours.
    if entries == 0 || columns == 0 || depths.iter().any(|depth| *depth >= 38) {
        return None;
    }
    let width = depths
        .iter()
        .map(|depth| usize::from(*depth).saturating_add(1).div_ceil(8))
        .sum::<usize>();
    let values = reader.take(entries.checked_mul(width)?)?;
    Some(Palette {
        depths,
        entries,
        values,
        width,
    })
}

/// What `data` holds: the boxes of a JP2 or JPX file, or a bare codestream.
pub(super) fn read<'data>(job: &Job<'_>, data: &'data [u8]) -> Result<File<'data>, PdfError> {
    let mut file = File::default();
    if data.starts_with(START) {
        file.codestream = data;
        return Ok(file);
    }
    let mut reader = Reader::new(data);
    let mut header = None;
    let mut codestream = None;
    while let Some((kind, body)) = next(&mut reader) {
        match kind {
            b"jp2h" if header.is_none() => header = Some(body),
            b"jp2c" if codestream.is_none() => {
                file.at = reader.at.saturating_sub(body.len());
                codestream = Some(body);
            }
            _ => {}
        }
    }
    file.codestream = codestream.ok_or_else(|| job.objects.malformed())?;
    let mut children = Reader::new(header.unwrap_or_default());
    while let Some((kind, body)) = next(&mut children) {
        match kind {
            b"colr" if file.implied.is_none() => {
                file.implied = colour(job, body)?;
                file.profile = codec::wide(body.len());
            }
            b"pclr" if file.palette.is_none() => file.palette = Some(body),
            b"cmap" if file.mapping.is_none() => file.mapping = Some(body),
            b"cdef" if file.definitions.is_none() => file.definitions = Some(body),
            _ => {}
        }
    }
    Ok(file)
}
