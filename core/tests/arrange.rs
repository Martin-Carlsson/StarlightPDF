//! Pages of created, changed and split documents, read before saving, and the source bytes
//! inserting and splitting read.

extern crate alloc;

use alloc::sync::Arc;
use core::error::Error;
use core::ops::Range;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::io;
use std::path::PathBuf;

use starlightpdf::arrange::{InsertBlankPageOptions, SplitBy, SplitOptions};
use starlightpdf::document::{CreateOptions, Document, OpenOptions, PageOptions, SaveOptions};
use starlightpdf::input::{FileSourceOptions, ReadAt, ReadAtSourceOptions, Source};
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf::page::{Boundary, BoundaryOptions, Page};
use starlightpdf::{ErrorCode, PdfError};

/// The corpus directory.
const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/corpus");

/// Pages of [`Flat`].
const PAGES: usize = 4_096;

/// A PDF whose root page tree node holds every page; counts the bytes it serves, in all and
/// from the root node.
struct Flat {
    /// The file.
    bytes: Vec<u8>,
    /// The root node's bytes.
    root: Range<usize>,
    /// Bytes served from the root node.
    root_served: AtomicUsize,
    /// Bytes served.
    served: AtomicUsize,
}

impl Flat {
    /// The file of [`PAGES`] pages: catalog 1, root 2, pages from 3 (ISO 32000-2:2020, 7.5,
    /// 7.7.3).
    fn new() -> Self {
        let mut bytes = b"%PDF-2.0\n".to_vec();
        let mut offsets = Vec::new();
        let mut object = |bytes: &mut Vec<u8>, number: usize, text: &str| {
            offsets.push(bytes.len());
            bytes.extend(format!("{number} 0 obj\n{text}\nendobj\n").into_bytes());
        };
        object(&mut bytes, 1, "<< /Type /Catalog /Pages 2 0 R >>");
        let kids: Vec<String> = (3..PAGES + 3).map(|kid| format!("{kid} 0 R")).collect();
        let start = bytes.len();
        object(
            &mut bytes,
            2,
            &format!(
                "<< /Type /Pages /MediaBox [0 0 612 792] /Count {PAGES} /Kids [{}] >>",
                kids.join(" ")
            ),
        );
        let root = start..bytes.len();
        for number in 3..PAGES + 3 {
            object(&mut bytes, number, "<< /Type /Page /Parent 2 0 R >>");
        }
        let table = bytes.len();
        let size = PAGES + 3;
        bytes.extend(format!("xref\n0 {size}\n0000000000 65535 f\r\n").into_bytes());
        for offset in offsets {
            bytes.extend(format!("{offset:010} 00000 n\r\n").into_bytes());
        }
        bytes.extend(
            format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{table}\n%%EOF\n")
                .into_bytes(),
        );
        Self {
            bytes,
            root,
            root_served: AtomicUsize::new(0),
            served: AtomicUsize::new(0),
        }
    }
}

impl ReadAt for Flat {
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let rest = self.bytes.get(start..).unwrap_or_default();
        let count = rest.len().min(buffer.len());
        for (to, from) in buffer.iter_mut().zip(rest) {
            *to = *from;
        }
        let end = start.saturating_add(count);
        let overlap = end
            .min(self.root.end)
            .saturating_sub(start.max(self.root.start));
        let _root = self.root_served.fetch_add(overlap, Ordering::Relaxed);
        let _all = self.served.fetch_add(count, Ordering::Relaxed);
        Ok(count)
    }

    fn size(&self) -> io::Result<u64> {
        u64::try_from(self.bytes.len()).map_err(|_error| io::ErrorKind::InvalidData.into())
    }
}

/// `page`'s boundary `kind` as `x y width height`, and its rotation.
fn describe(page: &Page, kind: Boundary) -> Result<String, PdfError> {
    let mut options = BoundaryOptions::default();
    options.kind = kind;
    let rect = page.boundary(&options)?;
    Ok(format!(
        "{} {} {} {} {}",
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        page.rotation()?
    ))
}

/// Each page of `document`, as [`describe`] gives its crop box.
fn every(document: &Document) -> Result<Vec<String>, PdfError> {
    (0..document.page_count()?)
        .map(|index| {
            let mut options = PageOptions::default();
            options.index = index;
            describe(&document.page(&options)?, Boundary::Crop)
        })
        .collect()
}

/// A blank page of `width` × `height` points at `index`, or the end.
fn blank(width: f64, height: f64, index: Option<u32>) -> InsertBlankPageOptions {
    let mut options = InsertBlankPageOptions::default();
    options.width = Some(width);
    options.height = Some(height);
    options.index = index;
    options
}

/// Opens the corpus file `relative`.
fn corpus(relative: &str) -> Result<Document, PdfError> {
    let mut options = FileSourceOptions::default();
    options.file = PathBuf::from(format!("{CORPUS}/{relative}"));
    Document::open(&Source::from_file(&options)?, None, &OpenOptions::default())
}

/// Opens `flat`.
fn open_flat(flat: &Arc<Flat>) -> Result<Document, PdfError> {
    let read_at: Arc<dyn ReadAt> = Arc::<Flat>::clone(flat);
    let source = Source::from_read_at(&read_at, &ReadAtSourceOptions::default())?;
    Document::open(&source, None, &OpenOptions::default())
}

/// A split by `count` pages.
fn by_pages(count: u32) -> SplitOptions {
    let mut options = SplitOptions::default();
    options.by = SplitBy::Pages;
    options.count = count;
    options
}

/// The crop boxes and rotations of split/pages.pdf's pages (ISO 32000-2:2020, 7.7.3.4: the
/// root gives MediaBox 0 0 612 792 and Rotate 90, the node over pages 1 and 2 CropBox
/// 10 10 600 780 and Rotate 180).
const SOURCE_PAGES: [&str; 5] = [
    "0 0 612 792 90",
    "10 10 590 770 180",
    "20 20 280 380 270",
    "0 0 420 595 0",
    "50 50 450 650 90",
];

/// A part's pages read before saving are its own (ISO 32000-2:2020, 7.7.3).
#[test]
fn a_parts_pages_are_its_own_before_saving() -> Result<(), Box<dyn Error>> {
    let parts = corpus("split/pages.pdf")?.split(&by_pages(2))?;
    let _first = parts.next()?;
    let second = parts.next()?.ok_or("no part")?;
    assert_eq!(every(&second)?, SOURCE_PAGES.get(2..4).unwrap_or_default());
    Ok(())
}

/// A created document's pages, and the page `insert_blank_page` gives, read before saving
/// (ISO 32000-2:2020, 7.7.3.3).
#[test]
fn created_pages_read_before_saving() -> Result<(), Box<dyn Error>> {
    let document = Document::create(None, &CreateOptions::default())?;
    let inserted = document.insert_blank_page(&blank(200.0, 100.0, None))?;
    assert_eq!(describe(&inserted, Boundary::Media)?, "0 0 200 100 0");
    drop(document.insert_blank_page(&blank(300.0, 400.0, Some(0)))?);
    assert_eq!(every(&document)?, ["0 0 300 400 0", "0 0 200 100 0"]);
    Ok(())
}

/// A blank page inserted into an opened file is at its index, the others after it, and has its
/// own size and no rotation, whatever its new ancestors set (ISO 32000-2:2020, 7.7.3.4).
#[test]
fn inserted_pages_keep_their_index_size_and_rotation() -> Result<(), Box<dyn Error>> {
    let document = corpus("split/pages.pdf")?;
    drop(document.insert_blank_page(&blank(200.0, 100.0, Some(2)))?);
    drop(document.insert_blank_page(&blank(300.0, 400.0, Some(0)))?);
    let mut expected = SOURCE_PAGES.to_vec();
    expected.insert(2, "0 0 200 100 0");
    expected.insert(0, "0 0 300 400 0");
    assert_eq!(every(&document)?, expected);
    let sink = Sink::in_memory(&MemorySinkOptions::default())?;
    let _report = document.save(&sink, None, &SaveOptions::default())?;
    let mut options = starlightpdf::input::BytesSourceOptions::default();
    options.bytes = Arc::new(sink.take_bytes()?);
    let saved = Document::open(&Source::from_bytes(&options)?, None, &OpenOptions::default())?;
    assert_eq!(every(&saved)?, expected);
    Ok(())
}

/// A part refused for a link that would be lost is refused again, not skipped (ISO 32000-2:2020,
/// 12.3.2.2).
#[test]
fn a_refused_part_is_not_skipped() -> Result<(), Box<dyn Error>> {
    let parts = corpus("split/outside-link.pdf")?.split(&by_pages(1))?;
    for _ in 0..2 {
        assert_eq!(
            parts.next().map(|part| part.is_some()).map_err(|error| error.code()),
            Err(ErrorCode::LossRefused)
        );
    }
    Ok(())
}

/// Appending pages to a flat page tree reads no page already there.
#[test]
fn appending_pages_reads_no_other_page() -> Result<(), Box<dyn Error>> {
    let flat = Arc::new(Flat::new());
    let document = open_flat(&flat)?;
    let before = flat.served.load(Ordering::Relaxed);
    for _ in 0..64 {
        drop(document.insert_blank_page(&InsertBlankPageOptions::default())?);
    }
    let read = flat.served.load(Ordering::Relaxed).saturating_sub(before);
    assert!(read < flat.bytes.len(), "{read} bytes read");
    assert_eq!(document.page_count()?, 4_160);
    Ok(())
}

/// Splitting a flat page tree and saving the part reads the root node's kids a bounded number of
/// times, not once per new node (ISO 32000-2:2020, 7.7.3.2).
#[test]
fn splitting_a_flat_tree_reads_its_kids_a_few_times() -> Result<(), Box<dyn Error>> {
    let flat = Arc::new(Flat::new());
    let parts = open_flat(&flat)?.split(&by_pages(4_096))?;
    let part = parts.next()?.ok_or("no part")?;
    let sink = Sink::in_memory(&MemorySinkOptions::default())?;
    let _report = part.save(&sink, None, &SaveOptions::default())?;
    let read = flat.root_served.load(Ordering::Relaxed);
    let root = flat.root.len();
    assert!(read <= 8 * root, "{read} bytes of a {root}-byte root read");
    Ok(())
}
