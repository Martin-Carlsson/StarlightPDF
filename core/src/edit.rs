//! Editing existing content.

/// Where on the page a stamp sits.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum Anchor {
    /// Bottom center.
    Bottom,
    /// Bottom left.
    BottomLeft,
    /// Bottom right.
    BottomRight,
    /// Center.
    #[default]
    Center,
    /// Left center.
    Left,
    /// Right center.
    Right,
    /// Top center.
    Top,
    /// Top left.
    TopLeft,
    /// Top right.
    TopRight,
}

/// Options for [`Document::apply_redactions`](crate::document::Document::apply_redactions).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ApplyRedactionsOptions {
    /// `None`: all pages.
    pub pages: Option<Vec<u32>>,
}

/// Options for [`Document::flatten`](crate::document::Document::flatten).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FlattenOptions {
    /// `None`: all pages.
    pub pages: Option<Vec<u32>>,
    /// What to flatten; `None`: all four.
    pub parts: Option<Vec<Flattening>>,
}

/// What to merge into the page content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Flattening {
    /// Annotations.
    Annotations,
    /// Form fields.
    Fields,
    /// Layers.
    Layers,
    /// Transparency.
    Transparency,
}

/// Options for [`Document::replace_text`](crate::document::Document::replace_text).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ReplaceTextOptions {
    /// Match case.
    pub case_sensitive: bool,
    /// `None`: all pages.
    pub pages: Option<Vec<u32>>,
    /// The text to find.
    pub query: String,
    /// `query` is a regular expression.
    pub regex: bool,
    /// The new text.
    pub replacement: String,
}

/// Options for [`Document::stamp`](crate::document::Document::stamp).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct StampOptions {
    /// Where on the page.
    pub anchor: Anchor,
    /// First Bates number; `{bates}` in `text` becomes it, counting up.
    pub bates_start: Option<u64>,
    /// A [color](crate::drawing#colors); empty: black.
    pub color: String,
    /// Clockwise.
    pub degrees: f64,
    /// A [font name](crate::fonts#font-names); empty: Helvetica.
    pub font: String,
    /// Font size in points; `None`: 12.
    pub font_size: Option<f64>,
    /// Image height in points; `None`: from `width`, or the image's height at 72 DPI.
    pub height: Option<f64>,
    /// Distance from the page edge, in points.
    pub margin: f64,
    /// From 0 to 1; `None`: 1.
    pub opacity: Option<f64>,
    /// `None`: all pages.
    pub pages: Option<Vec<u32>>,
    /// `{page}` and `{pages}` become numbers.
    pub text: String,
    /// Beneath the page content.
    pub under: bool,
    /// Image width in points; `None`: from `height`, or the image's width at 72 DPI.
    pub width: Option<f64>,
}
