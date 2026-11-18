//! Vector graphics on pages.

use alloc::sync::Arc;

use crate::drawing::{BlendMode, FillRule};
use crate::layers::Layer;
use crate::{ErrorCode, PdfError};

/// Options for [`Shape::delete`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct DeleteShapeOptions;

/// Options for [`Shape::set_paint`]; `None` leaves a property unchanged.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SetPaintOptions {
    /// A [paint](crate::drawing#paints); empty: none.
    pub fill: Option<String>,
    /// A [paint](crate::drawing#paints); empty: none.
    pub stroke: Option<String>,
    /// Line width in points.
    pub stroke_width: Option<f64>,
}

/// A vector object on a page.
#[derive(Clone, Debug)]
pub struct Shape {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Shape {
    /// Removes the shape.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn delete(&self, options: &DeleteShapeOptions) -> Result<(), PdfError> {
        let _: &DeleteShapeOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Shape::delete"))
    }

    /// The shape's properties.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn info(&self) -> Result<ShapeInfo, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Shape::info"))
    }

    /// The layer it is drawn on, or `None`.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn layer(&self) -> Result<Option<Layer>, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Shape::layer"))
    }

    /// Sets fill and stroke.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn set_paint(&self, options: &SetPaintOptions) -> Result<(), PdfError> {
        let _: &SetPaintOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Shape::set_paint"))
    }

    /// Moves, rotates or scales the shape.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn transform_by(&self, options: &TransformByOptions) -> Result<(), PdfError> {
        let _: &TransformByOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Shape::transform_by",
        ))
    }
}

/// A vector object's properties.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ShapeInfo {
    /// How it blends with the backdrop.
    pub blend_mode: BlendMode,
    /// SVG path data clipping it, displayed coordinates; empty: none.
    pub clip: String,
    /// Dash pattern lengths; empty: solid.
    pub dash: Vec<f64>,
    /// A [paint](crate::drawing#paints); empty: none.
    pub fill: String,
    /// Which areas `fill` paints.
    pub fill_rule: FillRule,
    /// The height.
    pub height: f64,
    /// The opacity.
    pub opacity: f64,
    /// SVG path data, displayed coordinates.
    pub path: String,
    /// SVG path data of one tile, painted with `fill` and repeated; empty: a plain fill.
    pub pattern: String,
    /// Tile size in points; 0 without `pattern`.
    pub pattern_size: f64,
    /// A [paint](crate::drawing#paints); empty: none.
    pub stroke: String,
    /// Line width in points.
    pub stroke_width: f64,
    /// The width.
    pub width: f64,
    /// The left edge.
    pub x: f64,
    /// The top edge.
    pub y: f64,
}

/// The vector objects on a page, one at a time.
#[derive(Clone, Debug)]
pub struct Shapes {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Shapes {
    /// The next shape, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn next(&self) -> Result<Option<Shape>, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Shapes::next"))
    }
}

/// Options for [`Shape::transform_by`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct TransformByOptions {
    /// Clockwise about the shape's center.
    pub degrees: f64,
    /// Points to move right.
    pub dx: f64,
    /// Points to move down.
    pub dy: f64,
    /// `None`: 1.
    pub scale: Option<f64>,
}
