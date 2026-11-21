//! Pages of created, changed and split documents, read before saving, and the source bytes
//! inserting and splitting read.

extern crate alloc;

use alloc::sync::Arc;
use core::error::Error;
use core::ops::Range;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::path::PathBuf;
use std::{fs, io};

use starlightpdf::arrange::{InsertBlankPageOptions, SplitBy, SplitOptions};
use starlightpdf::document::{CreateOptions, Document, OpenOptions, PageOptions, SaveOptions};
use starlightpdf::drawing::DrawImageOptions;
use starlightpdf::input::{
    BytesSourceOptions, FileSourceOptions, ReadAt, ReadAtSourceOptions, Source,
};
use starlightpdf::objects::{ObjectId, ObjectOptions, StreamDataOptions};
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf::page::{Boundary, BoundaryOptions, Page};
use starlightpdf::{ErrorCode, PdfError};

/// The corpus directory.
const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/corpus");

/// Pages of [`Counted::flat`].
const PAGES: usize = 4_096;

/// Links of [`Counted::named`], one per destination name.
const LINKS: usize = 1_024;

/// The crop boxes and rotations of split/pages.pdf's pages (ISO 32000-2:2020, 7.7.3.4: the
/// root gives `MediaBox` 0 0 612 792 and `Rotate` 90, the node over pages 1 and 2 `CropBox`
/// 10 10 600 780 and `Rotate` 180).
const SOURCE_PAGES: [&str; 5] = [
    "0 0 612 792 90",
    "10 10 590 770 180",
    "20 20 280 380 270",
    "0 0 420 595 0",
    "50 50 450 650 90",
];

/// The name link `at` of [`Counted::named`] uses, by its index.
type Order = fn(usize) -> usize;

/// The entries by which a link of [`Counted::named`] names its destination, given the name.
type Naming = fn(&str) -> String;

/// A PDF that counts the bytes it serves, in all and from objects it watches.
struct Counted {
    /// The file.
    bytes: Vec<u8>,
    /// Bytes served.
    served: AtomicUsize,
    /// The watched objects' bytes.
    watched: Range<usize>,
    /// Bytes served from the watched objects.
    watched_served: AtomicUsize,
}

impl Counted {
    /// The file of [`PAGES`] pages, all kids of the root page tree node: catalog 1, root 2,
    /// pages from 3; the root watched (ISO 32000-2:2020, 7.5, 7.7.3).
    fn flat() -> Self {
        let kids: Vec<String> = (3..PAGES + 3).map(|kid| format!("{kid} 0 R")).collect();
        let mut objects = vec![
            String::from("<< /Type /Catalog /Pages 2 0 R >>"),
            format!(
                "<< /Type /Pages /MediaBox [0 0 612 792] /Count {PAGES} /Kids [{}] >>",
                kids.join(" ")
            ),
        ];
        objects.resize(PAGES + 2, String::from("<< /Type /Page /Parent 2 0 R >>"));
        Self::new(&objects, 2..3)
    }

    /// A page with [`LINKS`] links, link `at` to it by name `order(at)` of the destinations name
    /// tree, given by `naming`, whose root holds leaves of `leaf` names each: catalog 1, root page
    /// tree node 2, page 3, the name tree from 4, watched (ISO 32000-2:2020, 7.9.6, 12.3.2.4,
    /// 12.5.6.5).
    fn named(leaf: usize, order: Order, naming: Naming) -> Self {
        let name = |at: usize| format!("(d{at:05})");
        let links: Vec<String> = (0..LINKS)
            .map(|at| {
                format!(
                    "<< /Type /Annot /Subtype /Link /Rect [0 0 9 9] {} >>",
                    naming(&name(order(at)))
                )
            })
            .collect();
        let leaves = LINKS.div_ceil(leaf);
        let kids: Vec<String> = (5..leaves.saturating_add(5))
            .map(|kid| format!("{kid} 0 R"))
            .collect();
        let mut objects = vec![
            String::from("<< /Type /Catalog /Pages 2 0 R /Names << /Dests 4 0 R >> >>"),
            String::from("<< /Type /Pages /MediaBox [0 0 612 792] /Count 1 /Kids [3 0 R] >>"),
            format!(
                "<< /Type /Page /Parent 2 0 R /Annots [{}] >>",
                links.join(" ")
            ),
            format!("<< /Kids [{}] >>", kids.join(" ")),
        ];
        for index in 0..leaves {
            let first = index.saturating_mul(leaf);
            let last = first.saturating_add(leaf).min(LINKS).saturating_sub(1);
            let names: Vec<String> = (first..=last)
                .map(|at| format!("{} [3 0 R /Fit]", name(at)))
                .collect();
            objects.push(format!(
                "<< /Limits [{} {}] /Names [{}] >>",
                name(first),
                name(last),
                names.join(" ")
            ));
        }
        let end = objects.len().saturating_add(1);
        Self::new(&objects, 4..end)
    }

    /// A file of numbered `objects`, from 1, watching objects `watched` (ISO 32000-2:2020, 7.5).
    fn new(objects: &[String], watched: Range<usize>) -> Self {
        let mut bytes = b"%PDF-2.0\n".to_vec();
        let mut offsets = Vec::new();
        let mut range = 0..0;
        for (number, text) in (1..).zip(objects) {
            if number == watched.start {
                range.start = bytes.len();
            }
            offsets.push(bytes.len());
            bytes.extend(format!("{number} 0 obj\n{text}\nendobj\n").into_bytes());
            if number.saturating_add(1) == watched.end {
                range.end = bytes.len();
            }
        }
        let table = bytes.len();
        let size = objects.len().saturating_add(1);
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
            served: AtomicUsize::new(0),
            watched: range,
            watched_served: AtomicUsize::new(0),
        }
    }
}

impl ReadAt for Counted {
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let rest = self.bytes.get(start..).unwrap_or_default();
        let count = rest.len().min(buffer.len());
        for (to, from) in buffer.iter_mut().zip(rest) {
            *to = *from;
        }
        let end = start.saturating_add(count);
        let overlap = end
            .min(self.watched.end)
            .saturating_sub(start.max(self.watched.start));
        let _watched = self.watched_served.fetch_add(overlap, Ordering::Relaxed);
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

/// Fails unless `found` is `expected`.
fn same(found: &[String], expected: &[&str]) -> Result<(), Box<dyn Error>> {
    if found == expected {
        Ok(())
    } else {
        Err(format!("{found:?}, not {expected:?}").into())
    }
}

/// Opens `counted`.
fn open_counted(counted: &Arc<Counted>) -> Result<Document, PdfError> {
    let read_at: Arc<dyn ReadAt> = Arc::<Counted>::clone(counted);
    let source = Source::from_read_at(&read_at, &ReadAtSourceOptions::default())?;
    Document::open(&source, None, &OpenOptions::default())
}

/// Object `id` of `document` as PDF syntax.
fn text(document: &Document, id: ObjectId) -> Result<String, PdfError> {
    let mut options = ObjectOptions::default();
    options.id = id;
    document.object(&options)
}

/// The data of stream `id` of `document`, decoded when `decode`.
fn data(document: &Document, id: ObjectId, decode: bool) -> Result<Vec<u8>, PdfError> {
    let sink = Sink::in_memory(&MemorySinkOptions::default())?;
    let mut options = StreamDataOptions::default();
    options.id = id;
    options.decode = decode;
    document.stream_data(&sink, &options)?;
    sink.take_bytes()
}

/// Object `number`, generation 0.
fn id(number: u32) -> ObjectId {
    let mut found = ObjectId::default();
    found.number = number;
    found
}

/// The object the reference after `key` in the PDF syntax `syntax` refers to, the first of an
/// array.
fn referred(syntax: &str, key: &str) -> Result<ObjectId, Box<dyn Error>> {
    let rest = syntax
        .split(&format!("{key} "))
        .nth(1)
        .ok_or_else(|| format!("no {key} in {syntax}"))?;
    let mut words = rest.trim_start_matches('[').split(' ');
    let mut found = id(words.next().unwrap_or_default().parse()?);
    found.generation = words.next().unwrap_or_default().parse()?;
    Ok(found)
}

/// The first page of `document`, found from its catalog by `object`.
fn first_page(document: &Document) -> Result<String, Box<dyn Error>> {
    let catalog = text(document, document.catalog()?)?;
    let pages = text(document, referred(&catalog, "/Pages")?)?;
    Ok(text(document, referred(&pages, "/Kids")?)?)
}

/// `document` saved and opened again.
fn saved(document: &Document) -> Result<Document, PdfError> {
    let sink = Sink::in_memory(&MemorySinkOptions::default())?;
    let _report = document.save(&sink, None, &SaveOptions::default())?;
    let mut options = BytesSourceOptions::default();
    options.bytes = Arc::new(sink.take_bytes()?);
    Document::open(
        &Source::from_bytes(&options)?,
        None,
        &OpenOptions::default(),
    )
}

/// A split by `count` pages.
fn by_pages(count: u32) -> SplitOptions {
    let mut options = SplitOptions::default();
    options.by = SplitBy::Pages;
    options.count = count;
    options
}

/// A part's pages read before saving are its own (ISO 32000-2:2020, 7.7.3).
#[test]
fn a_parts_pages_are_its_own_before_saving() -> Result<(), Box<dyn Error>> {
    let parts = corpus("split/pages.pdf")?.split(&by_pages(2))?;
    let _first = parts.next()?;
    let second = parts.next()?.ok_or("no part")?;
    same(&every(&second)?, SOURCE_PAGES.get(2..4).unwrap_or_default())?;
    Ok(())
}

/// A created document's pages, and the page `insert_blank_page` gives, read before saving
/// (ISO 32000-2:2020, 7.7.3.3).
#[test]
fn created_pages_read_before_saving() -> Result<(), Box<dyn Error>> {
    let document = Document::create(None, &CreateOptions::default())?;
    let inserted = document.insert_blank_page(&blank(200.0, 100.0, None))?;
    same(&[describe(&inserted, Boundary::Media)?], &["0 0 200 100 0"])?;
    drop(document.insert_blank_page(&blank(300.0, 400.0, Some(0)))?);
    same(&every(&document)?, &["0 0 300 400 0", "0 0 200 100 0"])?;
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
    same(&every(&document)?, &expected)?;
    same(&every(&saved(&document)?)?, &expected)?;
    Ok(())
}

/// `object` and `stream_data` read a part as split and a document as edited, before saving: the
/// part's page with its inherited attributes its own, another part's page null; after an edit
/// the edit, and the part fails `StaleHandle` (ISO 32000-2:2020, 7.3.10, 7.7.3.4; handles ADR).
#[test]
fn objects_read_parts_and_edits() -> Result<(), Box<dyn Error>> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /MediaBox [0 0 612 792] /Count 2 /Kids [3 0 R 4 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>",
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>",
        "<< /Length 3 >>\nstream\nabc\nendstream",
        "<< /Length 3 >>\nstream\ndef\nendstream",
    ]
    .map(String::from);
    let document = open_counted(&Arc::new(Counted::new(&objects, 0..0)))?;
    let part = document.split(&by_pages(1))?.next()?.ok_or("no part")?;
    if !text(&part, id(3))?.contains("/MediaBox [0 0 612 792]") {
        return Err("a part's page does not hold its inherited media box".into());
    }
    if text(&part, id(4))? != "null" || data(&part, id(5), false)? != b"abc" {
        return Err("a part's objects read wrong".into());
    }
    drop(document.insert_blank_page(&InsertBlankPageOptions::default())?);
    if !text(&document, id(2))?.contains("/Count 3") {
        return Err("an edited object read as before the edit".into());
    }
    let codes = [
        text(&part, id(3)).err().map(|error| error.code()),
        data(&part, id(5), false).err().map(|error| error.code()),
    ];
    if codes != [Some(ErrorCode::StaleHandle); 2] {
        return Err(format!("{codes:?}, not StaleHandle").into());
    }
    Ok(())
}

/// Images drawn on a page read before saving: a JPEG's bytes unchanged, a PNG's samples as after
/// saving, as many as its header gives, with the content painting them (ISO 32000-2:2020, 7.4.8,
/// 8.9.5; ISO/IEC 15948:2004, 11.2.2).
#[test]
fn drawn_images_read_before_saving() -> Result<(), Box<dyn Error>> {
    let document = Document::create(None, &CreateOptions::default())?;
    let page = document.insert_blank_page(&InsertBlankPageOptions::default())?;
    let jpeg = fs::read(format!("{CORPUS}/images/baseline-rgb.jpg"))?;
    let png = fs::read(format!("{CORPUS}/images/c2-b8.png"))?;
    for image in [&jpeg, &png] {
        let mut options = BytesSourceOptions::default();
        options.bytes = Arc::new(image.clone());
        page.draw_image(
            &Source::from_bytes(&options)?,
            None,
            None,
            &DrawImageOptions::default(),
        )?;
    }
    let drawn = first_page(&document)?;
    let content = data(&document, referred(&drawn, "/Contents")?, false)?;
    let painted = referred(&drawn, "/Im1")?;
    let length = format!("/Length {}", jpeg.len());
    if !content.ends_with(b"/Im1 Do Q\n")
        || data(&document, painted, false)? != jpeg
        || !text(&document, painted)?.contains(&length)
    {
        return Err("a drawn JPEG read wrong".into());
    }
    let samples = data(&document, referred(&drawn, "/Im2")?, true)?;
    // IHDR's width and height, big-endian, at bytes 16 and 20 (ISO/IEC 15948:2004, 11.2.2).
    let header = |at: usize| {
        png.get(at..at.saturating_add(4))
            .unwrap_or_default()
            .iter()
            .fold(0_u32, |value, &byte| {
                value.wrapping_shl(8) | u32::from(byte)
            })
    };
    let size = usize::try_from(header(16).saturating_mul(header(20)).saturating_mul(3))?;
    let reopened = saved(&document)?;
    let after = data(&reopened, referred(&first_page(&reopened)?, "/Im2")?, true)?;
    if samples.len() != size || samples != after {
        return Err("a drawn PNG's samples read wrong".into());
    }
    Ok(())
}

/// A part refused for a link that would be lost is refused again, not skipped (ISO 32000-2:2020,
/// 12.3.2.2).
#[test]
fn a_refused_part_is_not_skipped() -> Result<(), Box<dyn Error>> {
    let parts = corpus("split/outside-link.pdf")?.split(&by_pages(1))?;
    for _ in 0_u8..2 {
        let code = parts.next().err().map(|error| error.code());
        if code != Some(ErrorCode::LossRefused) {
            return Err(format!("{code:?}, not LossRefused").into());
        }
    }
    Ok(())
}

/// A link's `GoTo` action to another part behind a reference to a reference, its `S` too, is
/// refused as when direct; the part without bookmarks keeps it (ISO 32000-2:2020, 7.3.10,
/// 12.6.4.2).
#[test]
fn a_link_behind_references_is_refused() -> Result<(), Box<dyn Error>> {
    let mut whole = by_pages(2);
    whole.allow_loss = true;
    let part = corpus("split/references-to-references.pdf")?
        .split(&whole)?
        .next()?
        .ok_or("no part")?;
    let code = part
        .split(&by_pages(1))
        .and_then(|parts| parts.next())
        .err()
        .map(|error| error.code());
    if code != Some(ErrorCode::LossRefused) {
        return Err(format!("{code:?}, not LossRefused").into());
    }
    Ok(())
}

/// An edit succeeds while a split's cursor and a part of it live, which then fail with
/// `StaleHandle`; a split after the edit reads the document as edited (handles ADR).
#[test]
fn an_edit_makes_a_split_stale() -> Result<(), Box<dyn Error>> {
    let document = corpus("split/pages.pdf")?;
    let parts = document.split(&by_pages(2))?;
    let part = parts.next()?.ok_or("no part")?;
    drop(document.insert_blank_page(&InsertBlankPageOptions::default())?);
    let stale = Some(ErrorCode::StaleHandle);
    if part.page_count().err().map(|error| error.code()) != stale {
        return Err("a part made before an edit was used".into());
    }
    if parts.next().err().map(|error| error.code()) != stale {
        return Err("a split made before an edit went on".into());
    }
    let after = document.split(&by_pages(7))?.next()?.ok_or("no part")?;
    if after.page_count()? != 6 {
        return Err("a split after an edit missed it".into());
    }
    Ok(())
}

/// A page tree kid that is a free object, so null, is malformed after an edit as before it
/// (ISO 32000-2:2020, 7.3.10, 7.7.3.2).
#[test]
fn a_free_kid_is_malformed_after_an_edit() -> Result<(), Box<dyn Error>> {
    let document = corpus("pages/kid-missing.pdf")?;
    drop(document.insert_blank_page(&InsertBlankPageOptions::default())?);
    let mut options = PageOptions::default();
    options.index = 1;
    let code = document.page(&options).err().map(|error| error.code());
    if code != Some(ErrorCode::Malformed) {
        return Err(format!("{code:?}, not Malformed").into());
    }
    Ok(())
}

/// Checking a split for what its parts would lose reads the root page tree node no more often
/// than splitting with `allow_loss` does (ISO 32000-2:2020, 7.7.2, 7.7.3.2).
#[test]
fn refusing_loss_reads_no_page_tree_node() -> Result<(), Box<dyn Error>> {
    let mut read = [0; 2];
    let mut root = 0;
    for (allow_loss, count) in [false, true].into_iter().zip(&mut read) {
        let flat = Arc::new(Counted::flat());
        let document = open_counted(&flat)?;
        let before = flat.watched_served.load(Ordering::Relaxed);
        let mut options = by_pages(1);
        options.allow_loss = allow_loss;
        drop(document.split(&options)?);
        *count = flat
            .watched_served
            .load(Ordering::Relaxed)
            .saturating_sub(before);
        root = flat.watched.len();
    }
    let [refusing, allowing] = read;
    if refusing.saturating_sub(allowing) >= root {
        return Err(format!("{refusing} bytes of a {root}-byte root read, not {allowing}").into());
    }
    Ok(())
}

/// Splitting off a page whose links name their destinations and saving it reads the name tree
/// a bounded number of times, not once per link: links by `Dest` in key order over leaves of 32
/// names, and links by `Dest`, by a `GoTo` action of `AA` and by one after another action,
/// alternating between two leaves (ISO 32000-2:2020, 7.9.6, 12.3.2.4, 12.6.2, 12.6.3, 12.6.4.2).
#[test]
fn named_links_read_the_name_tree_a_few_times() -> Result<(), Box<dyn Error>> {
    let alternating: Order = |at| (at & 1).saturating_mul(LINKS >> 1).saturating_add(at >> 1);
    let cases: [(usize, Order, Naming); 4] = [
        (32, |at| at, |name| format!("/Dest {name}")),
        (LINKS >> 1, alternating, |name| format!("/Dest {name}")),
        (LINKS >> 1, alternating, |name| {
            format!("/AA << /U << /S /GoTo /D {name} >> >>")
        }),
        (LINKS >> 1, alternating, |name| {
            format!("/A << /S /URI /URI (u) /Next [<< /S /GoTo /D {name} >>] >>")
        }),
    ];
    for (at, (leaf, order, naming)) in cases.into_iter().enumerate() {
        let named = Arc::new(Counted::named(leaf, order, naming));
        let parts = open_counted(&named)?.split(&by_pages(1))?;
        let part = parts.next()?.ok_or("no part")?;
        let sink = Sink::in_memory(&MemorySinkOptions::default())?;
        let _report = part.save(&sink, None, &SaveOptions::default())?;
        let read = named.watched_served.load(Ordering::Relaxed);
        let tree = named.watched.len();
        if read > 64 * tree {
            return Err(format!("case {at}: {read} bytes of a {tree}-byte name tree read").into());
        }
    }
    Ok(())
}

/// Appending pages to a flat page tree reads no page already there.
#[test]
fn appending_pages_reads_no_other_page() -> Result<(), Box<dyn Error>> {
    let flat = Arc::new(Counted::flat());
    let document = open_counted(&flat)?;
    let before = flat.served.load(Ordering::Relaxed);
    for _ in 0_u8..64 {
        drop(document.insert_blank_page(&InsertBlankPageOptions::default())?);
    }
    let read = flat.served.load(Ordering::Relaxed).saturating_sub(before);
    if read >= flat.bytes.len() || document.page_count()? != 4_160 {
        return Err(format!("{read} bytes read").into());
    }
    Ok(())
}

/// Splitting a flat page tree into one part or a part per page and saving every part reads the
/// root node's kids a bounded number of times, not once per new node or part
/// (ISO 32000-2:2020, 7.7.3.2).
#[test]
fn splitting_a_flat_tree_reads_its_kids_a_few_times() -> Result<(), Box<dyn Error>> {
    for count in [4_096, 1] {
        let flat = Arc::new(Counted::flat());
        let parts = open_counted(&flat)?.split(&by_pages(count))?;
        while let Some(part) = parts.next()? {
            let sink = Sink::in_memory(&MemorySinkOptions::default())?;
            let _report = part.save(&sink, None, &SaveOptions::default())?;
        }
        let read = flat.watched_served.load(Ordering::Relaxed);
        let root = flat.watched.len();
        if read > 8 * root {
            return Err(format!("by {count}: {read} bytes of a {root}-byte root read").into());
        }
    }
    Ok(())
}
