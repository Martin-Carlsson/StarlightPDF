//! Rendering and printing.

use crate::images::RasterFormat;
use crate::layers::Layer;

/// A device color space.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum ColorSpace {
    /// CMYK.
    Cmyk,
    /// Grayscale.
    Gray,
    /// RGB.
    #[default]
    Rgb,
}

/// How a printer uses both sides of the paper.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum Duplex {
    /// Both sides, flipped on the long edge.
    LongEdge,
    /// Both sides, flipped on the short edge.
    ShortEdge,
    /// One side.
    #[default]
    Simplex,
}

/// Options for [`Page::export_vector`](crate::page::Page::export_vector).
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct ExportVectorOptions {
    /// The format to write.
    pub format: VectorFormat,
}

/// Options for [`Document::print`](crate::document::Document::print).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct PrintOptions {
    /// `None`: 1.
    pub copies: Option<u32>,
    /// Sides of the paper.
    pub duplex: Duplex,
    /// Empty: all pages.
    pub pages: Vec<u32>,
    /// Empty: the default printer.
    pub printer: String,
    /// How pages fit the paper.
    pub scaling: PrintScaling,
    /// Empty: the printer's default.
    pub tray: String,
}

/// How pages fit the paper.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum PrintScaling {
    /// No scaling.
    ActualSize,
    /// Scale to fit.
    #[default]
    Fit,
    /// Shrink pages larger than the paper.
    Shrink,
}

/// Options for [`Page::render`](crate::page::Page::render).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct RenderOptions {
    /// The color space.
    pub color_space: ColorSpace,
    /// Fit within this many pixels per side, for thumbnails; overrides `resolution`.
    pub fit: Option<u32>,
    /// The image format.
    pub format: RasterFormat,
    /// Layers hidden in this rendering, whatever their default.
    pub hidden_layers: Vec<Layer>,
    /// Pixels per inch; `None`: 72.
    pub resolution: Option<u32>,
    /// What the rendering is for.
    pub usage: RenderUsage,
}

/// What a rendering is for.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum RenderUsage {
    /// On screen.
    #[default]
    Display,
    /// Annotations as printed.
    Print,
}

/// A vector format to export to.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum VectorFormat {
    /// PostScript.
    PostScript,
    /// SVG.
    #[default]
    Svg,
}
