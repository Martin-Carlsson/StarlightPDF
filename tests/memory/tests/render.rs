//! Peak allocation of rendering.

extern crate alloc;

use alloc::sync::Arc;
use core::error::Error;
use core::iter;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::fs;
use std::io;

use starlightpdf::ErrorCode;
use starlightpdf::document::{Document, OpenOptions, PageOptions};
use starlightpdf::input::{BytesSourceOptions, Source};
use starlightpdf::limits::{ResourcePolicy, ResourcePolicyOptions};
use starlightpdf::output::{Append, AppendSinkOptions, MemorySinkOptions, Sink};
use starlightpdf::rendering::RenderOptions;
use starlightpdf_memory::{adler_run, adler_value, peak};

/// Most bytes opening and rendering a page may hold allocated at once, just above the 12,730,409 measured for the
/// turned scan (5,735,312 for the other): below a quarter of the page's 5000 x 5000 x 3 bytes of pixels, 18,750,000.
const BOUND: usize = 13_000_000;

/// The scan page's contents: the RGB image turned a quarter, which bands decode again.
const TURNED: &str = "q 0 5000 -5000 0 5000 0 cm /Flate Do Q\n";

/// Most bytes rendering the page of a `CMap` cut into 262,144 ranges may hold at once under [`CMAP_LIMIT`], just
/// above the 16,816,819 measured.
const CMAP_BOUND: usize = 17_301_504;

/// The decoded limit of the page of a `CMap` cut into 262,144 ranges.
const CMAP_LIMIT: u64 = 0x100_0000;

/// Most bytes rendering a page of a 40 MB font may hold at once under [`FONT_LIMIT`], just above the 40,229,047
/// measured.
const FONT_BOUND: usize = 41_943_040;

/// Most bytes rendering a held image and then a JBIG2 region may hold at once under [`HELD_LIMIT`], just above the
/// 8,265,377 measured.
const HELD_BOUND: usize = 8_650_752;

/// The decoded limit of the page of a held image and a JBIG2 region: 12 MiB.
const HELD_LIMIT: u64 = 0xC0_0000;

/// Most bytes rendering a page of 30 MB of inline or JPX data may hold at once under [`TWICE_LIMIT`], just above
/// the 30,635,554 and 30,225,916 measured: below the 33,554,432 a buffer grown by doubling reaches.
const ONCE_BOUND: usize = 31_457_280;

/// Most bytes rendering a page of 40 inline images of 1 MiB and a byte may hold at once at the default limits, just
/// above the 2,329,827 measured.
const MANY_BOUND: usize = 2_490_368;

/// Most bytes rendering the page of 256 fonts of many glyph names may hold at once under [`POST_LIMIT`], just above
/// the 17,805,445 measured.
const POST_BOUND: usize = 0x114_0000;

/// Most bytes rendering the page of 8 Type 1 fonts of 65,000 subroutines may hold at once under [`SUBRS_LIMIT`]: the
/// limit and its caches (6,133,835 measured).
const SUBRS_BOUND: usize = 0x60_0000;

/// The decoded limit of the page of 8 Type 1 fonts of 65,000 subroutines: 4 MiB.
const SUBRS_LIMIT: u64 = 0x40_0000;

/// The decoded limit of the page of 256 fonts of many glyph names: 16 MiB.
const POST_LIMIT: u64 = 0x100_0000;

/// Most bytes rendering a page under [`COPIES_LIMIT`] may hold at once, just above what was measured: an `Extend` of
/// 100 references to one large string, 234,090.
const EXTEND_BOUND: usize = 0x3_C000;

/// The same of a large direct pattern saved 300 times, 18,138,360.
const PATTERN_BOUND: usize = 0x118_0000;

/// The same of an `Intent` of 3,000 references to one long name, 591,112.
const INTENT_BOUND: usize = 0x9_8000;

/// The same of 16 nested forms sharing one large category dictionary, 3,925,989.
const CATEGORY_BOUND: usize = 0x3F_0000;

/// The decoded limit of the pages that copy one large object many times: 16 MiB.
const COPIES_LIMIT: u64 = 0x100_0000;

/// Most bytes rendering 20 nested knockout groups of text, each widened by a clip inside `BT`, may hold at once at
/// 10,000 pixels wide, just above the 5,254,193 measured.
const KNOCKOUT_BOUND: usize = 0x54_0000;

/// Most bytes rendering the page of 100 JPX images, each with an ICC profile, may hold at once under [`ICC_LIMIT`],
/// just above the 7,124,260 measured.
const ICC_BOUND: usize = 0x70_0000;

/// The decoded limit of the page of 100 JPX images, each with an ICC profile: 16 MiB.
const ICC_LIMIT: u64 = 0x100_0000;

/// Most bytes rendering the page of a held ON set and a JPX image may hold at once under a limit of 62,000,000, just
/// above the 37,757,805 measured.
const LAYERS_BOUND: usize = 0x250_0000;

/// Most bytes rendering a page whose resources or group hold one token of 16 MiB may hold at once under a limit of
/// 1 MiB, just above the 775,986 measured.
const TOKEN_BOUND: usize = 0xC_8000;

/// Most bytes rendering 15 nested Type 3 fonts at 25 megapixels may hold at once, just above the 5,005,495 measured.
const NESTED_TEXT_BOUND: usize = 0x50_0000;

/// Most bytes rendering 200,000 distinct mesh shadings in forms, empty or damaged, may hold at once at fit 64: just above the
/// 1,271,372 measured for the same page of plain shading dictionaries (1,107,029 measured for the meshes).
const MESHES_BOUND: usize = 0x14_0000;

/// The head of a type 4 shading of 8-bit flags, coordinates and grey.
const MESH_HEAD: &str = "/ShadingType 4 /ColorSpace /DeviceGray /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 \
                         /Decode [0 100 0 100 0 1]";

/// Most bytes rendering a progressive JPEG of 64 scans behind 15 Flate stages may hold at once under a decoded limit of
/// 4,000,000: the limit and its caches, about 6 MB (2,326,371 measured).
const SCANS_BOUND: usize = 0x60_0000;

/// Most bytes rendering the page of a progressive JPEG behind 15 Flate stages with a soft mask of the same may hold at once
/// under [`TWO_SCANS_LIMIT`]: the limit and its caches (39,952,159 measured).
const TWO_SCANS_BOUND: usize = 70_254_592;

/// A decoded limit that one such JPEG decoder fits within and two do not.
const TWO_SCANS_LIMIT: u64 = 68_000_000;

/// Most bytes rendering 10 nested forms, each with resources of its own of the same 64 direct fonts of 20,000-byte
/// names, may hold at once under [`FONT_NAMES_LIMIT`]: 44 MiB.
const FONT_NAMES_BOUND: usize = 0x2C0_0000;

/// The decoded limit of the page of nested forms of fonts of long names: 40 MiB.
const FONT_NAMES_LIMIT: u64 = 0x280_0000;

/// Most bytes rendering 30 nested forms reached by names of 500,000 bytes may hold at once under
/// [`DO_NAMES_LIMIT`]: the limit, its caches and room for what is read before it is counted (35,501,660 measured).
const DO_NAMES_BOUND: usize = 0x240_0000;

/// The decoded limit of the page of 30 nested forms reached by long names: 32 MiB.
const DO_NAMES_LIMIT: u64 = 0x200_0000;

/// Most bytes rendering 30 nested forms, each with a string of 900,000 bytes left among its operands, may hold at once
/// under [`STRINGS_LIMIT`]: the limit, its caches and room for what is read before it is counted (20,190,749 measured).
const STRINGS_BOUND: usize = 0x140_0000;

/// The decoded limit of the page of 30 nested forms each with a long string left: 16 MiB.
const STRINGS_LIMIT: u64 = 0x100_0000;

/// Most bytes rendering 31 nested forms each behind 16 Flate stages may hold at once under [`STAGES_LIMIT`]: the limit
/// and its caches (2,813,237 measured, 2,583,857 damaged).
const STAGES_BOUND: usize = 0x30_0000;

/// The decoded limit of the page of 31 nested forms behind 16 Flate stages each: 1 MiB.
const STAGES_LIMIT: u64 = 0x10_0000;

/// Most bytes rendering 31 nested forms each behind a predictor of 4 MiB rows may hold at once under
/// [`PREDICTORS_LIMIT`], or the same of an object stream or an inline image behind 16 or 15 such stages, or of an inline
/// image's DCT data behind one: the limit and its caches (6,344,470 and 13,970,219 measured; 216,984, 103,271 and
/// 15,782,156).
const PREDICTORS_BOUND: usize = 0x120_0000;

/// The decoded limit of the page of 31 nested forms behind a predictor each: 16 MiB.
const PREDICTORS_LIMIT: u64 = 0x100_0000;

/// Most bytes rendering a page of an object stream of 131,073 members may hold at once under [`MEMBERS_LIMIT`]: the
/// limit and its caches (2,976,162 measured).
const MEMBERS_BOUND: usize = 0x30_0000;

/// The decoded limit of the page of an object stream of 131,073 members: 1 MiB.
const MEMBERS_LIMIT: u64 = 0x10_0000;

/// Most bytes rendering a page whose cross-reference stream declares 36 bytes but decodes to 100 MB may hold at once at the
/// default limits: about the 373,721 of the same page without the excess, as main reads it by streaming (375,169
/// measured).
const LONG_SECTION_BOUND: usize = 0x10_0000;

/// The PNG predictor of 65,536 columns of 32 colours of 16 bits, 4 MiB rows (ISO 32000-2:2020, 7.4.4.4).
const PREDICTOR: &str = "<< /Predictor 10 /Colors 32 /BitsPerComponent 16 /Columns 65536 >>";

/// A decoded limit that allows 30 MB of data twice: 128 MiB.
const TWICE_LIMIT: u64 = 0x800_0000;

/// The decoded limit of the page of a 40 MB font.
const FONT_LIMIT: u64 = 0x400_0000;

/// The one-code ranges that cut the `CMap`'s range of every code, 262,143.
const CUTS: u32 = 0x0003_FFFF;

/// Most bytes rendering a page of one large object may hold at once at the default limits, just above the 218,610
/// and 221,404 measured.
const OBJECT_BOUND: usize = 229_376;

/// Most bytes rendering a page of 4,096 saved graphics states may hold at once at the default limits, just above the
/// 4,161,261 and 3,246,139 measured.
const STATE_BOUND: usize = 4_325_376;

/// The page's side in points, and in pixels at the default resolution.
const SIDE: u16 = 5_000;

/// Samples of an image of a component covering the page.
const PIXELS: u64 = 25_000_000;

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

/// A sink's appends: how many bytes, and whether they begin as a PNG of the page's size does.
struct Written {
    /// Bytes appended.
    bytes: AtomicU64,
    /// Whether the header is right.
    header: AtomicBool,
}

impl Append for Written {
    fn append(&self, bytes: &[u8]) -> io::Result<()> {
        let length = u64::try_from(bytes.len()).map_err(io::Error::other)?;
        let before = self.bytes.fetch_add(length, Ordering::Relaxed);
        if before < 24 {
            // The signature, then IHDR's length, name, width and height (PNG, 5.2, 11.2.1).
            let start = b"\x89PNG\r\n\x1A\n\x00\x00\x00\x0DIHDR\x00\x00\x13\x88\x00\x00\x13\x88";
            let skipped = usize::try_from(before).map_err(io::Error::other)?;
            if start
                .iter()
                .skip(skipped)
                .zip(bytes)
                .any(|(want, have)| want != have)
            {
                self.header.store(false, Ordering::Relaxed);
            }
        }
        Ok(())
    }
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

/// A zlib stream of `count` copies of `byte`.
fn repeated(byte: u8, count: u64) -> Made<Vec<u8>> {
    zlib_runs(&[(byte, count)])
}

/// A zlib stream of `runs`, each so many copies of a byte, in one fixed-Huffman block: each run's first byte a
/// literal, then repeats of the byte before (RFC 1951, 3.2.6).
fn zlib_runs(runs: &[(u8, u64)]) -> Made<Vec<u8>> {
    let mut bits = Bits::default();
    bits.bytes.extend_from_slice(&[0x78, 0x01]);
    for bit in [true, true, false] {
        bits.push(bit);
    }
    let mut sum = (1, 0);
    for &(byte, count) in runs {
        let literal = |out: &mut Bits| {
            if byte < 144 {
                out.code(u16::from(byte).saturating_add(0x30), 8);
            } else {
                out.code(u16::from(byte).saturating_add(0x100), 9);
            }
        };
        literal(&mut bits);
        let mut rest = count.checked_sub(1).ok_or("an empty run")?;
        while rest >= 258 {
            // Length 258 is code 285; distance 1 is code 0.
            bits.code(0xC5, 8);
            bits.code(0, 5);
            rest = rest.saturating_sub(258);
        }
        for _ in 0..rest {
            literal(&mut bits);
        }
        sum = adler_run(sum, byte, count).ok_or("Adler-32")?;
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

/// A baseline JPEG of the page's size, all 128: two bits a block (ITU-T T.81, B.2).
fn grey_jpeg() -> Vec<u8> {
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xDB, 0, 67, 0];
    jpeg.extend_from_slice(&[1; 64]);
    jpeg.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8, 0x13, 0x88, 0x13, 0x88, 1, 1, 0x11, 0]);
    jpeg.extend_from_slice(&[0xFF, 0xC4, 0, 38]);
    for class in [0x00, 0x10] {
        // One symbol, 0, of one bit.
        jpeg.extend_from_slice(&[class, 1]);
        jpeg.extend_from_slice(&[0; 16]);
    }
    jpeg.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 63, 0]);
    // 390,625 blocks of two zero bits, padded with ones.
    jpeg.resize(jpeg.len().saturating_add(97_656), 0);
    jpeg.extend_from_slice(&[0x3F, 0xFF, 0xD9]);
    jpeg
}

/// A stream object of `data` with `entries` beside its Length.
fn stream(entries: &str, data: &[u8]) -> Vec<u8> {
    [
        format!("<< {entries} /Length {} >>\nstream\n", data.len()).as_bytes(),
        data,
        b"\nendstream",
    ]
    .concat()
}

/// The page's contents.
fn contents() -> String {
    let mut text = String::from(
        "q 5000 0 0 5000 0 0 cm /Flate Do Q\n\
         q /Faint gs 5000 0 0 5000 0 0 cm /Jpeg Do Q\n\
         q /Faint gs 5000 0 0 5000 0 0 cm /Masked Do Q\n\
         q /Outer Do Q\n\
         q /Soft gs 1 0 0 rg 0 0 5000 5000 re f Q\n\
         q 100 100 4800 2400 re W n /Axial sh Q\n\
         q 1000 1000 3000 3000 re W n /Radial sh Q\n\
         BT /Type1 40 Tf 60 TL 50 4900 Td\n",
    );
    let line = format!("({}) Tj T*\n", "A".repeat(120));
    for _ in 0_u8..80 {
        text.push_str(&line);
    }
    text.push_str("ET\n");
    text
}

/// The objects from 5: the images, the groups, the mask, the shadings and the font.
fn parts(font: &[u8]) -> Made<Vec<Vec<u8>>> {
    let image =
        format!("/Type /XObject /Subtype /Image /Width {SIDE} /Height {SIDE} /BitsPerComponent 8");
    let form = format!(
        "/Type /XObject /Subtype /Form /BBox [0 0 {SIDE} {SIDE}] /Group << /S /Transparency"
    );
    let samples = PIXELS.checked_mul(3).ok_or("too many samples")?;
    Ok(vec![
        stream(
            &format!("{image} /ColorSpace /DeviceRGB /Decode [1 0 0 1 0.2 0.8] /Filter /FlateDecode"),
            &repeated(0x40, samples)?,
        ),
        stream(&format!("{image} /ColorSpace /DeviceGray /Filter /DCTDecode"), &grey_jpeg()),
        stream(
            &format!("{image} /ColorSpace /DeviceGray /SMask 8 0 R /Filter /FlateDecode"),
            &repeated(0x20, PIXELS)?,
        ),
        stream(
            &format!("{image} /ColorSpace /DeviceGray /Filter /FlateDecode"),
            &repeated(0x50, PIXELS)?,
        ),
        stream(
            &format!(
                "{form} >> /Resources << /XObject << /Inner 10 0 R >> \
                 /ExtGState << /Mix << /BM /Multiply /ca 0.6 >> >> >>"
            ),
            b"/Mix gs 0.2 0.7 0.4 rg 0 0 4000 4000 re f /Inner Do",
        ),
        stream(
            &format!("{form} /I true /K true >> /Resources << >>"),
            b"0.9 0.1 0.2 rg 1000 1000 4000 4000 re f 0.1 0.2 0.9 rg 2000 0 2000 5000 re f",
        ),
        stream(
            &format!("{form} /I true /CS /DeviceGray >> /Resources << >>"),
            b"0.5 g 500 500 4000 4000 re f",
        ),
        b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [100 0 4900 0] /Extend [true true] \
          /Function 14 0 R >>"
            .to_vec(),
        b"<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords [2500 2500 0 2500 2500 1500] /Function 14 0 R >>"
            .to_vec(),
        b"<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /SLTest /FirstChar 65 /LastChar 65 /Widths [1000] \
          /FontDescriptor 16 0 R >>"
            .to_vec(),
        b"<< /Type /FontDescriptor /FontName /SLTest /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 \
          /Ascent 800 /Descent 0 /CapHeight 800 /StemV 80 /FontFile3 17 0 R >>"
            .to_vec(),
        stream("/Subtype /Type1C", font),
    ])
}

/// A file of one page of `SIDE` points square with `content` and `objects` (ISO 32000-2:2020, 7.5).
fn pdf(content: &str, objects: Vec<Vec<u8>>) -> Vec<u8> {
    let resources = "<< /XObject << /Flate 5 0 R /Jpeg 6 0 R /Masked 7 0 R /Outer 9 0 R >> \
         /ExtGState << /Faint << /ca 0.5 >> /Soft << /ca 0.5 /SMask << /S /Luminosity /G 11 0 R >> >> >> \
         /Shading << /Axial 12 0 R /Radial 13 0 R >> /Font << /Type1 15 0 R >> >>";
    let page = if objects.is_empty() {
        "<< >>"
    } else {
        resources
    };
    let mut all = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {SIDE} {SIDE}] /Resources {page} /Contents 4 0 R >>"
        )
        .into_bytes(),
        stream("", content.as_bytes()),
    ];
    all.extend(objects);
    written(&all)
}

/// A file of `objects`, numbered from 1, the first the catalog (ISO 32000-2:2020, 7.5).
fn written(all: &[Vec<u8>]) -> Vec<u8> {
    let mut file = b"%PDF-1.7\n".to_vec();
    let mut table = b"0000000000 65535 f \n".to_vec();
    for (object, number) in all.iter().zip(1_u32..) {
        table.extend_from_slice(format!("{:010} 00000 n \n", file.len()).as_bytes());
        file.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        file.extend_from_slice(object);
        file.extend_from_slice(b"\nendobj\n");
    }
    let size = all.len().saturating_add(1);
    let start = file.len();
    file.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
    file.extend(table);
    file.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n").as_bytes(),
    );
    file
}

/// A file of a 100-point page with `entries` in its dictionary, `resources` and `content`, `objects` from 5 and
/// `catalog` entries.
fn object_page(
    entries: &str,
    resources: &str,
    content: &str,
    objects: Vec<Vec<u8>>,
    catalog: &str,
) -> Vec<u8> {
    let mut all = vec![
        format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>").into_bytes(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << {resources} >> /Contents 4 0 R {entries} >>"
        )
        .into_bytes(),
        stream("", content.as_bytes()),
    ];
    all.extend(objects);
    written(&all)
}

/// The outcome and peak of opening `file` and rendering its page, at the default limits.
fn limited(file: Vec<u8>) -> (Result<(), ErrorCode>, usize) {
    limited_to(file, None)
}

/// The outcome and peak of opening `file` under a decoded limit of `decoded` bytes, if given, and rendering its page.
fn limited_to(file: Vec<u8>, decoded: Option<u64>) -> (Result<(), ErrorCode>, usize) {
    limited_fit(file, decoded, None)
}

/// The outcome and peak of opening `file` under a decoded limit of `decoded` bytes, if given, and rendering its page
/// to `fit` pixels, if given.
fn limited_fit(
    file: Vec<u8>,
    decoded: Option<u64>,
    fit: Option<u32>,
) -> (Result<(), ErrorCode>, usize) {
    let mut options = RenderOptions::default();
    options.fit = fit;
    let mut source = BytesSourceOptions::default();
    source.bytes = Arc::new(file);
    let mut limits = ResourcePolicyOptions::default();
    limits.decoded_limit = decoded;
    peak(|| -> Result<(), ErrorCode> {
        let attempt = || -> Result<(), starlightpdf::PdfError> {
            let policy = ResourcePolicy::new(&limits)?;
            let document = Document::open(
                &Source::from_bytes(&source)?,
                Some(&policy),
                &OpenOptions::default(),
            )?;
            let sink = Sink::in_memory(&MemorySinkOptions::default())?;
            document
                .page(&PageOptions::default())?
                .render(&sink, None, &options)
        };
        attempt().map_err(|error| error.code())
    })
}

/// Fails unless the render gave `outcome`, holding at most `bound`.
fn bounded_by(
    (result, top): (Result<(), ErrorCode>, usize),
    outcome: Result<(), ErrorCode>,
    bound: usize,
) {
    assert_eq!(result, outcome, "outcome");
    assert!((1..=bound).contains(&top), "peak {top} bytes");
}

/// Renders `file`'s page strictly to `sink`.
fn render(file: &BytesSourceOptions, sink: &Sink, options: &RenderOptions) -> Made<()> {
    let mut open = OpenOptions::default();
    open.strict = true;
    let document = Document::open(&Source::from_bytes(file)?, None, &open)?;
    document
        .page(&PageOptions::default())?
        .render(sink, None, options)?;
    Ok(())
}

/// The peak of rendering a page of `content` at 72 pixels an inch, after showing at 50 pixels that it is not blank.
fn measured(content: &str) -> Made<(u64, bool, usize)> {
    let font = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../corpus/render/font-cff.cff"
    ))?;
    // Held by the test throughout, so the peak counts only what rendering adds.
    let mut file = BytesSourceOptions::default();
    file.bytes = Arc::new(pdf(content, parts(&font)?));
    let mut blank = BytesSourceOptions::default();
    blank.bytes = Arc::new(pdf("", Vec::new()));
    let mut small = RenderOptions::default();
    small.fit = Some(50);
    let thumbnail = |of: &BytesSourceOptions| -> Made<Vec<u8>> {
        let sink = Sink::in_memory(&MemorySinkOptions::default())?;
        render(of, &sink, &small)?;
        Ok(sink.take_bytes()?)
    };
    if thumbnail(&file)? == thumbnail(&blank)? {
        return Err("the page renders blank".into());
    }
    let written = Arc::new(Written {
        bytes: AtomicU64::new(0),
        header: AtomicBool::new(true),
    });
    let append: Arc<dyn Append> = Arc::<Written>::clone(&written);
    let (result, top) = peak(|| -> Made<()> {
        let sink = Sink::from_append(&append, &AppendSinkOptions::default())?;
        render(&file, &sink, &RenderOptions::default())
    });
    result?;
    Ok((
        written.bytes.load(Ordering::Relaxed),
        written.header.load(Ordering::Relaxed),
        top,
    ))
}

/// Rendering 25 megapixels holds less than a quarter of them
/// (.architecture-decisions/memory-is-bounded-by-work-not-file-size.md).
#[test]
fn render_25_megapixel_page_is_bounded() {
    for content in [contents(), String::from(TURNED)] {
        let found = measured(&content).map_err(|error| error.to_string());
        assert!(
            matches!(&found, Ok((bytes, true, _)) if *bytes > 24),
            "PNG of 5000 x 5000: {found:?}"
        );
        let top = found.map_or(0, |(_, _, bytes)| bytes);
        assert!((1..=BOUND).contains(&top), "peak {top} bytes");
    }
}

/// A page dictionary with an array of 2,000,000 numbers holds no more than a fixed bound at the default limits,
/// whatever it costs to keep (ISO 32000-2:2020, 7.7.3.3).
#[test]
fn render_of_a_large_page_dictionary_is_bounded() {
    let junk = format!("/Junk [{}]", "0 ".repeat(2_000_000));
    bounded_by(
        limited(object_page(&junk, "", "", Vec::new(), "")),
        Ok(()),
        OBJECT_BOUND,
    );
}

/// A visibility expression of 65,536 operands, one of them itself, holds no more than a fixed bound
/// (ISO 32000-2:2020, 8.11.2.3).
#[test]
fn render_of_a_self_referring_visibility_expression_is_bounded() {
    let expression = format!("[/Or 6 0 R {}]", "0 ".repeat(0x1_0000));
    let objects = vec![
        b"<< /Type /OCMD /OCGs 7 0 R /VE 6 0 R >>".to_vec(),
        expression.into_bytes(),
        b"<< /Type /OCG /Name (L) >>".to_vec(),
    ];
    bounded_by(
        limited(object_page(
            "",
            "/Properties << /M 5 0 R >>",
            "/OC /M BDC 0 0 10 10 re f EMC",
            objects,
            "/OCProperties << /OCGs [7 0 R] /D << >> >>",
        )),
        Ok(()),
        OBJECT_BOUND,
    );
}

/// 4,096 saved graphics states, each with its own dash array of 20,000 numbers, hold no more than a fixed bound
/// (ISO 32000-2:2020, 8.4.2, 8.4.3.6).
#[test]
fn render_of_saved_dash_states_is_bounded() {
    let dash = format!("q [{}] 0 d\n", "1 ".repeat(20_000));
    bounded_by(
        limited(object_page("", "", &dash.repeat(4_096), Vec::new(), "")),
        Ok(()),
        STATE_BOUND,
    );
}

/// 4,096 saved graphics states, each set to one of 20 shading patterns of about 60 KB, hold no more than a fixed
/// bound (ISO 32000-2:2020, 8.4.2, 8.7.4.2).
#[test]
fn render_of_saved_pattern_states_is_bounded() {
    let pattern = format!(
        "<< /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] \
         /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >> /Padding [{}] >>",
        "0 ".repeat(30_000)
    );
    let names = (0_u32..20)
        .zip(5_u32..)
        .map(|(index, number)| format!("/P{index} {number} 0 R "))
        .collect::<Vec<String>>()
        .concat();
    let patterns = vec![pattern.into_bytes(); 20];
    let content = (0_u32..20)
        .cycle()
        .take(4_096)
        .map(|index| format!("q /Pattern cs /P{index} scn\n"))
        .collect::<Vec<String>>()
        .concat();
    bounded_by(
        limited(object_page(
            "",
            &format!("/Pattern << {names}>>"),
            &content,
            patterns,
            "",
        )),
        Err(ErrorCode::LimitExceeded),
        STATE_BOUND,
    );
}

/// The page of a `CMap` range of every 4-byte code cut by [`CUTS`] one-code ranges (ISO 32000-2:2020, 9.7.6.2).
fn cut_cmap_page() -> Made<Vec<u8>> {
    let font = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../corpus/render/font-truetype.ttf"
    ))?;
    let mut lines = vec![String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /Cut def\n\
         1 begincodespacerange <00000000> <FFFFFFFF> endcodespacerange\n\
         1 begincidrange <00000000> <FFFFFFFF> 1 endcidrange\n",
    )];
    for block in (0..CUTS).step_by(100) {
        let count = CUTS.saturating_sub(block).min(100);
        lines.push(format!("{count} begincidrange\n"));
        for one in block..block.saturating_add(count) {
            let code = one.saturating_mul(2).saturating_add(1);
            lines.push(format!("<{code:08X}> <{code:08X}> 1\n"));
        }
        lines.push(String::from("endcidrange\n"));
    }
    lines.push(String::from(
        "endcmap CMapName currentdict /CMap defineresource pop end end\n",
    ));
    let objects = vec![
        stream(&format!("/Length1 {}", font.len()), &font),
        b"<< /Type /FontDescriptor /FontName /SLTest /Flags 4 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 \
          /Descent 0 /CapHeight 800 /StemV 80 /FontFile2 5 0 R >>"
            .to_vec(),
        b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /SLTest /FontDescriptor 6 0 R \
          /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 1000 >>"
            .to_vec(),
        b"<< /Type /Font /Subtype /Type0 /BaseFont /SLTest /Encoding 9 0 R /DescendantFonts [7 0 R] >>".to_vec(),
        stream("/Type /CMap", lines.concat().as_bytes()),
    ];
    Ok(object_page(
        "",
        "/Font << /F 8 0 R >>",
        "1 0 0 rg BT /F 50 Tf 10 30 Td <00000002> Tj ET",
        objects,
        "",
    ))
}

/// A `CMap` cut into 262,144 ranges fails `LimitExceeded` under a 16 MiB decoded limit, holding little more than the
/// limit.
#[test]
fn render_of_a_cut_cmap_is_bounded() {
    let found = cut_cmap_page()
        .map(|page| limited_to(page, Some(CMAP_LIMIT)))
        .map_err(|error| error.to_string());
    assert!(
        matches!(&found, Ok((Err(ErrorCode::LimitExceeded), _))),
        "outcome {found:?}"
    );
    let top = found.map_or(0, |(_, bytes)| bytes);
    assert!((1..=CMAP_BOUND).contains(&top), "peak {top} bytes");
}

/// The page of a font program of 40 MB, shown once (ISO 32000-2:2020, 9.9).
fn large_font_page() -> Made<Vec<u8>> {
    let mut font = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../corpus/render/font-truetype.ttf"
    ))?;
    font.resize(40_000_000, 0);
    let objects = vec![
        stream(&format!("/Length1 {}", font.len()), &font),
        b"<< /Type /FontDescriptor /FontName /SLTest /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 \
          /Descent 0 /CapHeight 800 /StemV 80 /FontFile2 5 0 R >>"
            .to_vec(),
        b"<< /Type /Font /Subtype /TrueType /BaseFont /SLTest /FirstChar 65 /LastChar 65 /Widths [1000] \
          /Encoding /WinAnsiEncoding /FontDescriptor 6 0 R >>"
            .to_vec(),
    ];
    Ok(object_page(
        "",
        "/Font << /F 7 0 R >>",
        "BT /F 10 Tf 10 10 Td (A) Tj ET",
        objects,
        "",
    ))
}

/// A 40 MB font program held while used holds no more than a 64 MiB decoded limit allows.
#[test]
fn render_of_a_large_font_is_bounded() {
    let found = large_font_page()
        .map(|page| limited_to(page, Some(FONT_LIMIT)))
        .map_err(|error| error.to_string());
    assert!(matches!(&found, Ok((Ok(()), _))), "outcome {found:?}");
    let top = found.map_or(0, |(_, bytes)| bytes);
    assert!((1..=FONT_BOUND).contains(&top), "peak {top} bytes");
}

/// The page of an 8000 x 8000 CCITT image, 8 MB held once decoded, then an 8 x 8 JBIG2 page of one 8000 x 8000 generic
/// region, 8 MB more to decode (ITU-T T.6; T.88, 6.2, 7.4.6).
fn held_image_and_jbig2_page() -> Vec<u8> {
    // Every row V0 below a white reference row: white (ITU-T T.6, 2.2).
    let white = vec![0xFF; 1_000];
    let fax = "/Type /XObject /Subtype /Image /Width 8000 /Height 8000 /BitsPerComponent 1 /ColorSpace /DeviceGray \
               /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 8000 /Rows 8000 >>";
    let side = [0, 0, 0x1F, 0x40];
    let info: &[u8] = &[0, 0, 0, 8, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    // Arithmetic coding, template 0, its four AT pixels, then two bytes of data.
    let tail: &[u8] = &[0, 0, 3, 0xFF, 0xFD, 0xFF, 2, 0xFE, 0xFE, 0xFE, 0xFF, 0xAC];
    let region = [&side[..], &side, &[0; 8], &[0], tail].concat();
    let jbig2 = [
        &[0, 0, 0, 0, 48, 0, 1, 0, 0, 0, 19][..],
        info,
        &[0, 0, 0, 1, 38, 0, 1, 0, 0, 0, 28],
        &region,
    ]
    .concat();
    let objects = vec![
        stream(fax, &white),
        stream(
            "/Type /XObject /Subtype /Image /Width 8 /Height 8 /BitsPerComponent 1 /ColorSpace /DeviceGray \
             /Filter /JBIG2Decode",
            &jbig2,
        ),
    ];
    object_page(
        "",
        "/XObject << /I 5 0 R /J 6 0 R >>",
        "0 0 1 rg 0 0 100 100 re f q 50 0 0 100 0 0 cm /I Do Q q 50 0 0 100 50 0 cm /J Do Q",
        objects,
        "",
    )
}

/// A JBIG2 region decoded while an image is held counts beside it: 8 MB and 8 MB fail `LimitExceeded` under 12 MiB,
/// holding no more than the limit.
#[test]
fn render_of_an_image_held_beside_jbig2_is_bounded() {
    let (result, top) = limited_to(held_image_and_jbig2_page(), Some(HELD_LIMIT));
    assert_eq!(result, Err(ErrorCode::LimitExceeded), "outcome");
    assert!((1..=HELD_BOUND).contains(&top), "peak {top} bytes");
}

/// A page of an inline image of `data` bytes of `DCTDecode` data, its length given (ISO 32000-2:2020, 8.9.7).
fn inline_image_page(data: usize) -> Vec<u8> {
    let content = format!(
        "q 100 0 0 100 0 0 cm BI /W 8 /H 8 /CS /G /BPC 8 /F /DCT /L {data} ID {}\nEI Q",
        "\0".repeat(data)
    );
    object_page("", "", &content, Vec::new(), "")
}

/// An inline image of 30 MB is held once.
#[test]
fn render_of_an_inline_image_holds_it_once() {
    let (result, top) = limited_to(inline_image_page(30_000_000), Some(TWICE_LIMIT));
    assert_eq!(result, Ok(()), "outcome");
    assert!((1..=ONCE_BOUND).contains(&top), "peak {top} bytes");
}

/// 40 inline images, each of `DCTDecode` data of 1 MiB and a byte, render at the default limits, each read once and
/// not kept.
#[test]
fn render_of_many_large_inline_images_is_bounded() {
    let one = format!(
        "q 3 0 0 3 0 0 cm BI /W 8 /H 8 /CS /G /BPC 8 /F /DCT /L 1048577 ID {}\nEI Q\n",
        "\0".repeat(0x10_0001)
    );
    let page = object_page("", "", &one.repeat(40), Vec::new(), "");
    let (result, top) = limited(page);
    assert_eq!(result, Ok(()), "outcome");
    assert!((1..=MANY_BOUND).contains(&top), "peak {top} bytes");
}

/// A codestream of `length` bytes: an 8 x 8 grey image of one tile whose one packet is empty, padded with zeros before
/// its end (ITU-T T.800, A.4, A.5.1, A.6.1, A.6.4, B.10.3).
fn padded_codestream(length: usize) -> Vec<u8> {
    let head: &[u8] = &[
        0xFF, 0x4F, // SOC
        0xFF, 0x51, 0, 41, 0, 0, 0, 0, 0, 8, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0,
        0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 7, 1,
        1, // SIZ: 8 x 8, one tile, one 8-bit component
        0xFF, 0x52, 0, 12, 0, 0, 0, 1, 0, 0, 4, 4, 0,
        1, // COD: one layer, no decomposition, reversible
        0xFF, 0x5C, 0, 4, 0, 0x40, // QCD: no quantisation
        0xFF, 0x90, 0, 10, 0, 0, 0, 0, 0, 0, 0, 1, // SOT: Psot 0, to the end
        0xFF, 0x93, // SOD
        0,    // the empty packet
    ];
    let mut stream = head.to_vec();
    stream.resize(length.saturating_sub(2), 0);
    stream.extend_from_slice(&[0xFF, 0xD9]);
    stream
}

/// A JPX image of 30 MB is held once.
#[test]
fn render_of_a_jpx_image_holds_it_once() {
    let data = padded_codestream(30_000_000);
    let image = stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /Filter /JPXDecode",
        &data,
    );
    let page = object_page(
        "",
        "/XObject << /I 5 0 R >>",
        "q 100 0 0 100 0 0 cm /I Do Q",
        vec![image],
        "",
    );
    let (result, top) = limited_to(page, Some(TWICE_LIMIT));
    assert_eq!(result, Ok(()), "outcome");
    assert!((1..=ONCE_BOUND).contains(&top), "peak {top} bytes");
}

/// A zlib stream of a TrueType font of one empty glyph whose version 2.0 `post` table names 65,535 glyphs, all
/// `.notdef`: 131 KB of names, whatever the glyph count (OpenType 1.9.1: the table directory, glyf, head, loca, maxp,
/// post).
fn named_font() -> Made<Vec<u8>> {
    let start = [
        // The table directory: five tables, then each one's tag, checksum, offset and length.
        &[0, 1, 0, 0, 0, 5, 0, 0x40, 0, 2, 0, 0x10][..],
        b"glyf\0\0\0\0\0\0\0\x5C\0\0\0\x04",
        b"head\0\0\0\0\0\0\0\x60\0\0\0\x36",
        b"loca\0\0\0\0\0\0\0\x98\0\0\0\x04",
        b"maxp\0\0\0\0\0\0\0\x9C\0\0\0\x06",
        b"post\0\0\0\0\0\0\0\xA4\0\x02\0\x20",
        // glyf: the empty glyph; head: version 1, 1000 units an em, short offsets, padded.
        &[0; 4],
        &[0, 1, 0, 0],
        &[0; 14],
        &[0x03, 0xE8],
        &[0; 36],
        // loca: two offsets of 0; maxp: version 0.5 of one glyph, padded.
        &[0; 4],
        &[0, 0, 0x50, 0, 0, 1, 0, 0],
        // post: version 2.0, then 65,535 indices of glyph names, all 0.
        &[0, 2, 0, 0],
        &[0; 28],
        &[0xFF, 0xFF],
    ]
    .concat();
    let mut runs: Vec<(u8, u64)> = start
        .chunk_by(|one, next| one == next)
        .map(|run| Ok((*run.first().ok_or("a run")?, u64::try_from(run.len())?)))
        .collect::<Made<_>>()?;
    runs.push((0, 0x1_FFFE));
    zlib_runs(&runs)
}

/// The page of 256 fonts, each a stream of [`named_font`], each shown once.
fn named_fonts_page() -> Made<Vec<u8>> {
    let font = named_font()?;
    let mut objects = Vec::new();
    for index in 0..256_u32 {
        let first = index
            .checked_mul(3)
            .and_then(|at| at.checked_add(5))
            .ok_or("object numbers")?;
        objects.push(stream("/Filter /FlateDecode", &font));
        objects.push(
            format!(
                "<< /Type /FontDescriptor /FontName /X{index} /Flags 4 /FontBBox [0 0 1000 1000] /ItalicAngle 0 \
                 /Ascent 1000 /Descent 0 /CapHeight 1000 /StemV 1 /FontFile2 {first} 0 R >>"
            )
            .into_bytes(),
        );
        objects.push(
            format!(
                "<< /Type /Font /Subtype /TrueType /BaseFont /X{index} /FirstChar 0 /LastChar 0 /Widths [500] \
                 /FontDescriptor {} 0 R >>",
                first.saturating_add(1)
            )
            .into_bytes(),
        );
    }
    let names = (0..256_u32)
        .map(|index| {
            format!(
                "/F{index} {} 0 R ",
                index.saturating_mul(3).saturating_add(7)
            )
        })
        .collect::<Vec<_>>()
        .concat();
    let content = (0..256_u32)
        .map(|index| format!("BT /F{index} 12 Tf 10 10 Td <00> Tj ET\n"))
        .collect::<Vec<_>>()
        .concat();
    Ok(object_page(
        "",
        &format!("/Font << {names}>>"),
        &content,
        objects,
        "",
    ))
}

/// 256 fonts, each naming 65,535 glyphs in its `post` table though it has one, hold no more than a 16 MiB decoded
/// limit allows.
#[test]
fn render_of_fonts_of_many_glyph_names_is_bounded() {
    let found = named_fonts_page()
        .map(|page| limited_to(page, Some(POST_LIMIT)))
        .map_err(|error| error.to_string());
    assert!(
        matches!(&found, Ok((Err(ErrorCode::LimitExceeded), _))),
        "outcome {found:?}"
    );
    let top = found.map_or(0, |(_, bytes)| bytes);
    assert!((1..=POST_BOUND).contains(&top), "peak {top} bytes");
}

/// `plain` encrypted from `key` (Adobe Type 1 Font Format, 7.1).
fn encrypted(plain: &[u8], key: u16) -> Vec<u8> {
    let mut state = key;
    plain
        .iter()
        .map(|byte| {
            let cipher = byte ^ u8::try_from(state.wrapping_shr(8)).unwrap_or_default();
            state = u16::from(cipher)
                .wrapping_add(state)
                .wrapping_mul(52_845)
                .wrapping_add(22_719);
            cipher
        })
        .collect()
}

/// A charstring after 4 bytes of 0, encrypted (Adobe Type 1 Font Format, 7.2).
fn charstring(code: &[u8]) -> Vec<u8> {
    encrypted(&[&[0; 4], code].concat(), 4330)
}

/// A Type 1 font program of 65,000 subroutines that each return at once, and a glyph A that calls the first; its
/// lengths before, in and after the encrypted part (Adobe Type 1 Font Format, 2, 6.3, 7, 8).
fn subroutines_font() -> (Vec<u8>, [usize; 3]) {
    let head = b"%!PS-AdobeFont-1.0: SLTest 001\n12 dict begin\n/FontName /SLTest def\n/PaintType 0 def\n\
        /FontType 1 def\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n/Encoding StandardEncoding def\n\
        /FontBBox {0 0 1000 1000} readonly def\ncurrentdict end\ncurrentfile eexec\n";
    let mut body = b"dup /Private 9 dict dup begin\n/RD{string currentfile exch readstring pop}executeonly def\n\
        /ND{noaccess def}executeonly def\n/NP{noaccess put}executeonly def\n/BlueValues [] def\n\
        /MinFeature{16 16} def\n/password 5839 def\n/lenIV 4 def\n/Subrs 65000 array\n"
        .to_vec();
    // return.
    let back = charstring(&[11]);
    for index in 0..65_000_u32 {
        body.extend_from_slice(format!("dup {index} 5 RD ").as_bytes());
        body.extend_from_slice(&back);
        body.extend_from_slice(b" NP\n");
    }
    body.extend_from_slice(b"ND\n2 index /CharStrings 2 dict dup begin\n");
    // 0 1000 hsbw endchar, and 0 1000 hsbw 0 callsubr endchar.
    for (name, code) in [
        (".notdef", &[139, 250, 124, 13, 14][..]),
        ("A", &[139, 250, 124, 13, 139, 10, 14]),
    ] {
        let sealed = charstring(code);
        body.extend_from_slice(format!("/{name} {} RD ", sealed.len()).as_bytes());
        body.extend_from_slice(&sealed);
        body.extend_from_slice(b" ND\n");
    }
    body.extend_from_slice(
        b"end\nend\nreadonly put\nnoaccess put\ndup/FontName get exch definefont pop\nmark currentfile closefile\n",
    );
    let secret = encrypted(&[b"\0\0\0\0".as_slice(), &body].concat(), 55_665);
    let tail = [&b"\n"[..], &[b'0'; 512], b"\ncleartomark\n"].concat();
    let lengths = [head.len(), secret.len(), tail.len()];
    ([&head[..], &secret, &tail].concat(), lengths)
}

/// The page of 8 fonts, each a stream of [`subroutines_font`], each shown once.
fn subroutines_page() -> Vec<u8> {
    let (font, [clear, secret, tail]) = subroutines_font();
    let mut objects = Vec::new();
    for index in 0..8_u32 {
        let first = index.saturating_mul(3).saturating_add(5);
        objects.push(stream(
            &format!("/Length1 {clear} /Length2 {secret} /Length3 {tail}"),
            &font,
        ));
        objects.push(
            format!(
                "<< /Type /FontDescriptor /FontName /SLTest /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 \
                 /Ascent 1000 /Descent 0 /CapHeight 1000 /StemV 1 /FontFile {first} 0 R >>"
            )
            .into_bytes(),
        );
        objects.push(
            format!(
                "<< /Type /Font /Subtype /Type1 /BaseFont /SLTest /FirstChar 65 /LastChar 65 /Widths [1000] \
                 /FontDescriptor {} 0 R >>",
                first.saturating_add(1)
            )
            .into_bytes(),
        );
    }
    let names = (0..8_u32)
        .map(|index| {
            format!(
                "/F{index} {} 0 R ",
                index.saturating_mul(3).saturating_add(7)
            )
        })
        .collect::<Vec<_>>()
        .concat();
    let content = (0..8_u32)
        .map(|index| format!("BT /F{index} 12 Tf 10 10 Td (A) Tj ET\n"))
        .collect::<Vec<_>>()
        .concat();
    object_page("", &format!("/Font << {names}>>"), &content, objects, "")
}

/// 8 Type 1 fonts, each of 65,000 subroutines, counted with what they hold, give way to each other when room is needed:
/// the page renders, holding no more than a 4 MiB decoded limit allows.
#[test]
fn render_of_fonts_of_many_subroutines_is_bounded() {
    let (result, top) = limited_to(subroutines_page(), Some(SUBRS_LIMIT));
    assert_eq!(result, Ok(()), "outcome");
    assert!((1..=SUBRS_BOUND).contains(&top), "peak {top} bytes");
}

/// A literal string of 1 MiB of `a` (ISO 32000-2:2020, 7.3.4.2).
fn large_string() -> Vec<u8> {
    format!("({})", "a".repeat(0x10_0000)).into_bytes()
}

/// Fails unless rendering `page` under [`COPIES_LIMIT`] gives `outcome`, holding at most `bound`.
fn copies_bounded(page: Vec<u8>, outcome: Result<(), ErrorCode>, bound: usize) {
    let (result, top) = limited_to(page, Some(COPIES_LIMIT));
    assert_eq!(result, outcome, "outcome");
    assert!((1..=bound).contains(&top), "peak {top} bytes");
}

/// An axial shading's `Extend` of 100 references to one string of 1 MiB holds no more than the decoded limit allows
/// (ISO 32000-2:2020, 8.7.4.5.3).
#[test]
fn render_of_an_array_of_one_large_object_is_bounded() {
    let refs = vec!["5 0 R"; 100].join(" ");
    let shading = format!(
        "<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Function << /FunctionType 2 /Domain [0 1] \
         /C0 [0 0 0] /C1 [1 1 1] /N 1 >> /Extend [{refs}] >>"
    );
    let objects = vec![large_string(), shading.into_bytes()];
    copies_bounded(
        object_page("", "/Shading << /S 6 0 R >>", "/S sh", objects, ""),
        Ok(()),
        EXTEND_BOUND,
    );
}

/// A pattern of 1 MiB given directly in the resources, set and then saved 300 times, fails `LimitExceeded` holding
/// little more than the decoded limit allows (ISO 32000-2:2020, 8.4.2, 8.7.3).
#[test]
fn render_of_a_direct_pattern_saved_many_times_is_bounded() {
    let pattern = [
        &b"/Pattern << /P << /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] \
           /Function << /FunctionType 2 /Domain [0 1] /C0 [0 0 0] /C1 [1 1 1] /N 1 >> >> /Pad "[..],
        &large_string(),
        b" >> >>",
    ]
    .concat();
    let content = format!("/Pattern cs {}0 0 10 10 re f", "/P scn q ".repeat(300));
    let resources = String::from_utf8(pattern).unwrap_or_default();
    copies_bounded(
        object_page("", &resources, &content, Vec::new(), ""),
        Err(ErrorCode::LimitExceeded),
        PATTERN_BOUND,
    );
}

/// An optional content configuration's `Intent` of 3,000 references to one name of 60,000 bytes holds no more than
/// the decoded limit allows (ISO 32000-2:2020, 8.11.4.3).
#[test]
fn render_of_intents_of_one_long_name_is_bounded() {
    let refs = vec!["6 0 R"; 3_000].join(" ");
    let catalog =
        format!("/OCProperties << /OCGs [5 0 R] /D << /OFF [5 0 R] /Intent [{refs}] >> >>");
    let objects = vec![
        b"<< /Type /OCG /Name (L) >>".to_vec(),
        format!("/{}", "a".repeat(60_000)).into_bytes(),
    ];
    copies_bounded(
        object_page(
            "",
            "/Properties << /L 5 0 R >>",
            "/OC /L BDC 0 0 10 10 re f EMC",
            objects,
            &catalog,
        ),
        Ok(()),
        INTENT_BOUND,
    );
}

/// 16 nested forms whose resources share one category dictionary of 2 MiB hold no more than the decoded limit allows
/// (ISO 32000-2:2020, 7.8.3, 8.10.1).
#[test]
fn render_of_nested_forms_sharing_a_large_category_is_bounded() {
    let forms = 7..23_u32;
    let names = forms
        .clone()
        .map(|number| format!("/F{number} {number} 0 R "))
        .collect::<Vec<_>>()
        .concat();
    let category = [
        format!("<< /G << >> {names}/Pad (").as_bytes(),
        "a".repeat(0x20_0000).as_bytes(),
        b") >>",
    ]
    .concat();
    let mut all = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources 5 0 R /Contents 4 0 R >>"
            .to_vec(),
        stream("", b"/G gs /F7 Do"),
        b"<< /ExtGState 6 0 R /XObject 6 0 R >>".to_vec(),
        category,
    ];
    for number in forms {
        let content = if number < 22 {
            format!("/G gs /F{} Do", number.saturating_add(1))
        } else {
            String::from("/G gs 0 0 10 10 re f")
        };
        all.push(stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources 5 0 R",
            content.as_bytes(),
        ));
    }
    copies_bounded(written(&all), Ok(()), CATEGORY_BOUND);
}

/// The contents of a level of [`knockout_page`]: text in a group of its own, whose clip widens inside `BT`, then
/// the next level.
fn knockout_level(next: Option<usize>) -> String {
    let tail = next.map_or_else(String::new, |level| format!("/X{level} Do "));
    format!("/K gs BT q 0 0 1 1 re W n /F 1 Tf 0 0 Td (A) Tj Q /F 1000 Tf (A) Tj {tail}ET")
}

/// A page of 1000 x 100 points of 20 nested forms, each showing a Type 3 glyph of a square under a text knockout
/// group whose clip widens inside `BT` (ISO 32000-2:2020, 9.4, 11.7.4.5).
fn knockout_page() -> Vec<u8> {
    let names = (0..20_usize)
        .map(|level| format!("/X{level} {} 0 R ", level.saturating_add(8)))
        .collect::<Vec<_>>()
        .concat();
    let resources =
        format!("<< /Font << /F 5 0 R >> /ExtGState << /K 7 0 R >> /XObject << {names}>> >>");
    let mut all = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1000 100] /Resources {resources} /Contents 4 0 R >>")
            .into_bytes(),
        stream("", knockout_level(Some(0)).as_bytes()),
        b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /FirstChar 65 \
          /LastChar 65 /Widths [1000] /Encoding << /Differences [65 /a] >> /CharProcs << /a 6 0 R >> >>"
            .to_vec(),
        stream("", b"1000 0 0 0 1000 1000 d1 0 0 1000 1000 re f"),
        b"<< /Type /ExtGState /ca 0.5 >>".to_vec(),
    ];
    for level in 0..20_usize {
        let next = Some(level.saturating_add(1)).filter(|after| *after < 20);
        all.push(stream(
            &format!("/Type /XObject /Subtype /Form /BBox [0 0 1000 100] /Resources {resources}"),
            knockout_level(next).as_bytes(),
        ));
    }
    written(&all)
}

/// 20 nested text knockout groups, each widened by a clip inside `BT`, hold no more than measuring counted, at 10,000
/// pixels wide.
#[test]
fn render_of_widened_text_knockout_groups_is_bounded() {
    let (result, top) = limited_fit(knockout_page(), None, Some(10_000));
    assert_eq!(result, Ok(()), "outcome");
    assert!((1..=KNOCKOUT_BOUND).contains(&top), "peak {top} bytes");
}

/// `value`'s 4 bytes, the highest first.
fn big_endian(value: usize) -> [u8; 4] {
    [24_u32, 16, 8, 0]
        .map(|shift| u8::try_from(value.wrapping_shr(shift) & 0xFF).unwrap_or_default())
}

/// A box of `kind` holding `parts` (ISO/IEC 15444-1, I.4).
fn jp2_box(kind: [u8; 4], parts: &[&[u8]]) -> Vec<u8> {
    let body = parts.concat();
    [&big_endian(body.len().saturating_add(8))[..], &kind, &body].concat()
}

/// A JP2 file of an 8 x 8 grey image whose `colr` box holds an ICC display profile of one grey curve of 100,000
/// samples, all 0, that its parse keeps (ISO/IEC 15444-1, I.5.3.3; ICC.1:2022, 7.2, 9.2.29, 10.6).
fn profiled_jp2() -> Vec<u8> {
    let samples = 100_000_usize;
    let curve = samples.saturating_mul(2).saturating_add(12);
    let header = [
        &big_endian(curve.saturating_add(144))[..],
        &[0; 4],
        &[2, 0x10, 0, 0],
        b"mntrGRAYXYZ ",
        &[0; 12],
        b"acsp",
        &[0; 28],
        // D50 (ICC.1:2022, 7.2.16).
        &[0, 0, 0xF6, 0xD6, 0, 1, 0, 0, 0, 0, 0xD3, 0x2D],
        &[0; 48],
        &big_endian(1),
        b"kTRC",
        &big_endian(144),
        &big_endian(curve),
        b"curv",
        &[0; 4],
        &big_endian(samples),
        &vec![0; samples.saturating_mul(2)],
    ]
    .concat();
    let image = [&[0, 0, 0, 8, 0, 0, 0, 8, 0, 1][..], &[7, 7, 0, 0]].concat();
    [
        jp2_box(*b"jP  ", &[&[0x0D, 0x0A, 0x87, 0x0A]]),
        jp2_box(*b"ftyp", &[b"jp2 ", &[0; 4], b"jp2 "]),
        jp2_box(
            *b"jp2h",
            &[
                &jp2_box(*b"ihdr", &[&image]),
                &jp2_box(*b"colr", &[&[2, 0, 0], &header]),
            ],
        ),
        jp2_box(*b"jp2c", &[&padded_codestream(82)]),
    ]
    .concat()
}

/// The page of 100 JPX images, each a stream of [`profiled_jp2`], each drawn once.
fn profiled_page() -> Made<Vec<u8>> {
    let file = profiled_jp2();
    let runs = file
        .chunk_by(|one, next| one == next)
        .map(|run| Ok((*run.first().ok_or("a run")?, u64::try_from(run.len())?)))
        .collect::<Made<Vec<_>>>()?;
    let image = stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /Filter [/FlateDecode /JPXDecode]",
        &zlib_runs(&runs)?,
    );
    let names = (0..100_u32)
        .map(|index| format!("/I{index} {} 0 R ", index.saturating_add(5)))
        .collect::<Vec<_>>()
        .concat();
    let content = (0..100_u32)
        .map(|index| format!("q 1 0 0 1 {index} 0 cm /I{index} Do Q\n"))
        .collect::<Vec<_>>()
        .concat();
    Ok(object_page(
        "",
        &format!("/XObject << {names}>>"),
        &content,
        vec![image; 100],
        "",
    ))
}

/// 100 JPX images, each with an ICC profile of 200 KB parsed, hold no more than a 16 MiB decoded limit allows.
#[test]
fn render_of_jpx_images_with_profiles_is_bounded() {
    let found = profiled_page()
        .map(|page| limited_to(page, Some(ICC_LIMIT)))
        .map_err(|error| error.to_string());
    assert!(matches!(&found, Ok((Ok(()), _))), "outcome {found:?}");
    let top = found.map_or(0, |(_, bytes)| bytes);
    assert!((1..=ICC_BOUND).contains(&top), "peak {top} bytes");
}

/// An ON set of 300,000 layers, held for the render, counts beside a JPX image of 60 MB decoded: together they exceed a
/// decoded limit of 62,000,000 (ISO 32000-2:2020, 8.11.4.3).
#[test]
fn render_of_a_held_layer_set_is_counted() {
    let made = zlib_runs(&[(0, 60_000_000)]).map(|data| {
        let on = (0..300_000_u32)
            .map(|index| format!("{} 0 R ", index.saturating_add(10)))
            .collect::<Vec<_>>()
            .concat();
        let objects = vec![
            b"<< /Type /OCG /Name (L) >>".to_vec(),
            format!("[{on}]").into_bytes(),
            stream(
                "/Type /XObject /Subtype /Image /Width 8 /Height 8 /Filter [/FlateDecode /JPXDecode]",
                &data,
            ),
        ];
        let page = object_page(
            "",
            "/Properties << /P 5 0 R >> /XObject << /X 7 0 R >>",
            "/OC /P BDC 0 0 100 100 re f EMC q 100 0 0 100 0 0 cm /X Do Q",
            objects,
            "/OCProperties << /OCGs [5 0 R] /D << /ON 6 0 R >> >>",
        );
        limited_to(page, Some(62_000_000))
    });
    let found = made.map_err(|error| error.to_string());
    assert!(
        matches!(&found, Ok((Err(ErrorCode::LimitExceeded), _))),
        "outcome {found:?}"
    );
    let top = found.map_or(0, |(_, bytes)| bytes);
    assert!((1..=LAYERS_BOUND).contains(&top), "peak {top} bytes");
}

/// A page with `more` in its dictionary whose resources, an object of their own, hold `entry`.
fn token_page(more: &str, entry: &str) -> Vec<u8> {
    written(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources 5 0 R /Contents 4 0 R {more} >>"
        )
        .into_bytes(),
        stream("", b"0 0 10 10 re f"),
        format!("<< {entry} >>").into_bytes(),
    ])
}

/// One token of 16 MiB in a page's resources, a string or a name, or a string in its group, fails `LimitExceeded`
/// under a decoded limit of 1 MiB without being held whole (ISO 32000-2:2020, 7.3.4, 7.3.5, 11.4.7).
#[test]
fn render_of_a_large_token_is_bounded() {
    let long = "a".repeat(0x100_0000);
    for (more, entry) in [
        (String::new(), format!("/Pad ({long})")),
        (String::new(), format!("/Pad /{long}")),
        (
            format!("/Group << /S /Transparency /Pad ({long}) >>"),
            String::new(),
        ),
    ] {
        let (result, top) = limited_to(token_page(&more, &entry), Some(0x10_0000));
        assert_eq!(result, Err(ErrorCode::LimitExceeded), "outcome");
        assert!((1..=TOKEN_BOUND).contains(&top), "peak {top} bytes");
    }
}

/// A page of 1000 x 1000 points shows glyph `a` of the first of 15 Type 3 fonts; each font's glyph is a text object of an
/// opaque square glyph, the next font's glyph, then a half-transparent square glyph, so each text object is a knockout
/// group from its first glyph (ISO 32000-2:2020, 9.3.8, 9.6.4, 11.4.6).
fn nested_text_page() -> Vec<u8> {
    let mut all = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1000 1000] /Resources << /Font << /T 6 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        stream("", b"BT /T 1000 Tf 0 0 Td (a) Tj ET"),
        // 5: a square glyph b in the text colour.
        b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs \
          << /b 21 0 R >> /Encoding << /Differences [98 /b] >> /FirstChar 98 /LastChar 98 /Widths [1000] >>"
            .to_vec(),
    ];
    // 6 to 20: the nested fonts; 22 to 36: their glyphs.
    for level in 0..15_u32 {
        let next = level.saturating_add(7);
        let inner = if level < 14 {
            format!(" /T {next} 0 R")
        } else {
            String::new()
        };
        all.push(
            format!(
                "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs \
                 << /a {} 0 R >> /Encoding << /Differences [97 /a] >> /FirstChar 97 /LastChar 97 /Widths [1000] \
                 /Resources << /Font << /B 5 0 R{inner} >> /ExtGState << /H << /ca 0.5 >> >> >> >>",
                level.saturating_add(22)
            )
            .into_bytes(),
        );
    }
    all.push(stream("", b"1000 0 0 0 1000 1000 d1 0 0 1000 1000 re f"));
    for level in 0..15_u32 {
        let inner = if level < 14 {
            "/T 1000 Tf 0 0 Td (a) Tj "
        } else {
            ""
        };
        all.push(stream(
            "",
            format!(
                "1000 0 d0 BT /B 1250 Tf 0 0 0 rg 0 0 Td (b) Tj {inner}/H gs 1 0 0 rg /B 200 Tf 300 300 Td (b) Tj ET"
            )
            .as_bytes(),
        ));
    }
    written(&all)
}

/// 15 nested text objects, each a knockout group from its first glyph, hold no more at 25 megapixels than measuring
/// counted.
#[test]
fn render_of_nested_knockout_text_is_bounded() {
    let (result, top) = limited_fit(nested_text_page(), None, Some(5_000));
    assert_eq!(result, Ok(()), "outcome");
    assert!((1..=NESTED_TEXT_BOUND).contains(&top), "peak {top} bytes");
}

/// A page of 400 forms, each painting 500 distinct type 4 shadings, each `shading` (ISO 32000-2:2020, 8.7.4.5.5, 8.10).
fn many_meshes_page(shading: &[u8]) -> Vec<u8> {
    let (forms, each) = (400_usize, 500_usize);
    let first = forms.saturating_mul(2).saturating_add(5);
    let names = (0..forms)
        .map(|form| format!("/F{form} {} 0 R ", form.saturating_mul(2).saturating_add(5)))
        .collect::<Vec<_>>()
        .concat();
    let content = (0..forms)
        .map(|form| format!("/F{form} Do\n"))
        .collect::<Vec<_>>()
        .concat();
    let mut all = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << {names}>> >> /Contents 4 0 R >>"
        )
        .into_bytes(),
        stream("", content.as_bytes()),
    ];
    for form in 0..forms {
        let shadings = (0..each).map(|at| form.saturating_mul(each).saturating_add(at));
        let painted = shadings
            .clone()
            .map(|index| format!("/S{index} sh\n"))
            .collect::<Vec<_>>()
            .concat();
        let listed = shadings
            .map(|index| format!("/S{index} {} 0 R ", index.saturating_add(first)))
            .collect::<Vec<_>>()
            .concat();
        all.push(stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources {} 0 R",
                form.saturating_mul(2).saturating_add(6)
            ),
            painted.as_bytes(),
        ));
        all.push(format!("<< /Shading << {listed}>> >>").into_bytes());
    }
    all.extend(iter::repeat_n(shading.to_vec(), forms.saturating_mul(each)));
    written(&all)
}

/// 200,000 distinct mesh shadings in forms, empty or of damaged Flate data, read until the work runs out, hold no more
/// than as many plain shading dictionaries do: what is kept of each is counted (ISO 32000-2:2020, 7.4.4, 8.7.4.5).
#[test]
fn render_of_many_small_meshes_is_bounded() {
    for shading in [
        stream(MESH_HEAD, b""),
        stream(&format!("{MESH_HEAD} /Filter /FlateDecode"), b"zz"),
    ] {
        let (result, top) = limited_fit(many_meshes_page(&shading), None, Some(64));
        assert_eq!(result, Err(ErrorCode::LimitExceeded), "outcome");
        assert!((1..=MESHES_BOUND).contains(&top), "peak {top} bytes");
    }
}

/// A marker segment of `body` (ITU-T T.81, B.1.1.4).
fn segment(marker: u8, body: &[u8]) -> Made<Vec<u8>> {
    let length = u16::try_from(body.len().saturating_add(2))?;
    Ok([
        &[0xFF, marker][..],
        &[
            u8::try_from(length.wrapping_shr(8))?,
            u8::try_from(length & 0xFF)?,
        ],
        body,
    ]
    .concat())
}

/// A 64 x 64 grey progressive JPEG of a DC scan, then 63 AC refinement scans, each one end-of-band run; all coefficients
/// 0 (ITU-T T.81, G.1).
fn many_scans_jpeg() -> Made<Vec<u8>> {
    let mut file = vec![0xFF, 0xD8];
    file.extend(segment(0xDB, &[&[0][..], &[1; 64]].concat())?);
    file.extend(segment(0xC2, &[8, 0, 64, 0, 64, 1, 1, 0x11, 0])?);
    // DC: one code, of category 0; AC: one code, EOB13.
    file.extend(segment(
        0xC4,
        &[&[0x00, 1][..], &[0; 15], &[0x00]].concat(),
    )?);
    file.extend(segment(
        0xC4,
        &[&[0x10, 1][..], &[0; 15], &[0xD0]].concat(),
    )?);
    file.extend(segment(0xDA, &[1, 1, 0, 0, 0, 0])?);
    file.extend([0; 12]);
    for _ in 0..63_u8 {
        file.extend(segment(0xDA, &[1, 1, 0, 1, 63, 0x21])?);
        file.extend([0; 4]);
    }
    file.extend([0xFF, 0xD9]);
    Ok(file)
}

/// The page of [`many_scans_jpeg`] drawn once, behind 15 Flate stages, with a soft mask of the same when `masked`
/// (ISO 32000-2:2020, 7.4.4, 7.4.8, 11.6.5.3).
fn many_scans_page(masked: bool) -> Made<Vec<u8>> {
    let mut data = many_scans_jpeg()?;
    for _ in 0..15_u8 {
        let runs = data
            .chunk_by(|one, next| one == next)
            .map(|run| Ok((*run.first().ok_or("a run")?, u64::try_from(run.len())?)))
            .collect::<Made<Vec<_>>>()?;
        data = zlib_runs(&runs)?;
    }
    let filters = "/FlateDecode ".repeat(15);
    let entries = format!(
        "/Type /XObject /Subtype /Image /Width 64 /Height 64 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter [{filters}/DCTDecode]"
    );
    let mask = if masked { " /SMask 6 0 R" } else { "" };
    let image = stream(&format!("{entries}{mask}"), &data);
    Ok(object_page(
        "",
        "/XObject << /I 5 0 R >>",
        "q 100 0 0 100 0 0 cm /I Do Q",
        vec![image, stream(&entries, &data)],
        "",
    ))
}

/// The scans of a progressive JPEG behind 15 Flate stages, each read through every stage at once, are counted: under a
/// decoded limit of 4,000,000 they fail `LimitExceeded` holding no more than it allows (ITU-T T.81, G.1;
/// ISO 32000-2:2020, 7.4.4).
#[test]
fn render_of_scans_behind_many_filters_is_bounded() {
    let found = many_scans_page(false)
        .map(|page| limited_to(page, Some(4_000_000)))
        .map_err(|error| error.to_string());
    assert!(
        matches!(&found, Ok((Err(ErrorCode::LimitExceeded), _))),
        "outcome {found:?}"
    );
    let top = found.map_or(0, |(_, bytes)| bytes);
    assert!((1..=SCANS_BOUND).contains(&top), "peak {top} bytes");
}

/// An image and its soft mask, each a progressive JPEG behind 15 Flate stages whose decoders are alive together, count
/// together: under a decoded limit one fits and two do not, the page fails `LimitExceeded` (ISO 32000-2:2020, 11.6.5.3).
#[test]
fn render_of_an_image_and_its_mask_decoders_is_bounded() {
    let found = many_scans_page(true)
        .map(|page| limited_to(page, Some(TWO_SCANS_LIMIT)))
        .map_err(|error| error.to_string());
    assert!(
        matches!(&found, Ok((Err(ErrorCode::LimitExceeded), _))),
        "outcome {found:?}"
    );
    let top = found.map_or(0, |(_, bytes)| bytes);
    assert!((1..=TWO_SCANS_BOUND).contains(&top), "peak {top} bytes");
}

/// A page whose content and 10 nested forms each have resources of their own, the same 64 direct fonts, each named by
/// 20,000 bytes, and each set every font with `Tf` (ISO 32000-2:2020, 7.8.3, 8.10, 9.3.1).
fn font_names_page() -> Made<Vec<u8>> {
    let names = (0..64_u32)
        .map(|index| format!("N{index:02}{}", "A".repeat(20_000)))
        .collect::<Vec<_>>();
    let fonts = names
        .iter()
        .map(|name| format!("/{name} << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> "))
        .collect::<Vec<_>>()
        .concat();
    let forms = (1..=10_u32)
        .map(|level| format!("/F{level} {} 0 R ", level.saturating_add(5)))
        .collect::<Vec<_>>()
        .concat();
    let content = |next: Option<u32>| -> Made<Vec<u8>> {
        let set = names
            .iter()
            .map(|name| format!("/{name} 1 Tf\n"))
            .collect::<Vec<_>>()
            .concat();
        let tail = next.map_or_else(String::new, |level| format!("/F{level} Do\n"));
        let text = format!("BT\n{set}ET\n{tail}");
        let runs = text
            .as_bytes()
            .chunk_by(|one, other| one == other)
            .map(|run| Ok((*run.first().ok_or("a run")?, u64::try_from(run.len())?)))
            .collect::<Made<Vec<_>>>()?;
        zlib_runs(&runs)
    };
    let mut all = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 5 0 R /Resources 4 0 R >>"
            .to_vec(),
        format!("<< /Font << {fonts}>> /XObject << {forms}>> >>").into_bytes(),
        stream("/Filter /FlateDecode", &content(Some(1))?),
    ];
    for level in 1..=10_u32 {
        all.push(stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources {} 0 R /Filter /FlateDecode",
                level.saturating_add(15)
            ),
            &content((level < 10).then_some(level.saturating_add(1)))?,
        ));
    }
    // 16 to 25: each form's resources, the same as the page's.
    let resources = all.get(3).cloned().ok_or("the resources")?;
    all.extend(iter::repeat_n(resources, 10));
    Ok(written(&all))
}

/// Fonts given directly in the resources of each of 10 nested forms, named by 20,000 bytes, past the 127 bytes a name
/// may have, are not kept by their names: the page holds no more than about a 40 MiB decoded limit allows
/// (ISO 32000-2:2020, Annex C).
#[test]
fn render_of_nested_forms_with_long_font_names_is_bounded() {
    let found = font_names_page()
        .map(|page| limited_to(page, Some(FONT_NAMES_LIMIT)))
        .map_err(|error| error.to_string());
    assert!(matches!(&found, Ok((Ok(()), _))), "outcome {found:?}");
    let top = found.map_or(0, |(_, bytes)| bytes);
    assert!((1..=FONT_NAMES_BOUND).contains(&top), "peak {top} bytes");
}

/// A page that draws the first of 30 nested forms, each drawing the next, each by a name of `long` bytes more than its
/// level, and each with a string of `string` bytes left before its `Do` when `string` is not 0
/// (ISO 32000-2:2020, 7.3.4.2, 7.3.5, 8.10).
fn nested_forms_page(long: usize, string: usize) -> Made<Vec<u8>> {
    let name = |level: u32| format!("X{level:02}{}", "A".repeat(long));
    let left = if string == 0 {
        String::new()
    } else {
        format!("({}) ", "A".repeat(string))
    };
    let compressed = |text: String| -> Made<Vec<u8>> {
        let runs = text
            .as_bytes()
            .chunk_by(|one, other| one == other)
            .map(|run| Ok((*run.first().ok_or("a run")?, u64::try_from(run.len())?)))
            .collect::<Made<Vec<_>>>()?;
        zlib_runs(&runs)
    };
    let mut all = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /XObject << /{} 5 0 R >> >> >>",
            name(0)
        )
        .into_bytes(),
        stream("/Filter /FlateDecode", &compressed(format!("/{} Do", name(0)))?),
    ];
    // 5 to 34: the forms, form n drawing form n + 1 by the name of level n + 1.
    for level in 0..30_u32 {
        let next = level.saturating_add(1);
        let (resources, content) = if next < 30 {
            (
                format!(
                    "<< /XObject << /{} {} 0 R >> >>",
                    name(next),
                    next.saturating_add(5)
                ),
                format!("{left}/{} Do", name(next)),
            )
        } else {
            (String::from("<< >>"), String::from("0 0 10 10 re f"))
        };
        all.push(stream(
            &format!("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources {resources} /Filter /FlateDecode"),
            &compressed(content)?,
        ));
    }
    Ok(written(&all))
}

/// The names that reach 30 nested forms, each of 500,000 bytes, are counted while their forms run: the page fails
/// `LimitExceeded` holding no more than [`DO_NAMES_BOUND`].
#[test]
fn render_of_nested_forms_reached_by_long_names_is_bounded() {
    let found = nested_forms_page(500_000, 0)
        .map(|page| limited_to(page, Some(DO_NAMES_LIMIT)))
        .map_err(|error| error.to_string());
    assert!(
        matches!(&found, Ok((Err(ErrorCode::LimitExceeded), _))),
        "outcome {found:?}"
    );
    let top = found.map_or(0, |(_, bytes)| bytes);
    assert!((1..=DO_NAMES_BOUND).contains(&top), "peak {top} bytes");
}

/// A string of 900,000 bytes left among the operands of each of 30 nested forms is counted, at what it holds, while the
/// forms inside run: the page fails `LimitExceeded` holding no more than [`STRINGS_BOUND`].
#[test]
fn render_of_nested_forms_after_long_strings_is_bounded() {
    let found = nested_forms_page(0, 900_000)
        .map(|page| limited_to(page, Some(STRINGS_LIMIT)))
        .map_err(|error| error.to_string());
    assert!(
        matches!(&found, Ok((Err(ErrorCode::LimitExceeded), _))),
        "outcome {found:?}"
    );
    let top = found.map_or(0, |(_, bytes)| bytes);
    assert!((1..=STRINGS_BOUND).contains(&top), "peak {top} bytes");
}

/// A page that draws the first of 31 nested forms, each drawing the next before a comment of `pad` bytes, each content
/// stream behind 16 Flate stages, the innermost with its Adler-32 checksum inverted when `damaged`
/// (ISO 32000-2:2020, 7.2.4, 7.4.4, 8.10; RFC 1950, 2.2).
fn staged_forms_page(pad: usize, damaged: bool) -> Made<Vec<u8>> {
    let staged = |text: String| -> Made<Vec<u8>> {
        let mut data = text.into_bytes();
        for stage in 0..16_u8 {
            let runs = data
                .chunk_by(|one, other| one == other)
                .map(|run| Ok((*run.first().ok_or("a run")?, u64::try_from(run.len())?)))
                .collect::<Made<Vec<_>>>()?;
            data = zlib_runs(&runs)?;
            if damaged && stage == 0 {
                let start = data.len().checked_sub(4).ok_or("a checksum")?;
                for byte in data.get_mut(start..).ok_or("a checksum")? {
                    *byte = !*byte;
                }
            }
        }
        Ok(data)
    };
    let filters = "/FlateDecode ".repeat(16);
    let mut all =
        vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /XObject << /X 5 0 R >> >> >>"
            .to_vec(),
        stream("", b"/X Do"),
    ];
    // 5 to 35: the forms, each drawing the next.
    for level in 0..31_u32 {
        let next = level.saturating_add(6);
        let (resources, content) = if level < 30 {
            (
                format!("<< /XObject << /X {next} 0 R >> >>"),
                format!("/X Do\n%{}\n", "A".repeat(pad)),
            )
        } else {
            (String::from("<< >>"), String::from("0 0 10 10 re f"))
        };
        all.push(stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources {resources} /Filter [{filters}]"
            ),
            &staged(content)?,
        ));
    }
    Ok(written(&all))
}

/// The filter stages of 31 nested content streams, 16 each, open together while the forms inside run, are counted, also
/// when each stream is read again after damage at its end: the page fails `LimitExceeded` holding no more than
/// [`STAGES_BOUND`] (ISO 32000-2:2020, 7.8.2).
#[test]
fn render_of_nested_forms_behind_many_stages_is_bounded() {
    // Past the 64 KiB read ahead, so the stream stays open; or within it, so its damage is found there.
    for (pad, damaged) in [(200_000, false), (40_000, true)] {
        let found = staged_forms_page(pad, damaged)
            .map(|page| limited_to(page, Some(STAGES_LIMIT)))
            .map_err(|error| error.to_string());
        assert!(
            matches!(&found, Ok((Err(ErrorCode::LimitExceeded), _))),
            "outcome {found:?} damaged {damaged}"
        );
        let top = found.map_or(0, |(_, bytes)| bytes);
        assert!(
            (1..=STAGES_BOUND).contains(&top),
            "peak {top} bytes damaged {damaged}"
        );
    }
}

/// `input` behind `stages` Flate stages with [`PREDICTOR`], each stage one row of filter type None: `data`, then zeros
/// to the row's end (PNG, 9.2).
fn predicted(input: &[u8], stages: u8) -> Made<Vec<u8>> {
    let width = 0x40_0000_u64;
    let mut data = input.to_vec();
    for _ in 0..stages {
        let pad = width
            .checked_sub(u64::try_from(data.len())?)
            .ok_or("a row")?;
        let mut runs = vec![(0, 1)];
        for run in data.chunk_by(|one, other| one == other) {
            runs.push((*run.first().ok_or("a run")?, u64::try_from(run.len())?));
        }
        if pad > 0 {
            runs.push((0, pad));
        }
        data = zlib_runs(&runs)?;
    }
    Ok(data)
}

/// A page that draws the first of 31 nested forms, each drawing the next, each content stream `rows` rows of `columns`
/// columns of 32 colours of 16 bits behind a Flate stage with a PNG predictor, its operator in the last row
/// (ISO 32000-2:2020, 7.4.4.4, 8.10; PNG, 9).
fn predicted_forms_page(columns: u32, rows: u8) -> Made<Vec<u8>> {
    let width = u64::from(columns).checked_mul(64).ok_or("a row")?;
    let content = |text: &str| -> Made<Vec<u8>> {
        let mut runs = Vec::new();
        for row in 1..=rows {
            let shown = if row == rows { text } else { "" };
            let length = u64::try_from(shown.len())?;
            let pad = width
                .checked_sub(length)
                .and_then(|rest| rest.checked_sub(2))
                .ok_or("a row")?;
            // The row's filter type, None, then the row: the text, then a comment to its end.
            runs.push((0, 1));
            runs.extend(shown.bytes().map(|byte| (byte, 1)));
            runs.extend([(b'%', 1), (b'A', pad), (b'\n', 1)]);
        }
        zlib_runs(&runs)
    };
    let entries = format!(
        "/Filter /FlateDecode /DecodeParms << /Predictor 10 /Colors 32 /BitsPerComponent 16 /Columns {columns} >>"
    );
    let mut all = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /XObject << /X 5 0 R >> >> >>"
            .to_vec(),
        stream("", b"/X Do"),
    ];
    // 5 to 35: the forms, each drawing the next.
    for level in 0..31_u32 {
        let (resources, text) = if level < 30 {
            (
                format!("<< /XObject << /X {} 0 R >> >>", level.saturating_add(6)),
                "/X Do\n",
            )
        } else {
            (String::from("<< >>"), "0 0 10 10 re f\n")
        };
        all.push(stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources {resources} {entries}"
            ),
            &content(text)?,
        ));
    }
    Ok(written(&all))
}

/// The predictor rows of 31 nested content streams, held together while the forms inside run, are counted at what they
/// hold: one row of 4 MiB, or two rows of 2 MiB and 64 bytes, a width rows grow past by doubling; the page fails
/// `LimitExceeded` holding no more than [`PREDICTORS_BOUND`].
#[test]
fn render_of_nested_forms_behind_predictors_is_bounded() {
    for (columns, rows) in [(0x1_0000, 1), (0x8001, 2)] {
        let found = predicted_forms_page(columns, rows)
            .map(|page| limited_to(page, Some(PREDICTORS_LIMIT)))
            .map_err(|error| error.to_string());
        assert!(
            matches!(&found, Ok((Err(ErrorCode::LimitExceeded), _))),
            "outcome {found:?} columns {columns}"
        );
        let top = found.map_or(0, |(_, bytes)| bytes);
        assert!(
            (1..=PREDICTORS_BOUND).contains(&top),
            "peak {top} bytes columns {columns}"
        );
    }
}

/// A page whose graphics state parameters are packed in an object stream behind 16 Flate stages with [`PREDICTOR`]
/// (ISO 32000-2:2020, 7.5.7, 7.5.8).
fn predicted_container_page() -> Made<Vec<u8>> {
    let head = "5 0 ";
    contained_page(stream(
        &format!(
            "/Type /ObjStm /N 1 /First {} /Filter [{}] /DecodeParms [{}]",
            head.len(),
            "/FlateDecode ".repeat(16),
            format!("{PREDICTOR} ").repeat(16)
        ),
        &predicted(format!("{head}<< /CA 0.5 /ca 0.5 >>").as_bytes(), 16)?,
    ))
}

/// A page whose graphics state parameters, object 5, are first in `container`, object 6, by a cross-reference stream
/// (ISO 32000-2:2020, 7.5.7, 7.5.8).
fn contained_page(container: Vec<u8>) -> Made<Vec<u8>> {
    let plain = [
        (1_u32, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
        (
            3,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /ExtGState << /G 5 0 R >> >> >>"
                .to_vec(),
        ),
        (4, stream("", b"/G gs 0 0 10 10 re f")),
        (6, container),
    ];
    let mut file = b"%PDF-1.7\n".to_vec();
    // Each entry: type, offset or container, generation or index (7.5.8.3).
    let mut rows = [[0_u8; 7]; 8];
    if let Some(free) = rows.first_mut() {
        *free = [0, 0, 0, 0, 0, 0xFF, 0xFF];
    }
    if let Some(packed) = rows.get_mut(5) {
        *packed = [2, 0, 0, 0, 6, 0, 0];
    }
    let mut place = |number: u32, at: usize| -> Made<()> {
        let offset = u32::try_from(at)?;
        let byte = |shift: u32| u8::try_from(offset.wrapping_shr(shift) & 0xFF);
        *rows.get_mut(usize::try_from(number)?).ok_or("an entry")? =
            [1, byte(24)?, byte(16)?, byte(8)?, byte(0)?, 0, 0];
        Ok(())
    };
    for (number, body) in plain {
        place(number, file.len())?;
        file.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        file.extend(body);
        file.extend_from_slice(b"\nendobj\n");
    }
    let start = file.len();
    place(7, start)?;
    let table = rows.concat();
    file.extend_from_slice(b"7 0 obj\n");
    file.extend(stream("/Type /XRef /Size 8 /W [1 4 2] /Root 1 0 R", &table));
    file.extend_from_slice(format!("\nendobj\nstartxref\n{start}\n%%EOF\n").as_bytes());
    Ok(file)
}

/// The predictor rows of an object stream's 16 stages, read during the render, are counted together: the page fails
/// `LimitExceeded` holding no more than [`PREDICTORS_BOUND`].
#[test]
fn render_of_a_container_behind_predictors_is_bounded() {
    let found = predicted_container_page()
        .map(|page| limited_to(page, Some(PREDICTORS_LIMIT)))
        .map_err(|error| error.to_string());
    assert!(
        matches!(&found, Ok((Err(ErrorCode::LimitExceeded), _))),
        "outcome {found:?}"
    );
    let top = found.map_or(0, |(_, bytes)| bytes);
    assert!((1..=PREDICTORS_BOUND).contains(&top), "peak {top} bytes");
}

/// A page whose graphics state parameters are first of 131,073 members of an object stream, the rest pairs of no member
/// (ISO 32000-2:2020, 7.5.7).
fn many_members_page() -> Made<Vec<u8>> {
    let pairs = format!("5 0 {}", "1 0 ".repeat(0x2_0000));
    contained_page(stream(
        &format!("/Type /ObjStm /N 131073 /First {}", pairs.len()),
        format!("{pairs}<< /CA 0.5 /ca 0.5 >>").as_bytes(),
    ))
}

/// The table of an object stream's 131,073 members, read during the render, is counted: the page holds no more than
/// [`MEMBERS_BOUND`].
#[test]
fn render_of_a_container_of_many_members_is_bounded() {
    let found = many_members_page()
        .map(|page| limited_to(page, Some(MEMBERS_LIMIT)))
        .map_err(|error| error.to_string());
    assert!(matches!(&found, Ok((Ok(()), _))), "outcome {found:?}");
    let top = found.map_or(0, |(_, bytes)| bytes);
    assert!((1..=MEMBERS_BOUND).contains(&top), "peak {top} bytes");
}

/// A page of an inline image behind `/AHx` and Flate stages with [`PREDICTOR`]: when `jpeg`, 12 MiB of DCT data, all zeros,
/// behind one stage, read whole before it is decoded; otherwise a byte behind 15 stages (ISO 32000-2:2020, 8.9.7).
fn predicted_inline_page(jpeg: bool) -> Made<Vec<u8>> {
    let (data, filters, parameters) = if jpeg {
        // Three rows of filter type None, of 4 MiB zeros each (PNG, 9.2).
        (
            zlib_runs(&[(0, 0x40_0001 * 3)])?,
            String::from("/Fl /DCT"),
            format!("{PREDICTOR} null"),
        )
    } else {
        (
            predicted(b"\x80", 15)?,
            "/Fl ".repeat(15),
            format!("{PREDICTOR} ").repeat(15),
        )
    };
    let hex = data
        .iter()
        .flat_map(|byte| [byte.wrapping_shr(4), byte & 0xF])
        .map(|digit| char::from_digit(u32::from(digit), 16))
        .collect::<Option<String>>()
        .ok_or("a digit")?;
    let content = format!(
        "q 10 0 0 10 0 0 cm BI /W 8 /H 8 /BPC 8 /CS /G /F [/AHx {filters}] /DP [null {parameters}] ID {hex}> EI Q"
    );
    Ok(object_page("", "", &content, Vec::new(), ""))
}

/// The predictor rows of an inline image's stages are counted while it is read: under 15 stages, or beside its DCT data
/// read whole, the page fails `LimitExceeded` holding no more than [`PREDICTORS_BOUND`].
#[test]
fn render_of_an_inline_image_behind_predictors_is_bounded() {
    for jpeg in [false, true] {
        let found = predicted_inline_page(jpeg)
            .map(|page| limited_to(page, Some(PREDICTORS_LIMIT)))
            .map_err(|error| error.to_string());
        assert!(
            matches!(&found, Ok((Err(ErrorCode::LimitExceeded), _))),
            "outcome {found:?} jpeg {jpeg}"
        );
        let top = found.map_or(0, |(_, bytes)| bytes);
        assert!(
            (1..=PREDICTORS_BOUND).contains(&top),
            "peak {top} bytes jpeg {jpeg}"
        );
    }
}

/// A page of a blue square whose cross-reference stream declares 6 entries of 6 bytes, then decodes to 100,000,000 zero bytes
/// more (ISO 32000-2:2020, 7.5.8).
fn long_section_page() -> Made<Vec<u8>> {
    let content = b"0 0 1 rg 10 10 80 80 re f";
    let plain = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>".to_vec(),
        stream("", content),
    ];
    let mut file = b"%PDF-1.7\n".to_vec();
    // Each entry: type, offset, generation (7.5.8.3).
    let mut runs = vec![(0, 5), (0xFF, 1)];
    let mut entry = |at: usize| -> Made<()> {
        let offset = u32::try_from(at)?;
        runs.push((1, 1));
        for shift in [24_u32, 16, 8, 0] {
            runs.push((u8::try_from(offset.wrapping_shr(shift) & 0xFF)?, 1));
        }
        runs.push((0, 1));
        Ok(())
    };
    for (object, number) in plain.iter().zip(1_u32..) {
        entry(file.len())?;
        file.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        file.extend_from_slice(object);
        file.extend_from_slice(b"\nendobj\n");
    }
    let start = file.len();
    entry(start)?;
    runs.push((0, 100_000_000));
    let table = zlib_runs(&runs)?;
    file.extend_from_slice(b"5 0 obj\n");
    file.extend(stream(
        "/Type /XRef /Size 6 /W [1 4 1] /Root 1 0 R /Filter /FlateDecode",
        &table,
    ));
    file.extend_from_slice(format!("\nendobj\nstartxref\n{start}\n%%EOF\n").as_bytes());
    Ok(file)
}

/// A cross-reference stream that decodes to far more than its entries declare is read no further than they reach: the page
/// renders holding no more than [`LONG_SECTION_BOUND`].
#[test]
fn render_through_a_long_section_is_bounded() {
    let found = long_section_page()
        .map(|page| limited_to(page, None))
        .map_err(|error| error.to_string());
    assert!(matches!(&found, Ok((Ok(()), _))), "outcome {found:?}");
    let top = found.map_or(0, |(_, bytes)| bytes);
    assert!((1..=LONG_SECTION_BOUND).contains(&top), "peak {top} bytes");
}
