//! Documents: open, inspect, change and save.

use alloc::sync::Arc;
use core::fmt::{self, Debug, Formatter};
use std::path::PathBuf;

use crate::analysis::{CompareOptions, Differences};
use crate::annotations::GenerateAppearancesOptions;
use crate::arrange::{
    ImposeOptions, InsertBlankPageOptions, InsertPagesOptions, InsertTocOptions, SplitOptions,
};
use crate::attachments::{
    AddAttachmentOptions, Attachment, Attachments, Collection, SetCollectionOptions,
};
use crate::compliance::{
    Deviations, Elements, OutputIntent, SetOutputIntentOptions, Standard, TagOptions,
    ValidateOptions,
};
use crate::edit::{ApplyRedactionsOptions, FlattenOptions, ReplaceTextOptions, StampOptions};
use crate::extract::{ConvertToOptions, Hits, RecognizeOptions, SearchOptions};
use crate::fonts::{AddFontOptions, Font, Fonts};
use crate::forms::{ExportDataOptions, Fields, ImportDataOptions, SetXfaOptions, XfaOptions};
use crate::input::Source;
use crate::layers::{AddLayerOptions, Layer, Layers};
use crate::limits::ResourcePolicy;
use crate::metadata::{InfoDictionary, SetInfoDictionaryOptions, SetXmpOptions};
use crate::navigation::{
    AddArticleOptions, AddBookmarkOptions, Articles, Bookmark, Bookmarks, Destinations,
    LabelRanges, Scripts, SetDestinationOptions, SetLabelRangeOptions, SetScriptOptions,
    SetViewerSettingsOptions, ViewerSettings,
};
use crate::objects::{
    AddObjectOptions, ObjectId, ObjectOptions, SetObjectOptions, SetStreamDataOptions,
    StreamDataOptions, XrefEntries,
};
use crate::output::Sink;
use crate::page::Page;
use crate::progress::Progress;
use crate::rendering::{ColorSpace, PrintOptions};
use crate::security::{Cipher, Encryption, Permission, SanitizeOptions};
use crate::signatures::{
    AddVerificationDataOptions, PendingSignature, PrepareSignatureOptions, Signatures,
    VerificationItems,
};
use crate::{Error, ErrorCode, Version};

/// Options for [`Document::close`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct CloseOptions;

/// Options for [`Document::convert_from`].
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ConvertFromOptions {
    /// Directory relative resources load from; `None`: no external resources.
    pub base: Option<PathBuf>,
    /// Text columns; `None`: 1.
    pub columns: Option<u32>,
    /// JSON that fills a [`InputFormat::Template`].
    pub data: String,
    /// HTML repeated at each page's foot; `{page}` and `{pages}` become numbers.
    pub footer: String,
    /// The input's format.
    pub format: InputFormat,
    /// As `footer`, at the head.
    pub header: String,
    /// Page margin in points; `None`: 36.
    pub margin: Option<f64>,
    /// Page height in points; `None`: A4.
    pub page_height: Option<f64>,
    /// Page width in points; `None`: A4.
    pub page_width: Option<f64>,
}

/// Options for [`Document::create`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct CreateOptions {
    /// The PDF version; `None`: 2.0.
    pub version: Option<Version>,
}

/// An open PDF: its source plus an overlay of changes.
#[derive(Clone, Debug)]
pub struct Document {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

/// Documents, one at a time.
#[derive(Clone, Debug)]
pub struct Documents {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

/// Options for [`Document::fonts`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct FontsOptions;

/// The format [`Document::convert_from`] reads.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum InputFormat {
    /// HTML and CSS.
    #[default]
    Html,
    /// One page per image; formats as [`ImageFormat`](crate::images::ImageFormat).
    Image,
    /// Markdown.
    Markdown,
    /// Word, Excel and `PowerPoint` files.
    Office,
    /// HTML with placeholders filled from `data`.
    Template,
}

/// Options for [`Document::open`]; `Debug` leaves out the password and private key.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct OpenOptions {
    /// The user or owner password.
    pub password: String,
    /// For certificate-encrypted files, the recipient's private key (PKCS #8, DER).
    pub private_key: Vec<u8>,
    /// Fail at the first deviation instead of tolerating it.
    pub strict: bool,
}

impl Debug for OpenOptions {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenOptions")
            .field("strict", &self.strict)
            .finish_non_exhaustive()
    }
}

/// A whole-file optimization applied when saving.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Optimization {
    /// Compress uncompressed streams.
    CompressStreams,
    /// Merge duplicate fonts and images.
    Deduplicate,
    /// Fast web view.
    Linearize,
    /// Pack objects into object streams.
    ObjectStreams,
    /// Keep only the glyphs used.
    SubsetFonts,
}

/// Options for [`Document::page`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct PageOptions {
    /// The page index.
    pub index: u32,
}

/// Options for [`Document::revert`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct RevertOptions;

/// How [`Document::save`] writes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum SaveMode {
    /// Append only the changes to the opened file, unchanged since open; no atomic guarantee.
    InPlace,
    /// The source's bytes, then the changes appended.
    Incremental,
    /// A streamed full rewrite of the reachable objects.
    #[default]
    Rewrite,
}

/// Options for [`Document::save`]; whole-file work happens here. `Debug` leaves out the passwords.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct SaveOptions {
    /// Let `standards` or `version` drop what they forbid.
    pub allow_loss: bool,
    /// Allow a rewrite to invalidate signatures.
    pub break_signatures: bool,
    /// The encryption; `None`: keep the current protection; [`Cipher::Unencrypted`] removes it.
    pub cipher: Option<Cipher>,
    /// Convert every color to this space.
    pub color_space: Option<ColorSpace>,
    /// Recompress images as JPEG at this quality, 1 to 100.
    pub image_quality: Option<u8>,
    /// Downsample images above this many pixels per inch.
    pub image_resolution: Option<u32>,
    /// How to write.
    pub mode: SaveMode,
    /// Whole-file optimizations to apply.
    pub optimizations: Vec<Optimization>,
    /// The owner password.
    pub owner_password: String,
    /// [Certificates](crate::signatures#certificates) to encrypt for, instead of passwords.
    pub recipients: Vec<u8>,
    /// What the user password denies.
    pub restrictions: Vec<Permission>,
    /// Make the file conform: PDF/A, PDF/UA, PDF/X, PDF/E, PDF/VT, Factur-X; PDF/UA and PDF/A level a need
    /// [`Document::tag`] first.
    pub standards: Vec<Standard>,
    /// The user password.
    pub user_password: String,
    /// Write this PDF version.
    pub version: Option<Version>,
}

impl Debug for SaveOptions {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SaveOptions")
            .field("allow_loss", &self.allow_loss)
            .field("break_signatures", &self.break_signatures)
            .field("cipher", &self.cipher)
            .field("color_space", &self.color_space)
            .field("image_quality", &self.image_quality)
            .field("image_resolution", &self.image_resolution)
            .field("mode", &self.mode)
            .field("optimizations", &self.optimizations)
            .field("recipients", &self.recipients)
            .field("restrictions", &self.restrictions)
            .field("standards", &self.standards)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// What a save did.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SaveReport {
    /// Fields of signatures the save invalidated.
    pub broken_signatures: Vec<String>,
    /// Bytes written.
    pub length: u64,
}

impl Document {
    /// Adds an article (ISO 32000-2:2020, 12.4.3).
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn add_article(&self, options: &AddArticleOptions) -> Result<(), Error> {
        let _: &AddArticleOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::add_article",
        ))
    }

    /// Embeds `source` as a file.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`].
    pub fn add_attachment(
        &self,
        source: &Source,
        options: &AddAttachmentOptions,
    ) -> Result<Attachment, Error> {
        let _: &Source = source;
        let _: &AddAttachmentOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::add_attachment",
        ))
    }

    /// Adds a bookmark under `parent`, or at the top level.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::StaleHandle`].
    pub fn add_bookmark(
        &self,
        parent: Option<&Bookmark>,
        options: &AddBookmarkOptions,
    ) -> Result<Bookmark, Error> {
        let _: Option<&Bookmark> = parent;
        let _: &AddBookmarkOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::add_bookmark",
        ))
    }

    /// Embeds a TrueType or OpenType font for drawing text.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`], [`ErrorCode::Unsupported`].
    pub fn add_font(&self, source: &Source, options: &AddFontOptions) -> Result<Font, Error> {
        let _: &Source = source;
        let _: &AddFontOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::add_font"))
    }

    /// Adds a layer (optional content group).
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn add_layer(&self, options: &AddLayerOptions) -> Result<Layer, Error> {
        let _: &AddLayerOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::add_layer"))
    }

    /// Adds an object from PDF syntax.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn add_object(&self, options: &AddObjectOptions) -> Result<ObjectId, Error> {
        let _: &AddObjectOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::add_object",
        ))
    }

    /// Adds certificates and revocation data for long-term validation.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn add_verification_data(&self, options: &AddVerificationDataOptions) -> Result<(), Error> {
        let _: &AddVerificationDataOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::add_verification_data",
        ))
    }

    /// Removes the content under redaction marks, then the marks.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::Cancelled`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn apply_redactions(
        &self,
        progress: Option<&dyn Progress>,
        options: &ApplyRedactionsOptions,
    ) -> Result<(), Error> {
        let _: Option<&dyn Progress> = progress;
        let _: &ApplyRedactionsOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::apply_redactions",
        ))
    }

    /// The articles.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn articles(&self) -> Result<Articles, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Document::articles"))
    }

    /// The embedded files.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn attachments(&self) -> Result<Attachments, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::attachments",
        ))
    }

    /// The top-level bookmarks.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn bookmarks(&self) -> Result<Bookmarks, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Document::bookmarks"))
    }

    /// The document catalog's object.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn catalog(&self) -> Result<ObjectId, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Document::catalog"))
    }

    /// Discards unsaved changes, releases the source and invalidates handles; never writes.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::Busy`].
    pub fn close(&self, options: &CloseOptions) -> Result<(), Error> {
        let _: &CloseOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::close"))
    }

    /// The collection settings; `view` is `None` for a plain document.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn collection(&self) -> Result<Collection, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::collection",
        ))
    }

    /// Compares with `other`.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Cancelled`], [`ErrorCode::CapabilityMissing`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn compare(
        &self,
        other: &Self,
        progress: Option<&dyn Progress>,
        options: &CompareOptions,
    ) -> Result<Differences, Error> {
        let _: &Self = other;
        let _: Option<&dyn Progress> = progress;
        let _: &CompareOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::compare"))
    }

    /// A new document laid out from HTML, Markdown, a template, Office files or images.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Cancelled`], [`ErrorCode::CapabilityMissing`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`], [`ErrorCode::Unsupported`].
    pub fn convert_from(
        sources: &[Source],
        policy: Option<&ResourcePolicy>,
        progress: Option<&dyn Progress>,
        options: &ConvertFromOptions,
    ) -> Result<Self, Error> {
        let _: &[Source] = sources;
        let _: Option<&ResourcePolicy> = policy;
        let _: Option<&dyn Progress> = progress;
        let _: &ConvertFromOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::convert_from",
        ))
    }

    /// Writes the document as another format.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Cancelled`], [`ErrorCode::CapabilityMissing`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn convert_to(
        &self,
        sink: &Sink,
        progress: Option<&dyn Progress>,
        options: &ConvertToOptions,
    ) -> Result<(), Error> {
        let _: &Sink = sink;
        let _: Option<&dyn Progress> = progress;
        let _: &ConvertToOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::convert_to",
        ))
    }

    /// A new document with no pages.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::InvalidArgument`].
    pub fn create(policy: Option<&ResourcePolicy>, options: &CreateOptions) -> Result<Self, Error> {
        let _: Option<&ResourcePolicy> = policy;
        let _: &CreateOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::create"))
    }

    /// The named destinations.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn destinations(&self) -> Result<Destinations, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::destinations",
        ))
    }

    /// How many warnings the policy's limit kept out of `warnings`.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::DocumentClosed`].
    pub fn dropped_warnings(&self) -> Result<u64, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::dropped_warnings",
        ))
    }

    /// The structure elements, in reading order.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn elements(&self) -> Result<Elements, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Document::elements"))
    }

    /// How the document is protected.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn encryption(&self) -> Result<Encryption, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::encryption",
        ))
    }

    /// Writes form values or annotations as FDF, XFDF, XML or JSON.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn export_data(&self, sink: &Sink, options: &ExportDataOptions) -> Result<(), Error> {
        let _: &Sink = sink;
        let _: &ExportDataOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::export_data",
        ))
    }

    /// The form fields.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn fields(&self) -> Result<Fields, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Document::fields"))
    }

    /// Merges annotations, fields, layers or transparency into the page content.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::Cancelled`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn flatten(
        &self,
        progress: Option<&dyn Progress>,
        options: &FlattenOptions,
    ) -> Result<(), Error> {
        let _: Option<&dyn Progress> = progress;
        let _: &FlattenOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::flatten"))
    }

    /// The fonts used.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn fonts(&self, options: &FontsOptions) -> Result<Fonts, Error> {
        let _: &FontsOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::fonts"))
    }

    /// Generates appearance streams for annotations and fields.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::Cancelled`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn generate_appearances(
        &self,
        progress: Option<&dyn Progress>,
        options: &GenerateAppearancesOptions,
    ) -> Result<(), Error> {
        let _: Option<&dyn Progress> = progress;
        let _: &GenerateAppearancesOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::generate_appearances",
        ))
    }

    /// Whether there are unsaved changes.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::DocumentClosed`].
    pub fn has_changes(&self) -> Result<bool, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::has_changes",
        ))
    }

    /// Reads form values or annotations from FDF, XFDF, XML or JSON.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`].
    pub fn import_data(&self, source: &Source, options: &ImportDataOptions) -> Result<(), Error> {
        let _: &Source = source;
        let _: &ImportDataOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::import_data",
        ))
    }

    /// A new document with these pages imposed on sheets.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Cancelled`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::LossRefused`], [`ErrorCode::Malformed`].
    pub fn impose(
        &self,
        progress: Option<&dyn Progress>,
        options: &ImposeOptions,
    ) -> Result<Self, Error> {
        let _: Option<&dyn Progress> = progress;
        let _: &ImposeOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::impose"))
    }

    /// The document information dictionary (ISO 32000-2:2020, 14.3.3).
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn info_dictionary(&self) -> Result<InfoDictionary, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::info_dictionary",
        ))
    }

    /// Inserts a blank page.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn insert_blank_page(&self, options: &InsertBlankPageOptions) -> Result<Page, Error> {
        let _: &InsertBlankPageOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::insert_blank_page",
        ))
    }

    /// Inserts pages of `from`, which may be this document.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn insert_pages(&self, from: &Self, options: &InsertPagesOptions) -> Result<(), Error> {
        let _: &Self = from;
        let _: &InsertPagesOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::insert_pages",
        ))
    }

    /// Inserts table-of-contents pages built from the bookmarks.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn insert_toc(&self, options: &InsertTocOptions) -> Result<(), Error> {
        let _: &InsertTocOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::insert_toc",
        ))
    }

    /// The page label ranges (ISO 32000-2:2020, 12.4.2).
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn label_ranges(&self) -> Result<LabelRanges, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::label_ranges",
        ))
    }

    /// The document's language, BCP 47; empty when unset.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn language(&self) -> Result<String, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Document::language"))
    }

    /// The layers.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn layers(&self) -> Result<Layers, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Document::layers"))
    }

    /// An object as PDF syntax; a missing object is `null`.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn object(&self, options: &ObjectOptions) -> Result<String, Error> {
        let _: &ObjectOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::object"))
    }

    /// Opens `source`, reading lazily and tolerating what viewers tolerate.
    ///
    /// A forward-only source spools to temporary storage, or fails with [`ErrorCode::NotSeekable`].
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::InvalidArgument`], [`ErrorCode::InvalidHeader`], [`ErrorCode::InvalidPassword`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`], [`ErrorCode::Unsupported`].
    pub fn open(
        source: &Source,
        policy: Option<&ResourcePolicy>,
        options: &OpenOptions,
    ) -> Result<Self, Error> {
        let _: &Source = source;
        let _: Option<&ResourcePolicy> = policy;
        let _: &OpenOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::open"))
    }

    /// The output intent, when there is one.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn output_intent(&self) -> Result<Option<OutputIntent>, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::output_intent",
        ))
    }

    /// The page at `index`.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn page(&self, options: &PageOptions) -> Result<Page, Error> {
        let _: &PageOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::page"))
    }

    /// The number of pages.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn page_count(&self) -> Result<u32, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::page_count",
        ))
    }

    /// Saves incrementally with room for a signature, for the caller to sign.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::Cancelled`], [`ErrorCode::CapabilityMissing`], [`ErrorCode::DocumentClosed`], [`ErrorCode::IncrementalRefused`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`].
    pub fn prepare_signature(
        &self,
        sink: &Sink,
        progress: Option<&dyn Progress>,
        options: &PrepareSignatureOptions,
    ) -> Result<PendingSignature, Error> {
        let _: &Sink = sink;
        let _: Option<&dyn Progress> = progress;
        let _: &PrepareSignatureOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::prepare_signature",
        ))
    }

    /// Sends pages to a printer.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Cancelled`], [`ErrorCode::CapabilityMissing`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn print(
        &self,
        progress: Option<&dyn Progress>,
        options: &PrintOptions,
    ) -> Result<(), Error> {
        let _: Option<&dyn Progress> = progress;
        let _: &PrintOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::print"))
    }

    /// Recognizes text in scanned pages and adds it as invisible, searchable text.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::Cancelled`], [`ErrorCode::CapabilityMissing`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn recognize(
        &self,
        progress: Option<&dyn Progress>,
        options: &RecognizeOptions,
    ) -> Result<(), Error> {
        let _: Option<&dyn Progress> = progress;
        let _: &RecognizeOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::recognize"))
    }

    /// Replaces text; returns how many matches changed.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::Cancelled`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn replace_text(
        &self,
        progress: Option<&dyn Progress>,
        options: &ReplaceTextOptions,
    ) -> Result<u64, Error> {
        let _: Option<&dyn Progress> = progress;
        let _: &ReplaceTextOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::replace_text",
        ))
    }

    /// Drops unsaved changes.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`].
    pub fn revert(&self, options: &RevertOptions) -> Result<(), Error> {
        let _: &RevertOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::revert"))
    }

    /// Removes hidden data, scripts, metadata or attachments.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::Cancelled`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn sanitize(
        &self,
        progress: Option<&dyn Progress>,
        options: &SanitizeOptions,
    ) -> Result<(), Error> {
        let _: Option<&dyn Progress> = progress;
        let _: &SanitizeOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::sanitize"))
    }

    /// Writes the document; the session stays open.
    ///
    /// A failed save to a file leaves the previous file intact.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::Cancelled`], [`ErrorCode::DocumentClosed`], [`ErrorCode::IncrementalRefused`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::LossRefused`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`], [`ErrorCode::SignaturesWouldBreak`].
    pub fn save(
        &self,
        sink: &Sink,
        progress: Option<&dyn Progress>,
        options: &SaveOptions,
    ) -> Result<SaveReport, Error> {
        let _: &Sink = sink;
        let _: Option<&dyn Progress> = progress;
        let _: &SaveOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::save"))
    }

    /// The document-level JavaScript; never run.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn scripts(&self) -> Result<Scripts, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Document::scripts"))
    }

    /// Finds text or a regular expression.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn search(&self, options: &SearchOptions) -> Result<Hits, Error> {
        let _: &SearchOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::search"))
    }

    /// Makes the document a collection (ISO 32000-2:2020, 12.3.5), or a plain document again.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn set_collection(&self, options: &SetCollectionOptions) -> Result<(), Error> {
        let _: &SetCollectionOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::set_collection",
        ))
    }

    /// Sets or removes a named destination.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn set_destination(&self, options: &SetDestinationOptions) -> Result<(), Error> {
        let _: &SetDestinationOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::set_destination",
        ))
    }

    /// Sets the document information dictionary (ISO 32000-2:2020, 14.3.3).
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn set_info_dictionary(&self, options: &SetInfoDictionaryOptions) -> Result<(), Error> {
        let _: &SetInfoDictionaryOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::set_info_dictionary",
        ))
    }

    /// Labels pages from `start` up to the next label range.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn set_label_range(&self, options: &SetLabelRangeOptions) -> Result<(), Error> {
        let _: &SetLabelRangeOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::set_label_range",
        ))
    }

    /// Replaces an object from PDF syntax.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn set_object(&self, options: &SetObjectOptions) -> Result<(), Error> {
        let _: &SetObjectOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::set_object",
        ))
    }

    /// Sets the output intent from an ICC profile.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`], [`ErrorCode::Unsupported`].
    pub fn set_output_intent(
        &self,
        profile: &Source,
        options: &SetOutputIntentOptions,
    ) -> Result<(), Error> {
        let _: &Source = profile;
        let _: &SetOutputIntentOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::set_output_intent",
        ))
    }

    /// Sets or removes a document-level script.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn set_script(&self, options: &SetScriptOptions) -> Result<(), Error> {
        let _: &SetScriptOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::set_script",
        ))
    }

    /// Replaces a stream's data.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`].
    pub fn set_stream_data(
        &self,
        source: &Source,
        options: &SetStreamDataOptions,
    ) -> Result<(), Error> {
        let _: &Source = source;
        let _: &SetStreamDataOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::set_stream_data",
        ))
    }

    /// Sets how viewers open the document.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn set_viewer_settings(&self, options: &SetViewerSettingsOptions) -> Result<(), Error> {
        let _: &SetViewerSettingsOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::set_viewer_settings",
        ))
    }

    /// Sets the XFA form's XML from `source`; `None` removes the form.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`].
    pub fn set_xfa(&self, source: Option<&Source>, options: &SetXfaOptions) -> Result<(), Error> {
        let _: Option<&Source> = source;
        let _: &SetXfaOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::set_xfa"))
    }

    /// Sets or removes the XMP metadata.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn set_xmp(&self, options: &SetXmpOptions) -> Result<(), Error> {
        let _: &SetXmpOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::set_xmp"))
    }

    /// The signatures.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn signatures(&self) -> Result<Signatures, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::signatures",
        ))
    }

    /// Splits the pages into new documents.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn split(&self, options: &SplitOptions) -> Result<Documents, Error> {
        let _: &SplitOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::split"))
    }

    /// Stamps text or `image` on pages: watermarks, headers, footers, page and Bates numbers.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::Cancelled`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`], [`ErrorCode::Unsupported`].
    pub fn stamp(
        &self,
        image: Option<&Source>,
        progress: Option<&dyn Progress>,
        options: &StampOptions,
    ) -> Result<(), Error> {
        let _: Option<&Source> = image;
        let _: Option<&dyn Progress> = progress;
        let _: &StampOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::stamp"))
    }

    /// Writes a stream's data; [`ErrorCode::InvalidArgument`] when `id` is missing or not a stream.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::Unsupported`].
    pub fn stream_data(&self, sink: &Sink, options: &StreamDataOptions) -> Result<(), Error> {
        let _: &Sink = sink;
        let _: &StreamDataOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::stream_data",
        ))
    }

    /// Builds a structure tree with reading order from the layout.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::Cancelled`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::LossRefused`], [`ErrorCode::Malformed`].
    pub fn tag(&self, progress: Option<&dyn Progress>, options: &TagOptions) -> Result<(), Error> {
        let _: Option<&dyn Progress> = progress;
        let _: &TagOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::tag"))
    }

    /// Reads everything and reports each deviation from a standard.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn validate(&self, options: &ValidateOptions) -> Result<Deviations, Error> {
        let _: &ValidateOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::validate"))
    }

    /// The certificates and revocation data stored for long-term validation.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn verification_data(&self) -> Result<VerificationItems, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::verification_data",
        ))
    }

    /// The effective version: the header's, or the catalog's when later.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn version(&self) -> Result<Version, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Document::version"))
    }

    /// How viewers open the document.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn viewer_settings(&self) -> Result<ViewerSettings, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::viewer_settings",
        ))
    }

    /// The deviations tolerated so far, up to the policy's limit.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::DocumentClosed`].
    pub fn warnings(&self) -> Result<Deviations, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Document::warnings"))
    }

    /// Writes the XFA form's XML; nothing when there is none.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn xfa(&self, sink: &Sink, options: &XfaOptions) -> Result<(), Error> {
        let _: &Sink = sink;
        let _: &XfaOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Document::xfa"))
    }

    /// The XMP metadata packet; empty when there is none.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn xmp(&self) -> Result<String, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Document::xmp"))
    }

    /// The cross-reference entries.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn xref_entries(&self) -> Result<XrefEntries, Error> {
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Document::xref_entries",
        ))
    }
}

impl Documents {
    /// The next document, or `None` after the last.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<Document>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Documents::next"))
    }
}
