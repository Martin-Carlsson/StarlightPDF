//! `JPXDecode` through hayro-jpeg2000, behind a guard (ISO 32000-2:2020, 7.4.9;
//! ISO/IEC 15444-1).
//!
//! The boxes are read in the core. The codestream's headers, every tile-part's included, are
//! walked as the decoder walks them, then it gets the codestream in empty boxes, so that its
//! palette and colour code never runs.

use alloc::rc::Rc;
use core::mem;

use hayro_jpeg2000::{DecodeSettings, DecodedImage, DecoderContext, Image};

use crate::PdfError;
use crate::render::codec::boxes::{self, CHANNELS, Channel, START};
use crate::render::codec::{self, Held, Info, Reader};
use crate::render::fixed::Fixed;
use crate::render::job::Job;

/// The bytes of the decoder's fixed state.
const BASE: u64 = 0x4_0000;

/// The most bit planes: their passes are [`PASSES`].
const BITPLANES: u8 = 32;

/// The most entries of a progression order change (A.6.6).
const CHANGES: usize = 32;

/// The largest coordinate: sums of two stay far from overflow.
const COORDINATE: u32 = 0x100_0000;

/// Work of a sample for each bit plane past [`SHALLOW`].
const DEEP_WORK: u64 = 4;

/// Work of a byte of the data: the passes read it bit by bit.
const INPUT_WORK: u64 = 8;

/// The most layers the decoder has bits for.
const LAYERS: u16 = 32;

/// The most decomposition levels.
const LEVELS: u8 = 32;

/// The coding passes of a code-block, and so the most segments of one.
const PASSES: u64 = 94;

/// The most bits of a sample: shifts by more overflow in the decoder.
const PRECISION: u8 = 16;

/// The spare bytes the data needs before it, for the boxes the decoder reads.
pub(crate) const ROOM: usize = WRAPPER.len();

/// Work of a sample of up to [`SHALLOW`] bit planes.
const SAMPLE_WORK: u64 = 16;

/// The bit planes of honest data, mostly: each one more has passes that read every sample.
const SHALLOW: u8 = 16;

/// The largest step between precincts on the reference grid.
const STEP: u64 = 0x2000_0000;

/// The most step sizes of a quantization: one a subband.
const STEP_SIZES: usize = 97;

/// The most tiles.
const TILES: u64 = 4096;

/// The most work of one image's structures: tag trees, precincts and packets.
const WORK: u64 = 0x800_0000;

/// Signature, file type, an empty header, so that nothing is converted, and a box of a codestream
/// to the end.
const WRAPPER: &[u8] =
    b"\0\0\0\x0cjP  \r\n\x87\n\0\0\0\x14ftypjp2 \0\0\0\0jp2 \0\0\0\x08jp2h\0\0\0\0jp2c";

/// What the guard found.
struct Frame {
    /// The most bit planes of a sample.
    bitplanes: u8,
    /// The bytes the decoder takes at most.
    bytes: u64,
    /// Data is missing.
    damaged: bool,
    /// Each component's depth.
    depths: Vec<u8>,
    /// Pixels times components, at full resolution.
    samples: u64,
    /// The decoder's work besides the samples'.
    work: u64,
}

/// The coding style a main header gives (A.4.1).
struct Main {
    /// The most bit planes its quantization leaves a sample.
    bitplanes: u8,
    /// The entries of progression changes.
    changes: usize,
    /// The bytes of packed packet headers.
    packed: u64,
    /// The tile every one starts from.
    tile: Tile,
}

/// A component of a tile, as the totals need it.
#[derive(Clone, Copy)]
struct Part {
    /// The tile's columns and rows on the reference grid.
    area: [[u64; 2]; 2],
    /// The layers.
    layers: u64,
    /// The component's subsampling.
    sampling: [u64; 2],
    /// Its coding style.
    style: Style,
}

/// A reference grid and what lies on it (A.5.1).
struct Size {
    /// Each component's depth.
    depths: Vec<u8>,
    /// The image's area: its start and end across and down.
    image: [[u64; 2]; 2],
    /// Each component's subsampling.
    sampling: Vec<[u64; 2]>,
    /// The tiles across and down.
    tiles: [u64; 2],
    /// The tiles' start and side, across and down.
    tiling: [[u64; 2]; 2],
}

/// A component's coding style (A.6.1).
#[derive(Clone, Copy, Eq, PartialEq)]
struct Style {
    /// A code-block's exponents.
    block: [u8; 2],
    /// The decomposition levels.
    levels: u8,
    /// A precinct's exponents, by resolution.
    precincts: [[u8; 2]; 33],
    /// The wavelet is the reversible one.
    reversible: bool,
    /// A code-block's passes may end in segments of their own.
    split: bool,
}

/// One tile's structures.
#[derive(Default)]
struct Sum {
    /// Subbands.
    bands: u64,
    /// Code-blocks.
    blocks: u64,
    /// Nodes of tag trees.
    nodes: u64,
    /// Precincts, once a subband.
    precincts: u64,
    /// Rows of precincts.
    rows: u64,
    /// The most coefficients of a component.
    samples: u64,
    /// Segments of code-blocks.
    segments: u64,
    /// Packets of one layer.
    sheet: u64,
}

/// What a tile's headers leave of its style (A.4.2).
#[derive(Clone)]
struct Tile {
    /// The entries of its progression changes.
    changes: usize,
    /// The layers.
    layers: u16,
    /// The components are transformed together.
    mixed: bool,
    /// Its tile-parts.
    parts: u64,
    /// Each component's style.
    styles: Vec<Style>,
}

/// The decoder's lists, each as long as the largest tile needs: they are kept across tiles.
#[derive(Default)]
struct Totals {
    /// Subbands.
    bands: u64,
    /// Code-blocks.
    blocks: u64,
    /// Precincts listed to order packets.
    elements: u64,
    /// The longest row filtered.
    filter: u64,
    /// The transform's output.
    idwt: u64,
    /// Layers of code-blocks.
    layers: u64,
    /// Nodes of tag trees.
    nodes: u64,
    /// Packets remembered with progression changes.
    packets: u64,
    /// Precincts.
    precincts: u64,
    /// Coefficients.
    samples: u64,
    /// The transform's scratch.
    scratch: u64,
    /// Segments.
    segments: u64,
    /// The work.
    work: u64,
}

/// What walking the tile-parts found.
#[derive(Default)]
struct Walked {
    /// The most bit planes a tile-part's header leaves a sample.
    bitplanes: u8,
    /// Data is missing.
    damaged: bool,
    /// Packed header markers.
    packed: u64,
    /// Tile-parts.
    parts: u64,
}

impl Totals {
    /// Adds a subband of `precincts` precincts, the high halves where `upper` says.
    fn band(
        &mut self,
        sum: &mut Sum,
        part: &Part,
        level: u8,
        reduced: [[u64; 2]; 2],
        upper: [bool; 2],
        precincts: u64,
    ) {
        let place = part.style.precincts.get(usize::from(level)).copied();
        let exponents = place.unwrap_or_default();
        let mut total = 1_u64;
        let mut widest = 0_u64;
        for (((ends, exponent), block), high) in reduced
            .into_iter()
            .zip(exponents)
            .zip(part.style.block)
            .zip(upper)
        {
            let (band, precinct) = if level == 0 {
                (ends, exponent)
            } else {
                let half = ends.map(|end| if high { end >> 1_u32 } else { end.div_ceil(2) });
                (half, exponent.saturating_sub(1))
            };
            let side = block.min(precinct);
            let count = partitions(band, side);
            let inside = 1_u64.wrapping_shl(u32::from(precinct.saturating_sub(side)));
            total = total.saturating_mul(count);
            widest = widest.max(count.min(inside));
        }
        let height = widest.next_power_of_two().trailing_zeros();
        // Building a tag tree visits 4 to the levels nodes, whatever its shape.
        let visits = 4_u64
            .wrapping_shl(height.saturating_mul(2))
            .saturating_sub(1)
            .checked_div(3)
            .unwrap_or_default();
        self.work = self
            .work
            .saturating_add(precincts.saturating_mul(visits.saturating_mul(2)));
        let trees = u64::from(height).saturating_add(1).saturating_mul(2);
        sum.blocks = sum.blocks.saturating_add(total);
        sum.nodes = sum
            .nodes
            .saturating_add(total.saturating_mul(4))
            .saturating_add(precincts.saturating_mul(trees));
        let each = if part.style.split {
            PASSES
        } else {
            part.layers
        };
        sum.segments = sum.segments.saturating_add(total.saturating_mul(each));
        sum.precincts = sum.precincts.saturating_add(precincts);
        sum.bands = sum.bands.saturating_add(1);
    }

    /// The bytes of the lists; `steady` when every component of the one tile has the same shape,
    /// so that none of the buffers of samples grows.
    fn bytes(&self, steady: bool) -> u64 {
        let listed = [
            (self.bands, 64_u64),
            (self.blocks, 72),
            (self.elements, 36),
            (self.layers, 24),
            (self.nodes, 56),
            (self.precincts, 56),
            (self.segments, 24),
        ]
        .into_iter()
        .fold(0_u64, |total, entry| {
            total.saturating_add(entry.0.saturating_mul(entry.1))
        });
        let buffers = self
            .samples
            .saturating_add(self.idwt)
            .saturating_add(self.scratch)
            .saturating_mul(4);
        // A list that doubles holds old and new while it moves.
        let growth = if steady { 1 } else { 3 };
        listed
            .saturating_mul(3)
            .saturating_add(buffers.saturating_mul(growth))
            .saturating_add(self.packets.saturating_mul(64))
            .saturating_add(self.filter.saturating_mul(8))
    }

    /// Adds one resolution `level` of a component of a tile (B.5 to B.9).
    fn resolution(
        &mut self,
        job: &Job<'_>,
        sum: &mut Sum,
        part: &Part,
        level: u8,
    ) -> Result<(), PdfError> {
        let style = part.style;
        let down = u32::from(style.levels.saturating_sub(level));
        let place = style.precincts.get(usize::from(level)).copied();
        let exponents = place.unwrap_or_default();
        let [wide, high] = exponents;
        if level > 0 && (wide == 0 || high == 0) {
            return Err(job.objects.malformed());
        }
        for (exponent, sampling) in exponents.iter().zip(part.sampling) {
            let shift = u32::from(*exponent).saturating_add(down);
            if shift > 31 || sampling.wrapping_shl(shift) > STEP {
                // Stepping to the next precinct overflows in the decoder.
                return Err(job.limit());
            }
        }
        let [across, upright] = part.area;
        let [sampled_x, sampled_y] = part.sampling;
        let scaled = |ends: [u64; 2], sampling: u64| {
            ends.map(|end| {
                end.div_ceil(sampling.max(1))
                    .div_ceil(1_u64.wrapping_shl(down))
            })
        };
        let reduced = [scaled(across, sampled_x), scaled(upright, sampled_y)];
        let [columns, rows] = reduced.map(length);
        let [ends_x, ends_y] = reduced;
        let (split_x, split_y) = (partitions(ends_x, wide), partitions(ends_y, high));
        sum.rows = sum.rows.saturating_add(split_y);
        let here = split_x.saturating_mul(split_y);
        sum.sheet = sum.sheet.saturating_add(here);
        let padded = columns
            .saturating_add(1)
            .saturating_mul(rows.saturating_add(1));
        self.idwt = self.idwt.max(padded);
        if level < style.levels {
            self.scratch = self.scratch.max(padded);
        }
        self.filter = self.filter.max(columns.saturating_add(4));
        sum.samples = sum.samples.max(columns.saturating_mul(rows));
        let bands: &[[bool; 2]] = if level == 0 {
            &[[false, false]]
        } else {
            &[[true, false], [false, true], [true, true]]
        };
        for upper in bands {
            self.band(sum, part, level, reduced, *upper, here);
        }
        Ok(())
    }

    /// Adds a tile over `area`, with `changes` progression changes and `bits` of data.
    fn tile(
        &mut self,
        job: &Job<'_>,
        tile: &Tile,
        sampling: &[[u64; 2]],
        area: [[u64; 2]; 2],
        changes: usize,
        bits: u64,
    ) -> Result<(), PdfError> {
        let mut sum = Sum::default();
        let layers = u64::from(tile.layers);
        for (style, steps) in tile.styles.iter().zip(sampling) {
            let part = Part {
                area,
                layers,
                sampling: *steps,
                style: *style,
            };
            for level in 0..=style.levels {
                self.resolution(job, &mut sum, &part, level)?;
            }
        }
        let orders = codec::wide(changes.max(1));
        let packets = sum.sheet.saturating_mul(layers);
        // The decoder steps through rows of precincts even where there is no column: for each subband and
        // again for each progression change.
        let walks = orders.saturating_add(3);
        self.work = self.work.saturating_add(sum.rows.saturating_mul(walks));
        // The iterators spin over every layer, resolution and component.
        let spun = layers
            .saturating_mul(33)
            .saturating_mul(codec::wide(tile.styles.len()));
        self.work = self
            .work
            .saturating_add(sum.blocks.saturating_mul(layers.saturating_add(4)))
            .saturating_add(packets.saturating_add(spun).saturating_mul(orders));
        if self.work > WORK {
            return Err(job.limit());
        }
        // A part that ends in a packet's header leaves that code-block's segments behind.
        let abandoned = tile.parts.saturating_mul(PASSES);
        // Each segment's length takes 3 bits or more.
        let segments = sum
            .segments
            .saturating_add(abandoned)
            .min(bits.checked_div(3).unwrap_or(bits));
        self.bands = self.bands.max(sum.bands);
        self.blocks = self.blocks.max(sum.blocks);
        self.elements = self.elements.max(sum.sheet.saturating_mul(orders));
        self.layers = self.layers.max(sum.blocks.saturating_mul(layers));
        self.nodes = self.nodes.max(sum.nodes);
        self.precincts = self.precincts.max(sum.precincts);
        self.samples = self.samples.max(sum.samples);
        self.segments = self.segments.max(segments);
        if changes > 0 {
            self.packets = self.packets.max(packets);
        }
        Ok(())
    }
}

/// The samples of `data` after its first [`ROOM`] bytes, which are spare, in 8 bits, or fewer
/// where one component has 1, 2 or 4; one of `indices` as they are.
pub(crate) fn decode(
    job: &Job<'_>,
    data: &mut [u8],
    target: [u32; 2],
    indices: bool,
) -> Result<Held, PdfError> {
    // The data counts while it is decoded.
    let lent = Rc::new(());
    job.lend(&lent, codec::wide(data.len()))?;
    let own = data.get(ROOM..).unwrap_or_default();
    codec::admitted(job, own)?;
    let mut file = boxes::read(job, own)?;
    if indices && file.palette.is_some() {
        // The dictionary's space outranks the file's, and the palette's channels go with it.
        (file.palette, file.mapping, file.definitions) = (None, None, None);
    }
    let frame = guard(job, file.codestream)?;
    let components = frame.depths.len();
    let layout = boxes::layout(job, &file, components)?;
    let channels = &layout.channels;
    let small = frame
        .depths
        .first()
        .copied()
        .filter(|_| components == 1 && file.palette.is_none())
        .and_then(|depth| match depth {
            1 | 2 | 4 => Some(depth),
            // Indices of another depth are not scaled to 8 bits: they stay.
            _ => indices.then_some(8),
        });
    let plain = small.is_none()
        && channels.len() == components
        && channels
            .iter()
            .enumerate()
            .all(|(index, channel)| channel.component == index && channel.table.is_none());
    let pixels = frame
        .samples
        .checked_div(codec::wide(components))
        .unwrap_or(0);
    let made = if plain {
        0
    } else {
        // A copy, and a palette's columns.
        pixels
            .saturating_add(0x1_0000)
            .saturating_mul(codec::wide(channels.len()))
    };
    let input = codec::wide(file.codestream.len());
    let profile = file.profile;
    let bytes = frame
        .bytes
        .saturating_add(frame.samples)
        .saturating_add(profile)
        .saturating_add(made);
    codec::charged(job, bytes)?;
    let work = frame
        .work
        .saturating_add(frame.samples.saturating_mul(sample_work(frame.bitplanes)))
        .saturating_add(input.saturating_mul(INPUT_WORK))
        .saturating_add(made);
    job.spend(work)?;
    if frame.damaged {
        job.warn("7.4.9", "JPX data damaged or short")?;
    }
    let settings = DecodeSettings {
        resolve_palette_indices: false,
        strict: false,
        // Averaged indices would be other colours.
        target_resolution: (file.palette.is_none() && small.is_none()).then(|| target.into()),
    };
    let implied = mem::take(&mut file.implied);
    let (at, length) = (file.at, file.codestream.len());
    let malformed = || job.objects.malformed();
    // The boxes the decoder reads go just before the codestream, in place: no copy is made.
    let end = at.saturating_add(ROOM).saturating_add(length);
    let wrapped = data.get_mut(at..end).ok_or_else(malformed)?;
    wrapped
        .get_mut(..ROOM)
        .ok_or_else(malformed)?
        .copy_from_slice(WRAPPER);
    let image = Image::new(wrapped, &settings).map_err(|_error| malformed())?;
    let mut context = DecoderContext::default();
    let decoded = image.decode(&mut context).map_err(|_error| malformed())?;
    let (width, height) = (image.width(), image.height());
    let size = u64::from(width).saturating_mul(u64::from(height));
    let stored = stored(job, &decoded, size, components)?;
    let samples = if let Some(depth) = small {
        packed(job, &decoded, [width, height], depth)?
    } else if plain {
        stored
    } else {
        arranged(job, &decoded, &stored, channels)?
    };
    Ok(Held {
        alpha: layout.alpha,
        data: samples,
        implied,
        info: Info {
            bits: small.unwrap_or(8),
            components: u8::try_from(channels.len()).map_err(|_error| job.unsupported())?,
            height,
            width,
        },
        premultiplied: layout.premultiplied,
    })
}

/// The pixels of `stored` as `channels` order them, indices looked up.
fn arranged(
    job: &Job<'_>,
    decoded: &DecodedImage<'_>,
    stored: &[u8],
    channels: &[Channel],
) -> Result<Vec<u8>, PdfError> {
    let parts = decoded.components();
    let stride = parts.len();
    if stride == 0 || channels.is_empty() {
        return Err(job.objects.malformed());
    }
    let pixels = stored.chunks_exact(stride);
    let total = codec::wide(pixels.len()).saturating_mul(codec::wide(channels.len()));
    let mut found = codec::filled(job, total, 0)?;
    let outputs = found.chunks_exact_mut(channels.len());
    for (index, (pixel, out)) in pixels.zip(outputs).enumerate() {
        for (channel, slot) in channels.iter().zip(out.iter_mut()) {
            *slot = channel.table.as_deref().map_or_else(
                || pixel.get(channel.component).copied().unwrap_or_default(),
                |table| {
                    let sample = parts
                        .get(channel.component)
                        .and_then(|part| part.samples().get(index));
                    let last = table.len().saturating_sub(1);
                    let place = sample.map_or(0, |value| nearest(*value)).min(last);
                    table.get(place).copied().unwrap_or_default()
                },
            );
        }
    }
    Ok(found)
}

/// The error of a marker the decoder does not read: one of the standard's is unsupported.
fn foreign(job: &Job<'_>, code: u8) -> PdfError {
    // CAP, PLM and CPF, and the markers of extensions (ISO/IEC 15444-2, -15).
    if matches!(code, 0x50 | 0x57 | 0x59 | 0x70..=0x79) {
        job.unsupported()
    } else {
        job.objects.malformed()
    }
}

/// Walks the codestream: what decoding it takes at most, or why it is not decoded.
fn guard(job: &Job<'_>, stream: &[u8]) -> Result<Frame, PdfError> {
    let malformed = || job.objects.malformed();
    let mut reader = Reader::new(stream);
    if reader.take(START.len()) != Some(START) {
        return Err(malformed());
    }
    let size = sized(job, &mut reader)?;
    let count = codec::wide(size.depths.len());
    let [width, height] = shown(&size);
    if width == 0 || height == 0 {
        return Err(malformed());
    }
    let pixels = width.saturating_mul(height);
    let planes = pixels
        .saturating_add(8)
        .saturating_mul(count.saturating_mul(4));
    let [across, down] = size.tiles;
    let tiles = across.saturating_mul(down);
    // Each tile is made with copies of every component's headers before any is read.
    let headers = tiles.saturating_mul(count.saturating_mul(768).saturating_add(256));
    let fixed = planes.saturating_add(headers).saturating_add(BASE);
    codec::charged(job, fixed)?;
    let main = main_header(job, &mut reader, size.depths.len())?;
    let amount = usize::try_from(tiles).map_err(|_error| job.limit())?;
    let mut held = vec![main.tile.clone(); amount];
    let walked = tile_parts(job, &mut reader, &mut held)?;
    if !uniform(&main.tile, &held) {
        // The decoder would give other colours: it has one transform for every tile.
        return Err(job.unsupported());
    }
    let input = codec::wide(stream.len());
    let totals = totalled(job, &size, &held, main.changes, input.saturating_mul(8))?;
    let steady = matches!(held.as_slice(), [only] if alike(&only.styles) && alike(&size.sampling));
    let lists = walked
        .parts
        .saturating_mul(128)
        .saturating_add(walked.packed.saturating_mul(64))
        // A code-block's data is gathered in a list.
        .saturating_add(input)
        .saturating_mul(3)
        .saturating_add(main.packed.saturating_mul(20));
    let bytes = fixed
        .saturating_add(lists)
        .saturating_add(totals.bytes(steady));
    codec::charged(job, bytes)?;
    Ok(Frame {
        bitplanes: main.bitplanes.max(walked.bitplanes),
        bytes,
        damaged: walked.damaged || held.iter().any(|tile| tile.parts == 0),
        samples: pixels.saturating_mul(count),
        depths: size.depths,
        work: totals.work,
    })
}

/// Whether all of `items` are the same.
fn alike<T>(items: &[T]) -> bool
where
    T: PartialEq,
{
    items
        .iter()
        .zip(items.iter().skip(1))
        .all(|(left, right)| left == right)
}

/// The count of a run of `ends`.
const fn length(ends: [u64; 2]) -> u64 {
    let [start, end] = ends;
    end.saturating_sub(start)
}

/// The main header after SIZ (A.4.1), as the decoder reads it.
fn main_header(job: &Job<'_>, reader: &mut Reader<'_>, count: usize) -> Result<Main, PdfError> {
    let malformed = || job.objects.malformed();
    let mut default = None;
    let mut overrides = vec![None; count];
    let (mut quantized, mut changes, mut packed) = (false, 0_usize, 0_u64);
    let (mut bitplanes, mut shifted) = (0_u8, 0_u8);
    loop {
        let code = marker(reader).ok_or_else(malformed)?;
        if code == 0x90 {
            break;
        }
        reader.skip(2).ok_or_else(malformed)?;
        match code {
            0x52 => default = Some(style_default(job, reader)?),
            0x53 => {
                let (index, style) = style_component(reader).ok_or_else(malformed)?;
                *overrides.get_mut(index).ok_or_else(malformed)? = Some(style);
            }
            0x5C => {
                bitplanes = bitplanes.max(quantization(job, reader, false)?);
                quantized = true;
            }
            0x5D => bitplanes = bitplanes.max(quantization(job, reader, true)?),
            0x5E => shifted = shifted.max(shift(reader).ok_or_else(malformed)?),
            0x55 | 0x63 | 0x64 => {
                let _body: &[u8] = segment(reader).ok_or_else(malformed)?;
            }
            0x60 => {
                let body = segment(reader).ok_or_else(malformed)?;
                packed = packed
                    .saturating_add(codec::wide(body.len()))
                    .saturating_add(4);
            }
            0x5F => {
                let body = segment(reader).ok_or_else(malformed)?;
                changes = changed(job, changes, body)?;
            }
            0x30..=0x3F => {}
            _ => return Err(foreign(job, code)),
        }
    }
    let (base, layers, mixed) = default.filter(|_found| quantized).ok_or_else(malformed)?;
    Ok(Main {
        bitplanes: bitplanes.saturating_add(shifted).min(BITPLANES),
        changes,
        packed,
        tile: Tile {
            changes: 0,
            layers,
            mixed,
            parts: 0,
            styles: overrides
                .into_iter()
                .map(|found| found.unwrap_or(base))
                .collect(),
        },
    })
}

/// The progression changes so far and those in `body`, which the decoder expands one by one.
fn changed(job: &Job<'_>, before: usize, body: &[u8]) -> Result<usize, PdfError> {
    let total = before.saturating_add(body.len().checked_div(7).unwrap_or_default());
    if total > CHANGES {
        Err(job.limit())
    } else {
        Ok(total)
    }
}

/// The marker that is next.
fn marker(reader: &Reader<'_>) -> Option<u8> {
    let rest = reader.rest();
    (rest.first() == Some(&0xFF)).then(|| rest.get(1).copied())?
}

/// The integer nearest to a decoded sample; 0 below it.
fn nearest(sample: f32) -> usize {
    let half = Fixed::from_bits(1_i64 << 31_u32);
    let rounded = Fixed::from_f64(f64::from(sample)).plus(half).floor();
    usize::try_from(rounded.max(0)).unwrap_or(usize::MAX)
}

/// The samples of the one component in rows of `depth` bits.
fn packed(
    job: &Job<'_>,
    decoded: &DecodedImage<'_>,
    size: [u32; 2],
    depth: u8,
) -> Result<Vec<u8>, PdfError> {
    let [width, height] = size;
    let bits = u32::from(depth);
    let stride = u64::from(width).saturating_mul(u64::from(bits)).div_ceil(8);
    let total = stride.saturating_mul(u64::from(height));
    let mut found = codec::filled(job, total, 0)?;
    let parts = decoded.components();
    let samples = parts
        .first()
        .ok_or_else(|| job.objects.malformed())?
        .samples();
    let top = usize::from(u8::MAX.wrapping_shr(8_u32.saturating_sub(bits)));
    let across = usize::try_from(width).map_err(|_error| job.limit())?;
    let bytes = usize::try_from(stride).map_err(|_error| job.limit())?;
    if across == 0 || bytes == 0 {
        return Err(job.objects.malformed());
    }
    for (out, row) in found
        .chunks_exact_mut(bytes)
        .zip(samples.chunks_exact(across))
    {
        let mut at = 0_u32;
        for sample in row {
            let value = u8::try_from(nearest(*sample).min(top)).unwrap_or_default();
            let shift = 8_u32.saturating_sub(bits).saturating_sub(at & 7);
            let place = usize::try_from(at >> 3_u32).unwrap_or(usize::MAX);
            if let Some(byte) = out.get_mut(place) {
                *byte |= value.wrapping_shl(shift);
            }
            at = at.saturating_add(bits);
        }
    }
    Ok(found)
}

/// The cells of side 2 to the `exponent` that `ends` touches.
fn partitions(ends: [u64; 2], exponent: u8) -> u64 {
    let [start, end] = ends;
    if start == end {
        return 0;
    }
    let side = u32::from(exponent);
    end.div_ceil(1_u64.wrapping_shl(side))
        .saturating_sub(start.wrapping_shr(side))
}

/// Reads QCD, or QCC for a `component` (A.6.4, A.6.5), as far as the decoder does; the most bit
/// planes it leaves (E-2).
fn quantization(job: &Job<'_>, reader: &mut Reader<'_>, component: bool) -> Result<u8, PdfError> {
    let malformed = || job.objects.malformed();
    let declared = usize::from(reader.short().ok_or_else(malformed)?);
    let mut before = 3_usize;
    if component {
        before = 4;
        reader.skip(1).ok_or_else(malformed)?;
    }
    let style = reader.byte().ok_or_else(malformed)?;
    let kind = style & 0x1F;
    let rest = declared.checked_sub(before).ok_or_else(malformed)?;
    let pairs = rest >> 1_u32;
    let (entries, bytes) = match kind {
        0 => (rest, rest),
        1 => (1, 2),
        2 => (pairs, pairs.saturating_mul(2)),
        _ => return Err(malformed()),
    };
    if entries > STEP_SIZES {
        return Err(job.limit());
    }
    let steps = reader.take(bytes).ok_or_else(malformed)?;
    let each = if kind == 0 { 1_usize } else { 2 };
    let exponent = steps.iter().step_by(each).map(|byte| byte >> 3_u32).max();
    let guarded = (style >> 5_u32).saturating_add(exponent.unwrap_or_default());
    Ok(guarded.saturating_sub(1))
}

/// The work of a sample of `bitplanes`.
fn sample_work(bitplanes: u8) -> u64 {
    u64::from(bitplanes.saturating_sub(SHALLOW))
        .saturating_mul(DEEP_WORK)
        .saturating_add(SAMPLE_WORK)
}

/// A marker segment's body.
fn segment<'data>(reader: &mut Reader<'data>) -> Option<&'data [u8]> {
    let declared = reader.short()?.checked_sub(2)?;
    reader.take(usize::from(declared))
}

/// Reads RGN (A.6.3): the bit planes it adds.
fn shift(reader: &mut Reader<'_>) -> Option<u8> {
    reader.take(5)?.last().copied()
}

/// The image's columns and rows as the decoder makes them: shrunk where all components are
/// subsampled alike.
fn shown(size: &Size) -> [u64; 2] {
    let shrink = match size.sampling.first() {
        Some(first) if alike(&size.sampling) => *first,
        Some(_) | None => [1, 1],
    };
    let [across, upright] = size.image;
    let [step_x, step_y] = shrink;
    [(across, step_x), (upright, step_y)]
        .map(|(ends, step)| length(ends.map(|end| end.div_ceil(step.max(1)))))
}

/// SIZ (A.5.1), after its marker.
fn sized(job: &Job<'_>, reader: &mut Reader<'_>) -> Result<Size, PdfError> {
    let malformed = || job.objects.malformed();
    reader.skip(4).ok_or_else(malformed)?;
    let mut field = || reader.long().ok_or_else(malformed);
    let (right, bottom, left, top) = (field()?, field()?, field()?, field()?);
    let (tile_width, tile_height, tile_left, tile_top) = (field()?, field()?, field()?, field()?);
    let count = usize::from(reader.short().ok_or_else(malformed)?);
    if count > CHANNELS {
        return Err(job.unsupported());
    }
    if count == 0 || tile_width == 0 || tile_height == 0 {
        return Err(malformed());
    }
    if right > COORDINATE || bottom > COORDINATE {
        return Err(job.limit());
    }
    let first_right = u64::from(tile_left).saturating_add(u64::from(tile_width));
    let first_bottom = u64::from(tile_top).saturating_add(u64::from(tile_height));
    if left >= right
        || top >= bottom
        || tile_left > left
        || tile_top > top
        || first_right <= u64::from(left)
        || first_bottom <= u64::from(top)
    {
        return Err(malformed());
    }
    let mut depths = Vec::with_capacity(count);
    let mut sampling = Vec::with_capacity(count);
    for _ in 0..count {
        let depth = (reader.byte().ok_or_else(malformed)? & 0x7F).saturating_add(1);
        let step_x = reader.byte().ok_or_else(malformed)?;
        let step_y = reader.byte().ok_or_else(malformed)?;
        if depth > PRECISION {
            return Err(job.unsupported());
        }
        if step_x == 0 || step_y == 0 {
            return Err(malformed());
        }
        depths.push(depth);
        sampling.push([u64::from(step_x), u64::from(step_y)]);
    }
    let across = u64::from(right.saturating_sub(tile_left).div_ceil(tile_width));
    let down = u64::from(bottom.saturating_sub(tile_top).div_ceil(tile_height));
    if across.saturating_mul(down) > TILES {
        return Err(job.limit());
    }
    Ok(Size {
        depths,
        image: [[left, right], [top, bottom]].map(|ends| ends.map(u64::from)),
        sampling,
        tiles: [across, down],
        tiling: [[tile_left, tile_width], [tile_top, tile_height]].map(|ends| ends.map(u64::from)),
    })
}

/// The interleaved 8-bit samples of all `components` as the decoder rounds them.
fn stored(
    job: &Job<'_>,
    decoded: &DecodedImage<'_>,
    pixels: u64,
    components: usize,
) -> Result<Vec<u8>, PdfError> {
    let parts = decoded.components();
    let whole = parts.len() == components
        && parts
            .iter()
            .all(|part| codec::wide(part.samples().len()) == pixels);
    if !whole {
        return Err(job.objects.malformed());
    }
    let total = pixels.saturating_mul(codec::wide(components));
    let mut found = codec::filled(job, total, 0)?;
    decoded.store_u8_into(&mut found);
    Ok(found)
}

/// A coding style's parameters under `flags` (A.6.1).
fn style(reader: &mut Reader<'_>, flags: u8) -> Option<Style> {
    let levels = reader.byte().filter(|found| *found <= LEVELS)?;
    let width = reader.byte()?.checked_add(2)?;
    let height = reader.byte()?.checked_add(2)?;
    if width > 10 || height > 10 || width.saturating_add(height) > 12 {
        return None;
    }
    let coding = reader.byte()?;
    let wavelet = reader.byte()?;
    if wavelet > 1 {
        return None;
    }
    let mut precincts = [[15_u8; 2]; 33];
    if flags & 1 != 0 {
        for slot in precincts
            .iter_mut()
            .take(usize::from(levels).saturating_add(1))
        {
            let packed = reader.byte()?;
            *slot = [packed & 15, packed >> 4_u32];
        }
    }
    Some(Style {
        block: [width, height],
        levels,
        precincts,
        reversible: wavelet == 1,
        split: coding & 5 != 0,
    })
}

/// COC's component and style (A.6.2).
fn style_component(reader: &mut Reader<'_>) -> Option<(usize, Style)> {
    let _length: u16 = reader.short()?;
    let index = usize::from(reader.byte()?);
    let flags = reader.byte()?;
    Some((index, style(reader, flags)?))
}

/// COD's style, layers and whether components are transformed together (A.6.1).
fn style_default(job: &Job<'_>, reader: &mut Reader<'_>) -> Result<(Style, u16, bool), PdfError> {
    let malformed = || job.objects.malformed();
    let _length: u16 = reader.short().ok_or_else(malformed)?;
    let flags = reader.byte().ok_or_else(malformed)?;
    if reader.byte().ok_or_else(malformed)? > 4 {
        return Err(malformed());
    }
    let layers = reader.short().ok_or_else(malformed)?;
    if layers == 0 {
        return Err(malformed());
    }
    if layers > LAYERS {
        return Err(job.unsupported());
    }
    let mixed = reader.byte().ok_or_else(malformed)? == 1;
    let found = style(reader, flags).ok_or_else(malformed)?;
    Ok((found, layers, mixed))
}

/// Reads every tile-part's header, as each can replace a tile's styles (A.4.2).
fn tile_parts(
    job: &Job<'_>,
    reader: &mut Reader<'_>,
    tiles: &mut [Tile],
) -> Result<Walked, PdfError> {
    let malformed = || job.objects.malformed();
    let mut walked = Walked::default();
    while marker(reader) == Some(0x90) {
        let fields = reader.take(12).ok_or_else(malformed)?;
        let mut fields_reader = Reader::new(fields.get(4..).unwrap_or_default());
        let index = usize::from(fields_reader.short().ok_or_else(malformed)?);
        let declared = fields_reader.long().ok_or_else(malformed)?;
        let tile = tiles.get_mut(index).ok_or_else(malformed)?;
        let length = match declared {
            0 => reader.left(),
            _ => usize::try_from(declared.checked_sub(12).ok_or_else(malformed)?)
                .map_err(|_error| job.limit())?,
        };
        walked.parts = walked.parts.saturating_add(1);
        tile.parts = tile.parts.saturating_add(1);
        let start = reader.at;
        loop {
            if reader.at.saturating_sub(start) >= length {
                break;
            }
            let Some(code) = marker(reader) else {
                // The decoder leaves the part here, without moving.
                walked.damaged = true;
                break;
            };
            if code != 0xD9 {
                reader.skip(2).ok_or_else(malformed)?;
            }
            if code == 0x93 || code == 0xD9 {
                let rest = length.checked_sub(reader.at.saturating_sub(start));
                walked.damaged |= rest.is_none_or(|body| body > reader.left());
                if let Some(body) = rest {
                    reader.skip(body.min(reader.left())).ok_or_else(malformed)?;
                }
                break;
            }
            tile_marker(job, reader, tile, &mut walked, code)?;
        }
    }
    Ok(walked)
}

/// One marker segment of a tile-part's header.
fn tile_marker(
    job: &Job<'_>,
    reader: &mut Reader<'_>,
    tile: &mut Tile,
    walked: &mut Walked,
    code: u8,
) -> Result<(), PdfError> {
    let malformed = || job.objects.malformed();
    match code {
        0x52 => {
            let (found, layers, mixed) = style_default(job, reader)?;
            tile.styles.fill(found);
            tile.layers = layers;
            tile.mixed = mixed;
        }
        0x53 => {
            let (index, found) = style_component(reader).ok_or_else(malformed)?;
            *tile.styles.get_mut(index).ok_or_else(malformed)? = found;
        }
        0x5C | 0x5D => {
            let found = quantization(job, reader, code == 0x5D)?;
            walked.bitplanes = walked.bitplanes.max(found.min(BITPLANES));
        }
        0x5E => {
            // Its planes come on top of any quantization's.
            if shift(reader).ok_or_else(malformed)? > 0 {
                walked.bitplanes = BITPLANES;
            }
        }
        0x58 | 0x64 => {
            let _body: &[u8] = segment(reader).ok_or_else(malformed)?;
        }
        0x61 => {
            let _body: &[u8] = segment(reader).ok_or_else(malformed)?;
            walked.packed = walked.packed.saturating_add(1);
        }
        0x5F => {
            let body = segment(reader).ok_or_else(malformed)?;
            tile.changes = changed(job, tile.changes, body)?;
        }
        0x30..=0x3F => {}
        _ => return Err(foreign(job, code)),
    }
    Ok(())
}

/// The totals of all `tiles` (B.3).
fn totalled(
    job: &Job<'_>,
    size: &Size,
    tiles: &[Tile],
    changes: usize,
    bits: u64,
) -> Result<Totals, PdfError> {
    let mut totals = Totals::default();
    let [across, _] = size.tiles;
    let wide = usize::try_from(across.max(1)).map_err(|_error| job.limit())?;
    let [image_x, image_y] = size.image;
    let [tiling_x, tiling_y] = size.tiling;
    for (row, held) in tiles.chunks(wide).enumerate() {
        for (column, tile) in held.iter().enumerate() {
            let area = [
                edges(tiling_x, image_x, codec::wide(column)),
                edges(tiling_y, image_y, codec::wide(row)),
            ];
            let count = if tile.changes > 0 {
                tile.changes
            } else {
                changes
            };
            totals.tile(job, tile, &size.sampling, area, count, bits)?;
        }
    }
    Ok(totals)
}

/// Where the tile at `index` lies on one axis of the image.
fn edges(tiling: [u64; 2], image: [u64; 2], index: u64) -> [u64; 2] {
    let [origin, side] = tiling;
    let [low, high] = image;
    let start = origin.saturating_add(index.saturating_mul(side));
    [start.max(low), start.saturating_add(side).min(high)]
}

/// Whether the decoder transforms all of `tiles` right: it takes the wavelets of the main header's
/// `main` and the first tile's component transform for all.
fn uniform(main: &Tile, tiles: &[Tile]) -> bool {
    let mixed = tiles.first().map(|tile| tile.mixed);
    tiles.iter().all(|tile| {
        mixed == Some(tile.mixed)
            && tile
                .styles
                .iter()
                .zip(&main.styles)
                .all(|(own, shared)| own.reversible == shared.reversible)
    })
}
