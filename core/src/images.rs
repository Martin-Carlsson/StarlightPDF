//! Images.

use alloc::sync::Arc;

use crate::input::Source;
use crate::layers::Layer;
use crate::output::Sink;
use crate::{Error, ErrorCode};

/// Options for [`Image::delete`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct DeleteImageOptions;

/// An image drawn on a page.
#[derive(Clone, Debug)]
pub struct Image {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Image {
    /// Removes the image from the page.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn delete(&self, options: &DeleteImageOptions) -> Result<(), Error> {
        let _: &DeleteImageOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Image::delete"))
    }

    /// The image's properties.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn info(&self) -> Result<ImageInfo, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Image::info"))
    }

    /// The layer it is drawn on, or `None`.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn layer(&self) -> Result<Option<Layer>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Image::layer"))
    }

    /// Replaces the image, keeping its place.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`], [`ErrorCode::StaleHandle`], [`ErrorCode::Unsupported`].
    pub fn replace(&self, source: &Source, options: &ReplaceImageOptions) -> Result<(), Error> {
        let _: &Source = source;
        let _: &ReplaceImageOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Image::replace"))
    }

    /// Writes the image, in its original encoding or converted.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`], [`ErrorCode::Unsupported`].
    pub fn write_to(&self, sink: &Sink, options: &WriteImageOptions) -> Result<(), Error> {
        let _: &Sink = sink;
        let _: &WriteImageOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Image::write_to"))
    }
}

/// An image file format to draw.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ImageFormat {
    /// Sony RAW.
    Arw,
    /// AVIF.
    Avif,
    /// BMP.
    Bmp,
    /// CCITT Group 3 and 4 fax.
    Ccitt,
    /// Canon RAW 2.
    Cr2,
    /// Canon RAW 3.
    Cr3,
    /// DDS.
    Dds,
    /// DICOM.
    Dicom,
    /// Digital Negative.
    Dng,
    /// Digital Picture Exchange.
    Dpx,
    /// Enhanced Metafile.
    Emf,
    /// Encapsulated PostScript.
    Eps,
    /// GIF.
    Gif,
    /// Radiance HDR.
    Hdr,
    /// HEIC and HEIF.
    Heif,
    /// Windows icon.
    Ico,
    /// JBIG2.
    Jbig2,
    /// JPEG.
    Jpeg,
    /// JPEG 2000.
    Jpeg2000,
    /// JPEG XL.
    JpegXl,
    /// Nikon RAW.
    Nef,
    /// `OpenEXR`.
    OpenExr,
    /// Olympus RAW.
    Orf,
    /// PCX.
    Pcx,
    /// PNG.
    Png,
    /// PBM, PGM and PPM.
    Pnm,
    /// Photoshop.
    Psd,
    /// QOI.
    Qoi,
    /// Fujifilm RAW.
    Raf,
    /// Panasonic RAW.
    Rw2,
    /// SVG.
    Svg,
    /// TGA.
    Tga,
    /// TIFF.
    Tiff,
    /// WebP.
    WebP,
    /// Windows Metafile.
    Wmf,
    /// XBM.
    Xbm,
    /// XPM.
    Xpm,
}

/// An image's properties.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ImageInfo {
    /// Bits per component.
    pub bits: u8,
    /// The PDF color space name.
    pub color_space: String,
    /// The stored encoding: `jpeg`, `jpeg2000`, `jbig2`, `ccitt` or `raw`.
    pub encoding: String,
    /// The displayed height in points.
    pub height: f64,
    /// Has a soft mask; [`WriteImageOptions::mask`] writes it.
    pub mask: bool,
    /// The opacity.
    pub opacity: f64,
    /// The height in pixels.
    pub pixel_height: u32,
    /// The width in pixels.
    pub pixel_width: u32,
    /// The displayed width in points.
    pub width: f64,
    /// The left edge.
    pub x: f64,
    /// The top edge.
    pub y: f64,
}

/// The images on a page, one at a time.
#[derive(Clone, Debug)]
pub struct Images {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Images {
    /// The next image, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn next(&self) -> Result<Option<Image>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Images::next"))
    }
}

/// An image format to write.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum RasterFormat {
    /// HEIF.
    Heif,
    /// JPEG.
    Jpeg,
    /// PNG.
    #[default]
    Png,
    /// TIFF.
    Tiff,
}

/// Options for [`Image::replace`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct ReplaceImageOptions {
    /// `None`: detect.
    pub format: Option<ImageFormat>,
    /// Which image of a multi-image file.
    pub frame: u32,
}

/// Options for [`Image::write_to`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct WriteImageOptions {
    /// `None`: the original encoding.
    pub format: Option<RasterFormat>,
    /// Write the soft mask instead; nothing when there is none.
    pub mask: bool,
}
