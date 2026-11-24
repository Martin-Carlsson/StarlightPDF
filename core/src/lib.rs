//! `StarlightPDF` core.
//!
//! # Features
//!
//! ## Reading and inspecting
//!
//! - **Open and parse:** [`document::Document::open`], [`input::Source`], [`limits::ResourcePolicy`].
//! - **Document info:** [`document::Document::page_count`], [`document::Document::version`], [`page::Page::boundary`], [`document::Document::fonts`], [`document::Document::attachments`], [`document::Document::encryption`].
//! - **Metadata:** [`document::Document::info_dictionary`], [`document::Document::xmp`].
//! - **Structure access:** [`document::Document::object`], [`document::Document::stream_data`], [`document::Document::xref_entries`], [`document::Document::catalog`].
//! - **Validation:** [`document::Document::validate`], [`compliance::Deviation`].
//! - **Repair:** [`document::Document::open`], [`document::Document::warnings`], [`document::Document::save`].
//!
//! ## Content extraction
//!
//! - **Text:** [`page::Page::text`], [`extract::TextSpan`], [`document::Document::convert_to`].
//! - **Layout analysis:** [`page::Page::blocks`], [`extract::Block`].
//! - **Tables:** [`page::Page::tables`], [`extract::Table`].
//! - **Images:** [`page::Page::images`], [`images::Image::write_to`], [`images::RasterFormat`].
//! - **Other content:** [`page::Page::shapes`], [`document::Document::attachments`], [`page::Page::annotations`], [`document::Document::bookmarks`], [`document::Document::fields`], [`fonts::Font::write_to`].
//! - **Conversion out:** [`document::Document::convert_to`], [`extract::OutputFormat`].
//! - **OCR:** [`document::Document::recognize`].
//! - **Search:** [`document::Document::search`], [`extract::Hit`].
//!
//! ## Page operations
//!
//! - **Combine and divide:** [`document::Document::insert_pages`], [`document::Document::split`], [`arrange::SplitBy`].
//! - **Arrange:** [`page::Page::set_index`], [`page::Page::delete`], [`document::Document::insert_pages`], [`document::Document::insert_blank_page`].
//! - **Transform:** [`page::Page::rotate_by`], [`page::Page::set_rotation`], [`page::Page::set_boundary`], [`page::Page::scale_by`], [`page::Boundary`].
//! - **Imposition:** [`document::Document::impose`], [`arrange::Imposition`].
//! - **Superimpose:** [`page::Page::superimpose`].
//!
//! ## Creating content
//!
//! - **Text:** [`page::Page::draw_text`], [`drawing::DrawTextOptions`].
//! - **Fonts:** [`document::Document::add_font`], [`fonts::AddFontOptions`], [`drawing::DrawTextOptions`].
//! - **Graphics:** [`page::Page::draw_path`], [`drawing::DrawPathOptions`], [`drawing::BlendMode`].
//! - **Images:** [`page::Page::draw_image`], [`drawing::DrawImageOptions`].
//!   - **Common raster:** [`images::ImageFormat`].
//!   - **Modern raster:** [`images::ImageFormat`].
//!   - **Vector:** [`images::ImageFormat`].
//!   - **Scanning and fax:** [`images::ImageFormat`].
//!   - **Camera RAW:** [`images::ImageFormat`].
//!   - **Professional and print:** [`images::ImageFormat`].
//!   - **Medical:** [`images::ImageFormat`].
//!   - **Legacy and other:** [`images::ImageFormat`].
//! - **Image handling:** [`drawing::DrawImageOptions`].
//! - **Layout engine:** [`document::Document::convert_from`], [`document::ConvertFromOptions`].
//! - **Generators:** [`document::Document::insert_toc`], [`page::Page::draw_barcode`], [`page::Page::draw_chart`].
//! - **Conversion in:** [`document::Document::convert_from`], [`document::InputFormat`].
//!
//! ## Editing existing content
//!
//! - **Add:** [`page::Page::draw_text`], [`page::Page::draw_image`], [`page::Page::draw_path`].
//! - **Stamps:** [`document::Document::stamp`], [`edit::StampOptions`].
//! - **Change:** [`document::Document::replace_text`], [`images::Image::replace`], [`images::Image::delete`], [`shapes::Shape::set_paint`], [`shapes::Shape::transform_by`], [`shapes::Shape::delete`].
//! - **Redaction:** [`document::Document::apply_redactions`], [`annotations::AnnotationKind`].
//! - **Flatten:** [`document::Document::flatten`], [`edit::Flattening`].
//! - **Metadata:** [`document::Document::set_info_dictionary`], [`document::Document::set_xmp`].
//!
//! ## Navigation and interactivity
//!
//! - **Bookmarks:** [`document::Document::bookmarks`], [`document::Document::add_bookmark`], [`navigation::Bookmark`].
//! - **Links:** [`page::Page::add_annotation`], [`document::Document::destinations`], [`document::Document::set_destination`].
//! - **Viewer settings:** [`document::Document::viewer_settings`], [`document::Document::set_viewer_settings`], [`document::Document::label_ranges`], [`document::Document::set_label_range`], [`document::Document::delete_label_range`], [`page::Page::label`].
//! - **Actions:** [`document::Document::scripts`], [`document::Document::set_script`], [`page::Page::set_transition`], [`document::Document::articles`], [`document::Document::add_article`].
//! - **Rich content:** [`document::Document::layers`], [`document::Document::add_layer`], [`page::Page::add_annotation`], [`annotations::AnnotationKind`].
//!
//! ## Annotations
//!
//! - **Markup:** [`page::Page::add_annotation`], [`annotations::AnnotationKind`].
//! - **Notes and shapes:** [`page::Page::add_annotation`], [`annotations::AnnotationKind`], [`annotations::LineEnding`].
//! - **Other types:** [`page::Page::add_annotation`], [`annotations::AnnotationKind`].
//! - **Management:** [`page::Page::annotations`], [`annotations::Annotation`], [`document::Document::flatten`], [`document::Document::export_data`], [`document::Document::import_data`], [`document::Document::generate_appearances`], [`annotations::Annotation::write_to`].
//!
//! ## Forms
//!
//! - **Fields:** [`document::Document::fields`], [`forms::Field`], [`forms::FieldKind`].
//! - **Use:** [`forms::Field::info`], [`forms::Field::set_value`], [`document::Document::flatten`], [`document::Document::export_data`], [`document::Document::import_data`].
//! - **Build:** [`page::Page::add_field`], [`forms::Field::set_info`], [`forms::Field::delete`].
//! - **Legacy:** [`document::Document::xfa`], [`document::Document::set_xfa`].
//!
//! ## Security
//!
//! - **Encryption:** [`document::SaveOptions`], [`security::Cipher`], [`document::OpenOptions`].
//! - **Permissions:** [`document::SaveOptions::restrictions`], [`security::Permission`], [`document::Document::encryption`].
//! - **Decryption:** [`document::OpenOptions`], [`document::SaveOptions::cipher`], [`security::Cipher`].
//! - **Sanitizing:** [`document::Document::sanitize`], [`security::HiddenData`].
//!
//! ## Digital signatures
//!
//! - **Sign:** [`document::Document::prepare_signature`], [`signatures::PendingSignature::finalize`].
//! - **Advanced:** [`signatures::SignatureKind`], [`signatures::PendingSignature::finalize`], [`document::Document::add_verification_data`], [`document::Document::verification_data`].
//! - **Hardware:** [`signatures::PendingSignature::to_sign`], [`signatures::PendingSignature::finalize`].
//! - **Verify:** [`document::Document::signatures`], [`signatures::Signature::verify`].
//!
//! ## Standards and compliance
//!
//! - **Archiving:** [`document::SaveOptions`], [`compliance::Standard`], [`document::Document::validate`].
//! - **Accessibility:** [`document::Document::tag`], [`document::Document::language`], [`document::Document::set_language`], [`document::Document::elements`], [`compliance::Element::set_alt_text`].
//! - **Print:** [`document::Document::set_output_intent`], [`document::Document::output_intent`], [`compliance::Standard`].
//! - **Other:** [`compliance::Standard`], [`document::SaveOptions`].
//!
//! ## Rendering and printing
//!
//! - **Rasterize:** [`page::Page::render`], [`rendering::RenderOptions`].
//! - **Vector export:** [`page::Page::export_vector`], [`rendering::VectorFormat`].
//! - **Print:** [`document::Document::print`], [`rendering::PrintOptions`].
//! - **Display:** [`page::Page::render`], [`rendering::RenderOptions::hidden_layers`], [`layers::Layer::set_visibility`].
//!
//! ## Optimization
//!
//! - **Compress:** [`document::SaveOptions`], [`document::Optimization`].
//! - **Clean up:** [`document::Optimization`], [`document::SaveMode::Rewrite`].
//! - **Linearize:** [`document::Optimization`].
//! - **Convert:** [`document::SaveOptions`], [`rendering::ColorSpace`].
//!
//! ## Attachments and packaging
//!
//! - **Embedded files:** [`document::Document::add_attachment`], [`attachments::Attachment::write_to`], [`attachments::Attachment::delete`].
//! - **Portfolios:** [`document::Document::set_collection`], [`document::Document::collection`], [`attachments::Relationship`].
//!
//! ## Comparison and analysis
//!
//! - **Diff:** [`document::Document::compare`], [`analysis::CompareMethod`].
//! - **Preflight:** [`document::Document::validate`], [`compliance::ValidateOptions`].
//!
//! # Dates
//!
//! A date is ISO 8601 text, such as `2026-11-17T10:15:00Z`; empty means none.

#![forbid(unsafe_code)]

extern crate alloc;

pub mod analysis;
pub mod annotations;
pub mod arrange;
mod ascii85;
mod ascii_hex;
pub mod attachments;
pub mod compliance;
mod crypto;
#[cfg(feature = "rendering")]
mod dct;
mod decryption;
pub mod document;
pub mod drawing;
pub mod edit;
pub mod extract;
#[cfg(feature = "rendering")]
mod font_program;
pub mod fonts;
pub mod forms;
pub mod images;
mod inflate;
pub mod input;
pub mod layers;
pub mod limits;
mod lzw;
pub mod metadata;
pub mod navigation;
mod object_stream;
pub mod objects;
pub mod output;
mod overlay;
pub mod page;
mod part;
mod predictor;
pub mod progress;
mod pull;
mod raster;
#[cfg(feature = "rendering")]
mod render;
pub mod rendering;
mod repair;
mod run_length;
pub mod security;
pub mod shapes;
pub mod signatures;
mod store;
mod stream;
mod syntax;
mod writer;
mod xref;
mod xref_stream;

use core::error::Error;
use core::fmt::{self, Display, Formatter};
use std::io;

/// A part of the library a build can leave out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Capability {
    /// Conversion in and out.
    Conversion,
    /// Text recognition.
    Ocr,
    /// Sending to a printer.
    Printing,
    /// Rasterizing and vector export.
    Rendering,
    /// Creating and verifying signatures.
    Signing,
}

/// The stable code of a [`PdfError`]; bindings must accept codes they do not know.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorCode {
    /// Work held in memory outgrew the resource policy's budget, and temporary storage was off or full.
    BudgetExceeded,
    /// The document is in use by a conflicting call, such as one made from its own progress callback.
    Busy,
    /// A progress callback cancelled the operation.
    Cancelled,
    /// The operation needs a part this build leaves out; see [`capabilities`].
    CapabilityMissing,
    /// The document was closed.
    DocumentClosed,
    /// An incremental save is impossible; the message says why.
    IncrementalRefused,
    /// An argument or option is out of range or malformed.
    InvalidArgument,
    /// The bytes do not begin with a valid PDF header.
    InvalidHeader,
    /// The document is encrypted and the password or key is missing or wrong.
    InvalidPassword,
    /// Reading the source or writing the sink failed; see [`PdfError::io_cause`].
    Io,
    /// Input exceeded a resource policy limit: decoded data, nesting depth or object count.
    LimitExceeded,
    /// The operation would drop content it cannot preserve, and its options do not allow loss.
    LossRefused,
    /// The input breaks its format's specification.
    Malformed,
    /// The operation is declared but has no implementation yet.
    NotImplemented,
    /// Random access the source lacks, or a sink not over a file lacks, with temporary storage off.
    NotSeekable,
    /// The save would invalidate signatures, and its options do not allow that.
    SignaturesWouldBreak,
    /// The handle's object was deleted.
    StaleHandle,
    /// The file uses a feature the library cannot handle, such as an unknown filter or security handler.
    Unsupported,
}

impl ErrorCode {
    /// The variant's name, such as `InvalidHeader`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BudgetExceeded => "BudgetExceeded",
            Self::Busy => "Busy",
            Self::Cancelled => "Cancelled",
            Self::CapabilityMissing => "CapabilityMissing",
            Self::DocumentClosed => "DocumentClosed",
            Self::IncrementalRefused => "IncrementalRefused",
            Self::InvalidArgument => "InvalidArgument",
            Self::InvalidHeader => "InvalidHeader",
            Self::InvalidPassword => "InvalidPassword",
            Self::Io => "Io",
            Self::LimitExceeded => "LimitExceeded",
            Self::LossRefused => "LossRefused",
            Self::Malformed => "Malformed",
            Self::NotImplemented => "NotImplemented",
            Self::NotSeekable => "NotSeekable",
            Self::SignaturesWouldBreak => "SignaturesWouldBreak",
            Self::StaleHandle => "StaleHandle",
            Self::Unsupported => "Unsupported",
        }
    }

    /// The code's number, for C and bindings; fixed forever, never reused, and never 0, which is success.
    #[must_use]
    pub const fn number(self) -> u16 {
        match self {
            Self::BudgetExceeded => 1,
            Self::Busy => 2,
            Self::Cancelled => 3,
            Self::CapabilityMissing => 4,
            Self::DocumentClosed => 5,
            Self::IncrementalRefused => 6,
            Self::InvalidArgument => 7,
            Self::InvalidHeader => 8,
            Self::InvalidPassword => 9,
            Self::Io => 10,
            Self::LimitExceeded => 11,
            Self::LossRefused => 12,
            Self::Malformed => 13,
            Self::NotImplemented => 14,
            Self::NotSeekable => 15,
            Self::SignaturesWouldBreak => 16,
            Self::StaleHandle => 17,
            Self::Unsupported => 18,
        }
    }

    /// A short lowercase description.
    const fn summary(self) -> &'static str {
        match self {
            Self::BudgetExceeded => "memory budget exceeded",
            Self::Busy => "document busy",
            Self::Cancelled => "cancelled",
            Self::CapabilityMissing => "capability missing",
            Self::DocumentClosed => "document closed",
            Self::IncrementalRefused => "incremental save refused",
            Self::InvalidArgument => "invalid argument",
            Self::InvalidHeader => "no valid PDF header",
            Self::InvalidPassword => "invalid password",
            Self::Io => "input or output failed",
            Self::LimitExceeded => "resource limit exceeded",
            Self::LossRefused => "content would be lost",
            Self::Malformed => "malformed input",
            Self::NotImplemented => "no implementation yet",
            Self::NotSeekable => "random access needed",
            Self::SignaturesWouldBreak => "signatures would break",
            Self::StaleHandle => "stale handle",
            Self::Unsupported => "unsupported feature",
        }
    }
}

/// The one error every operation returns.
#[derive(Debug)]
pub struct PdfError {
    /// The stable code.
    code: ErrorCode,
    /// The underlying I/O error's kind.
    io_cause: Option<io::ErrorKind>,
    /// What went wrong, for people.
    message: String,
    /// The object number.
    object_number: Option<u32>,
    /// The byte offset in the source.
    offset: Option<u64>,
    /// The operation that failed.
    operation: &'static str,
}

impl Display for PdfError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.operation, self.message)
    }
}

impl Error for PdfError {
    fn cause(&self) -> Option<&dyn Error> {
        None
    }

    fn description(&self) -> &str {
        self.message.as_str()
    }

    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

impl PdfError {
    /// The stable code.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    /// An [`ErrorCode::Io`] error caused by `error`.
    fn io(error: &io::Error, operation: &'static str) -> Self {
        Self {
            io_cause: Some(error.kind()),
            message: error.to_string(),
            ..Self::new(ErrorCode::Io, operation)
        }
    }

    /// The underlying I/O error's kind, when there is one.
    #[must_use]
    pub const fn io_cause(&self) -> Option<io::ErrorKind> {
        self.io_cause
    }

    /// What went wrong, for people.
    #[must_use]
    pub const fn message(&self) -> &str {
        self.message.as_str()
    }

    /// An error with `code`'s summary as its message.
    fn new(code: ErrorCode, operation: &'static str) -> Self {
        Self {
            code,
            io_cause: None,
            message: String::from(code.summary()),
            object_number: None,
            offset: None,
            operation,
        }
    }

    /// The object number, when known.
    #[must_use]
    pub const fn object_number(&self) -> Option<u32> {
        self.object_number
    }

    /// The byte offset in the source, when known.
    #[must_use]
    pub const fn offset(&self) -> Option<u64> {
        self.offset
    }

    /// The operation that failed.
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        self.operation
    }
}

/// A PDF version.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct Version {
    /// Major version.
    pub major: u8,
    /// Minor version.
    pub minor: u8,
}

/// The parts this build includes.
///
/// # Errors
///
/// None.
pub fn capabilities() -> Result<Vec<Capability>, PdfError> {
    Ok(if cfg!(feature = "rendering") {
        vec![Capability::Rendering]
    } else {
        Vec::new()
    })
}

/// Reads the version from the header (ISO 32000-2:2020, 7.5.2).
///
/// # Errors
///
/// [`ErrorCode::InvalidHeader`] unless `bytes` begins `%PDF-1.n` or `%PDF-2.n`, n a digit, then CR or LF.
pub fn header_version(bytes: &[u8]) -> Result<Version, PdfError> {
    parse_header(bytes, "header_version")
}

/// The library's version.
#[must_use]
pub const fn library_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// [`header_version`], failing as `operation`.
fn parse_header(bytes: &[u8], operation: &'static str) -> Result<Version, PdfError> {
    match *bytes {
        [
            b'%',
            b'P',
            b'D',
            b'F',
            b'-',
            major @ (b'1' | b'2'),
            b'.',
            minor @ b'0'..=b'9',
            b'\r' | b'\n',
            ..,
        ] => Ok(Version {
            major: major.wrapping_sub(b'0'),
            minor: minor.wrapping_sub(b'0'),
        }),
        _ => Err(PdfError::new(ErrorCode::InvalidHeader, operation)),
    }
}
