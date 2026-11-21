//! Pages.
//!
//! # Coordinates
//!
//! Positions and sizes are in points on the page as displayed, origin top-left, y down;
//! [`Page::user_space_matrix`] maps them to user space. Page boundaries are in default user space
//! (ISO 32000-2:2020, 14.11.2).

mod image;

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
use crate::overlay::{Object, Overlay};
use crate::progress::Progress;
use crate::rendering::{ExportVectorOptions, RenderOptions};
use crate::shapes::Shapes;
use crate::store::{Objects, Store};
use crate::syntax::{At, Token, Value};
use crate::{ErrorCode, PdfError};

/// The media box of a page without one: US Letter, as viewers show it.
const LETTER: [f64; 4] = [0.0, 0.0, 612.0, 792.0];

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
    /// The page object.
    id: ObjectId,
    /// The document's changes.
    overlay: Arc<Overlay>,
    /// The document's shared state.
    store: Arc<Store>,
}

impl Page {
    /// Adds an annotation; `data` is the file, media, 3D model or rubber stamp image.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`], [`ErrorCode::StaleHandle`], [`ErrorCode::Unsupported`].
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
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
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

    /// A page boundary, in default user space: `MediaBox` and `CropBox` inherited, `CropBox`
    /// defaulting to the media box and the others to the crop box, each with corners normalized
    /// and clipped to the media box (ISO 32000-2:2020, 7.7.3.4, 7.9.5 and 14.11.2). A missing
    /// media box is US Letter, with a warning unless strict.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`], [`ErrorCode::Unsupported`].
    pub fn boundary(&self, options: &BoundaryOptions) -> Result<Rect, PdfError> {
        let objects = self.store.objects("Page::boundary");
        let media = if let Some(found) = self.rectangle(objects, b"MediaBox", true)? {
            found
        } else {
            objects.deviation("7.7.3.3", "MediaBox missing", None)?;
            LETTER
        };
        let crop = self
            .rectangle(objects, b"CropBox", true)?
            .map_or(media, |found| clip(found, media));
        let key: &[u8] = match options.kind {
            Boundary::Art => b"ArtBox",
            Boundary::Bleed => b"BleedBox",
            Boundary::Crop => return Ok(rect(crop)),
            Boundary::Media => return Ok(rect(media)),
            Boundary::Trim => b"TrimBox",
        };
        Ok(rect(
            self.rectangle(objects, key, false)?
                .map_or(crop, |found| clip(found, media)),
        ))
    }

    /// Deletes the page.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
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
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
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
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn draw_chart(
        &self,
        layer: Option<&Layer>,
        options: &DrawChartOptions,
    ) -> Result<(), PdfError> {
        let _: Option<&Layer> = layer;
        let _: &DrawChartOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::draw_chart"))
    }

    /// Draws a JPEG or PNG image with its top-left corner at `x`, `y`, at `width` × `height`
    /// points, one given keeping the ratio, none at 72 DPI, as its Exif orientation shows it
    /// unless ignored; the file is read again at save (ISO 32000-2:2020, 8.9). `mask`, `layer`
    /// and `opacity` are not implemented; a `frame` other than 0 is invalid.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`], [`ErrorCode::StaleHandle`], [`ErrorCode::Unsupported`].
    pub fn draw_image(
        &self,
        image: &Source,
        mask: Option<&Source>,
        layer: Option<&Layer>,
        options: &DrawImageOptions,
    ) -> Result<(), PdfError> {
        if mask.is_some() || layer.is_some() || options.opacity.is_some() {
            return Err(PdfError::new(ErrorCode::NotImplemented, "Page::draw_image"));
        }
        image::draw(self, image, options)
    }

    /// Draws lines, shapes and curves.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
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
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
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

    /// The page object `id` of the document of `store` and `overlay`.
    pub(crate) fn new(id: ObjectId, store: &Arc<Store>, overlay: &Arc<Overlay>) -> Self {
        Self {
            id,
            overlay: Arc::clone(overlay),
            store: Arc::clone(store),
        }
    }

    /// The rectangle `key` of the page, `inherit`ed or its own, normalized; through the overlay
    /// when it holds changes (ISO 32000-2:2020, 7.7.3.4, 7.9.5).
    fn rectangle(
        &self,
        objects: Objects<'_>,
        key: &[u8],
        inherit: bool,
    ) -> Result<Option<[f64; 4]>, PdfError> {
        if self.overlay.is_changed() {
            return held(objects, &self.overlay, self.id, key, inherit)?
                .map(|found| held_rectangle(objects, &self.overlay, found))
                .transpose();
        }
        let found = if inherit {
            inherited(objects, self.id, key)?
        } else {
            own(
                objects,
                objects.dictionary(Some(Value::Reference(self.id)))?,
                key,
            )?
        };
        found.map(|value| rectangle(objects, &value)).transpose()
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
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn rotate_by(&self, options: &RotateByOptions) -> Result<(), PdfError> {
        let _: &RotateByOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Page::rotate_by"))
    }

    /// The rotation in degrees clockwise: `Rotate`, inherited, as 0, 90, 180 or 270
    /// (ISO 32000-2:2020, 7.7.3.3); one not a multiple of 90 is 0, with a warning unless strict.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`], [`ErrorCode::Unsupported`].
    pub fn rotation(&self) -> Result<u16, PdfError> {
        let objects = self.store.objects("Page::rotation");
        let rotate = if self.overlay.is_changed() {
            held(objects, &self.overlay, self.id, b"Rotate", true)?.map(|found| match found {
                Object::Integer(value) => Some(value),
                Object::Array(_)
                | Object::Boolean(_)
                | Object::Dictionary(_)
                | Object::Name(_)
                | Object::Null
                | Object::Real(_)
                | Object::Reference(_)
                | Object::Stream(..)
                | Object::String(_) => None,
            })
        } else {
            inherited(objects, self.id, b"Rotate")?.map(|found| found.integer())
        };
        let degrees = rotate.map_or(Some(0), |found| {
            found
                .filter(|value| value.checked_rem(90) == Some(0))?
                .checked_rem_euclid(360)
        });
        if let Some(found) = degrees.and_then(|value| u16::try_from(value).ok()) {
            return Ok(found);
        }
        objects.deviation("7.7.3.3", "Rotate not a multiple of 90", None)?;
        Ok(0)
    }

    /// Scales the page and its content.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
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
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
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
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
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
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
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
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
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
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::LossRefused`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
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

/// `rectangle`, normalized, reduced to its intersection with `media` (ISO 32000-2:2020, 14.11.2).
const fn clip(rectangle: [f64; 4], media: [f64; 4]) -> [f64; 4] {
    let [left, bottom, right, top] = rectangle;
    let [media_left, media_bottom, media_right, media_top] = media;
    let low_x = left.max(media_left);
    let low_y = bottom.max(media_bottom);
    [
        low_x,
        low_y,
        right.min(media_right).max(low_x),
        top.min(media_top).max(low_y),
    ]
}

/// `left` less `right`, both `width` digits, most significant first, modulo 10 to the `width`.
fn difference(left: &[u8], right: &[u8]) -> Vec<u8> {
    let mut borrow = 0_u8;
    let mut digits: Vec<u8> = left
        .iter()
        .rev()
        .zip(right.iter().rev())
        .map(|(&minuend, &subtrahend)| {
            let taken = subtrahend.saturating_add(borrow);
            borrow = u8::from(minuend < taken);
            minuend
                .saturating_add(borrow.saturating_mul(10))
                .saturating_sub(taken)
        })
        .collect();
    digits.reverse();
    digits
}

/// The page at `index` of the page tree, found by descending through each node's `Count`; past
/// the last page is [`ErrorCode::InvalidArgument`] (ISO 32000-2:2020, 7.7.3).
pub(crate) fn find(
    store: &Arc<Store>,
    overlay: &Arc<Overlay>,
    index: u32,
) -> Result<Page, PdfError> {
    let objects = store.objects("Document::page");
    if overlay.is_changed() {
        return find_held(objects, (store, overlay), index);
    }
    let catalog = objects.dictionary(objects.get(objects.trailer()?, b"Root")?)?;
    let root = objects.get(catalog, b"Pages")?;
    // The nodes descended through, by id; the root, when direct, has none.
    let mut path: Vec<ObjectId> = root
        .as_ref()
        .and_then(Value::reference)
        .into_iter()
        .collect();
    let mut dictionary = objects.dictionary(root)?;
    let mut depth: u32 = 0;
    let mut left = u64::from(index);
    loop {
        if depth >= objects.depth_limit() {
            return Err(PdfError::new(ErrorCode::LimitExceeded, objects.operation()));
        }
        depth = depth.saturating_add(1);
        let kids = objects
            .get(dictionary, b"Kids")?
            .as_ref()
            .and_then(Value::array)
            .ok_or_else(|| objects.malformed())?;
        let mut reader = objects.reader(kids)?;
        drop(reader.token()?);
        loop {
            let kid = match reader.item()? {
                Token::Reference(kid) => kid,
                Token::ArrayEnd if depth == 1 => {
                    return Err(PdfError::new(
                        ErrorCode::InvalidArgument,
                        objects.operation(),
                    ));
                }
                Token::ArrayEnd
                | Token::ArrayStart
                | Token::DictionaryEnd
                | Token::DictionaryStart
                | Token::Integer(_)
                | Token::Keyword(_)
                | Token::Name(_)
                | Token::Real(_)
                | Token::String(_) => return Err(objects.malformed()),
            };
            let child = objects.dictionary(Some(Value::Reference(kid)))?;
            let pages = if objects.get(child, b"Kids")?.is_some() {
                match own(objects, child, b"Count")? {
                    Some(Value::Integer(count)) => u64::try_from(count).ok(),
                    Some(_) | None => None,
                }
                .ok_or_else(|| objects.malformed())?
            } else if left == 0 {
                return Ok(Page::new(kid, store, overlay));
            } else {
                1
            };
            if left < pages {
                if path.contains(&kid) {
                    return Err(objects.malformed());
                }
                path.push(kid);
                dictionary = child;
                break;
            }
            left = left.saturating_sub(pages);
        }
    }
}

/// The digits of `text`, a shortest decimal, with `scale` fraction digits, as `width` digits
/// modulo 10 to the `width`, most significant first.
fn fixed(text: &str, scale: usize, width: usize) -> Vec<u8> {
    let magnitude = text.trim_start_matches('-');
    let (whole, fraction) = magnitude.split_once('.').unwrap_or((magnitude, ""));
    let mut digits = vec![0; width.saturating_sub(whole.len()).saturating_sub(scale)];
    digits.extend(
        whole
            .bytes()
            .chain(fraction.bytes())
            .map(|byte| byte.saturating_sub(b'0')),
    );
    digits.resize(width, 0);
    if text.starts_with('-') {
        difference(&vec![0; width], &digits)
    } else {
        digits
    }
}

/// The pages under the page tree node `node`: its `Count`, 1 for a page (ISO 32000-2:2020, 7.7.3).
pub(crate) fn count(
    objects: Objects<'_>,
    overlay: &Overlay,
    node: &Object,
) -> Result<u64, PdfError> {
    if node.get(b"Kids").is_none() {
        return Ok(1);
    }
    match overlay.entry(objects, node, b"Count")? {
        Object::Integer(found) => u64::try_from(found).map_err(|_error| objects.malformed()),
        Object::Array(_)
        | Object::Boolean(_)
        | Object::Dictionary(_)
        | Object::Name(_)
        | Object::Null
        | Object::Real(_)
        | Object::Reference(_)
        | Object::Stream(..)
        | Object::String(_) => Err(objects.malformed()),
    }
}

/// [`find`] through `overlay`.
fn find_held(
    objects: Objects<'_>,
    (store, overlay): (&Arc<Store>, &Arc<Overlay>),
    index: u32,
) -> Result<Page, PdfError> {
    let trailer = Object::Dictionary(overlay.trailer(objects)?);
    let catalog = overlay.entry(objects, &trailer, b"Root")?;
    let mut path: Vec<u32> = match catalog.get(b"Pages") {
        Some(Object::Reference(id)) => vec![id.number],
        Some(_) | None => Vec::new(),
    };
    let mut node = overlay.entry(objects, &catalog, b"Pages")?;
    let mut left = u64::from(index);
    for depth in 0..objects.depth_limit() {
        let Object::Array(kids) = overlay.entry(objects, &node, b"Kids")? else {
            return Err(objects.malformed());
        };
        let mut below = None;
        for kid in kids {
            let Object::Reference(id) = kid else {
                return Err(objects.malformed());
            };
            let child = overlay.resolve(objects, id)?;
            if !matches!(child, Object::Dictionary(_)) {
                return Err(objects.malformed());
            }
            let pages = count(objects, overlay, &child)?;
            if child.get(b"Kids").is_none() && left == 0 {
                return Ok(Page::new(id, store, overlay));
            }
            if left < pages {
                if path.contains(&id.number) {
                    return Err(objects.malformed());
                }
                path.push(id.number);
                below = Some(child);
                break;
            }
            left = left.saturating_sub(pages);
        }
        node = match below {
            Some(found) => found,
            None if depth == 0 => {
                return Err(PdfError::new(
                    ErrorCode::InvalidArgument,
                    objects.operation(),
                ));
            }
            None => return Err(objects.malformed()),
        };
    }
    Err(PdfError::new(ErrorCode::LimitExceeded, objects.operation()))
}

/// The value of `key` in page `id`, or with `inherit` in the nearest ancestor that has it,
/// through `overlay`; null is none (ISO 32000-2:2020, 7.7.3.4).
fn held(
    objects: Objects<'_>,
    overlay: &Overlay,
    id: ObjectId,
    key: &[u8],
    inherit: bool,
) -> Result<Option<Object>, PdfError> {
    let mut node = overlay.resolve(objects, id)?;
    for _ in 0..=objects.depth_limit() {
        let found = overlay.entry(objects, &node, key)?;
        if !matches!(found, Object::Null) {
            return Ok(Some(found));
        }
        match node.get(b"Parent") {
            Some(Object::Reference(parent)) if inherit => {
                node = overlay.resolve(objects, parent)?;
            }
            Some(_) | None => return Ok(None),
        }
    }
    Err(PdfError::new(ErrorCode::LimitExceeded, objects.operation()))
}

/// The rectangle array `value`, its elements direct or indirect, through `overlay`, corners
/// normalized to `[left, bottom, right, top]` (ISO 32000-2:2020, 7.9.5).
fn held_rectangle(
    objects: Objects<'_>,
    overlay: &Overlay,
    value: Object,
) -> Result<[f64; 4], PdfError> {
    let Object::Array(items) = value else {
        return Err(objects.malformed());
    };
    if items.len() != 4 {
        return Err(objects.malformed());
    }
    let mut numbers = [0.0_f64; 4];
    for (number, item) in numbers.iter_mut().zip(items) {
        *number = match overlay.direct(objects, item)? {
            Object::Integer(integer) => integer.to_string().parse().ok(),
            Object::Real(real) => Some(real),
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Dictionary(_)
            | Object::Name(_)
            | Object::Null
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_) => None,
        }
        .ok_or_else(|| objects.malformed())?;
    }
    let [x0, y0, x1, y1] = numbers;
    Ok([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)])
}

/// The value of `key` in the page tree node `id` or the nearest ancestor that has it
/// (ISO 32000-2:2020, 7.7.3.4).
fn inherited(objects: Objects<'_>, id: ObjectId, key: &[u8]) -> Result<Option<Value>, PdfError> {
    let mut node = id;
    for _ in 0..=objects.depth_limit() {
        let dictionary = objects.dictionary(Some(Value::Reference(node)))?;
        if let Some(found) = own(objects, dictionary, key)? {
            return Ok(Some(found));
        }
        match objects.get(dictionary, b"Parent")? {
            Some(Value::Reference(parent)) => node = parent,
            Some(_) | None => return Ok(None),
        }
    }
    Err(PdfError::new(ErrorCode::LimitExceeded, objects.operation()))
}

/// The value of `key` in `dictionary`, resolved; null is none (ISO 32000-2:2020, 7.3.7).
fn own(objects: Objects<'_>, dictionary: At, key: &[u8]) -> Result<Option<Value>, PdfError> {
    let found = match objects.get(dictionary, key)? {
        Some(Value::Reference(id)) => Some(objects.resolve(id)?),
        other => other,
    };
    Ok(found.filter(|value| *value != Value::Null))
}

/// `[left, bottom, right, top]` as a [`Rect`].
fn rect(rectangle: [f64; 4]) -> Rect {
    let [left, bottom, right, top] = rectangle;
    Rect {
        height: span(top, bottom),
        width: span(right, left),
        x: left,
        y: bottom,
    }
}

/// The rectangle array `value`, corners normalized to `[left, bottom, right, top]`, its
/// elements direct or indirect (ISO 32000-2:2020, 7.3.6 and 7.9.5).
fn rectangle(objects: Objects<'_>, value: &Value) -> Result<[f64; 4], PdfError> {
    let mut reader = objects.reader(value.array().ok_or_else(|| objects.malformed())?)?;
    drop(reader.token()?);
    let mut numbers = [0.0_f64; 4];
    for number in &mut numbers {
        *number = match reader.item()? {
            Token::Integer(integer) => integer.to_string().parse().ok(),
            Token::Real(real) => Some(real),
            Token::Reference(id) => match objects.resolve(id)? {
                Value::Integer(integer) => integer.to_string().parse().ok(),
                Value::Real(real) => Some(real),
                Value::Array(_)
                | Value::Boolean(_)
                | Value::Dictionary(_)
                | Value::Name(_)
                | Value::Null
                | Value::Reference(_)
                | Value::String(_) => None,
            },
            Token::ArrayEnd
            | Token::ArrayStart
            | Token::DictionaryEnd
            | Token::DictionaryStart
            | Token::Keyword(_)
            | Token::Name(_)
            | Token::String(_) => None,
        }
        .ok_or_else(|| objects.malformed())?;
    }
    if reader.token()? != Token::ArrayEnd {
        return Err(objects.malformed());
    }
    let [x0, y0, x1, y1] = numbers;
    Ok([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)])
}

/// `high` less `low`, not above it, exact from their shortest decimals and then rounded once.
fn span(high: f64, low: f64) -> f64 {
    let (high_text, low_text) = (high.to_string(), low.to_string());
    let scale = |text: &str| {
        text.split_once('.')
            .map_or(0, |(_, fraction)| fraction.len())
    };
    let places = scale(&high_text).max(scale(&low_text));
    let width = high_text
        .len()
        .max(low_text.len())
        .saturating_add(places)
        .saturating_add(1);
    let digits = difference(
        &fixed(&high_text, places, width),
        &fixed(&low_text, places, width),
    );
    let point = width.saturating_sub(places);
    let mut text = String::with_capacity(width.saturating_add(1));
    for (at, &digit) in digits.iter().enumerate() {
        if at == point {
            text.push('.');
        }
        text.push(char::from(b'0'.saturating_add(digit)));
    }
    text.parse().unwrap_or_default()
}
