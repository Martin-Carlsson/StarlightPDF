//! Annotations.

use alloc::sync::Arc;

use crate::output::Sink;
use crate::{ErrorCode, PdfError};

/// Options for [`Page::add_annotation`](crate::page::Page::add_annotation).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct AddAnnotationOptions {
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

/// An annotation on a page.
#[derive(Clone, Debug)]
pub struct Annotation {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Annotation {
    /// Removes the annotation.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn delete(&self, options: &DeleteAnnotationOptions) -> Result<(), PdfError> {
        let _: &DeleteAnnotationOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Annotation::delete",
        ))
    }

    /// The annotation's properties.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn info(&self) -> Result<AnnotationInfo, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Annotation::info"))
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

/// A page's annotations.
#[derive(Clone, Debug)]
pub struct Annotations {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Annotations {
    /// The next annotation, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn next(&self) -> Result<Option<Annotation>, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Annotations::next",
        ))
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
