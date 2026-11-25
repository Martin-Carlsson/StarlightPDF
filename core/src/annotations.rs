//! Annotations.

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::str;
use std::sync::{Mutex, PoisonError};

use crate::input::Source;
use crate::objects::ObjectId;
use crate::output::Sink;
use crate::overlay::{Object, Overlay};
use crate::page::display::{Affine, SCALE, decimal, display, fixed, units};
use crate::store::{Objects, Store};
use crate::{ErrorCode, PdfError};

/// The operation adding errors name.
const ADD: &str = "Page::add_annotation";

/// Half a point in fixed point, for rounding.
const HALF: i128 = SCALE >> 1_u32;

/// Options for [`Page::add_annotation`](crate::page::Page::add_annotation).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct AddAnnotationOptions {
    /// The author.
    pub author: String,
    /// A [color](crate::drawing#colors); a redaction mark's interior colour: `#rgb`, `#rrggbb` or
    /// `device-cmyk()`, empty for none.
    pub color: String,
    /// The note or text.
    pub contents: String,
    /// The height.
    pub height: f64,
    /// Note icon or rubber stamp name, such as `Approved`.
    pub icon: String,
    /// The kind.
    pub kind: AnnotationKind,
    /// At a line's end point.
    pub line_ending: LineEnding,
    /// File name of attached data.
    pub name: String,
    /// The opacity; `None`: 1.
    pub opacity: Option<f64>,
    /// SVG path data for lines, ink, polygons and polylines.
    pub path: String,
    /// Add a popup.
    pub popup: bool,
    /// Markup: eight numbers per quadrilateral (ISO 32000-2:2020, 12.5.6.10), in [page coordinates](crate::page#coordinates).
    pub quads: Vec<f64>,
    /// For a link, a [target](crate::navigation#targets).
    pub target: String,
    /// The width.
    pub width: f64,
    /// The left edge.
    pub x: f64,
    /// The top edge.
    pub y: f64,
}

/// An annotation on a page; fails with [`ErrorCode::StaleHandle`] once its page no longer lists it.
#[derive(Clone, Debug)]
pub struct Annotation {
    /// Where the page's `Annots` lists it.
    key: Key,
    /// The document's changes.
    overlay: Arc<Overlay>,
    /// The page object.
    page: ObjectId,
    /// The document's shared state.
    store: Arc<Store>,
}

impl Annotation {
    /// Removes the annotation and its popup from the page.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn delete(&self, options: &DeleteAnnotationOptions) -> Result<(), PdfError> {
        let _: &DeleteAnnotationOptions = options;
        let objects = self.store.objects("Annotation::delete");
        let (mut page, mut annots, at) = self.listed(objects)?;
        let removed = self.overlay.direct(objects, annots.remove(at))?;
        if let Some(Object::Reference(popup)) = removed.get(b"Popup") {
            annots.retain(|item| !matches!(*item, Object::Reference(id) if id == popup));
        }
        page.set(b"Annots", Some(Object::Array(annots)));
        self.overlay.set(self.page, page);
        Ok(())
    }

    /// The annotation's kind, place, colour (a redaction mark's interior colour), quadrilaterals
    /// and opacity; the other properties are empty (ISO 32000-2:2020, 12.5.2, 12.5.6.23).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn info(&self) -> Result<AnnotationInfo, PdfError> {
        let objects = self.store.objects("Annotation::info");
        let overlay = &*self.overlay;
        let (page, annots, at) = self.listed(objects)?;
        let annotation =
            overlay.direct(objects, annots.into_iter().nth(at).unwrap_or(Object::Null))?;
        let matrix = display(objects, overlay, &page)?.matrix;
        let kind = if let Object::Name(subtype) = overlay.entry(objects, &annotation, b"Subtype")? {
            kind(&subtype)
        } else {
            AnnotationKind::Other
        };
        let read = |key: &[u8]| numbers(objects, overlay, annotation.get(key));
        let Some([left, bottom, right, top]) =
            read(b"Rect")?.and_then(|found| <[i128; 4]>::try_from(found).ok())
        else {
            return Err(objects.malformed());
        };
        let too_large = || PdfError::new(ErrorCode::LimitExceeded, objects.operation());
        let corners = [left, bottom, right, bottom, left, top, right, top];
        let [x0, y0, x1, y1] = bounds(&transformed(&matrix, &corners, true).ok_or_else(too_large)?);
        let quads = match read(b"QuadPoints")? {
            Some(found) if !found.is_empty() && found.len().is_multiple_of(8) => {
                transformed(&matrix, &found, true).ok_or_else(too_large)?
            }
            Some(_) | None => Vec::new(),
        };
        let colour = read(if kind == AnnotationKind::Redact {
            b"IC"
        } else {
            b"C"
        })?;
        let opacity = match overlay.entry(objects, &annotation, b"CA")? {
            number @ (Object::Integer(_) | Object::Real(_)) => units(&number).map_or(1.0_f64, real),
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Dictionary(_)
            | Object::Name(_)
            | Object::Null
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_) => 1.0_f64,
        };
        Ok(AnnotationInfo {
            author: String::new(),
            color: colour.map_or_default(|found| css(&found)),
            contents: String::new(),
            height: real(y1.saturating_sub(y0)),
            icon: String::new(),
            kind,
            line_ending: LineEnding::default(),
            modified: String::new(),
            name: String::new(),
            opacity,
            path: String::new(),
            popup: false,
            quads: quads.into_iter().map(real).collect(),
            target: String::new(),
            width: real(x1.saturating_sub(x0)),
            x: real(x0),
            y: real(y0),
        })
    }

    /// The page, its `Annots` and where they list this; [`ErrorCode::StaleHandle`] once they do not.
    fn listed(&self, objects: Objects<'_>) -> Result<(Object, Vec<Object>, usize), PdfError> {
        let page = page_dictionary(objects, &self.overlay, self.page)?;
        let annots = annots(objects, &self.overlay, &page)?;
        let at = match self.key {
            Key::Direct(ref annotation) => annots.iter().position(|item| same(item, annotation)),
            Key::Indirect(id) => annots
                .iter()
                .position(|item| matches!(*item, Object::Reference(found) if found == id)),
        };
        at.map(|found| (page, annots, found))
            .ok_or_else(|| PdfError::new(ErrorCode::StaleHandle, objects.operation()))
    }

    /// Sets the annotation's properties.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn set_info(&self, options: &SetAnnotationInfoOptions) -> Result<(), PdfError> {
        let _: &SetAnnotationInfoOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Annotation::set_info",
        ))
    }

    /// Writes the file, media, 3D model or rubber stamp image; nothing when there is none.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn write_to(&self, sink: &Sink, options: &WriteAnnotationOptions) -> Result<(), PdfError> {
        let _: &Sink = sink;
        let _: &WriteAnnotationOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Annotation::write_to",
        ))
    }
}

/// An annotation's properties.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct AnnotationInfo {
    /// The author.
    pub author: String,
    /// A [color](crate::drawing#colors).
    pub color: String,
    /// The note or text.
    pub contents: String,
    /// The height.
    pub height: f64,
    /// Note icon or rubber stamp name, such as `Approved`.
    pub icon: String,
    /// The kind.
    pub kind: AnnotationKind,
    /// At a line's end point.
    pub line_ending: LineEnding,
    /// When it last changed, a [date](crate#dates).
    pub modified: String,
    /// File name of attached data.
    pub name: String,
    /// The opacity.
    pub opacity: f64,
    /// SVG path data for lines, ink, polygons and polylines.
    pub path: String,
    /// Has a popup.
    pub popup: bool,
    /// Markup: eight numbers per quadrilateral (ISO 32000-2:2020, 12.5.6.10), in [page coordinates](crate::page#coordinates).
    pub quads: Vec<f64>,
    /// For a link, a [target](crate::navigation#targets).
    pub target: String,
    /// The width.
    pub width: f64,
    /// The left edge.
    pub x: f64,
    /// The top edge.
    pub y: f64,
}

/// The kind of an annotation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum AnnotationKind {
    /// An ellipse.
    Ellipse,
    /// An attached file.
    FileAttachment,
    /// Text on the page.
    FreeText,
    /// A highlight.
    Highlight,
    /// Freehand ink.
    Ink,
    /// A line.
    Line,
    /// A link.
    Link,
    /// Sound and video.
    Media,
    /// A 3D model.
    Model3d,
    /// A sticky note.
    #[default]
    Note,
    /// Any other subtype; read only.
    Other,
    /// A polygon.
    Polygon,
    /// A polyline.
    Polyline,
    /// A popup (ISO 32000-2:2020, 12.5.6.14); read only, added through [`AddAnnotationOptions::popup`].
    Popup,
    /// A rectangle.
    Rectangle,
    /// A redaction mark.
    Redact,
    /// A rubber stamp (ISO 32000-2:2020, 12.5.6.12).
    RubberStamp,
    /// A squiggly underline.
    Squiggly,
    /// A strikeout.
    Strikeout,
    /// An underline.
    Underline,
    /// A form field's; read only.
    Widget,
}

/// A page's annotations, as its `Annots` listed them when read.
#[derive(Clone, Debug)]
pub struct Annotations {
    /// Those not yet taken, shared by clones.
    rest: Arc<Mutex<VecDeque<Annotation>>>,
}

impl Annotations {
    /// The next annotation, or `None` after the last.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn next(&self) -> Result<Option<Annotation>, PdfError> {
        Ok(self
            .rest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front())
    }
}

/// Options for [`Annotation::delete`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct DeleteAnnotationOptions;

/// Options for [`Document::generate_appearances`](crate::document::Document::generate_appearances).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct GenerateAppearancesOptions {
    /// Only where none exists.
    pub missing_only: bool,
    /// `None`: all pages.
    pub pages: Option<Vec<u32>>,
}

/// Where a page's `Annots` lists an annotation.
#[derive(Clone, Debug)]
enum Key {
    /// Directly, as this dictionary.
    Direct(Object),
    /// By reference.
    Indirect(ObjectId),
}

/// How a line ends.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum LineEnding {
    /// A closed arrowhead.
    ClosedArrow,
    /// An open arrowhead.
    OpenArrow,
    /// No ending.
    #[default]
    Plain,
}

/// Options for [`Annotation::set_info`]; `None` leaves a property unchanged.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetAnnotationInfoOptions {
    /// The author.
    pub author: Option<String>,
    /// A [color](crate::drawing#colors).
    pub color: Option<String>,
    /// The note or text.
    pub contents: Option<String>,
    /// The height.
    pub height: Option<f64>,
    /// The opacity.
    pub opacity: Option<f64>,
    /// For a link, a [target](crate::navigation#targets).
    pub target: Option<String>,
    /// The width.
    pub width: Option<f64>,
    /// The left edge.
    pub x: Option<f64>,
    /// The top edge.
    pub y: Option<f64>,
}

/// Options for [`Annotation::write_to`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct WriteAnnotationOptions;

/// Adds to page `page` the annotation `options` describe: a redaction mark over `quads`, else
/// the rectangle, with `color` its interior colour (ISO 32000-2:2020, 12.5.6.23); `data` must be
/// `None`.
pub(crate) fn add(
    page: ObjectId,
    (store, overlay): (&Arc<Store>, &Arc<Overlay>),
    data: Option<&Source>,
    options: &AddAnnotationOptions,
) -> Result<Annotation, PdfError> {
    if options.kind != AnnotationKind::Redact {
        return Err(PdfError::new(ErrorCode::NotImplemented, ADD));
    }
    let invalid = || PdfError::new(ErrorCode::InvalidArgument, ADD);
    if data.is_some() {
        return Err(invalid());
    }
    let interior = if options.color.is_empty() {
        None
    } else {
        Some(colour(&options.color).ok_or_else(invalid)?)
    };
    let quads = options
        .quads
        .iter()
        .map(|value| fixed(*value))
        .collect::<Option<Vec<i128>>>()
        .ok_or_else(invalid)?;
    let given = if quads.is_empty() {
        let [Some(x), Some(y), Some(width), Some(height)] =
            [options.x, options.y, options.width, options.height].map(fixed)
        else {
            return Err(invalid());
        };
        if width <= 0 || height <= 0 {
            return Err(invalid());
        }
        let right = x.checked_add(width).ok_or_else(invalid)?;
        let bottom = y.checked_add(height).ok_or_else(invalid)?;
        vec![x, y, right, y, x, bottom, right, bottom]
    } else if quads.len().is_multiple_of(8) {
        quads
    } else {
        return Err(invalid());
    };
    let objects = store.objects(ADD);
    let mut dictionary = page_dictionary(objects, overlay, page)?;
    let matrix = display(objects, overlay, &dictionary)?.matrix;
    let user = transformed(&matrix, &given, false)
        .ok_or_else(|| PdfError::new(ErrorCode::LimitExceeded, ADD))?;
    let array =
        |values: &[i128]| Object::Array(values.iter().map(|value| number(*value)).collect());
    let mut entries = vec![
        (b"Type".to_vec(), Object::Name(b"Annot".to_vec())),
        (b"Subtype".to_vec(), Object::Name(b"Redact".to_vec())),
        (b"Rect".to_vec(), array(&bounds(&user))),
    ];
    if !options.quads.is_empty() {
        entries.push((b"QuadPoints".to_vec(), array(&user)));
    }
    if let Some(found) = interior {
        entries.push((b"IC".to_vec(), array(&found)));
    }
    entries.push((b"P".to_vec(), Object::Reference(page)));
    let mut annots = annots(objects, overlay, &dictionary)?;
    let id = overlay.add(Object::Dictionary(entries))?;
    annots.push(Object::Reference(id));
    dictionary.set(b"Annots", Some(Object::Array(annots)));
    overlay.set(page, dictionary);
    Ok(Annotation {
        key: Key::Indirect(id),
        overlay: Arc::clone(overlay),
        page,
        store: Arc::clone(store),
    })
}

/// The `Annots` of the page `dictionary`; none when missing (ISO 32000-2:2020, 7.7.3.3).
fn annots(
    objects: Objects<'_>,
    overlay: &Overlay,
    dictionary: &Object,
) -> Result<Vec<Object>, PdfError> {
    match overlay.entry(objects, dictionary, b"Annots")? {
        Object::Array(items) => Ok(items),
        Object::Null => Ok(Vec::new()),
        Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Integer(_)
        | Object::Name(_)
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => Err(objects.malformed()),
    }
}

/// The least x and y and the greatest of the x, y pairs `points`.
fn bounds(points: &[i128]) -> [i128; 4] {
    let mut found = [i128::MAX, i128::MAX, i128::MIN, i128::MIN];
    for pair in points.as_chunks::<2>().0 {
        let [x, y] = *pair;
        let [left, bottom, right, top] = found;
        found = [left.min(x), bottom.min(y), right.max(x), top.max(y)];
    }
    found
}

/// The components, in fixed point, of the colour `text`: `#rgb`, `#rrggbb` or `device-cmyk()`
/// (CSS Color Module Level 5, 5.2 and 15.2).
fn colour(text: &str) -> Option<Vec<i128>> {
    if let Some(digits) = text.strip_prefix('#') {
        let bytes = digits.as_bytes();
        if !bytes.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let pairs: Vec<[u8; 2]> = match bytes.len() {
            3 => bytes.iter().map(|&digit| [digit, digit]).collect(),
            6 => bytes.as_chunks::<2>().0.to_vec(),
            _ => return None,
        };
        return pairs
            .iter()
            .map(|pair| {
                let byte = u8::from_str_radix(str::from_utf8(pair).ok()?, 16).ok()?;
                i128::from(byte).checked_mul(SCALE)?.checked_div(255)
            })
            .collect();
    }
    let inner = text.strip_prefix("device-cmyk(")?.strip_suffix(')')?;
    let components = inner
        .split(|character: char| character == ',' || character.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (value, per) = part
                .strip_suffix('%')
                .map_or((part, 1), |found| (found, 100));
            Some(
                fixed(value.parse().ok()?)?
                    .checked_div(per)?
                    .clamp(0, SCALE),
            )
        })
        .collect::<Option<Vec<i128>>>()?;
    (components.len() == 4).then_some(components)
}

/// The colour of `components` in fixed point, gray, RGB or CMYK, as `#rrggbb` or
/// `device-cmyk()`; empty for any other count (ISO 32000-2:2020, 12.5.2).
fn css(components: &[i128]) -> String {
    let byte = |units: i128| {
        units
            .clamp(0, SCALE)
            .checked_mul(255)
            .and_then(|scaled| scaled.checked_add(HALF))
            .and_then(|scaled| scaled.checked_div(SCALE))
            .unwrap_or_default()
    };
    match *components {
        [gray] => {
            let level = byte(gray);
            format!("#{level:02x}{level:02x}{level:02x}")
        }
        [red, green, blue] => format!("#{:02x}{:02x}{:02x}", byte(red), byte(green), byte(blue)),
        [cyan, magenta, yellow, black] => {
            let [cyan_text, magenta_text, yellow_text, black_text] =
                [cyan, magenta, yellow, black].map(|value| decimal(value.clamp(0, SCALE)));
            format!("device-cmyk({cyan_text} {magenta_text} {yellow_text} {black_text})")
        }
        _ => String::new(),
    }
}

/// The kind of the annotation subtype `subtype` (ISO 32000-2:2020, 12.5.6.1).
const fn kind(subtype: &[u8]) -> AnnotationKind {
    match subtype {
        b"3D" => AnnotationKind::Model3d,
        b"Circle" => AnnotationKind::Ellipse,
        b"FileAttachment" => AnnotationKind::FileAttachment,
        b"FreeText" => AnnotationKind::FreeText,
        b"Highlight" => AnnotationKind::Highlight,
        b"Ink" => AnnotationKind::Ink,
        b"Line" => AnnotationKind::Line,
        b"Link" => AnnotationKind::Link,
        b"Movie" | b"RichMedia" | b"Screen" | b"Sound" => AnnotationKind::Media,
        b"PolyLine" => AnnotationKind::Polyline,
        b"Polygon" => AnnotationKind::Polygon,
        b"Popup" => AnnotationKind::Popup,
        b"Redact" => AnnotationKind::Redact,
        b"Square" => AnnotationKind::Rectangle,
        b"Squiggly" => AnnotationKind::Squiggly,
        b"Stamp" => AnnotationKind::RubberStamp,
        b"StrikeOut" => AnnotationKind::Strikeout,
        b"Text" => AnnotationKind::Note,
        b"Underline" => AnnotationKind::Underline,
        b"Widget" => AnnotationKind::Widget,
        _ => AnnotationKind::Other,
    }
}

/// The annotations page `page` lists in `Annots`, those not dictionaries left out
/// (ISO 32000-2:2020, 12.5.2).
pub(crate) fn list(
    page: ObjectId,
    (store, overlay): (&Arc<Store>, &Arc<Overlay>),
) -> Result<Annotations, PdfError> {
    let objects = store.objects("Page::annotations");
    let items = annots(objects, overlay, &page_dictionary(objects, overlay, page)?)?;
    let mut rest = VecDeque::new();
    for item in items {
        let key = match item {
            Object::Dictionary(_) => Key::Direct(item),
            Object::Reference(id)
                if matches!(overlay.resolve(objects, id)?, Object::Dictionary(_)) =>
            {
                Key::Indirect(id)
            }
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Integer(_)
            | Object::Name(_)
            | Object::Null
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_) => continue,
        };
        rest.push_back(Annotation {
            key,
            overlay: Arc::clone(overlay),
            page,
            store: Arc::clone(store),
        });
    }
    Ok(Annotations {
        rest: Arc::new(Mutex::new(rest)),
    })
}

/// `point` in displayed page coordinates mapped by `matrix` to default user space.
fn map(matrix: &Affine, point: [i128; 2]) -> Option<[i128; 2]> {
    let [ma, mb, mc, md, me, mf] = *matrix;
    let [x, y] = point;
    Some([
        product(ma, x)?
            .checked_add(product(mc, y)?)?
            .checked_add(me)?,
        product(mb, x)?
            .checked_add(product(md, y)?)?
            .checked_add(mf)?,
    ])
}

/// The fixed-point `units` as a PDF number: an integer when whole.
fn number(units: i128) -> Object {
    let text = decimal(units);
    text.parse().map_or_else(
        |_error| Object::Real(text.parse().unwrap_or_default()),
        Object::Integer,
    )
}

/// The numbers of the array `value` in fixed point; `None` when it is not an array of numbers.
fn numbers(
    objects: Objects<'_>,
    overlay: &Overlay,
    value: Option<Object>,
) -> Result<Option<Vec<i128>>, PdfError> {
    let Object::Array(items) = overlay.direct(objects, value.unwrap_or(Object::Null))? else {
        return Ok(None);
    };
    let mut found = Vec::with_capacity(items.len());
    for item in items {
        let Some(number) = units(&overlay.direct(objects, item)?) else {
            return Ok(None);
        };
        found.push(number);
    }
    Ok(Some(found))
}

/// The page object `page`, a dictionary.
fn page_dictionary(
    objects: Objects<'_>,
    overlay: &Overlay,
    page: ObjectId,
) -> Result<Object, PdfError> {
    let found = overlay.resolve(objects, page)?;
    if matches!(found, Object::Dictionary(_)) {
        Ok(found)
    } else {
        Err(objects.malformed())
    }
}

/// The fixed-point product of `left` and `right`.
fn product(left: i128, right: i128) -> Option<i128> {
    left.checked_mul(right)?.checked_div(SCALE)
}

/// Whether the direct objects `first` and `second` are equal; streams never are.
fn same(first: &Object, second: &Object) -> bool {
    match *first {
        Object::Array(ref left) => matches!(*second, Object::Array(ref right)
            if left.len() == right.len() && left.iter().zip(right).all(|(one, other)| same(one, other))),
        Object::Boolean(left) => matches!(*second, Object::Boolean(right) if left == right),
        Object::Dictionary(ref left) => matches!(*second, Object::Dictionary(ref right)
            if left.len() == right.len()
                && left.iter().zip(right).all(|(one, other)| one.0 == other.0 && same(&one.1, &other.1))),
        Object::Integer(left) => matches!(*second, Object::Integer(right) if left == right),
        Object::Name(ref left) => matches!(*second, Object::Name(ref right) if left == right),
        Object::Null => matches!(*second, Object::Null),
        Object::Real(left) => {
            matches!(*second, Object::Real(right) if left.to_bits() == right.to_bits())
        }
        Object::Reference(left) => matches!(*second, Object::Reference(right) if left == right),
        Object::Stream(..) => false,
        Object::String(ref left) => matches!(*second, Object::String(ref right) if left == right),
    }
}

/// The fixed-point `units` as a float, from their shortest decimal.
fn real(units: i128) -> f64 {
    decimal(units).parse().unwrap_or_default()
}

/// The x, y pairs `points` mapped by `matrix`, or with `inverse` by its inverse.
fn transformed(matrix: &Affine, points: &[i128], inverse: bool) -> Option<Vec<i128>> {
    let mut found = Vec::with_capacity(points.len());
    for pair in points.as_chunks::<2>().0 {
        found.extend(if inverse {
            unmap(matrix, *pair)?
        } else {
            map(matrix, *pair)?
        });
    }
    Some(found)
}

/// `point` in default user space mapped back by `matrix` to displayed page coordinates.
fn unmap(matrix: &Affine, point: [i128; 2]) -> Option<[i128; 2]> {
    let [ma, mb, mc, md, me, mf] = *matrix;
    let [x, y] = point;
    let (across, up) = (x.checked_sub(me)?, y.checked_sub(mf)?);
    let determinant = product(ma, md)?.checked_sub(product(mb, mc)?)?;
    let solve = |first: i128, second: i128| {
        first
            .checked_sub(second)?
            .checked_mul(SCALE)?
            .checked_div(determinant)
    };
    Some([
        solve(product(md, across)?, product(mc, up)?)?,
        solve(product(ma, up)?, product(mb, across)?)?,
    ])
}
