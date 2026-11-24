//! Rendering through the public API, where the conformance suite cannot reach.

extern crate alloc;

#[cfg(feature = "rendering")]
#[path = "conformance/decode.rs"]
mod decode;

use alloc::sync::Arc;
use core::error::Error;
use std::fs;

#[cfg(feature = "rendering")]
use core::sync::atomic::{AtomicU64, Ordering};
use starlightpdf::ErrorCode;
#[cfg(feature = "rendering")]
use starlightpdf::PdfError;
#[cfg(feature = "rendering")]
use starlightpdf::arrange::InsertBlankPageOptions;
#[cfg(feature = "rendering")]
use starlightpdf::document::CreateOptions;
use starlightpdf::document::{Document, OpenOptions, PageOptions};
#[cfg(feature = "rendering")]
use starlightpdf::drawing::DrawImageOptions;
#[cfg(feature = "rendering")]
use starlightpdf::images::RasterFormat;
use starlightpdf::input::{BytesSourceOptions, Source};
#[cfg(feature = "rendering")]
use starlightpdf::limits::{ResourcePolicy, ResourcePolicyOptions};
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf::page::Page;
#[cfg(feature = "rendering")]
use starlightpdf::progress::Progress;
#[cfg(feature = "rendering")]
use starlightpdf::rendering::ColorSpace;
use starlightpdf::rendering::RenderOptions;
use starlightpdf::{Capability, capabilities};

#[cfg(feature = "rendering")]
use decode::Image;

/// A page of 100 x 100 points in quarters: red, green, blue and none.
const COLOURS: &str = "output-colours.pdf";

/// A JPEG's markers, each with its segment.
#[cfg(feature = "rendering")]
type Segments<'file> = Vec<(u8, &'file [u8])>;

/// What a test returns.
type Outcome = Result<(), Box<dyn Error>>;

/// A progress callback that lets `allowed` calls through, then returns false; it counts every call.
#[cfg(feature = "rendering")]
struct After {
    /// The calls let through.
    allowed: u64,
    /// The calls made.
    calls: AtomicU64,
}

#[cfg(feature = "rendering")]
impl Progress for After {
    fn update(&self, done: u64, total: u64) -> bool {
        let _: (u64, u64) = (done, total);
        self.calls.fetch_add(1, Ordering::Relaxed) < self.allowed
    }
}

/// The first page of the corpus file `name` of `tests/corpus/render`.
fn page(name: &str) -> Result<(Document, Page), Box<dyn Error>> {
    let mut options = BytesSourceOptions::default();
    options.bytes = Arc::new(fs::read(format!(
        "{}/../tests/corpus/render/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))?);
    let source = Source::from_bytes(&options)?;
    let document = Document::open(&source, None, &OpenOptions::default())?;
    let first = document.page(&PageOptions::default())?;
    Ok((document, first))
}

/// What rendering `shown` with `options` writes.
#[cfg(feature = "rendering")]
fn rendered(shown: &Page, options: &RenderOptions) -> Result<Vec<u8>, PdfError> {
    let sink = Sink::in_memory(&MemorySinkOptions::default())?;
    shown.render(&sink, None, options)?;
    sink.take_bytes()
}

/// `capabilities` lists `Rendering` exactly where it is built in
/// (.architecture-decisions/packaging-capabilities-and-versions.md).
#[test]
fn capabilities_list_rendering() {
    assert_eq!(
        capabilities()
            .map(|listed| listed.contains(&Capability::Rendering))
            .map_err(|error| error.code()),
        Ok(cfg!(feature = "rendering")),
        "capabilities"
    );
}

/// Built without rendering, render fails with `CapabilityMissing`.
#[cfg(not(feature = "rendering"))]
#[test]
fn render_without_rendering_fails_capability_missing() -> Outcome {
    let (_document, shown) = page(COLOURS)?;
    let sink = Sink::in_memory(&MemorySinkOptions::default())?;
    let code = shown
        .render(&sink, None, &RenderOptions::default())
        .map_err(|error| error.code());
    if code == Err(ErrorCode::CapabilityMissing) {
        Ok(())
    } else {
        Err(format!("{code:?}").into())
    }
}

/// The marker segments of a JPEG, those after a scan included, up to a closing EOI (ITU-T T.81, B.1.1).
#[cfg(feature = "rendering")]
fn segments(file: &[u8]) -> Result<Segments<'_>, Box<dyn Error>> {
    let body = file
        .strip_prefix(&[0xFF, 0xD8])
        .and_then(|rest| rest.strip_suffix(&[0xFF, 0xD9]))
        .ok_or("no SOI and EOI")?;
    let mut found = Vec::new();
    let mut rest = body;
    while let Some((&first, after)) = rest.split_first() {
        rest = after;
        if first != 0xFF {
            continue;
        }
        let (&marker, following) = after.split_first().ok_or("a lone FF")?;
        rest = following;
        // A stuffed byte and a restart marker have no length.
        if marker == 0 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }
        let (&[high, low], tail) = following.split_first_chunk().ok_or("a short segment")?;
        let length = usize::from(high)
            .saturating_mul(256)
            .saturating_add(usize::from(low));
        let (segment, next) = length
            .checked_sub(2)
            .and_then(|size| tail.split_at_checked(size))
            .ok_or("a short segment")?;
        found.push((marker, segment));
        rest = next;
    }
    Ok(found)
}

/// JPEG output is baseline: one SOF0 of 8-bit samples, one scan, Huffman coding (ITU-T T.81, B.2, F.1.1).
#[cfg(feature = "rendering")]
#[test]
fn render_writes_baseline_jpeg() -> Outcome {
    let (_document, shown) = page(COLOURS)?;
    for (space, components) in [(ColorSpace::Rgb, 3), (ColorSpace::Gray, 1)] {
        let mut options = RenderOptions::default();
        options.format = RasterFormat::Jpeg;
        options.color_space = space;
        let file = rendered(&shown, &options)?;
        let found = segments(&file)?;
        let frames: Vec<(u8, Vec<u8>)> = found
            .iter()
            .filter(|segment| matches!(segment.0, 0xC0..=0xC3 | 0xC5..=0xCF))
            .map(|&(marker, body)| (marker, body.iter().copied().take(6).collect()))
            .collect();
        if frames != [(0xC0, vec![8, 0, 100, 0, 100, components])] {
            return Err(format!("frames {frames:?}").into());
        }
        if found.iter().filter(|segment| segment.0 == 0xDA).count() != 1 {
            return Err("more or fewer scans than one".into());
        }
    }
    Ok(())
}

/// `quality` sets the quantisation (ITU-T T.81, B.2.4.1): finer as it rises, 85 when unset.
#[cfg(feature = "rendering")]
#[test]
fn render_quality_sets_jpeg_quality() -> Outcome {
    let (_document, shown) = page(COLOURS)?;
    let written = |quality| {
        let mut options = RenderOptions::default();
        options.format = RasterFormat::Jpeg;
        options.quality = quality;
        rendered(&shown, &options)
    };
    let mut coarser = u64::MAX;
    for quality in [1, 50, 85, 100] {
        let file = written(Some(quality))?;
        let total = segments(&file)?
            .iter()
            .filter(|segment| segment.0 == 0xDB)
            .flat_map(|segment| segment.1)
            .fold(0_u64, |sum, &step| sum.saturating_add(u64::from(step)));
        if total == 0 || total >= coarser {
            return Err(format!("quality {quality}: steps total {total}, before {coarser}").into());
        }
        coarser = total;
    }
    if written(None)? != written(Some(85))? {
        return Err("unset is not 85".into());
    }
    Ok(())
}

/// What render writes reads back: a PNG drawn on a blank page renders, unsaved, to the same bytes; a JPEG at quality
/// 100 to within 4 of every sample, edges inside its blocks included (ITU-T T.81, A.3, F.1.1, F.2.1).
#[cfg(feature = "rendering")]
#[test]
fn render_output_reads_back() -> Outcome {
    let (_document, shown) = page("text-type1.pdf")?;
    let defaults = RenderOptions::default();
    let png = rendered(&shown, &defaults)?;
    let mut lossy = RenderOptions::default();
    lossy.format = RasterFormat::Jpeg;
    lossy.quality = Some(100);
    let mut again = Vec::new();
    for file in [png.clone(), rendered(&shown, &lossy)?] {
        let document = Document::create(None, &CreateOptions::default())?;
        let mut sheet = InsertBlankPageOptions::default();
        sheet.width = Some(100.0_f64);
        sheet.height = Some(100.0_f64);
        let blank = document.insert_blank_page(&sheet)?;
        let mut source = BytesSourceOptions::default();
        source.bytes = Arc::new(file);
        let mut draw = DrawImageOptions::default();
        draw.width = Some(100.0_f64);
        draw.height = Some(100.0_f64);
        blank.draw_image(&Source::from_bytes(&source)?, None, None, &draw)?;
        again.push(rendered(&blank, &defaults)?);
    }
    let (Some(from_png), Some(from_jpeg)) = (again.first(), again.get(1)) else {
        return Err("not two renders".into());
    };
    if *from_png != png {
        return Err("the PNG read back differs".into());
    }
    let (want, have) = (Image::read(&png)?, Image::read(from_jpeg)?);
    let shape = |image: &Image| (image.width, image.height, image.channels, image.jpeg);
    if shape(&want) != shape(&have) || want.pixel(50, 50) == want.pixel(5, 5) {
        return Err("the renders differ in shape, or the page is blank".into());
    }
    let far = want
        .samples
        .iter()
        .zip(&have.samples)
        .map(|(one, other)| one.abs_diff(*other))
        .max()
        .unwrap_or(u8::MAX);
    if far > 4 {
        return Err(format!("the JPEG read back differs by {far}").into());
    }
    Ok(())
}

/// An interlaced PNG drawn and not saved is held whole while it renders, and that counts: 1,414 x 1,414 grey and alpha
/// samples fail `LimitExceeded` under a decoded limit of 4 MiB, where the same image not interlaced renders
/// (W3C PNG, 8.2).
#[cfg(feature = "rendering")]
#[test]
fn render_counts_a_drawn_interlaced_png() -> Outcome {
    let mut limits = ResourcePolicyOptions::default();
    limits.decoded_limit = Some(0x40_0000);
    let policy = ResourcePolicy::new(&limits)?;
    let mut outcomes = Vec::new();
    for name in ["image-grey-alpha.png", "image-grey-alpha-interlaced.png"] {
        let document = Document::create(Some(&policy), &CreateOptions::default())?;
        let mut sheet = InsertBlankPageOptions::default();
        sheet.width = Some(100.0_f64);
        sheet.height = Some(100.0_f64);
        let blank = document.insert_blank_page(&sheet)?;
        let mut source = BytesSourceOptions::default();
        source.bytes = Arc::new(fs::read(format!(
            "{}/../tests/corpus/render/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))?);
        blank.draw_image(
            &Source::from_bytes(&source)?,
            None,
            None,
            &DrawImageOptions::default(),
        )?;
        outcomes.push(
            rendered(&blank, &RenderOptions::default())
                .map(|_bytes| ())
                .map_err(|error| error.code()),
        );
    }
    if outcomes != [Ok(()), Err(ErrorCode::LimitExceeded)] {
        return Err(format!("outcomes {outcomes:?}").into());
    }
    Ok(())
}

/// Repair on demand pays for each candidate it examines: a page whose resources' entry in the cross-reference table is
/// wrong, found again by scanning 24 MB of `/` bytes, fails `LimitExceeded` under a decoded limit of 100,000
/// (ISO 32000-2:2020, 7.5.4).
#[cfg(feature = "rendering")]
#[test]
fn render_pays_for_repair() -> Outcome {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject 5 0 R >> /Contents 4 0 R >>",
        "<< /Length 5 >>\nstream\n/A Do\nendstream",
        "<< /A 6 0 R >>",
    ];
    let mut file = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (number, object) in (1_usize..).zip(objects) {
        offsets.push(file.len());
        file.extend_from_slice(format!("{number} 0 obj\n{object}\nendobj\n").as_bytes());
    }
    file.push(b'%');
    file.resize(file.len().saturating_add(24_000_000), b'/');
    file.push(b'\n');
    let table = file.len();
    file.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for (index, offset) in offsets.iter().enumerate() {
        // Object 5's entry is 3 bytes past it.
        let given = if index == 4 {
            offset.saturating_add(3)
        } else {
            *offset
        };
        file.extend_from_slice(format!("{given:010} 00000 n \n").as_bytes());
    }
    file.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{table}\n%%EOF\n").as_bytes(),
    );
    let mut source = BytesSourceOptions::default();
    source.bytes = Arc::new(file);
    let mut limits = ResourcePolicyOptions::default();
    limits.decoded_limit = Some(100_000);
    let document = Document::open(
        &Source::from_bytes(&source)?,
        Some(&ResourcePolicy::new(&limits)?),
        &OpenOptions::default(),
    )?;
    let shown = document.page(&PageOptions::default())?;
    match rendered(&shown, &RenderOptions::default()) {
        Err(error) if error.code() == ErrorCode::LimitExceeded => Ok(()),
        other => Err(format!(
            "render gave {:?}",
            other.map(|bytes| bytes.len()).map_err(|error| error.code())
        )
        .into()),
    }
}

/// A valid file of 200 cross-reference stream updates, each dictionary past 600 bytes, after a first revision whose 100
/// forms the page draws (ISO 32000-2:2020, 7.5.6, 7.5.8).
#[cfg(feature = "rendering")]
fn updated_file() -> Vec<u8> {
    let names = (0..100_usize)
        .map(|index| format!("/A{index} {} 0 R ", index.saturating_add(10)))
        .collect::<Vec<_>>()
        .concat();
    let content = (0..100_usize)
        .map(|index| format!("/A{index} Do "))
        .collect::<Vec<_>>()
        .concat();
    let mut objects = vec![
        (1, "<< /Type /Catalog /Pages 2 0 R >>".to_owned()),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned()),
        (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject 5 0 R >> /Contents 4 0 R >>".to_owned()),
        (4, format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len())),
        (5, format!("<< {names}>>")),
    ];
    objects.extend((10..110_usize).map(|number| {
        (
            number,
            "<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Length 0 >>\nstream\n\nendstream"
                .to_owned(),
        )
    }));
    let mut file = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for entry in &objects {
        let (number, object) = (entry.0, &entry.1);
        offsets.push(file.len());
        file.extend_from_slice(format!("{number} 0 obj\n{object}\nendobj\n").as_bytes());
    }
    let mut previous = file.len();
    file.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets.iter().take(5) {
        file.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    file.extend_from_slice(b"10 100\n");
    for offset in offsets.iter().skip(5) {
        file.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    file.extend_from_slice(b"trailer\n<< /Size 110 /Root 1 0 R >>\n");
    let pad = "x".repeat(600);
    for number in 110..310_usize {
        let at = file.len();
        // A type 1 entry: the update's own offset, in 4 bytes, then generation 0 in 2 (7.5.8.3).
        let entry = [24_u32, 16, 8, 0]
            .map(|shift| u8::try_from(at.wrapping_shr(shift) & 0xFF).unwrap_or_default());
        file.extend_from_slice(
            format!(
                "{number} 0 obj\n<< /Type /XRef /Size {} /Index [{number} 1] /W [1 4 2] /Root 1 0 R /Prev {previous} \
                 /Pad ({pad}) /Length 7 >>\nstream\n",
                number.saturating_add(1)
            )
            .as_bytes(),
        );
        file.push(1);
        file.extend_from_slice(&entry);
        file.extend_from_slice(&[0, 0]);
        file.extend_from_slice(b"\nendstream\nendobj\n");
        previous = at;
    }
    file.extend_from_slice(format!("startxref\n{previous}\n%%EOF\n").as_bytes());
    file
}

/// Under strict, a render of a valid file of many cross-reference stream updates that its callback stops at any call
/// fails `Cancelled`, never `Malformed` (ISO 32000-2:2020, 7.5.8).
#[cfg(feature = "rendering")]
#[test]
fn render_cancelled_within_updates_is_cancelled() -> Outcome {
    let mut source = BytesSourceOptions::default();
    source.bytes = Arc::new(updated_file());
    let mut open = OpenOptions::default();
    open.strict = true;
    let render = |allowed: u64| -> Result<u64, Box<dyn Error>> {
        let document = Document::open(&Source::from_bytes(&source)?, None, &open)?;
        let shown = document.page(&PageOptions::default())?;
        let sink = Sink::in_memory(&MemorySinkOptions::default())?;
        let stop = After {
            allowed,
            calls: AtomicU64::new(0),
        };
        match shown.render(&sink, Some(&stop), &RenderOptions::default()) {
            Ok(()) => Ok(stop.calls.load(Ordering::Relaxed)),
            Err(error) if error.code() == ErrorCode::Cancelled => Ok(0),
            Err(error) => Err(format!("stopped after {allowed} calls: {:?}", error.code()).into()),
        }
    };
    let calls = render(u64::MAX)?;
    for allowed in 0..calls {
        if render(allowed)? != 0 {
            return Err(format!("not cancelled after {allowed} calls").into());
        }
    }
    Ok(())
}
