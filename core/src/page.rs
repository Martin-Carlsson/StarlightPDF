//! Pages.
//!
//! # Coordinates
//!
//! Positions and sizes are in points on the page as displayed, origin top-left, y down;
//! [`Page::user_space_matrix`] maps them to user space. Page boundaries are in default user space
//! (ISO 32000-2:2020, 14.11.2).

use alloc::sync::Arc;

use crate::annotations::{AddAnnotationOptions, Annotation, Annotations};
use crate::drawing::{
    DrawBarcodeOptions, DrawChartOptions, DrawImageOptions, DrawPathOptions, DrawTextOptions,
};
use crate::extract::{Blocks, BlocksOptions, Tables, TablesOptions, TextOptions, TextSpans};
use crate::forms::{AddFieldOptions, Field};
use crate::images::Images;
use crate::input::Source;
use crate::layers::Layer;
use crate::objects::ObjectId;
use crate::output::Sink;
use crate::progress::Progress;
use crate::rendering::{ExportVectorOptions, RenderOptions};
use crate::shapes::Shapes;
use crate::{ErrorCode, PdfError};

/// A page boundary (ISO 32000-2:2020, 14.11.2).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum Boundary {
    /// The art box: the meaningful content.
    Art,
    /// The bleed box: the clip region in production.
    Bleed,
    /// The crop box: the region displayed or printed.
    #[default]
    Crop,
    /// The media box: the physical medium.
    Media,
    /// The trim box: the finished page after trimming.
    Trim,
}

/// Options for [`Page::boundary`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct BoundaryOptions {
    /// Which boundary.
    pub kind: Boundary,
}

/// Options for [`Page::delete`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct DeletePageOptions;

/// An affine transform: x' = m11·x + m21·y + dx, y' = m12·x + m22·y + dy.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Matrix {
    /// Horizontal translation.
    pub dx: f64,
    /// Vertical translation.
    pub dy: f64,
    /// Scale of x into x'.
    pub m11: f64,
    /// Share of x in y'.
    pub m12: f64,
    /// Share of y in x'.
    pub m21: f64,
    /// Scale of y into y'.
    pub m22: f64,
}

/// A page; it follows the page when others move, and fails with [`ErrorCode::StaleHandle`] once deleted.
#[derive(Clone, Debug)]
pub struct Page {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Page {
    /// Adds an annotation; `data` is the file, media, 3D model or rubber stamp image.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`], [`ErrorCode::StaleHandle`], [`ErrorCode::Unsupported`].
    pub fn add_annotation(
        &self,
        data: Option<&Source>,
        options: &AddAnnotationOptions,
    ) -> Result<Annotation, PdfError> {
        let _: Option<&Source> = data;
        let _: &AddAnnotationOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Page::add_annotation",
        ))
    }

    /// Adds a form field with a widget on this page.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn add_field(&self, options: &AddFieldOptions) -> Result<Field, PdfError> {
        let _: &AddFieldOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::add_field"))
    }

    /// The annotations, links included.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn annotations(&self) -> Result<Annotations, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Page::annotations",
        ))
    }

    /// Words, lines, paragraphs or columns, with headings.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn blocks(&self, options: &BlocksOptions) -> Result<Blocks, PdfError> {
        let _: &BlocksOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::blocks"))
    }

    /// A page boundary, in default user space.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn boundary(&self, options: &BoundaryOptions) -> Result<Rect, PdfError> {
        let _: &BoundaryOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::boundary"))
    }

    /// Deletes the page.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn delete(&self, options: &DeletePageOptions) -> Result<(), PdfError> {
        let _: &DeletePageOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::delete"))
    }

    /// Draws a barcode or QR code.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn draw_barcode(
        &self,
        layer: Option<&Layer>,
        options: &DrawBarcodeOptions,
    ) -> Result<(), PdfError> {
        let _: Option<&Layer> = layer;
        let _: &DrawBarcodeOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Page::draw_barcode",
        ))
    }

    /// Draws a chart.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn draw_chart(
        &self,
        layer: Option<&Layer>,
        options: &DrawChartOptions,
    ) -> Result<(), PdfError> {
        let _: Option<&Layer> = layer;
        let _: &DrawChartOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::draw_chart"))
    }

    /// Draws an image; `mask` is a soft mask.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`], [`ErrorCode::StaleHandle`], [`ErrorCode::Unsupported`].
    pub fn draw_image(
        &self,
        image: &Source,
        mask: Option<&Source>,
        layer: Option<&Layer>,
        options: &DrawImageOptions,
    ) -> Result<(), PdfError> {
        let _: &Source = image;
        let _: Option<&Source> = mask;
        let _: Option<&Layer> = layer;
        let _: &DrawImageOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::draw_image"))
    }

    /// Draws lines, shapes and curves.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn draw_path(
        &self,
        layer: Option<&Layer>,
        options: &DrawPathOptions,
    ) -> Result<(), PdfError> {
        let _: Option<&Layer> = layer;
        let _: &DrawPathOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::draw_path"))
    }

    /// Draws text.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn draw_text(
        &self,
        layer: Option<&Layer>,
        options: &DrawTextOptions,
    ) -> Result<(), PdfError> {
        let _: Option<&Layer> = layer;
        let _: &DrawTextOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::draw_text"))
    }

    /// Writes the page as SVG or PostScript.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Cancelled`], [`ErrorCode::CapabilityMissing`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn export_vector(
        &self,
        sink: &Sink,
        progress: Option<&dyn Progress>,
        options: &ExportVectorOptions,
    ) -> Result<(), PdfError> {
        let _: &Sink = sink;
        let _: Option<&dyn Progress> = progress;
        let _: &ExportVectorOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Page::export_vector",
        ))
    }

    /// The page's object.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn id(&self) -> Result<ObjectId, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::id"))
    }

    /// The images drawn on the page.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn images(&self) -> Result<Images, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::images"))
    }

    /// The page's position.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn index(&self) -> Result<u32, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::index"))
    }

    /// The page label a viewer shows.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn label(&self) -> Result<String, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::label"))
    }

    /// Rasterizes the page with its annotations and fields.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Cancelled`], [`ErrorCode::CapabilityMissing`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn render(
        &self,
        sink: &Sink,
        progress: Option<&dyn Progress>,
        options: &RenderOptions,
    ) -> Result<(), PdfError> {
        let _: &Sink = sink;
        let _: Option<&dyn Progress> = progress;
        let _: &RenderOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::render"))
    }

    /// Rotates clockwise by `degrees`.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn rotate_by(&self, options: &RotateByOptions) -> Result<(), PdfError> {
        let _: &RotateByOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::rotate_by"))
    }

    /// The rotation in degrees clockwise.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn rotation(&self) -> Result<u16, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::rotation"))
    }

    /// Scales the page and its content.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn scale_by(&self, options: &ScaleByOptions) -> Result<(), PdfError> {
        let _: &ScaleByOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::scale_by"))
    }

    /// Sets a page boundary (crop, resize), in default user space.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn set_boundary(&self, options: &SetBoundaryOptions) -> Result<(), PdfError> {
        let _: &SetBoundaryOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Page::set_boundary",
        ))
    }

    /// Moves the page to `index`.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn set_index(&self, options: &SetIndexOptions) -> Result<(), PdfError> {
        let _: &SetIndexOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::set_index"))
    }

    /// Sets the rotation.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn set_rotation(&self, options: &SetRotationOptions) -> Result<(), PdfError> {
        let _: &SetRotationOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Page::set_rotation",
        ))
    }

    /// Sets the transition shown when the page opens.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn set_transition(&self, options: &SetTransitionOptions) -> Result<(), PdfError> {
        let _: &SetTransitionOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Page::set_transition",
        ))
    }

    /// The vector graphics drawn on the page.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn shapes(&self) -> Result<Shapes, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::shapes"))
    }

    /// Places `other` over or under this page.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::LossRefused`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn superimpose(&self, other: &Self, options: &SuperimposeOptions) -> Result<(), PdfError> {
        let _: &Self = other;
        let _: &SuperimposeOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Page::superimpose",
        ))
    }

    /// The tables found, as rows and columns.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn tables(&self, options: &TablesOptions) -> Result<Tables, PdfError> {
        let _: &TablesOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::tables"))
    }

    /// The text, with positions, fonts and colors.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn text(&self, options: &TextOptions) -> Result<TextSpans, PdfError> {
        let _: &TextOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::text"))
    }

    /// The transition shown when the page opens.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn transition(&self) -> Result<Transition, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::transition"))
    }

    /// Maps displayed coordinates to user space, with rotation, crop offset and `UserUnit`.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn user_space_matrix(&self) -> Result<Matrix, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Page::user_space_matrix",
        ))
    }
}

/// A rectangle in default user space, y up.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Rect {
    /// The height.
    pub height: f64,
    /// The width.
    pub width: f64,
    /// The left edge.
    pub x: f64,
    /// The bottom edge.
    pub y: f64,
}

/// Options for [`Page::rotate_by`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct RotateByOptions {
    /// Clockwise; a multiple of 90.
    pub degrees: i32,
}

/// Options for [`Page::scale_by`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct ScaleByOptions {
    /// Greater than 0.
    pub scale: f64,
}

/// Options for [`Page::set_boundary`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct SetBoundaryOptions {
    /// The height.
    pub height: f64,
    /// Which boundary.
    pub kind: Boundary,
    /// The width.
    pub width: f64,
    /// The left edge.
    pub x: f64,
    /// The bottom edge.
    pub y: f64,
}

/// Options for [`Page::set_index`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct SetIndexOptions {
    /// The new position.
    pub index: u32,
}

/// Options for [`Page::set_rotation`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct SetRotationOptions {
    /// Clockwise: 0, 90, 180 or 270.
    pub degrees: u16,
}

/// Options for [`Page::set_transition`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct SetTransitionOptions {
    /// Seconds; `None`: 1.
    pub duration: Option<f64>,
    /// The effect.
    pub style: TransitionStyle,
}

/// Options for [`Page::superimpose`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct SuperimposeOptions {
    /// Let `other`'s annotations and fields drop.
    pub allow_loss: bool,
    /// Scale of `other`; `None`: 1.
    pub scale: Option<f64>,
    /// Place `other` beneath this page's content.
    pub under: bool,
    /// Where `other`'s left edge goes.
    pub x: f64,
    /// Where `other`'s top edge goes.
    pub y: f64,
}

/// A page transition.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Transition {
    /// Seconds.
    pub duration: f64,
    /// The effect.
    pub style: TransitionStyle,
}

/// A transition effect (ISO 32000-2:2020, 12.4.4.1).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum TransitionStyle {
    /// Lines sweep across the page.
    Blinds,
    /// A rectangle grows or shrinks.
    Box,
    /// The new page slides over the old.
    Cover,
    /// The old page dissolves into the new.
    Dissolve,
    /// The new page fades in.
    Fade,
    /// Changes fly in or out.
    Fly,
    /// Squares sweep across the page.
    Glitter,
    /// The new page pushes the old away.
    Push,
    /// The new page replaces the old at once.
    #[default]
    Replace,
    /// Lines split across the page.
    Split,
    /// The old page slides away.
    Uncover,
    /// A line sweeps across the page.
    Wipe,
}
