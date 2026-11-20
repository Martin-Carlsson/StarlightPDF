//! Peak allocation of a page's boundary and of a stream decoded to a sink, on the two virtual
//! 1 GiB files of `.architecture-decisions/memory-is-bounded-by-work-not-file-size.md`.

extern crate alloc;

use alloc::sync::Arc;
use core::fmt::Write as _;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::io::{self, Cursor, Write as _};

use starlightpdf::PdfError;
use starlightpdf::document::{Document, OpenOptions, PageOptions};
use starlightpdf::input::{ReadAt, ReadAtSourceOptions, Source};
use starlightpdf::objects::{ObjectId, StreamDataOptions};
use starlightpdf::output::{Append, AppendSinkOptions, Sink};
use starlightpdf::page::BoundaryOptions;
use starlightpdf_memory::{adler_run, adler_value, peak, stored_lengths};

/// Most bytes opening and reading a page's boundary may hold allocated at once.
const BOUNDARY_BOUND: usize = 2_048;

/// Most bytes opening and decoding a stream to a sink may hold allocated at once.
const STREAM_BOUND: usize = 49_152;

/// Bytes of each virtual file.
const SIZE: u64 = 1 << 30;

/// Most data bytes of a stored block (RFC 1951, 3.2.4).
const BLOCK: u64 = 0xFFFF;

/// Bytes of a full stored block: its header byte, `LEN`, `NLEN` and data (RFC 1951, 3.2.4).
const FULL_BLOCK: u64 = 0x0001_0004;

/// Bytes of a zlib stream besides its full blocks and final data: the zlib header, the final
/// block's header and the Adler-32 (RFC 1950, 2.2).
const ZLIB: u64 = 11;

/// Decoded bytes of each stream of the file of large streams but the last.
const LARGE: u64 = 1 << 28;

/// Kids of a page tree node.
const FANOUT: u64 = 64;

/// Pages of the file of small objects: about 136 bytes each, leaving at least 64 MiB.
const LEAVES: u64 = 7_400_000;

/// Bytes of a page object, padded with spaces.
const LEAF: u64 = 96;

/// Bytes of a page tree node object, padded with spaces.
const NODE: u64 = 1_024;

/// Bytes of a cross-reference entry (ISO 32000-2:2020, 7.5.4).
const ENTRY: u64 = 20;

/// Bytes a record renders into.
const LINE: usize = 1_024;

/// The page tree root's media box, `[0 0 612 792]`, as x, y, width and height.
const MEDIA_BOX: [f64; 4] = [0.0, 0.0, 612.0, 792.0];

/// A record's text from its index.
type Render = Box<dyn Fn(u64, &mut Cursor<&mut [u8]>) -> io::Result<()> + Send + Sync>;

/// A range of a virtual file.
trait Part: Send + Sync {
    /// Writes bytes from `at` into the start of `out`, returning how many: at least one.
    fn fill(&self, at: u64, out: &mut [u8]) -> Option<usize>;

    /// Bytes of the range.
    fn length(&self) -> Option<u64>;
}

/// Literal bytes.
struct Bytes(Vec<u8>);

/// A file, case (a) or (b), and what to read from it.
struct Case {
    /// Bytes the stream decodes to.
    decoded: u64,
    /// The file.
    file: Virtual,
    /// The page index to read.
    page: u32,
    /// The stream's object number.
    stream: u32,
}

/// A sink's appends, counted.
struct Counted {
    /// Bytes appended.
    bytes: AtomicU64,
    /// Whether every byte appended was zero.
    only_zeros: AtomicBool,
}

/// `FlateDecode` data of zeros in stored blocks (RFC 1950; RFC 1951, 3.2.4).
#[derive(Clone, Copy)]
struct Flate {
    /// Bytes of zeros.
    decoded: u64,
}

/// Fixed-width records.
struct Records {
    /// Records.
    count: u64,
    /// The text of each.
    render: Render,
    /// Bytes of each.
    width: u64,
}

/// Where the objects of the file of small objects start.
#[derive(Clone, Copy)]
struct Starts {
    /// The catalog.
    catalog: u64,
    /// The first page.
    leaves: u64,
    /// The first page tree node.
    nodes: u64,
    /// The stream.
    stream: u64,
}

/// A page tree of `leaves` pages, nodes of [`FANOUT`] kids (ISO 32000-2:2020, 7.7.3.2); pages
/// are objects 1 to `leaves`, then nodes bottom level first, the stream and the catalog.
struct Tree {
    /// Pages.
    leaves: u64,
    /// Nodes per level, bottom up; the last level is the root.
    levels: Vec<u64>,
}

/// A file generated on the fly from its parts.
#[derive(Default)]
struct Virtual {
    /// Each part and where it starts.
    parts: Vec<(u64, Box<dyn Part>)>,
    /// Bytes in all.
    size: u64,
}

impl Append for Counted {
    fn append(&self, bytes: &[u8]) -> io::Result<()> {
        let length = u64::try_from(bytes.len()).map_err(io::Error::other)?;
        let _previous = self.bytes.fetch_add(length, Ordering::Relaxed);
        if bytes.iter().any(|&byte| byte != 0) {
            self.only_zeros.store(false, Ordering::Relaxed);
        }
        Ok(())
    }
}

impl Flate {
    /// The Adler-32 of the zeros (RFC 1950, 8.2).
    fn adler(self) -> Option<u32> {
        adler_value(adler_run((1, 0), 0, self.decoded)?)
    }
}

impl Part for Bytes {
    fn fill(&self, at: u64, out: &mut [u8]) -> Option<usize> {
        let rest = self.0.get(usize::try_from(at).ok()?..)?;
        let count = rest.len().min(out.len());
        out.get_mut(..count)?.copy_from_slice(rest.get(..count)?);
        Some(count)
    }

    fn length(&self) -> Option<u64> {
        u64::try_from(self.0.len()).ok()
    }
}

impl Part for Flate {
    fn fill(&self, at: u64, out: &mut [u8]) -> Option<usize> {
        let blocks_end = self
            .decoded
            .checked_div(BLOCK)?
            .checked_mul(FULL_BLOCK)?
            .checked_add(2)?;
        if at < 2 {
            return one(out, *[0x78, 0x01].get(usize::try_from(at).ok()?)?);
        }
        if at < blocks_end {
            let within = at.checked_sub(2)?.checked_rem(FULL_BLOCK)?;
            if within < 5 {
                return one(
                    out,
                    *block_header(false, BLOCK)?.get(usize::try_from(within).ok()?)?,
                );
            }
            return zeros(out, FULL_BLOCK.checked_sub(within)?);
        }
        let rest = self.decoded.checked_rem(BLOCK)?;
        let within = at.checked_sub(blocks_end)?;
        if within < 5 {
            return one(
                out,
                *block_header(true, rest)?.get(usize::try_from(within).ok()?)?,
            );
        }
        let data_end = rest.checked_add(5)?;
        if within < data_end {
            return zeros(out, data_end.checked_sub(within)?);
        }
        let shift = 3_u64
            .checked_sub(within.checked_sub(data_end)?)?
            .checked_mul(8)?;
        let byte = self
            .adler()?
            .checked_shr(u32::try_from(shift).ok()?)?
            .checked_rem(256)?;
        one(out, u8::try_from(byte).ok()?)
    }

    fn length(&self) -> Option<u64> {
        ZLIB.checked_add(self.decoded)?
            .checked_add(self.decoded.checked_div(BLOCK)?.checked_mul(5)?)
    }
}

impl Part for Records {
    fn fill(&self, at: u64, out: &mut [u8]) -> Option<usize> {
        let mut bytes = [b' '; LINE];
        let mut line = Cursor::new(bytes.as_mut_slice());
        (self.render)(at.checked_div(self.width)?, &mut line).ok()?;
        let width = usize::try_from(self.width).ok()?;
        if usize::try_from(line.position()).ok()? > width {
            return None;
        }
        let within = usize::try_from(at.checked_rem(self.width)?).ok()?;
        let rest = bytes.get(within..width)?;
        let count = rest.len().min(out.len());
        out.get_mut(..count)?.copy_from_slice(rest.get(..count)?);
        Some(count)
    }

    fn length(&self) -> Option<u64> {
        self.count.checked_mul(self.width)
    }
}

impl Starts {
    /// The offset of object `number` of `tree`'s file.
    fn offset(self, tree: &Tree, number: u64) -> Option<u64> {
        let nodes_end = tree.leaves.checked_add(tree.nodes()?)?;
        if number <= tree.leaves {
            self.leaves
                .checked_add(number.checked_sub(1)?.checked_mul(LEAF)?)
        } else if number <= nodes_end {
            self.nodes.checked_add(
                number
                    .checked_sub(tree.leaves)?
                    .checked_sub(1)?
                    .checked_mul(NODE)?,
            )
        } else if number == tree.stream()? {
            Some(self.stream)
        } else {
            Some(self.catalog)
        }
    }
}

impl Tree {
    /// The catalog's object number.
    fn catalog(&self) -> Option<u64> {
        self.stream()?.checked_add(1)
    }

    /// Pages under node `index` of `level`.
    fn count(&self, level: usize, index: u64) -> Option<u64> {
        let span = FANOUT.checked_pow(u32::try_from(level).ok()?)?;
        let first = index.checked_mul(span)?;
        first.checked_add(span)?.min(self.leaves).checked_sub(first)
    }

    /// The object number of item `index` of `level`; level 0 holds the pages.
    fn id(&self, level: usize, index: u64) -> Option<u64> {
        let Some(above) = level.checked_sub(1) else {
            return index.checked_add(1);
        };
        let mut below = self.leaves.checked_add(1)?;
        for &nodes in self.levels.get(..above)? {
            below = below.checked_add(nodes)?;
        }
        below.checked_add(index)
    }

    /// Renders page `index`.
    fn leaf(&self, index: u64, line: &mut Cursor<&mut [u8]>) -> io::Result<()> {
        let number = index.checked_add(1).ok_or_else(overflow)?;
        let parent = self
            .id(1, index.checked_div(FANOUT).ok_or_else(overflow)?)
            .ok_or_else(overflow)?;
        let stream = self.stream().ok_or_else(overflow)?;
        write!(
            line,
            "{number} 0 obj\n<< /Type /Page /Parent {parent} 0 R /Contents {stream} 0 R >>\nendobj\n"
        )
    }

    /// The tree of `leaves` pages.
    fn new(leaves: u64) -> Option<Self> {
        let mut levels = Vec::new();
        let mut below = leaves;
        loop {
            let nodes = below
                .checked_add(FANOUT.checked_sub(1)?)?
                .checked_div(FANOUT)?;
            levels.push(nodes);
            if nodes <= 1 {
                return Some(Self { leaves, levels });
            }
            below = nodes;
        }
    }

    /// Renders node `record`, counted over all levels bottom up.
    fn node(&self, record: u64, line: &mut Cursor<&mut [u8]>) -> io::Result<()> {
        let mut level = 1_usize;
        let mut index = record;
        for &nodes in &self.levels {
            if index < nodes {
                break;
            }
            index = index.checked_sub(nodes).ok_or_else(overflow)?;
            level = level.checked_add(1).ok_or_else(overflow)?;
        }
        let number = self.id(level, index).ok_or_else(overflow)?;
        write!(line, "{number} 0 obj\n<< /Type /Pages ")?;
        if level == self.levels.len() {
            line.write_all(b"/MediaBox [0 0 612 792] ")?;
        } else {
            let parent = self
                .id(
                    level.checked_add(1).ok_or_else(overflow)?,
                    index.checked_div(FANOUT).ok_or_else(overflow)?,
                )
                .ok_or_else(overflow)?;
            write!(line, "/Parent {parent} 0 R ")?;
        }
        let count = self.count(level, index).ok_or_else(overflow)?;
        write!(line, "/Count {count} /Kids [")?;
        let child = level.checked_sub(1).ok_or_else(overflow)?;
        let children = match child.checked_sub(1) {
            Some(lower) => *self.levels.get(lower).ok_or_else(overflow)?,
            None => self.leaves,
        };
        let first = index.checked_mul(FANOUT).ok_or_else(overflow)?;
        let end = first
            .checked_add(FANOUT)
            .ok_or_else(overflow)?
            .min(children);
        for kid in first..end {
            let kid_number = self.id(child, kid).ok_or_else(overflow)?;
            write!(line, "{kid_number} 0 R ")?;
        }
        line.write_all(b"] >>\nendobj\n")
    }

    /// Nodes in all.
    fn nodes(&self) -> Option<u64> {
        self.levels
            .iter()
            .try_fold(0_u64, |sum, &nodes| sum.checked_add(nodes))
    }

    /// The root node's object number.
    fn root(&self) -> Option<u64> {
        self.leaves.checked_add(self.nodes()?)
    }

    /// The stream's object number.
    fn stream(&self) -> Option<u64> {
        self.root()?.checked_add(1)
    }
}

impl Virtual {
    /// Writes bytes from `at` into the start of `out`, returning how many: at least one.
    fn fill(&self, at: u64, out: &mut [u8]) -> Option<usize> {
        let found = self.parts.iter().rev().find(|part| part.0 <= at)?;
        let count = found.1.fill(at.checked_sub(found.0)?, out)?;
        (count > 0).then_some(count)
    }

    /// Appends `part`, returning where it starts.
    fn push(&mut self, part: Box<dyn Part>) -> Option<u64> {
        let start = self.size;
        self.size = start.checked_add(part.length()?)?;
        self.parts.push((start, part));
        Some(start)
    }

    /// Appends `text`, returning where it starts.
    fn text(&mut self, text: String) -> Option<u64> {
        self.push(Box::new(Bytes(text.into_bytes())))
    }
}

impl ReadAt for Virtual {
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let mut done = 0_usize;
        while let Some(rest) = buffer.get_mut(done..) {
            let at = u64::try_from(done)
                .ok()
                .and_then(|read| offset.checked_add(read))
                .ok_or_else(overflow)?;
            if rest.is_empty() || at >= self.size {
                break;
            }
            let count = self.fill(at, rest).ok_or_else(overflow)?;
            done = done.checked_add(count).ok_or_else(overflow)?;
        }
        Ok(done)
    }

    fn size(&self) -> io::Result<u64> {
        Ok(self.size)
    }
}

/// A stored block's header byte, `LEN` and `NLEN` (RFC 1951, 3.2.4).
fn block_header(last: bool, length: u64) -> Option<[u8; 5]> {
    let [low, high, complement_low, complement_high] = stored_lengths(u16::try_from(length).ok()?)?;
    Some([u8::from(last), low, high, complement_low, complement_high])
}

/// Asserts that `top` bytes were allocated, at most `bound`.
fn bounded(top: Option<usize>, bound: usize) {
    let bytes = top.unwrap_or(0);
    assert!((1..=bound).contains(&bytes), "peak {bytes} bytes");
}

/// The two files: (a) a few large streams, (b) millions of small objects.
fn cases() -> [Option<Case>; 2] {
    [
        fit(&few_large_streams, 1 << 28),
        fit(&many_small_objects, 1 << 26),
    ]
}

/// Decoded bytes and padding whose stream and padding take `stored` bytes in all.
fn decoded_for(stored: u64) -> Option<(u64, u64)> {
    let blocks_and_data = stored.checked_sub(ZLIB)?;
    let blocks = blocks_and_data.checked_div(FULL_BLOCK)?;
    let data = blocks_and_data
        .checked_rem(FULL_BLOCK)?
        .min(BLOCK.checked_sub(1)?);
    let pad = blocks_and_data.checked_rem(FULL_BLOCK)?.checked_sub(data)?;
    Some((blocks.checked_mul(BLOCK)?.checked_add(data)?, pad))
}

/// The cross-reference entry of object `number` (ISO 32000-2:2020, 7.5.4).
fn entry(tree: &Tree, starts: Starts, number: u64, line: &mut Cursor<&mut [u8]>) -> io::Result<()> {
    if number == 0 {
        return line.write_all(b"0000000000 65535 f\r\n");
    }
    let offset = starts.offset(tree, number).ok_or_else(overflow)?;
    write!(line, "{offset:010} 00000 n\r\n")
}

/// Four `FlateDecode` streams of about 256 MiB each, the last of `last` decoded bytes, then the
/// catalog, the page tree root and its one page whose contents they are, after `pad` spaces.
fn few_large_streams(pad: u64, last: u64) -> Option<Case> {
    let mut file = Virtual::default();
    let _: u64 = file.text(format!(
        "%PDF-2.0\n{}",
        " ".repeat(usize::try_from(pad).ok()?)
    ))?;
    let mut offsets = Vec::new();
    for (number, decoded) in [(1_u8, LARGE), (2, LARGE), (3, LARGE), (4, last)] {
        let data = Flate { decoded };
        offsets.push(file.text(format!(
            "{number} 0 obj\n<< /Filter /FlateDecode /Length {} >>\nstream\n",
            data.length()?
        ))?);
        let _: u64 = file.push(Box::new(data))?;
        let _: u64 = file.text("\nendstream\nendobj\n".to_owned())?;
    }
    for object in [
        "5 0 obj\n<< /Type /Catalog /Pages 6 0 R >>\nendobj\n",
        "6 0 obj\n<< /Type /Pages /Kids [7 0 R] /Count 1 /MediaBox [0 0 612 792] >>\nendobj\n",
        "7 0 obj\n<< /Type /Page /Parent 6 0 R /Contents [1 0 R 2 0 R 3 0 R 4 0 R] >>\nendobj\n",
    ] {
        offsets.push(file.text(object.to_owned())?);
    }
    let mut table = String::from("xref\n0 8\n0000000000 65535 f\r\n");
    for offset in offsets {
        write!(table, "{offset:010} 00000 n\r\n").ok()?;
    }
    let xref = file.text(table)?;
    let _: u64 = file.text(format!(
        "trailer\n<< /Root 5 0 R /Size 8 >>\nstartxref\n{xref}\n%%EOF\n"
    ))?;
    Some(Case {
        decoded: last,
        file,
        page: 0,
        stream: 4,
    })
}

/// The file of `build` with the stream's decoded bytes and the padding set so it is [`SIZE`]
/// bytes, starting from `decoded`.
fn fit(build: &dyn Fn(u64, u64) -> Option<Case>, decoded: u64) -> Option<Case> {
    let mut data = Flate { decoded };
    let mut pad = 0;
    for _ in 0_u8..4 {
        let case = build(pad, data.decoded)?;
        if case.file.size == SIZE {
            return Some(case);
        }
        let stored = data
            .length()?
            .checked_add(pad)?
            .checked_add(SIZE)?
            .checked_sub(case.file.size)?;
        (data.decoded, pad) = decoded_for(stored)?;
    }
    None
}

/// [`LEAVES`] pages in a page tree, then one `FlateDecode` stream of `decoded` bytes that is
/// every page's contents, after `pad` spaces.
fn many_small_objects(pad: u64, decoded: u64) -> Option<Case> {
    let tree = Arc::new(Tree::new(LEAVES)?);
    let mut file = Virtual::default();
    let _: u64 = file.text(format!(
        "%PDF-2.0\n{}",
        " ".repeat(usize::try_from(pad).ok()?)
    ))?;
    let leaf_tree = Arc::clone(&tree);
    let leaves = file.push(Box::new(Records {
        count: LEAVES,
        render: Box::new(move |index, line| leaf_tree.leaf(index, line)),
        width: LEAF,
    }))?;
    let node_tree = Arc::clone(&tree);
    let nodes = file.push(Box::new(Records {
        count: tree.nodes()?,
        render: Box::new(move |record, line| node_tree.node(record, line)),
        width: NODE,
    }))?;
    let data = Flate { decoded };
    let stream_number = tree.stream()?;
    let stream = file.text(format!(
        "{stream_number} 0 obj\n<< /Filter /FlateDecode /Length {} >>\nstream\n",
        data.length()?
    ))?;
    let _: u64 = file.push(Box::new(data))?;
    let _: u64 = file.text("\nendstream\nendobj\n".to_owned())?;
    let catalog_number = tree.catalog()?;
    let catalog = file.text(format!(
        "{catalog_number} 0 obj\n<< /Type /Catalog /Pages {} 0 R >>\nendobj\n",
        tree.root()?
    ))?;
    let objects = catalog_number.checked_add(1)?;
    let xref = file.text(format!("xref\n0 {objects}\n"))?;
    let starts = Starts {
        catalog,
        leaves,
        nodes,
        stream,
    };
    let entry_tree = Arc::clone(&tree);
    let _: u64 = file.push(Box::new(Records {
        count: objects,
        render: Box::new(move |number, line| entry(&entry_tree, starts, number, line)),
        width: ENTRY,
    }))?;
    let _: u64 = file.text(format!(
        "trailer\n<< /Root {catalog_number} 0 R /Size {objects} >>\nstartxref\n{xref}\n%%EOF\n"
    ))?;
    Some(Case {
        decoded,
        file,
        page: u32::try_from(LEAVES.checked_sub(1)?).ok()?,
        stream: u32::try_from(stream_number).ok()?,
    })
}

/// Writes `byte` into the start of `out`.
fn one(out: &mut [u8], byte: u8) -> Option<usize> {
    *out.first_mut()? = byte;
    Some(1)
}

/// Opens `file` with the default limits, which are above its stream and object count.
fn open(file: &Arc<dyn ReadAt>) -> Result<Document, PdfError> {
    let source = Source::from_read_at(file, &ReadAtSourceOptions::default())?;
    Document::open(&source, None, &OpenOptions::default())
}

/// The failure of an offset computation.
fn overflow() -> io::Error {
    io::Error::other("offset overflow")
}

/// Writes up to `available` zeros into the start of `out`.
fn zeros(out: &mut [u8], available: u64) -> Option<usize> {
    let count = usize::try_from(available).map_or(out.len(), |limit| limit.min(out.len()));
    out.get_mut(..count)?.fill(0);
    Some(count)
}

/// The crop box of a page deep in each file defaults to the media box its root node gives
/// (ISO 32000-2:2020, 7.7.3.4, 14.11.2).
#[test]
fn boundary_is_bounded() {
    for case in cases() {
        let measured = case.map(|found| {
            let read_at: Arc<dyn ReadAt> = Arc::new(found.file);
            let mut options = PageOptions::default();
            options.index = found.page;
            peak(|| {
                open(&read_at)
                    .and_then(|document| document.page(&options))
                    .and_then(|page| page.boundary(&BoundaryOptions::default()))
                    .map(|rect| [rect.x, rect.y, rect.width, rect.height])
                    .map_err(|error| error.code())
            })
        });
        assert_eq!(
            measured.as_ref().map(|&(rect, _)| rect),
            Some(Ok(MEDIA_BOX)),
            "crop box"
        );
        bounded(measured.map(|(_, bytes)| bytes), BOUNDARY_BOUND);
    }
}

/// A large `FlateDecode` stream deep in each file decodes to a sink (ISO 32000-2:2020, 7.4.4;
/// RFC 1950; RFC 1951, 3.2.4).
#[test]
fn stream_data_is_bounded() {
    for case in cases() {
        let counted = Arc::new(Counted {
            bytes: AtomicU64::new(0),
            only_zeros: AtomicBool::new(true),
        });
        let measured = case.map(|found| {
            let read_at: Arc<dyn ReadAt> = Arc::new(found.file);
            let append: Arc<dyn Append> = Arc::<Counted>::clone(&counted);
            let mut id = ObjectId::default();
            id.number = found.stream;
            let mut options = StreamDataOptions::default();
            options.decode = true;
            options.id = id;
            let (result, bytes) = peak(|| -> Result<(), PdfError> {
                let sink = Sink::from_append(&append, &AppendSinkOptions::default())?;
                open(&read_at)?.stream_data(&sink, &options)
            });
            (result.map_err(|error| error.code()), found.decoded, bytes)
        });
        assert_eq!(
            measured
                .as_ref()
                .map(|&(result, decoded, _)| (result, decoded)),
            Some((Ok(()), counted.bytes.load(Ordering::Relaxed))),
            "decoded bytes"
        );
        assert!(counted.only_zeros.load(Ordering::Relaxed), "decoded data");
        bounded(measured.map(|(_, _, bytes)| bytes), STREAM_BOUND);
    }
}
