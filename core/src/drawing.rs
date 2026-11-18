//! Drawing new content on pages.
//!
//! # Colors
//!
//! A color is CSS color syntax (CSS Color Module Level 5), `device-cmyk()` included, such as `#1e90ff`.
//!
//! # Paints
//!
//! A paint is a color, or a CSS `linear-gradient()` or `radial-gradient()` (CSS Images Module Level 3) over the
//! shape's bounding box.

use crate::images::ImageFormat;

/// Horizontal text alignment.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum Align {
    /// Centered.
    Center,
    /// At the end of the line.
    End,
    /// Stretched to both edges.
    Justify,
    /// At the start of the line.
    #[default]
    Start,
}

/// A barcode symbology.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum Barcode {
    /// Aztec Code.
    Aztec,
    /// Code 128.
    Code128,
    /// Code 39.
    Code39,
    /// Data Matrix.
    DataMatrix,
    /// EAN-13.
    Ean13,
    /// EAN-8.
    Ean8,
    /// Interleaved 2 of 5.
    Itf,
    /// PDF417.
    Pdf417,
    /// QR code.
    #[default]
    Qr,
    /// UPC-A.
    UpcA,
}

/// A blend mode (ISO 32000-2:2020, 11.3.5).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlendMode {
    /// Backdrop luminosity, source hue and saturation.
    Color,
    /// Darkens the backdrop to reflect the source.
    ColorBurn,
    /// Brightens the backdrop to reflect the source.
    ColorDodge,
    /// The darker of the two.
    Darken,
    /// The difference of the two.
    Difference,
    /// Like `Difference`, lower in contrast.
    Exclusion,
    /// Multiplies or screens, by the source.
    HardLight,
    /// Source hue, backdrop saturation and luminosity.
    Hue,
    /// The lighter of the two.
    Lighten,
    /// Source luminosity, backdrop hue and saturation.
    Luminosity,
    /// The product of the two.
    Multiply,
    /// The source alone.
    #[default]
    Normal,
    /// Multiplies or screens, by the backdrop.
    Overlay,
    /// Source saturation, backdrop hue and luminosity.
    Saturation,
    /// The inverted product of the inverses.
    Screen,
    /// Darkens or lightens, by the source.
    SoftLight,
}

/// A chart type.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum Chart {
    /// Filled lines.
    Area,
    /// Bars.
    #[default]
    Bar,
    /// Lines.
    Line,
    /// A pie of the first series.
    Pie,
    /// Points.
    Scatter,
}

/// Options for [`Page::draw_barcode`](crate::page::Page::draw_barcode).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct DrawBarcodeOptions {
    /// A [color](crate::drawing#colors); empty: black.
    pub color: String,
    /// What the code holds.
    pub data: String,
    /// Points.
    pub height: f64,
    /// The symbology.
    pub kind: Barcode,
    /// Points.
    pub width: f64,
    /// The left edge.
    pub x: f64,
    /// The top edge.
    pub y: f64,
}

/// Options for [`Page::draw_chart`](crate::page::Page::draw_chart).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct DrawChartOptions {
    /// Points.
    pub height: f64,
    /// The chart type.
    pub kind: Chart,
    /// One per category.
    pub labels: Vec<String>,
    /// Series names.
    pub series: Vec<String>,
    /// Empty: none.
    pub title: String,
    /// `labels.len()` values per series, series after series.
    pub values: Vec<f64>,
    /// Points.
    pub width: f64,
    /// The left edge.
    pub x: f64,
    /// The top edge.
    pub y: f64,
}

/// Options for [`Page::draw_image`](crate::page::Page::draw_image).
///
/// EXIF orientation applies, ICC profiles are kept, HDR and more than 8 bits per component convert to standard
/// range, and a Live Photo's still is its HEIC image.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct DrawImageOptions {
    /// `None`: detect.
    pub format: Option<ImageFormat>,
    /// Which image of a multi-image file, such as a HEIF sequence or multi-page TIFF.
    pub frame: u32,
    /// Points; `None`: from `width`, or the image's height at 72 DPI.
    pub height: Option<f64>,
    /// Ignore EXIF orientation.
    pub ignore_orientation: bool,
    /// `None`: 1.
    pub opacity: Option<f64>,
    /// Points; `None`: from `height`, or the image's width at 72 DPI.
    pub width: Option<f64>,
    /// The left edge.
    pub x: f64,
    /// The top edge.
    pub y: f64,
}

/// Options for [`Page::draw_path`](crate::page::Page::draw_path).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct DrawPathOptions {
    /// How it blends with the backdrop.
    pub blend_mode: BlendMode,
    /// SVG path data clipping the drawing; empty: none.
    pub clip: String,
    /// Dash pattern lengths; empty: solid.
    pub dash: Vec<f64>,
    /// A [paint](crate::drawing#paints); empty: none.
    pub fill: String,
    /// Which areas `fill` paints.
    pub fill_rule: FillRule,
    /// `None`: 1.
    pub opacity: Option<f64>,
    /// SVG path data (SVG 2, 9.3).
    pub path: String,
    /// SVG path data of one tile, painted with `fill` and repeated; empty: a plain fill.
    pub pattern: String,
    /// Tile size in points; `None`: 10.
    pub pattern_size: Option<f64>,
    /// A [paint](crate::drawing#paints); empty: none.
    pub stroke: String,
    /// Points; `None`: 1.
    pub stroke_width: Option<f64>,
}

/// Options for [`Page::draw_text`](crate::page::Page::draw_text).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct DrawTextOptions {
    /// Horizontal alignment.
    pub align: Align,
    /// A [color](crate::drawing#colors); empty: black.
    pub color: String,
    /// Writing direction.
    pub direction: TextDirection,
    /// [Font names](crate::fonts#font-names) tried in order for glyphs `font` lacks.
    pub fallback: Vec<String>,
    /// A [font name](crate::fonts#font-names); empty: Helvetica.
    pub font: String,
    /// Points; `None`: 12.
    pub font_size: Option<f64>,
    /// Points; `None`: 1.2 × `font_size`.
    pub line_height: Option<f64>,
    /// `None`: 1.
    pub opacity: Option<f64>,
    /// The text.
    pub text: String,
    /// Wrap within this width; `None`: no wrapping.
    pub width: Option<f64>,
    /// The text box's left edge.
    pub x: f64,
    /// The text box's top edge.
    pub y: f64,
}

/// Which areas a fill paints (ISO 32000-2:2020, 8.5.3.3).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum FillRule {
    /// Even-odd.
    EvenOdd,
    /// Nonzero winding number.
    #[default]
    NonZero,
}

/// Writing direction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum TextDirection {
    /// From the text (Unicode Bidirectional Algorithm).
    #[default]
    Auto,
    /// Left to right.
    LeftToRight,
    /// Right to left.
    RightToLeft,
    /// Top to bottom, as in CJK.
    Vertical,
}
