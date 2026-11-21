//! Documents: open, inspect, change and save.

use alloc::sync::Arc;
use core::fmt::{self, Debug, Formatter};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use crate::analysis::{CompareOptions, Differences};
use crate::annotations::GenerateAppearancesOptions;
use crate::arrange::{
    ImposeOptions, InsertBlankPageOptions, InsertPagesOptions, InsertTocOptions, SplitBy,
    SplitOptions,
};
use crate::attachments::{
    AddAttachmentOptions, Attachment, Attachments, Collection, SetCollectionOptions,
};
use crate::compliance::{
    Deviations, Elements, OutputIntent, SetLanguageOptions, SetOutputIntentOptions, Standard,
    TagOptions, ValidateOptions,
};
use crate::edit::{ApplyRedactionsOptions, FlattenOptions, ReplaceTextOptions, StampOptions};
use crate::extract::{ConvertToOptions, Hits, RecognizeOptions, SearchOptions};
use crate::fonts::{AddFontOptions, Font, Fonts};
use crate::forms::{ExportDataOptions, Fields, ImportDataOptions, SetXfaOptions, XfaOptions};
use crate::input::Source;
use crate::layers::{AddLayerOptions, Layer, Layers};
use crate::limits::ResourcePolicy;
use crate::metadata::{
    InfoDictionary, SetInfoDictionaryOptions, SetXmpOptions, iso_date, pdf_date, text, text_string,
};
use crate::navigation::{
    AddArticleOptions, AddBookmarkOptions, Articles, Bookmark, Bookmarks, DeleteLabelRangeOptions,
    Destinations, LabelRanges, Scripts, SetDestinationOptions, SetLabelRangeOptions,
    SetScriptOptions, SetViewerSettingsOptions, ViewerSettings,
};
use crate::objects::{
    AddObjectOptions, ObjectId, ObjectOptions, SetObjectOptions, SetStreamDataOptions,
    StreamDataOptions, XrefEntries,
};
use crate::output::Sink;
use crate::overlay::{Dictionary, Object, Overlay};
use crate::page::{self, Page};
use crate::part::{self, Pages, Part};
use crate::progress::Progress;
use crate::rendering::{ColorSpace, PrintOptions};
use crate::security::{Cipher, Encryption, Permission, SanitizeOptions};
use crate::signatures::{
    AddVerificationDataOptions, PendingSignature, PrepareSignatureOptions, Signatures,
    VerificationItems,
};
use crate::store::{Objects, Store};
use crate::stream;
use crate::syntax::Value;
use crate::writer::{self, Rewrite};
use crate::{ErrorCode, PdfError, Version};

/// A4, 210 × 297 mm, in points to 4 decimals (ISO 216; ISO 32000-2:2020, 8.3.2.3).
const A4: [f64; 2] = [595.2756, 841.8898];

/// The information dictionary's keys, as [`InfoDictionary`]'s fields are ordered, and whether
/// each holds a date (ISO 32000-2:2020, 14.3.3).
const INFO_KEYS: [(&[u8], bool); 8] = [
    (b"Author", false),
    (b"CreationDate", true),
    (b"Creator", false),
    (b"Keywords", false),
    (b"ModDate", true),
    (b"Producer", false),
    (b"Subject", false),
    (b"Title", false),
];

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
    /// The changes.
    overlay: Arc<Overlay>,
    /// Whether a part of a split: a new file, whose saves write the source decrypted and a new
    /// file identifier (ISO 32000-2:2020, 7.6, 14.4).
    part: bool,
    /// The shared state.
    store: Arc<Store>,
}

/// Documents, one at a time.
#[derive(Clone, Debug)]
pub struct Documents {
    /// The split giving them.
    parts: Arc<Mutex<Parts>>,
}

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

/// A split's state: its document, pages, where parts start and the next part.
#[derive(Debug)]
struct Parts {
    /// Leave out what a part cannot keep.
    allow_loss: bool,
    /// Pages per part; 0 for parts at `starts`.
    count: u32,
    /// The next part's index.
    next: u32,
    /// The document's pages.
    pages: Arc<Pages>,
    /// The document split.
    source: Document,
    /// The first pages of the parts, when not `count` pages each.
    starts: Vec<u32>,
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
    pub fn add_article(&self, options: &AddArticleOptions) -> Result<(), PdfError> {
        let _: &AddArticleOptions = options;
        Err(PdfError::new(
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
    ) -> Result<Attachment, PdfError> {
        let _: &Source = source;
        let _: &AddAttachmentOptions = options;
        Err(PdfError::new(
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
    ) -> Result<Bookmark, PdfError> {
        let _: Option<&Bookmark> = parent;
        let _: &AddBookmarkOptions = options;
        Err(PdfError::new(
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
    pub fn add_font(&self, source: &Source, options: &AddFontOptions) -> Result<Font, PdfError> {
        let _: &Source = source;
        let _: &AddFontOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::add_font",
        ))
    }

    /// Adds a layer (optional content group).
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn add_layer(&self, options: &AddLayerOptions) -> Result<Layer, PdfError> {
        let _: &AddLayerOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::add_layer",
        ))
    }

    /// Adds an object from PDF syntax.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn add_object(&self, options: &AddObjectOptions) -> Result<ObjectId, PdfError> {
        let _: &AddObjectOptions = options;
        Err(PdfError::new(
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
    pub fn add_verification_data(
        &self,
        options: &AddVerificationDataOptions,
    ) -> Result<(), PdfError> {
        let _: &AddVerificationDataOptions = options;
        Err(PdfError::new(
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
    ) -> Result<(), PdfError> {
        let _: Option<&dyn Progress> = progress;
        let _: &ApplyRedactionsOptions = options;
        Err(PdfError::new(
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
    pub fn articles(&self) -> Result<Articles, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::articles",
        ))
    }

    /// The embedded files.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn attachments(&self) -> Result<Attachments, PdfError> {
        Err(PdfError::new(
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
    pub fn bookmarks(&self) -> Result<Bookmarks, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::bookmarks",
        ))
    }

    /// The document catalog's object: the trailer's `Root` (ISO 32000-2:2020, 7.7.2).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn catalog(&self) -> Result<ObjectId, PdfError> {
        self.store.objects("Document::catalog").catalog()
    }

    /// Discards unsaved changes, releases the source and invalidates handles; never writes.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::Busy`].
    pub fn close(&self, options: &CloseOptions) -> Result<(), PdfError> {
        let _: &CloseOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Document::close"))
    }

    /// The collection settings; `view` is `None` for a plain document.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn collection(&self) -> Result<Collection, PdfError> {
        Err(PdfError::new(
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
    ) -> Result<Differences, PdfError> {
        let _: &Self = other;
        let _: Option<&dyn Progress> = progress;
        let _: &CompareOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::compare",
        ))
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
    ) -> Result<Self, PdfError> {
        let _: &[Source] = sources;
        let _: Option<&ResourcePolicy> = policy;
        let _: Option<&dyn Progress> = progress;
        let _: &ConvertFromOptions = options;
        Err(PdfError::new(
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
    ) -> Result<(), PdfError> {
        let _: &Sink = sink;
        let _: Option<&dyn Progress> = progress;
        let _: &ConvertToOptions = options;
        Err(PdfError::new(
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
    pub fn create(
        policy: Option<&ResourcePolicy>,
        options: &CreateOptions,
    ) -> Result<Self, PdfError> {
        /// The operation.
        const CREATE: &str = "Document::create";
        let version = options.version.unwrap_or(Version { major: 2, minor: 0 });
        if !matches!((version.major, version.minor), (1, 0..=7) | (2, 0)) {
            return Err(PdfError::new(ErrorCode::InvalidArgument, CREATE));
        }
        let overlay = Overlay::new(1);
        let pages = overlay.add(Object::Dictionary(vec![
            (b"Type".to_vec(), Object::Name(b"Pages".to_vec())),
            (b"Kids".to_vec(), Object::Array(Vec::new())),
            (b"Count".to_vec(), Object::Integer(0)),
        ]))?;
        let catalog = overlay.add(Object::Dictionary(vec![
            (b"Type".to_vec(), Object::Name(b"Catalog".to_vec())),
            (b"Pages".to_vec(), Object::Reference(pages)),
        ]))?;
        overlay.set_trailer(vec![(b"Root".to_vec(), Object::Reference(catalog))]);
        Ok(Self {
            overlay: Arc::new(overlay),
            part: false,
            store: Arc::new(Store::create(policy, version)?),
        })
    }

    /// Removes the label range that begins at `start`; its pages join the range before.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn delete_label_range(&self, options: &DeleteLabelRangeOptions) -> Result<(), PdfError> {
        let _: &DeleteLabelRangeOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::delete_label_range",
        ))
    }

    /// The named destinations.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn destinations(&self) -> Result<Destinations, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::destinations",
        ))
    }

    /// How many warnings the policy's limit kept out of `warnings`.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::DocumentClosed`].
    pub fn dropped_warnings(&self) -> Result<u64, PdfError> {
        Ok(self.store.dropped_warnings())
    }

    /// The structure elements, in reading order.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn elements(&self) -> Result<Elements, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::elements",
        ))
    }

    /// How the document is protected: the cipher, what `P` denies and whether certificates are
    /// used (ISO 32000-2:2020, 7.6.2 and 7.6.4.2).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn encryption(&self) -> Result<Encryption, PdfError> {
        Ok(self.store.encryption())
    }

    /// Writes form values or annotations as FDF, XFDF, XML or JSON.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn export_data(&self, sink: &Sink, options: &ExportDataOptions) -> Result<(), PdfError> {
        let _: &Sink = sink;
        let _: &ExportDataOptions = options;
        Err(PdfError::new(
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
    pub fn fields(&self) -> Result<Fields, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Document::fields"))
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
    ) -> Result<(), PdfError> {
        let _: Option<&dyn Progress> = progress;
        let _: &FlattenOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::flatten",
        ))
    }

    /// The fonts used.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn fonts(&self) -> Result<Fonts, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Document::fonts"))
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
    ) -> Result<(), PdfError> {
        let _: Option<&dyn Progress> = progress;
        let _: &GenerateAppearancesOptions = options;
        Err(PdfError::new(
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
    pub fn has_changes(&self) -> Result<bool, PdfError> {
        Err(PdfError::new(
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
    pub fn import_data(
        &self,
        source: &Source,
        options: &ImportDataOptions,
    ) -> Result<(), PdfError> {
        let _: &Source = source;
        let _: &ImportDataOptions = options;
        Err(PdfError::new(
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
    ) -> Result<Self, PdfError> {
        let _: Option<&dyn Progress> = progress;
        let _: &ImposeOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Document::impose"))
    }

    /// The trailer, the information dictionary's id when indirect, and the dictionary, empty
    /// when there is none (ISO 32000-2:2020, 14.3.3).
    fn info(
        &self,
        objects: Objects<'_>,
    ) -> Result<(Dictionary, Option<ObjectId>, Object), PdfError> {
        let trailer = self.overlay.trailer(objects)?;
        let entry = Object::Dictionary(trailer.clone())
            .take(b"Info")
            .unwrap_or(Object::Null);
        let id = if let Object::Reference(found) = entry {
            Some(found)
        } else {
            None
        };
        let info = match self.overlay.direct(objects, entry)? {
            found @ Object::Dictionary(_) => found,
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Integer(_)
            | Object::Name(_)
            | Object::Null
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_) => Object::Dictionary(Vec::new()),
        };
        Ok((trailer, id, info))
    }

    /// The document information dictionary: text strings as text, dates as ISO 8601, a date that
    /// is not one empty (ISO 32000-2:2020, 7.9.2.2, 7.9.4, 14.3.3).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn info_dictionary(&self) -> Result<InfoDictionary, PdfError> {
        let objects = self.store.objects("Document::info_dictionary");
        let info = self.info(objects)?.2;
        let mut values = Vec::new();
        for (key, date) in INFO_KEYS {
            values.push(match self.overlay.entry(objects, &info, key)? {
                Object::String(bytes) if date => iso_date(&text(&bytes)).unwrap_or_default(),
                Object::String(bytes) => text(&bytes),
                Object::Array(_)
                | Object::Boolean(_)
                | Object::Dictionary(_)
                | Object::Integer(_)
                | Object::Name(_)
                | Object::Null
                | Object::Real(_)
                | Object::Reference(_)
                | Object::Stream(..) => String::new(),
            });
        }
        let mut fields = values.into_iter();
        let mut next = || fields.next().unwrap_or_default();
        Ok(InfoDictionary {
            author: next(),
            created: next(),
            creator: next(),
            keywords: next(),
            modified: next(),
            producer: next(),
            subject: next(),
            title: next(),
        })
    }

    /// Inserts a blank page of `width` × `height` points, A4 for one not given, before the page at
    /// `index` or after the last (ISO 32000-2:2020, 7.7.3).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn insert_blank_page(&self, options: &InsertBlankPageOptions) -> Result<Page, PdfError> {
        /// The operation.
        const INSERT: &str = "Document::insert_blank_page";
        let invalid = || PdfError::new(ErrorCode::InvalidArgument, INSERT);
        let [width, height] = [(options.width, A4[0]), (options.height, A4[1])]
            .map(|(given, default)| given.unwrap_or(default));
        if ![width, height]
            .iter()
            .all(|side| side.is_finite() && *side > 0.0_f64)
        {
            return Err(invalid());
        }
        let objects = self.store.objects(INSERT);
        let overlay = &*self.overlay;
        let trailer = Object::Dictionary(overlay.trailer(objects)?);
        let Some(Object::Reference(root)) = trailer.take(b"Root") else {
            return Err(objects.malformed());
        };
        let Some(Object::Reference(top)) = overlay.resolve(objects, root)?.take(b"Pages") else {
            return Err(objects.malformed());
        };
        let node = overlay.resolve(objects, top)?;
        let pages = page::count(objects, overlay, &node)?;
        let index = options.index.map_or(pages, u64::from);
        if index > pages {
            return Err(invalid());
        }
        let (mut path, position) = place(objects, overlay, (top, node), index)?;
        let parent = path.last().ok_or_else(|| objects.malformed())?.0;
        let media = Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Real(width),
            Object::Real(height),
        ]);
        let mut entries = vec![
            (b"Type".to_vec(), Object::Name(b"Page".to_vec())),
            (b"Parent".to_vec(), Object::Reference(parent)),
            (b"Resources".to_vec(), Object::Dictionary(Vec::new())),
            (b"MediaBox".to_vec(), media.clone()),
        ];
        // Its own values where an ancestor would give others (7.7.3.4).
        for (key, own) in [
            (b"CropBox".as_slice(), media),
            (b"Rotate", Object::Integer(0)),
        ] {
            if path.iter().any(|ancestor| {
                ancestor
                    .1
                    .get(key)
                    .is_some_and(|found| !matches!(found, Object::Null))
            }) {
                entries.push((key.to_vec(), own));
            }
        }
        let page = overlay.add(Object::Dictionary(entries))?;
        if let Some(last) = path.last_mut() {
            let Object::Array(mut kids) = overlay.entry(objects, &last.1, b"Kids")? else {
                return Err(objects.malformed());
            };
            kids.insert(position.min(kids.len()), Object::Reference(page));
            last.1.set(b"Kids", Some(Object::Array(kids)));
        }
        for (id, mut changed) in path {
            let total = page::count(objects, overlay, &changed)?
                .checked_add(1)
                .and_then(|total| i64::try_from(total).ok())
                .ok_or_else(|| objects.malformed())?;
            changed.set(b"Count", Some(Object::Integer(total)));
            overlay.set(id, changed);
        }
        Ok(Page::new(page, &self.store, &self.overlay))
    }

    /// Inserts pages of `from`, which may be this document.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn insert_pages(&self, from: &Self, options: &InsertPagesOptions) -> Result<(), PdfError> {
        let _: &Self = from;
        let _: &InsertPagesOptions = options;
        Err(PdfError::new(
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
    pub fn insert_toc(&self, options: &InsertTocOptions) -> Result<(), PdfError> {
        let _: &InsertTocOptions = options;
        Err(PdfError::new(
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
    pub fn label_ranges(&self) -> Result<LabelRanges, PdfError> {
        Err(PdfError::new(
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
    pub fn language(&self) -> Result<String, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::language",
        ))
    }

    /// The layers.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn layers(&self) -> Result<Layers, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Document::layers"))
    }

    /// An object as PDF syntax, from the body, an object stream or a later update; a stream gives
    /// its dictionary; a free or missing object is `null` (ISO 32000-2:2020, 7.3, 7.5.6, 7.5.7).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::Unsupported`].
    pub fn object(&self, options: &ObjectOptions) -> Result<String, PdfError> {
        self.store.objects("Document::object").object(options.id)
    }

    /// Opens `source`, reading its header and cross-reference data (ISO 32000-2:2020, 7.5); a
    /// damaged table is rebuilt, bytes before the header tolerated, each with a warning, unless
    /// `strict`. An encrypted file opens with its user or owner password, or none when the user
    /// password is empty (ISO 32000-2:2020, 7.6.4); the private key is not used yet.
    ///
    /// A forward-only source fails with [`ErrorCode::NotSeekable`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidHeader`], [`ErrorCode::InvalidPassword`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`], [`ErrorCode::Unsupported`].
    pub fn open(
        source: &Source,
        policy: Option<&ResourcePolicy>,
        options: &OpenOptions,
    ) -> Result<Self, PdfError> {
        if source.forward_only() {
            return Err(PdfError::new(ErrorCode::NotSeekable, "Document::open"));
        }
        let store = Store::open(source, policy, options.strict, &options.password)?;
        let next = store.objects("Document::open").size()?;
        Ok(Self {
            overlay: Arc::new(Overlay::new(next)),
            part: false,
            store: Arc::new(store),
        })
    }

    /// The output intent, when there is one.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn output_intent(&self) -> Result<Option<OutputIntent>, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::output_intent",
        ))
    }

    /// The page at `index` of the page tree (ISO 32000-2:2020, 7.7.3); past the last is
    /// [`ErrorCode::InvalidArgument`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::Unsupported`].
    pub fn page(&self, options: &PageOptions) -> Result<Page, PdfError> {
        page::find(&self.store, &self.overlay, options.index)
    }

    /// The number of pages: the root page tree node's `Count` (ISO 32000-2:2020, 7.7.3.2).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::Unsupported`].
    pub fn page_count(&self) -> Result<u32, PdfError> {
        let objects = self.store.objects("Document::page_count");
        let overlay = &*self.overlay;
        let count = if overlay.is_changed() {
            let trailer = Object::Dictionary(overlay.trailer(objects)?);
            let catalog = overlay.entry(objects, &trailer, b"Root")?;
            let pages = overlay.entry(objects, &catalog, b"Pages")?;
            match overlay.entry(objects, &pages, b"Count")? {
                Object::Integer(found) => Some(found),
                Object::Array(_)
                | Object::Boolean(_)
                | Object::Dictionary(_)
                | Object::Name(_)
                | Object::Null
                | Object::Real(_)
                | Object::Reference(_)
                | Object::Stream(..)
                | Object::String(_) => None,
            }
        } else {
            // Read in place: a catalog or root node can be too large to hold.
            let catalog = objects.dictionary(objects.get(objects.trailer()?, b"Root")?)?;
            let pages = objects.dictionary(objects.get(catalog, b"Pages")?)?;
            match objects.get(pages, b"Count")? {
                Some(Value::Reference(id)) => objects.resolve(id)?.integer(),
                found => found.as_ref().and_then(Value::integer),
            }
        };
        count
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| objects.malformed())
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
    ) -> Result<PendingSignature, PdfError> {
        let _: &Sink = sink;
        let _: Option<&dyn Progress> = progress;
        let _: &PrepareSignatureOptions = options;
        Err(PdfError::new(
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
    ) -> Result<(), PdfError> {
        let _: Option<&dyn Progress> = progress;
        let _: &PrintOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Document::print"))
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
    ) -> Result<(), PdfError> {
        let _: Option<&dyn Progress> = progress;
        let _: &RecognizeOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::recognize",
        ))
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
    ) -> Result<u64, PdfError> {
        let _: Option<&dyn Progress> = progress;
        let _: &ReplaceTextOptions = options;
        Err(PdfError::new(
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
    pub fn revert(&self, options: &RevertOptions) -> Result<(), PdfError> {
        let _: &RevertOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Document::revert"))
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
    ) -> Result<(), PdfError> {
        let _: Option<&dyn Progress> = progress;
        let _: &SanitizeOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::sanitize",
        ))
    }

    /// Writes the document; the session stays open. [`SaveMode::Rewrite`] writes only the objects
    /// reachable from the trailer, renumbered, unchanged streams byte for byte, with a file
    /// identifier from the content (ISO 32000-2:2020, 7.5, 14.4); an encrypted source is written
    /// decrypted with [`Cipher::Unencrypted`]. Other modes and options not at their defaults fail
    /// with [`ErrorCode::NotImplemented`], no pages with [`ErrorCode::InvalidArgument`].
    ///
    /// A failed save to a file leaves the previous file intact.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::Cancelled`], [`ErrorCode::DocumentClosed`], [`ErrorCode::IncrementalRefused`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::LossRefused`], [`ErrorCode::Malformed`], [`ErrorCode::NotSeekable`], [`ErrorCode::SignaturesWouldBreak`].
    pub fn save(
        &self,
        sink: &Sink,
        progress: Option<&dyn Progress>,
        options: &SaveOptions,
    ) -> Result<SaveReport, PdfError> {
        /// The operation.
        const SAVE: &str = "Document::save";
        let objects = self.store.objects(SAVE);
        let encrypted = objects.security().is_some() && !self.part;
        let defaults = !options.allow_loss
            && options.color_space.is_none()
            && options.image_quality.is_none()
            && options.image_resolution.is_none()
            && options.mode == SaveMode::Rewrite
            && options.optimizations.is_empty()
            && options.owner_password.is_empty()
            && options.recipients.is_empty()
            && options.restrictions.is_empty()
            && options.standards.is_empty()
            && options.user_password.is_empty()
            && options.version.is_none()
            && options
                .cipher
                .map_or(!encrypted, |cipher| cipher == Cipher::Unencrypted);
        let result = if defaults {
            self.version().and_then(|version| {
                writer::rewrite(
                    &Rewrite {
                        break_signatures: options.break_signatures,
                        objects,
                        overlay: &self.overlay,
                        part: self.part,
                        progress,
                        version,
                    },
                    sink,
                )
            })
        } else {
            Err(PdfError::new(ErrorCode::NotImplemented, SAVE))
        };
        let (broken_signatures, length) = sink.finish(result, SAVE)?;
        Ok(SaveReport {
            broken_signatures,
            length,
        })
    }

    /// The document-level JavaScript; never run.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn scripts(&self) -> Result<Scripts, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::scripts",
        ))
    }

    /// Finds text or a regular expression.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn search(&self, options: &SearchOptions) -> Result<Hits, PdfError> {
        let _: &SearchOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Document::search"))
    }

    /// Makes the document a collection (ISO 32000-2:2020, 12.3.5), or a plain document again.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn set_collection(&self, options: &SetCollectionOptions) -> Result<(), PdfError> {
        let _: &SetCollectionOptions = options;
        Err(PdfError::new(
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
    pub fn set_destination(&self, options: &SetDestinationOptions) -> Result<(), PdfError> {
        let _: &SetDestinationOptions = options;
        Err(PdfError::new(
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
    pub fn set_info_dictionary(&self, options: &SetInfoDictionaryOptions) -> Result<(), PdfError> {
        /// The operation.
        const SET: &str = "Document::set_info_dictionary";
        let given = [
            &options.author,
            &options.created,
            &options.creator,
            &options.keywords,
            &options.modified,
            &options.producer,
            &options.subject,
            &options.title,
        ];
        let mut changes = Vec::new();
        for (&(key, date), value) in INFO_KEYS.iter().zip(given) {
            let Some(found) = value.as_deref() else {
                continue;
            };
            let bytes = match (found.is_empty(), date) {
                (true, _) => None,
                (false, true) => Some(
                    pdf_date(found)
                        .ok_or_else(|| PdfError::new(ErrorCode::InvalidArgument, SET))?
                        .into_bytes(),
                ),
                (false, false) => Some(text_string(found)),
            };
            changes.push((key, bytes.map(Object::String)));
        }
        let objects = self.store.objects(SET);
        let (mut trailer, id, mut info) = self.info(objects)?;
        for (key, value) in changes {
            info.set(key, value);
        }
        let empty = matches!(&info, Object::Dictionary(entries) if entries.is_empty());
        match id {
            Some(found) if !empty => self.overlay.set(found, info),
            _ if empty && trailer.iter().all(|entry| entry.0 != b"Info") => {}
            _ => {
                trailer.retain(|entry| entry.0 != b"Info");
                if !empty {
                    let added = self.overlay.add(info)?;
                    trailer.push((b"Info".to_vec(), Object::Reference(added)));
                }
                self.overlay.set_trailer(trailer);
            }
        }
        Ok(())
    }

    /// Labels pages from `start` up to the next label range.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn set_label_range(&self, options: &SetLabelRangeOptions) -> Result<(), PdfError> {
        let _: &SetLabelRangeOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::set_label_range",
        ))
    }

    /// Sets or removes the document's language.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn set_language(&self, options: &SetLanguageOptions) -> Result<(), PdfError> {
        let _: &SetLanguageOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::set_language",
        ))
    }

    /// Replaces an object from PDF syntax.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn set_object(&self, options: &SetObjectOptions) -> Result<(), PdfError> {
        let _: &SetObjectOptions = options;
        Err(PdfError::new(
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
    ) -> Result<(), PdfError> {
        let _: &Source = profile;
        let _: &SetOutputIntentOptions = options;
        Err(PdfError::new(
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
    pub fn set_script(&self, options: &SetScriptOptions) -> Result<(), PdfError> {
        let _: &SetScriptOptions = options;
        Err(PdfError::new(
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
    ) -> Result<(), PdfError> {
        let _: &Source = source;
        let _: &SetStreamDataOptions = options;
        Err(PdfError::new(
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
    pub fn set_viewer_settings(&self, options: &SetViewerSettingsOptions) -> Result<(), PdfError> {
        let _: &SetViewerSettingsOptions = options;
        Err(PdfError::new(
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
    pub fn set_xfa(
        &self,
        source: Option<&Source>,
        options: &SetXfaOptions,
    ) -> Result<(), PdfError> {
        let _: Option<&Source> = source;
        let _: &SetXfaOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::set_xfa",
        ))
    }

    /// Sets or removes the XMP metadata.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn set_xmp(&self, options: &SetXmpOptions) -> Result<(), PdfError> {
        let _: &SetXmpOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::set_xmp",
        ))
    }

    /// The signatures.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn signatures(&self) -> Result<Signatures, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::signatures",
        ))
    }

    /// Splits the pages into parts of `count` pages, or from each of `starts` to the next, the
    /// last to the end; pages before the first start are in no part. A part holds only its pages,
    /// their inherited attributes made their own, under a page tree of its own, with the info
    /// dictionary, output intents and layers; links between its pages lead to them
    /// (ISO 32000-2:2020, 7.7.3, 8.11.4, 12.3.2, 14.3.3, 14.11.5).
    ///
    /// Encryption, form fields, a structure tree, bookmarks, attachments, document scripts,
    /// threads, associated files, any other catalog entry a part neither keeps nor rewrites, and
    /// a link or open action to a page in another part fail with [`ErrorCode::LossRefused`]
    /// unless `allow_loss`, which leaves them out; such a link keeps its appearance. A `count`
    /// of 0 and `starts` empty, not ascending or past the last page fail with
    /// [`ErrorCode::InvalidArgument`];
    /// [`SplitBy::Bookmarks`] and [`SplitBy::Size`] with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::LossRefused`], [`ErrorCode::Malformed`].
    pub fn split(&self, options: &SplitOptions) -> Result<Documents, PdfError> {
        /// The operation.
        const SPLIT: &str = "Document::split";
        let invalid = || PdfError::new(ErrorCode::InvalidArgument, SPLIT);
        let (count, starts) = match options.by {
            SplitBy::Pages if options.count == 0 => return Err(invalid()),
            SplitBy::Pages => (options.count, Vec::new()),
            SplitBy::Ranges => (0, options.starts.clone()),
            SplitBy::Bookmarks | SplitBy::Size => {
                return Err(PdfError::new(ErrorCode::NotImplemented, SPLIT));
            }
        };
        let total = self.page_count()?;
        if count == 0
            && (!starts.is_sorted_by(|before, after| before < after)
                || starts.last().is_none_or(|&last| last >= total))
        {
            return Err(invalid());
        }
        let objects = self.store.objects(SPLIT);
        if !options.allow_loss {
            part::refuse(
                objects,
                &self.overlay,
                objects.security().is_some() && !self.part,
            )?;
        }
        let pages = Pages::read(objects, &self.overlay)?;
        if pages.count() != total {
            return Err(objects.malformed());
        }
        Ok(Documents {
            parts: Arc::new(Mutex::new(Parts {
                allow_loss: options.allow_loss,
                count,
                next: 0,
                pages: Arc::new(pages),
                source: self.clone(),
                starts,
            })),
        })
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
    ) -> Result<(), PdfError> {
        let _: Option<&Source> = image;
        let _: Option<&dyn Progress> = progress;
        let _: &StampOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Document::stamp"))
    }

    /// Writes a stream's stored bytes, or with `decode` its data with its filters undone
    /// (ISO 32000-2:2020, 7.4); [`ErrorCode::InvalidArgument`] when `id` is missing or not a
    /// stream, [`ErrorCode::Unsupported`] for an image filter.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`], [`ErrorCode::Unsupported`].
    pub fn stream_data(&self, sink: &Sink, options: &StreamDataOptions) -> Result<(), PdfError> {
        /// The operation.
        const STREAM_DATA: &str = "Document::stream_data";
        sink.finish(
            stream::write(
                self.store.objects(STREAM_DATA),
                options.id,
                options.decode,
                sink,
            ),
            STREAM_DATA,
        )
    }

    /// Builds a structure tree with reading order from the layout.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::Busy`], [`ErrorCode::Cancelled`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::LossRefused`], [`ErrorCode::Malformed`].
    pub fn tag(
        &self,
        progress: Option<&dyn Progress>,
        options: &TagOptions,
    ) -> Result<(), PdfError> {
        let _: Option<&dyn Progress> = progress;
        let _: &TagOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Document::tag"))
    }

    /// Reads everything and reports each deviation from a standard.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn validate(&self, options: &ValidateOptions) -> Result<Deviations, PdfError> {
        let _: &ValidateOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::validate",
        ))
    }

    /// The certificates and revocation data stored for long-term validation.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn verification_data(&self) -> Result<VerificationItems, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::verification_data",
        ))
    }

    /// The effective version: the header's, or the catalog's `Version` when later
    /// (ISO 32000-2:2020, 7.5.2, 7.7.2).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn version(&self) -> Result<Version, PdfError> {
        let objects = self.store.objects("Document::version");
        let header = objects.version();
        let trailer = Object::Dictionary(self.overlay.trailer(objects)?);
        let Some(Object::Reference(root)) = trailer.take(b"Root") else {
            return Ok(header);
        };
        let catalog = self.overlay.resolve(objects, root)?;
        let named = match self.overlay.entry(objects, &catalog, b"Version")? {
            Object::Name(name) => match *name.as_slice() {
                [major @ (b'1' | b'2'), b'.', minor @ b'0'..=b'9'] => Some(Version {
                    major: major.wrapping_sub(b'0'),
                    minor: minor.wrapping_sub(b'0'),
                }),
                _ => None,
            },
            Object::Array(_)
            | Object::Boolean(_)
            | Object::Dictionary(_)
            | Object::Integer(_)
            | Object::Null
            | Object::Real(_)
            | Object::Reference(_)
            | Object::Stream(..)
            | Object::String(_) => None,
        };
        Ok(named
            .filter(|found| (found.major, found.minor) > (header.major, header.minor))
            .unwrap_or(header))
    }

    /// How viewers open the document.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn viewer_settings(&self) -> Result<ViewerSettings, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::viewer_settings",
        ))
    }

    /// The deviations tolerated so far, up to the policy's limit.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::DocumentClosed`].
    pub fn warnings(&self) -> Result<Deviations, PdfError> {
        Ok(Deviations::new(self.store.warnings()))
    }

    /// Writes the XFA form's XML; nothing when there is none.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn xfa(&self, sink: &Sink, options: &XfaOptions) -> Result<(), PdfError> {
        let _: &Sink = sink;
        let _: &XfaOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Document::xfa"))
    }

    /// The XMP metadata packet; empty when there is none.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn xmp(&self) -> Result<String, PdfError> {
        Err(PdfError::new(ErrorCode::NotImplemented, "Document::xmp"))
    }

    /// The cross-reference entries.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::Malformed`].
    pub fn xref_entries(&self) -> Result<XrefEntries, PdfError> {
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Document::xref_entries",
        ))
    }
}

impl Documents {
    /// The next document, or `None` after the last; a part is made only when asked for, and
    /// unsaved, never encrypted.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BudgetExceeded`], [`ErrorCode::DocumentClosed`], [`ErrorCode::Io`], [`ErrorCode::LimitExceeded`], [`ErrorCode::LossRefused`], [`ErrorCode::Malformed`].
    pub fn next(&self) -> Result<Option<Document>, PdfError> {
        // Held until the part is made, so a failed part is not skipped.
        let mut parts = self.parts.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(range) = parts.range() else {
            return Ok(None);
        };
        let source = parts.source.clone();
        let part = Part::new((&source.overlay, &source.store), &parts.pages, range)?;
        if !parts.allow_loss {
            part.refuse_links()?;
        }
        let (catalog, next) = (part.catalog(parts.allow_loss)?, part.next());
        parts.next = parts.next.saturating_add(1);
        drop(parts);
        let objects = source.store.objects("Documents::next");
        let info = Object::Dictionary(source.overlay.trailer(objects)?).take(b"Info");
        let overlay = Overlay::with_view(Box::new(part), next);
        let root = overlay.add(Object::Dictionary(catalog))?;
        let mut trailer = vec![(b"Root".to_vec(), Object::Reference(root))];
        trailer.extend(info.map(|found| (b"Info".to_vec(), found)));
        overlay.set_trailer(trailer);
        Ok(Some(Document {
            overlay: Arc::new(overlay),
            part: true,
            store: Arc::clone(&source.store),
        }))
    }
}

impl Parts {
    /// The pages of the next part, first and one past the last; `None` after the last.
    fn range(&self) -> Option<(u32, u32)> {
        let total = self.pages.count();
        if self.count > 0 {
            let first = self.next.checked_mul(self.count)?;
            return (first < total).then(|| (first, first.saturating_add(self.count).min(total)));
        }
        let at = usize::try_from(self.next).ok()?;
        let first = *self.starts.get(at)?;
        let end = self
            .starts
            .get(at.saturating_add(1))
            .copied()
            .unwrap_or(total);
        Some((first, end))
    }
}

/// The page tree nodes from `top` down to the one a page inserted before the page at `index`, or
/// after the last, goes into, and its place among that node's kids (ISO 32000-2:2020, 7.7.3.2).
fn place(
    objects: Objects<'_>,
    overlay: &Overlay,
    top: (ObjectId, Object),
    index: u64,
) -> Result<(Vec<(ObjectId, Object)>, usize), PdfError> {
    if index == page::count(objects, overlay, &top.1)? {
        return last(objects, overlay, vec![top]);
    }
    let mut path = vec![top];
    let mut left = index;
    'descend: loop {
        if path.len() > usize::try_from(objects.depth_limit()).unwrap_or(usize::MAX) {
            return Err(PdfError::new(ErrorCode::LimitExceeded, objects.operation()));
        }
        let node = path.last().map_or(Object::Null, |entry| entry.1.clone());
        let Object::Array(kids) = overlay.entry(objects, &node, b"Kids")? else {
            return Err(objects.malformed());
        };
        let length = kids.len();
        for (position, kid) in kids.into_iter().enumerate() {
            let Object::Reference(id) = kid else {
                return Err(objects.malformed());
            };
            let child = overlay.resolve(objects, id)?;
            let pages = page::count(objects, overlay, &child)?;
            if left < pages {
                if child.get(b"Kids").is_none() {
                    return Ok((path, position));
                }
                if path.iter().any(|entry| entry.0 == id) {
                    return Err(objects.malformed());
                }
                path.push((id, child));
                continue 'descend;
            }
            left = left.saturating_sub(pages);
        }
        if left > 0 {
            return Err(objects.malformed());
        }
        return Ok((path, length));
    }
}

/// The page tree nodes from the last of `path` down along last kids to the one a page appended
/// goes into, and its place after that node's kids, reading only last kids (ISO 32000-2:2020,
/// 7.7.3.2).
fn last(
    objects: Objects<'_>,
    overlay: &Overlay,
    mut path: Vec<(ObjectId, Object)>,
) -> Result<(Vec<(ObjectId, Object)>, usize), PdfError> {
    loop {
        if path.len() > usize::try_from(objects.depth_limit()).unwrap_or(usize::MAX) {
            return Err(PdfError::new(ErrorCode::LimitExceeded, objects.operation()));
        }
        let node = path.last().map_or(Object::Null, |entry| entry.1.clone());
        let Object::Array(kids) = overlay.entry(objects, &node, b"Kids")? else {
            return Err(objects.malformed());
        };
        let length = kids.len();
        let Some(kid) = kids.into_iter().next_back() else {
            return Ok((path, length));
        };
        let Object::Reference(id) = kid else {
            return Err(objects.malformed());
        };
        let child = overlay.resolve(objects, id)?;
        if child.get(b"Kids").is_none() {
            return Ok((path, length));
        }
        if path.iter().any(|entry| entry.0 == id) {
            return Err(objects.malformed());
        }
        path.push((id, child));
    }
}
