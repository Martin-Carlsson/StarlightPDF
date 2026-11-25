//! The annotate and redact cases: redaction marks, and what applying them leaves (ISO 32000-2:2020, 12.5.6.23).

use core::sync::atomic::{AtomicU64, Ordering};

use starlightpdf::annotations::{AddAnnotationOptions, AnnotationKind, DeleteAnnotationOptions};
use starlightpdf::document::{Document, PageOptions, SaveOptions};
use starlightpdf::edit::ApplyRedactionsOptions;
use starlightpdf::objects::ObjectId;
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf::page::{Boundary, BoundaryOptions, Rect};
use starlightpdf::progress::Progress;
use starlightpdf::rendering::RenderOptions;
use starlightpdf::{ErrorCode, PdfError};

use crate::decode::{Image, inflate};
use crate::render::{is_code, matched, pixels};
use crate::{
    Case, Failure, MODES, OPEN, Object, entry, filter, hex, lookup, name, number, object_at, open,
    parse, parse_object, reopen, resolve, round, save_bytes, sha256, source, stream_bytes, tokens,
    twice, unhex,
};

/// The keys of an annotate case besides those of open.
const ANNOTATE: [&str; 10] = [
    "color", "delete", "height", "index", "kind", "quads", "saved", "width", "x", "y",
];

/// Deepest nesting of forms whose points are counted.
const DEPTH: u32 = 16;

/// The identity matrix.
const IDENTITY: [f64; 6] = [1.0_f64, 0.0_f64, 0.0_f64, 1.0_f64, 0.0_f64, 0.0_f64];

/// Each kind's name.
const KINDS: [(AnnotationKind, &str); 21] = [
    (AnnotationKind::Ellipse, "Ellipse"),
    (AnnotationKind::FileAttachment, "FileAttachment"),
    (AnnotationKind::FreeText, "FreeText"),
    (AnnotationKind::Highlight, "Highlight"),
    (AnnotationKind::Ink, "Ink"),
    (AnnotationKind::Line, "Line"),
    (AnnotationKind::Link, "Link"),
    (AnnotationKind::Media, "Media"),
    (AnnotationKind::Model3d, "Model3d"),
    (AnnotationKind::Note, "Note"),
    (AnnotationKind::Other, "Other"),
    (AnnotationKind::Polygon, "Polygon"),
    (AnnotationKind::Polyline, "Polyline"),
    (AnnotationKind::Popup, "Popup"),
    (AnnotationKind::Rectangle, "Rectangle"),
    (AnnotationKind::Redact, "Redact"),
    (AnnotationKind::RubberStamp, "RubberStamp"),
    (AnnotationKind::Squiggly, "Squiggly"),
    (AnnotationKind::Strikeout, "Strikeout"),
    (AnnotationKind::Underline, "Underline"),
    (AnnotationKind::Widget, "Widget"),
];

/// Each `read=` of a redact case, and whether it takes `index=`.
const READS: [(&str, bool); 11] = [
    ("annotations", true),
    ("image_raw", false),
    ("image_samples", false),
    ("mask_samples", false),
    ("ok", false),
    ("painted", true),
    ("pixels", true),
    ("points", true),
    ("render", true),
    ("text", false),
    ("thumb", true),
];

/// The keys of a redact case besides those of open.
const REDACT: [&str; 9] = [
    "bytes", "cancel", "do", "except", "index", "marks", "mode", "pages", "read",
];

/// White-space bytes (ISO 32000-2:2020, 7.2.3).
const WHITE: &[u8] = b"\0\t\n\x0C\r ";

/// A progress callback that returns false after `allowed` calls.
struct After {
    /// The calls let through.
    allowed: u64,
    /// The calls so far.
    calls: AtomicU64,
}

/// An axis-aligned area between two x and two y.
#[derive(Clone, Copy)]
struct Area {
    /// The larger y.
    high: f64,
    /// The smaller x.
    left: f64,
    /// The smaller y.
    low: f64,
    /// The larger x.
    right: f64,
}

/// A mark a case adds: `page@x,y,w,h[@rrggbb]`.
struct Mark {
    /// Its colour, `#rrggbb`, or empty for none.
    colour: String,
    /// Its page.
    page: u32,
    /// `x`, `y`, `w`, `h` in page coordinates.
    place: [f64; 4],
}

/// A case's keys beyond those of open.
#[derive(Default)]
struct Request {
    /// `bytes=`.
    bytes: Vec<u8>,
    /// `cancel` (0) or `cancel=N`: the progress calls let through.
    cancel: Option<u64>,
    /// `delete=`.
    delete: Vec<usize>,
    /// `except=`.
    except: Vec<Area>,
    /// `do=`.
    image: usize,
    /// `index=`.
    index: u32,
    /// `marks=`.
    marks: Vec<Mark>,
    /// `kind=`, `x=`, `y=`, `width=`, `height=`, `quads=`, `color=`.
    options: Option<AddAnnotationOptions>,
    /// `pages=`.
    pages: Option<Vec<u32>>,
    /// `read=`.
    read: String,
    /// `mode=`.
    save: SaveOptions,
    /// The keys given.
    seen: Vec<String>,
}

/// A count of the path points strictly inside a mark (ISO 32000-2:2020, 8.5.2).
struct Walk<'doc> {
    /// The document.
    document: &'doc Document,
    /// The points found inside.
    inside: u64,
    /// The marks, in default user space.
    marks: Vec<Area>,
}

impl Progress for After {
    fn update(&self, done: u64, total: u64) -> bool {
        let _: (u64, u64) = (done, total);
        self.calls.fetch_add(1, Ordering::Relaxed) < self.allowed
    }
}

impl Area {
    /// Whether `x`, `y` is inside, not on the edge.
    fn holds(&self, x: f64, y: f64) -> bool {
        x > self.left && x < self.right && y > self.low && y < self.high
    }

    /// The area `x`, `y`, `w`, `h`.
    const fn of(place: &[f64; 4]) -> Self {
        let [x, y, width, height] = *place;
        Self {
            high: height.mul_add(1.0_f64, y),
            left: x,
            low: y,
            right: width.mul_add(1.0_f64, x),
        }
    }

    /// Whether the unit square from `column`, `row` overlaps the area.
    fn overlaps(&self, column: f64, row: f64) -> bool {
        column.mul_add(1.0_f64, 1.0_f64) > self.left
            && column < self.right
            && row.mul_add(1.0_f64, 1.0_f64) > self.low
            && row < self.high
    }

    /// The area, in page coordinates, in default user space of an unrotated page cropped to `crop`
    /// (ISO 32000-2:2020, 14.11.2).
    const fn user(&self, crop: &Rect) -> Self {
        let top = crop.height.mul_add(1.0_f64, crop.y);
        Self {
            high: self.low.mul_add(-1.0_f64, top),
            left: self.left.mul_add(1.0_f64, crop.x),
            low: self.high.mul_add(-1.0_f64, top),
            right: self.right.mul_add(1.0_f64, crop.x),
        }
    }
}

impl Request {
    /// The options of the annotation to add.
    fn adding(&mut self) -> &mut AddAnnotationOptions {
        self.options
            .get_or_insert_with(AddAnnotationOptions::default)
    }

    /// Fails unless the keys go together.
    fn check(&self, redact: bool) -> Result<(), &'static str> {
        let has = |key: &str| self.seen.iter().any(|seen| seen == key);
        if !redact {
            return if !has("index") {
                Err("annotate needs index=")
            } else if self.options.is_some() && !has("kind") {
                Err("a mark needs kind=")
            } else {
                Ok(())
            };
        }
        let indexed = READS
            .iter()
            .find(|pair| pair.0 == self.read)
            .ok_or("redact needs read=")?
            .1;
        let images = self.read.starts_with("image_") || self.read == "mask_samples";
        if has("index") && !indexed
            || has("bytes") != (self.read == "text")
            || has("do") != images
            || has("except") && self.read != "pixels"
        {
            return Err("index=, bytes=, do= or except= with a read that takes none");
        }
        Ok(())
    }

    /// Takes `key` with its `value`; `None` when either is bad.
    fn set(&mut self, key: &str, value: Option<&str>) -> Option<()> {
        if self.seen.iter().any(|seen| seen == key) {
            return None;
        }
        self.seen.push(String::from(key));
        let real = |text: &str| text.parse::<f64>().ok().filter(|found| found.is_finite());
        match (key, value) {
            ("saved", None) => {}
            ("cancel", None) => self.cancel = Some(0),
            ("cancel", Some(count)) => self.cancel = Some(count.parse().ok()?),
            ("color", Some(colour)) => colour.clone_into(&mut self.adding().color),
            ("delete", Some(at)) => {
                self.delete = at
                    .split(',')
                    .map(|nth| nth.parse().ok())
                    .collect::<Option<_>>()?;
            }
            ("height", Some(text)) => self.adding().height = real(text)?,
            ("width", Some(text)) => self.adding().width = real(text)?,
            ("x", Some(text)) => self.adding().x = real(text)?,
            ("y", Some(text)) => self.adding().y = real(text)?,
            ("index", Some(text)) => self.index = text.parse().ok()?,
            ("kind", Some("Redact")) => self.adding().kind = AnnotationKind::Redact,
            ("quads", Some(text)) => {
                self.adding().quads = text.split(',').map(real).collect::<Option<Vec<f64>>>()?;
            }
            ("bytes", Some(text)) => {
                self.bytes = unhex(text).filter(|found| !found.is_empty())?;
            }
            ("do", Some(at)) => self.image = at.parse().ok()?,
            ("except", Some(text)) => {
                self.except = text
                    .split(';')
                    .map(|item| places(item).map(|place| Area::of(&place)))
                    .collect::<Option<Vec<Area>>>()?;
            }
            ("marks", Some(text)) => {
                self.marks = text.split(';').map(mark).collect::<Option<Vec<Mark>>>()?;
            }
            ("mode", Some(text)) => self.save.mode = lookup(&MODES, text)?,
            ("pages", Some("")) => self.pages = Some(Vec::new()),
            ("pages", Some(text)) => {
                self.pages = Some(
                    text.split(',')
                        .map(|item| item.parse().ok())
                        .collect::<Option<Vec<u32>>>()?,
                );
            }
            ("read", Some(text)) => {
                READS.iter().any(|pair| pair.0 == text).then_some(())?;
                self.read = String::from(text);
            }
            _ => return None,
        }
        Some(())
    }
}

impl Walk<'_> {
    /// Counts the points of `content` drawn with `resources` from `start`, following forms `depth` deep.
    fn content(
        &mut self,
        content: &[u8],
        resources: Option<&Object>,
        start: [f64; 6],
        depth: u32,
    ) -> Result<(), Failure> {
        let mut states: Vec<[f64; 6]> = Vec::new();
        let mut current = start;
        let mut operands: Vec<String> = Vec::new();
        for token in content_tokens(content) {
            match token.as_str() {
                "q" => states.push(current),
                "Q" => current = states.pop().unwrap_or(current),
                "cm" => {
                    let given = numbers(&operands, 6)?;
                    let mut matrix = IDENTITY;
                    matrix.copy_from_slice(&given);
                    current = product(&matrix, &current);
                }
                "m" | "l" => self.count(&current, &numbers(&operands, 2)?),
                "v" | "y" => self.count(&current, &numbers(&operands, 4)?),
                "c" => self.count(&current, &numbers(&operands, 6)?),
                "re" => {
                    let given = numbers(&operands, 4)?;
                    let [x, y, width, height] = *given.as_slice() else {
                        return Err(Failure::Case(String::from("re takes 4 numbers")));
                    };
                    let (right, top) = (width.mul_add(1.0_f64, x), height.mul_add(1.0_f64, y));
                    self.count(&current, &[x, y, right, y, x, top, right, top]);
                }
                "Do" => {
                    let label = operands
                        .last()
                        .and_then(|operand| operand.strip_prefix('/'))
                        .ok_or_else(|| Failure::Case(String::from("Do takes a name")))?;
                    self.form(label, resources, current, depth)?;
                }
                _ => {}
            }
            if is_operator(&token) {
                operands.clear();
            } else {
                operands.push(token);
            }
        }
        Ok(())
    }

    /// Counts each point of `coordinates`, pairs in the space `ctm` maps to default user space.
    fn count(&mut self, ctm: &[f64; 6], coordinates: &[f64]) {
        let [ma, mb, mc, md, me, mf] = *ctm;
        for pair in coordinates.as_chunks::<2>().0 {
            let [x, y] = *pair;
            let (across, up) = (
                ma.mul_add(x, mc.mul_add(y, me)),
                mb.mul_add(x, md.mul_add(y, mf)),
            );
            if self.marks.iter().any(|mark| mark.holds(across, up)) {
                self.inside = self.inside.saturating_add(1);
            }
        }
    }

    /// Counts the points of the form `label` names in `resources`, if it is one (ISO 32000-2:2020, 8.10.1).
    fn form(
        &mut self,
        label: &str,
        resources: Option<&Object>,
        ctm: [f64; 6],
        depth: u32,
    ) -> Result<(), Failure> {
        let Some(Object::Reference(id)) = named(self.document, resources, "XObject", label)? else {
            return Err(Failure::Case(format!("no XObject /{label}")));
        };
        let dictionary = object_at(self.document, id)?;
        if !matches!(entry(dictionary.clone(), "Subtype"), Some(Object::Name(subtype)) if subtype == "Form")
        {
            return Ok(());
        }
        if depth >= DEPTH {
            return Err(Failure::Case(String::from("forms nest too deep")));
        }
        let mut matrix = IDENTITY;
        if let Some(Object::Array(items)) = entry(dictionary.clone(), "Matrix") {
            let given = items
                .into_iter()
                .map(|item| number(Some(item)).map(|value| value.parse::<f64>().unwrap_or(0.0_f64)))
                .collect::<Result<Vec<f64>, Failure>>()?;
            if given.len() != 6 {
                return Err(Failure::Case(String::from("a Matrix not of 6 numbers")));
            }
            matrix.copy_from_slice(&given);
        }
        let own = entry(dictionary, "Resources");
        let content = decoded(self.document, id)?;
        self.content(
            &content,
            own.as_ref().or(resources),
            product(&matrix, &ctm),
            depth.saturating_add(1),
        )
    }
}

/// What `operation` on `relative` gives as `args` say: an annotate case's annotations, or a redact case's read.
pub(crate) fn call(
    operation: &str,
    redact: bool,
    file: bool,
    relative: &str,
    args: &[&str],
    expected: &str,
) -> Result<String, String> {
    let own: &[&str] = if redact { &REDACT } else { &ANNOTATE };
    let mut request = Request::default();
    let mut others = Vec::new();
    for &arg in args {
        let (key, value) = arg
            .split_once('=')
            .map_or((arg, None), |(head, tail)| (head, Some(tail)));
        if own.contains(&key) {
            request
                .set(key, value)
                .ok_or_else(|| format!("{operation}: bad argument '{arg}'"))?;
        } else {
            others.push(arg);
        }
    }
    let case = parse(operation, &OPEN, &others)?;
    request
        .check(redact)
        .map_err(|why| format!("{operation}: {why}"))?;
    if request.read == "render" && !is_code(expected) && pixels(expected).is_none() {
        return Err(format!("{operation}: bad expected value '{expected}'"));
    }
    let made = source(relative, file)?;
    let result = made.map_err(Failure::Pdf).and_then(|found| {
        let document = open(&found, &case)?;
        if redact {
            redacted(&document, &request, &case, expected)
        } else {
            annotated(&document, &request, &case)
        }
    });
    match result {
        Ok(value) => Ok(value),
        Err(Failure::Pdf(error)) => Ok(String::from(error.code().name())),
        Err(Failure::Case(why)) => Err(format!("{operation}: {why}")),
    }
}

/// Adds and deletes as `request` says, saving after when `saved`; the annotations of page `index=`.
fn annotated(document: &Document, request: &Request, case: &Case) -> Result<String, Failure> {
    let mut at = PageOptions::default();
    at.index = request.index;
    let page = document.page(&at)?;
    if let Some(adding) = request.options.as_ref() {
        drop(page.add_annotation(None, adding)?);
    }
    let all = page.annotations()?;
    let mut listed = Vec::new();
    while let Some(found) = all.next()? {
        listed.push(found);
    }
    for &nth in &request.delete {
        listed
            .get(nth)
            .ok_or_else(|| Failure::Case(format!("no annotation {nth}")))?
            .delete(&DeleteAnnotationOptions::default())?;
    }
    if request.seen.iter().any(|key| key == "saved") {
        let saved = reopen(twice(document, &SaveOptions::default(), false)?, case)?;
        return Ok(listing(&saved, request.index)?.0);
    }
    Ok(listing(document, request.index)?.0)
}

/// The bytes of `content` with each inline image's data left out (ISO 32000-2:2020, 8.9.7).
fn content_tokens(content: &[u8]) -> Vec<String> {
    let white = |at: usize| content.get(at).is_none_or(|byte| WHITE.contains(byte));
    let mut kept = Vec::new();
    let mut from = 0_usize;
    let mut at = 0_usize;
    while let Some(window) = content.get(at..at.saturating_add(2)) {
        let next = at.saturating_add(2);
        if window == b"ID" && (at == 0 || white(at.saturating_sub(1))) && white(next) {
            kept.extend_from_slice(content.get(from..next).unwrap_or_default());
            kept.push(b' ');
            let mut end = next.saturating_add(1);
            while end < content.len()
                && !(content.get(end..end.saturating_add(2)) == Some(b"EI".as_slice())
                    && white(end.saturating_sub(1))
                    && white(end.saturating_add(2)))
            {
                end = end.saturating_add(1);
            }
            from = end;
            at = end.saturating_add(2);
        } else {
            at = at.saturating_add(1);
        }
    }
    kept.extend_from_slice(content.get(from..).unwrap_or_default());
    tokens(&String::from_utf8_lossy(&kept))
}

/// The decoded data of the contents `value` names, each stream followed by a newline.
fn contents(document: &Document, value: Option<Object>) -> Result<Vec<u8>, Failure> {
    let streams = match value {
        Some(Object::Reference(id)) => vec![id],
        Some(Object::Array(items)) => items
            .into_iter()
            .filter_map(|item| match item {
                Object::Reference(id) => Some(id),
                Object::Array(_) | Object::Dictionary(_) | Object::Name(_) | Object::Other(_) => {
                    None
                }
            })
            .collect(),
        Some(Object::Dictionary(_) | Object::Name(_) | Object::Other(_)) | None => Vec::new(),
    };
    let mut content = Vec::new();
    for id in streams {
        content.extend(decoded(document, id)?);
        content.push(b'\n');
    }
    Ok(content)
}

/// The data of the stream `id`, inflated by the runner when it is `FlateDecode` (RFC 1950, RFC 1951).
fn decoded(document: &Document, id: ObjectId) -> Result<Vec<u8>, Failure> {
    let dictionary = object_at(document, id)?;
    if entry(dictionary.clone(), "DecodeParms").is_some() {
        return Err(Failure::Case(String::from("a stream with DecodeParms")));
    }
    let raw = stream_bytes(document, id, false)?;
    unfiltered(&filter(entry(dictionary, "Filter"))?, raw)
}

/// The index of `needle` in `haystack` from `from` on.
fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)?
        .checked_add(from)
}

/// `present` when `needle`, its bytes or their hexadecimal form, is in `saved` or in any stream of it once decoded,
/// else `absent`.
fn found(saved: &[u8], needle: &[u8]) -> Result<String, Failure> {
    let mut places = vec![saved.to_vec()];
    places.extend(streams(saved)?);
    let lower = hex(needle);
    let forms = [
        needle.to_vec(),
        lower.to_ascii_uppercase().into_bytes(),
        lower.into_bytes(),
    ];
    let present = places.iter().any(|place| {
        forms.iter().any(|form| {
            place
                .windows(form.len())
                .any(|window| window == form.as_slice())
        })
    });
    Ok(String::from(if present { "present" } else { "absent" }))
}

/// Whether `token` is an operator (ISO 32000-2:2020, 7.8.2).
fn is_operator(token: &str) -> bool {
    let first = token.chars().next().unwrap_or(' ');
    first.is_ascii_alphabetic() || first == '\'' || first == '"'
}

/// The page object of kid `index` of the root page tree node.
fn kid(document: &Document, index: u32) -> Result<Object, Failure> {
    let catalog = object_at(document, document.catalog()?)?;
    let pages = resolve(document, entry(catalog, "Pages"))?;
    let kid = match resolve(document, entry(pages, "Kids"))? {
        Object::Array(items) => items
            .into_iter()
            .nth(usize::try_from(index).unwrap_or(usize::MAX)),
        Object::Dictionary(_) | Object::Name(_) | Object::Other(_) | Object::Reference(_) => None,
    };
    resolve(document, kid)
}

/// Page `index`'s annotations as a case gives them, and the areas of its marks in page coordinates: each
/// quadrilateral's bounds, else the rectangle (ISO 32000-2:2020, 12.5.6.23).
fn listing(document: &Document, index: u32) -> Result<(String, Vec<Area>), Failure> {
    let mut at = PageOptions::default();
    at.index = index;
    let all = document.page(&at)?.annotations()?;
    let mut items = Vec::new();
    let mut marks = Vec::new();
    while let Some(annotation) = all.next()? {
        let info = annotation.info()?;
        let place = [info.x, info.y, info.width, info.height];
        let mut item = format!("{} {}", name(&KINDS, info.kind), joined(&place));
        if !info.quads.is_empty() {
            item.push_str(" q=");
            item.push_str(&joined(&info.quads));
        }
        if !info.color.is_empty() {
            item.push_str(" c=");
            item.push_str(info.color.trim_start_matches('#'));
        }
        items.push(item);
        if info.kind != AnnotationKind::Redact {
            continue;
        }
        if info.quads.is_empty() {
            marks.push(Area::of(&place));
        }
        for quad in info.quads.as_chunks::<8>().0 {
            let [x1, y1, x2, y2, x3, y3, x4, y4] = *quad;
            let (xs, ys) = ([x1, x2, x3, x4], [y1, y2, y3, y4]);
            marks.push(Area {
                high: ys.into_iter().fold(f64::MIN, f64::max),
                left: xs.into_iter().fold(f64::MAX, f64::min),
                low: ys.into_iter().fold(f64::MAX, f64::min),
                right: xs.into_iter().fold(f64::MIN, f64::max),
            });
        }
    }
    let shown = if items.is_empty() {
        String::from("none")
    } else {
        items.join(";")
    };
    Ok((shown, marks))
}

/// `values` rounded and comma-separated.
fn joined(values: &[f64]) -> String {
    values
        .iter()
        .map(|&value| round(value))
        .collect::<Vec<String>>()
        .join(",")
}

/// The mark `page@x,y,w,h[@rrggbb]`.
fn mark(text: &str) -> Option<Mark> {
    let mut parts = text.split('@');
    let page = parts.next()?.parse().ok()?;
    let place = places(parts.next()?)?;
    let colour = match parts.next() {
        Some(digits)
            if digits.len() == 6
                && digits
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')) =>
        {
            format!("#{digits}")
        }
        Some(_) => return None,
        None => String::new(),
    };
    parts.next().is_none().then_some(Mark {
        colour,
        page,
        place,
    })
}

/// The samples of the mask of image `id`, whose dictionary is `image`: its soft mask (ISO 32000-2:2020, 11.6.5.3),
/// its explicit mask (8.9.6.3) or itself if a stencil (8.9.6.2), as hex with 1 for masked; or `key` and the colour
/// key ranges (8.9.6.4); or `none`.
fn mask(document: &Document, image: &Object, id: ObjectId) -> Result<String, Failure> {
    if let Some(Object::Reference(soft)) = entry(image.clone(), "SMask") {
        return Ok(hex(&decoded(document, soft)?));
    }
    match entry(image.clone(), "Mask") {
        Some(Object::Reference(explicit)) => {
            let dictionary = object_at(document, explicit)?;
            return Ok(hex(&stencil(&dictionary, decoded(document, explicit)?)?));
        }
        Some(Object::Array(ranges)) => {
            let values = ranges
                .into_iter()
                .map(|item| number(Some(item)))
                .collect::<Result<Vec<String>, Failure>>()?;
            return Ok(format!("key {}", values.join(" ")));
        }
        Some(Object::Dictionary(_) | Object::Name(_) | Object::Other(_)) | None => {}
    }
    if matches!(entry(image.clone(), "ImageMask"), Some(Object::Other(flag)) if flag == "true") {
        return Ok(hex(&stencil(image, decoded(document, id)?)?));
    }
    Ok(String::from("none"))
}

/// The value `key` names in category `category` of `resources`, if any.
fn named(
    document: &Document,
    resources: Option<&Object>,
    category: &str,
    key: &str,
) -> Result<Option<Object>, Failure> {
    let Some(found) = resources else {
        return Ok(None);
    };
    let Some(items) = entry(resolve(document, Some(found.clone()))?, category) else {
        return Ok(None);
    };
    Ok(entry(resolve(document, Some(items))?, key))
}

/// The last `count` of `operands` as numbers.
fn numbers(operands: &[String], count: usize) -> Result<Vec<f64>, Failure> {
    let bad = || Failure::Case(format!("an operator takes {count} numbers"));
    let start = operands.len().checked_sub(count).ok_or_else(bad)?;
    operands
        .get(start..)
        .ok_or_else(bad)?
        .iter()
        .map(|operand| operand.parse::<f64>().map_err(|_err| bad()))
        .collect()
}

/// The image `XObject`s each `Do` of page `index`'s content paints, in order, and how many inline images it has
/// (ISO 32000-2:2020, 8.8, 8.9.7).
fn painted(document: &Document, index: u32) -> Result<(Vec<(ObjectId, Object)>, usize), Failure> {
    let page = kid(document, index)?;
    let resources = entry(page.clone(), "Resources");
    let content = contents(document, entry(page, "Contents"))?;
    let mut operands: Vec<String> = Vec::new();
    let mut images = Vec::new();
    let mut inline = 0_usize;
    for token in content_tokens(&content) {
        if token == "BI" {
            inline = inline.saturating_add(1);
        }
        if token == "Do" {
            let label = operands
                .last()
                .and_then(|operand| operand.strip_prefix('/'))
                .ok_or_else(|| Failure::Case(String::from("Do takes a name")))?;
            if let Some(Object::Reference(id)) =
                named(document, resources.as_ref(), "XObject", label)?
            {
                let image = object_at(document, id)?;
                if matches!(entry(image.clone(), "Subtype"), Some(Object::Name(subtype)) if subtype == "Image")
                {
                    images.push((id, image));
                }
            }
        }
        if is_operator(&token) {
            operands.clear();
        } else {
            operands.push(token);
        }
    }
    Ok((images, inline))
}

/// The numbers of `x,y,w,h`.
fn places(text: &str) -> Option<[f64; 4]> {
    let values = text
        .split(',')
        .map(|item| item.parse::<f64>().ok().filter(|found| found.is_finite()))
        .collect::<Option<Vec<f64>>>()?;
    let [x, y, width, height] = *values.as_slice() else {
        return None;
    };
    Some([x, y, width, height])
}

/// The points of page `index`'s paths strictly inside `marks`, in page coordinates: those of `m`, `l`, `c`, `v`, `y`
/// and the corners of `re`, through `cm` and form matrices (ISO 32000-2:2020, 8.3.4, 8.5.2.1, 8.10.1).
fn points(document: &Document, index: u32, marks: &[Area]) -> Result<String, Failure> {
    let mut at = PageOptions::default();
    at.index = index;
    let page = document.page(&at)?;
    if page.rotation()? != 0 {
        return Err(Failure::Case(String::from(
            "points needs an unrotated page",
        )));
    }
    let mut boundary = BoundaryOptions::default();
    boundary.kind = Boundary::Crop;
    let crop = page.boundary(&boundary)?;
    let object = kid(document, index)?;
    let mut walk = Walk {
        document,
        inside: 0,
        marks: marks.iter().map(|area| area.user(&crop)).collect(),
    };
    let content = contents(document, entry(object.clone(), "Contents"))?;
    let resources = entry(object, "Resources");
    walk.content(&content, resources.as_ref(), IDENTITY, 0)?;
    Ok(walk.inside.to_string())
}

/// The product of `first` and `second` (ISO 32000-2:2020, 8.3.4).
const fn product(first: &[f64; 6], second: &[f64; 6]) -> [f64; 6] {
    let [ma, mb, mc, md, me, mf] = *first;
    let [ca, cb, cc, cd, ce, cf] = *second;
    [
        ma.mul_add(ca, mb.mul_add(cc, 0.0_f64)),
        ma.mul_add(cb, mb.mul_add(cd, 0.0_f64)),
        mc.mul_add(ca, md.mul_add(cc, 0.0_f64)),
        mc.mul_add(cb, md.mul_add(cd, 0.0_f64)),
        me.mul_add(ca, mf.mul_add(cc, ce)),
        me.mul_add(cb, mf.mul_add(cd, cf)),
    ]
}

/// What `read` gives of the saved `document`, whose marks before applying were `marks`, a list a page, and whose
/// page `index=` rendered as `before` then.
fn read(
    document: &Document,
    request: &Request,
    saved: &[u8],
    marks: &[Vec<Area>],
    before: Option<Vec<u8>>,
    expected: &str,
) -> Result<String, Failure> {
    let index = request.index;
    let page_marks = marks
        .get(usize::try_from(index).unwrap_or(usize::MAX))
        .map_or(&[][..], Vec::as_slice);
    match request.read.as_str() {
        "ok" => Ok(String::from("ok")),
        "text" => found(saved, &request.bytes),
        "annotations" => Ok(listing(document, index)?.0),
        "thumb" => Ok(String::from(
            if entry(kid(document, index)?, "Thumb").is_some() {
                "kept"
            } else {
                "none"
            },
        )),
        "painted" => {
            let (images, inline) = painted(document, index)?;
            Ok(format!("images={} inline={inline}", images.len()))
        }
        "points" => points(document, index, page_marks),
        "pixels" => {
            let mut excluded = page_marks.to_vec();
            excluded.extend_from_slice(&request.except);
            same(before, document, index, &excluded, expected)
        }
        "render" => {
            let Some(image) = shown(document, index)? else {
                return Ok(String::from(expected));
            };
            let named_pixels =
                pixels(expected).ok_or_else(|| Failure::Case(String::from("bad pixels")))?;
            let mut parts = vec![format!("{}x{}", image.width, image.height)];
            parts.extend(named_pixels.iter().map(|pixel| matched(&image, pixel)));
            Ok(parts.join(" "))
        }
        _ => {
            let (images, _) = painted(document, 0)?;
            let (id, image) = images
                .into_iter()
                .nth(request.image)
                .ok_or_else(|| Failure::Case(format!("no image {}", request.image)))?;
            match request.read.as_str() {
                "image_raw" => Ok(sha256(&stream_bytes(document, id, false)?)),
                "image_samples" => Ok(hex(&decoded(document, id)?)),
                _ => mask(document, &image, id),
            }
        }
    }
}

/// Adds `request`'s marks to `document`, applies them, saves, reopens and reads; a failed apply gives its code, with
/// `changed` unless the document is as before.
fn redacted(
    document: &Document,
    request: &Request,
    case: &Case,
    expected: &str,
) -> Result<String, Failure> {
    for added in &request.marks {
        let mut at = PageOptions::default();
        at.index = added.page;
        let mut options = AddAnnotationOptions::default();
        options.kind = AnnotationKind::Redact;
        [options.x, options.y, options.width, options.height] = added.place;
        options.color.clone_from(&added.colour);
        drop(document.page(&at)?.add_annotation(None, &options)?);
    }
    let (listed, marks) = survey(document)?;
    let applied = request
        .pages
        .clone()
        .unwrap_or_else(|| (0..u32::try_from(marks.len()).unwrap_or(u32::MAX)).collect());
    let marked = applied.iter().any(|&page| {
        marks
            .get(usize::try_from(page).unwrap_or(usize::MAX))
            .is_some_and(|found| !found.is_empty())
    });
    let before = save_bytes(document, &SaveOptions::default(), false)?;
    let rendering = cfg!(feature = "rendering");
    let shown_before = if request.read == "pixels" && rendering {
        Some(rendered(document, request.index)?)
    } else {
        None
    };
    let counted = request.cancel.map(|allowed| After {
        allowed,
        calls: AtomicU64::new(0),
    });
    let stop = counted.as_ref().map(|after| -> &dyn Progress { after });
    let mut options = ApplyRedactionsOptions::default();
    options.pages.clone_from(&request.pages);
    if let Err(error) = document.apply_redactions(stop, &options) {
        let kept = save_bytes(document, &SaveOptions::default(), false)? == before
            && survey(document)?.0 == listed;
        let code = error.code();
        if !rendering && marked {
            return if code == ErrorCode::CapabilityMissing && kept {
                Ok(String::from(expected))
            } else {
                Err(Failure::Case(format!(
                    "{} without rendering, not CapabilityMissing with the document unchanged",
                    code.name()
                )))
            };
        }
        return Ok(if kept {
            String::from(code.name())
        } else {
            format!("{} changed", code.name())
        });
    }
    if !rendering && marked {
        return Err(Failure::Case(String::from(
            "marks applied without rendering",
        )));
    }
    let saved = twice(document, &request.save, false)?;
    let reopened = reopen(saved.clone(), case)?;
    read(&reopened, request, &saved, &marks, shown_before, expected)
}

/// The PNG of page `index` rendered at the defaults.
fn rendered(document: &Document, index: u32) -> Result<Vec<u8>, PdfError> {
    let sink = Sink::in_memory(&MemorySinkOptions::default())?;
    let mut at = PageOptions::default();
    at.index = index;
    document
        .page(&at)?
        .render(&sink, None, &RenderOptions::default())?;
    sink.take_bytes()
}

/// `same` when the render `before` and page `index`'s now agree on every pixel outside `excluded`, else how many differ.
fn same(
    before: Option<Vec<u8>>,
    document: &Document,
    index: u32,
    excluded: &[Area],
    expected: &str,
) -> Result<String, Failure> {
    let Some(after) = shown(document, index)? else {
        return Ok(String::from(expected));
    };
    let first = before.ok_or_else(|| Failure::Case(String::from("no render before")))?;
    let earlier = Image::read(&first).map_err(|why| Failure::Case(String::from(why)))?;
    if (earlier.width, earlier.height, earlier.channels)
        != (after.width, after.height, after.channels)
    {
        return Err(Failure::Case(String::from("the renders differ in size")));
    }
    let mut differ = 0_u64;
    for row in 0..after.height {
        for column in 0..after.width {
            let (Ok(across), Ok(down)) = (u32::try_from(column), u32::try_from(row)) else {
                return Err(Failure::Case(String::from("a render too large")));
            };
            if excluded
                .iter()
                .any(|area| area.overlaps(f64::from(across), f64::from(down)))
            {
                continue;
            }
            if earlier.pixel(column, row) != after.pixel(column, row) {
                differ = differ.saturating_add(1);
            }
        }
    }
    Ok(if differ == 0 {
        String::from("same")
    } else {
        format!("{differ} differ")
    })
}

/// Page `index` rendered and read back; `None` without rendering, where the render fails `CapabilityMissing`.
fn shown(document: &Document, index: u32) -> Result<Option<Image>, Failure> {
    match rendered(document, index) {
        Ok(bytes) => Ok(Some(
            Image::read(&bytes).map_err(|why| Failure::Case(String::from(why)))?,
        )),
        Err(error)
            if !cfg!(feature = "rendering") && error.code() == ErrorCode::CapabilityMissing =>
        {
            Ok(None)
        }
        Err(error) => Err(Failure::Pdf(error)),
    }
}

/// The samples of a stencil mask whose dictionary is `dictionary`, 1 for masked whatever its `Decode`
/// (ISO 32000-2:2020, 8.9.6.2).
fn stencil(dictionary: &Object, mut samples: Vec<u8>) -> Result<Vec<u8>, Failure> {
    let Some(Object::Array(decode)) = entry(dictionary.clone(), "Decode") else {
        return Ok(samples);
    };
    let ends = decode
        .into_iter()
        .map(|item| number(Some(item)))
        .collect::<Result<Vec<String>, Failure>>()?;
    if ends != ["1", "0"] {
        return Ok(samples);
    }
    let width = number(entry(dictionary.clone(), "Width"))?
        .parse::<usize>()
        .map_err(|_err| Failure::Case(String::from("a mask without a whole Width")))?;
    for row in samples.chunks_mut(width.div_ceil(8).max(1)) {
        for column in 0..width {
            let bit = 0x80_u8.wrapping_shr(u32::try_from(column & 7).unwrap_or(0));
            if let Some(byte) = row.get_mut(column.wrapping_shr(3)) {
                *byte ^= bit;
            }
        }
    }
    Ok(samples)
}

/// The data of each stream in the file `saved`, decoded by the runner (ISO 32000-2:2020, 7.3.8).
fn streams(saved: &[u8]) -> Result<Vec<Vec<u8>>, Failure> {
    let mut found = Vec::new();
    let mut from = 0_usize;
    while let Some(at) = find(saved, b"stream", from) {
        let after = at.saturating_add(6);
        from = after;
        let head = saved.get(..at).unwrap_or_default();
        let trimmed = head.trim_ascii_end();
        if !trimmed.ends_with(b">>") {
            continue;
        }
        let start = match saved.get(after..after.saturating_add(2)) {
            Some(b"\r\n") => after.saturating_add(2),
            Some(&[b'\n', ..]) => after.saturating_add(1),
            Some(_) | None => continue,
        };
        let end = find(saved, b"endstream", start)
            .ok_or_else(|| Failure::Case(String::from("a stream without endstream")))?;
        from = end.saturating_add(9);
        let raw = saved.get(start..end).unwrap_or_default();
        let data = raw
            .strip_suffix(b"\r\n")
            .or_else(|| raw.strip_suffix(b"\n"))
            .or_else(|| raw.strip_suffix(b"\r"))
            .unwrap_or(raw);
        let opened = trimmed
            .windows(3)
            .rposition(|window| window == b"obj")
            .map_or(0, |last| last.saturating_add(3));
        let syntax =
            String::from_utf8_lossy(trimmed.get(opened..).unwrap_or_default()).into_owned();
        let words = tokens(&syntax);
        let refs: Vec<&str> = words.iter().map(String::as_str).collect();
        let dictionary = parse_object(&refs)
            .map(|pair| pair.0)
            .ok_or_else(|| Failure::Case(format!("cannot parse {syntax}")))?;
        found.push(unfiltered(
            &filter(entry(dictionary, "Filter"))?,
            data.to_vec(),
        )?);
    }
    Ok(found)
}

/// Every page's annotations and its marks.
fn survey(document: &Document) -> Result<(String, Vec<Vec<Area>>), Failure> {
    let mut shown = Vec::new();
    let mut marks = Vec::new();
    for index in 0..document.page_count()? {
        let (listed, areas) = listing(document, index)?;
        shown.push(listed);
        marks.push(areas);
    }
    Ok((shown.join("|"), marks))
}

/// `raw` decoded through `names`, the filters: none or `FlateDecode`.
fn unfiltered(names: &str, raw: Vec<u8>) -> Result<Vec<u8>, Failure> {
    match names {
        "none" => Ok(raw),
        "FlateDecode" => inflate(&raw).map_err(|why| Failure::Case(String::from(why))),
        other => Err(Failure::Case(format!("a stream filtered by {other}"))),
    }
}
