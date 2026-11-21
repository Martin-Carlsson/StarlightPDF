//! JavaScript binding.

#![forbid(unsafe_code)]

extern crate alloc;

use alloc::sync::Arc;
use std::io::{self, ErrorKind};

use js_sys::{Array, BigInt, Error, Number, Object, RangeError, Reflect, TypeError, Uint8Array};
use starlightpdf::Version;
use starlightpdf::arrange::{InsertBlankPageOptions, SplitBy, SplitOptions};
use starlightpdf::compliance::Standard;
use starlightpdf::document::{
    CreateOptions, OpenOptions, Optimization, PageOptions, SaveMode, SaveOptions,
};
use starlightpdf::drawing::DrawImageOptions;
use starlightpdf::images::ImageFormat;
use starlightpdf::input::{ReadAt, ReadAtSourceOptions};
use starlightpdf::layers::AddLayerOptions;
use starlightpdf::limits::ResourcePolicyOptions;
use starlightpdf::metadata::SetInfoDictionaryOptions;
use starlightpdf::objects::{ObjectId, ObjectOptions, StreamDataOptions};
use starlightpdf::output::{Append, AppendSinkOptions, MemorySinkOptions};
use starlightpdf::page::{Boundary, BoundaryOptions};
use starlightpdf::progress::Progress;
use starlightpdf::rendering::ColorSpace;
use starlightpdf::security::{Cipher, Permission};
use starlightpdf::{compliance, document, input, layers, limits, output, page};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::{JsValue, wasm_bindgen};

#[wasm_bindgen(raw_module = "../table.js")]
extern "C" {
    /// Appends a copy of `bytes` with callback `id`.
    #[wasm_bindgen(catch)]
    fn append(id: u32, bytes: &[u8]) -> Result<(), JsValue>;

    /// Reads input `id` from `offset` into `buffer`; the count read.
    #[wasm_bindgen(catch)]
    fn read(id: u32, offset: u64, buffer: &mut [u8]) -> Result<usize, JsValue>;

    /// Forgets input or callback `id`.
    fn release(id: u32);

    /// Reports progress with callback `id`; `false` cancels.
    #[wasm_bindgen(catch)]
    fn update(id: u32, done: u64, total: u64) -> Result<bool, JsValue>;
}

/// Each cipher and its name.
const CIPHERS: [(Cipher, &str); 4] = [
    (Cipher::Aes128, "Aes128"),
    (Cipher::Aes256, "Aes256"),
    (Cipher::Rc4, "Rc4"),
    (Cipher::Unencrypted, "Unencrypted"),
];

/// Each color space and its name.
const COLOR_SPACES: [(ColorSpace, &str); 3] = [
    (ColorSpace::Cmyk, "Cmyk"),
    (ColorSpace::Gray, "Gray"),
    (ColorSpace::Rgb, "Rgb"),
];

/// Each image format and its name.
const IMAGE_FORMATS: [(ImageFormat, &str); 36] = [
    (ImageFormat::Arw, "Arw"),
    (ImageFormat::Avif, "Avif"),
    (ImageFormat::Bmp, "Bmp"),
    (ImageFormat::Ccitt, "Ccitt"),
    (ImageFormat::Cr2, "Cr2"),
    (ImageFormat::Cr3, "Cr3"),
    (ImageFormat::Dds, "Dds"),
    (ImageFormat::Dicom, "Dicom"),
    (ImageFormat::Dng, "Dng"),
    (ImageFormat::Dpx, "Dpx"),
    (ImageFormat::Emf, "Emf"),
    (ImageFormat::Eps, "Eps"),
    (ImageFormat::Gif, "Gif"),
    (ImageFormat::Hdr, "Hdr"),
    (ImageFormat::Heif, "Heif"),
    (ImageFormat::Ico, "Ico"),
    (ImageFormat::Jbig2, "Jbig2"),
    (ImageFormat::Jpeg, "Jpeg"),
    (ImageFormat::Jpeg2000, "Jpeg2000"),
    (ImageFormat::JpegXl, "JpegXl"),
    (ImageFormat::Nef, "Nef"),
    (ImageFormat::OpenExr, "OpenExr"),
    (ImageFormat::Orf, "Orf"),
    (ImageFormat::Pcx, "Pcx"),
    (ImageFormat::Png, "Png"),
    (ImageFormat::Pnm, "Pnm"),
    (ImageFormat::Psd, "Psd"),
    (ImageFormat::Qoi, "Qoi"),
    (ImageFormat::Raf, "Raf"),
    (ImageFormat::Rw2, "Rw2"),
    (ImageFormat::Svg, "Svg"),
    (ImageFormat::Tga, "Tga"),
    (ImageFormat::Tiff, "Tiff"),
    (ImageFormat::WebP, "WebP"),
    (ImageFormat::Wmf, "Wmf"),
    (ImageFormat::Xbm, "Xbm"),
];

/// Each save mode and its name.
const MODES: [(SaveMode, &str); 3] = [
    (SaveMode::InPlace, "InPlace"),
    (SaveMode::Incremental, "Incremental"),
    (SaveMode::Rewrite, "Rewrite"),
];

/// Each optimization and its name.
const OPTIMIZATIONS: [(Optimization, &str); 5] = [
    (Optimization::CompressStreams, "CompressStreams"),
    (Optimization::Deduplicate, "Deduplicate"),
    (Optimization::Linearize, "Linearize"),
    (Optimization::ObjectStreams, "ObjectStreams"),
    (Optimization::SubsetFonts, "SubsetFonts"),
];

/// Each permission and its name.
const PERMISSIONS: [(Permission, &str); 7] = [
    (Permission::Annotate, "Annotate"),
    (Permission::Assemble, "Assemble"),
    (Permission::Copy, "Copy"),
    (Permission::Edit, "Edit"),
    (Permission::FillForms, "FillForms"),
    (Permission::Print, "Print"),
    (Permission::PrintHighQuality, "PrintHighQuality"),
];

/// Each split and its name.
const SPLITS: [(SplitBy, &str); 4] = [
    (SplitBy::Bookmarks, "Bookmarks"),
    (SplitBy::Pages, "Pages"),
    (SplitBy::Ranges, "Ranges"),
    (SplitBy::Size, "Size"),
];

/// Each standard and its name.
const STANDARDS: [(Standard, &str); 21] = [
    (Standard::FacturX, "FacturX"),
    (Standard::Pdf, "Pdf"),
    (Standard::PdfA1A, "PdfA1A"),
    (Standard::PdfA1B, "PdfA1B"),
    (Standard::PdfA2A, "PdfA2A"),
    (Standard::PdfA2B, "PdfA2B"),
    (Standard::PdfA2U, "PdfA2U"),
    (Standard::PdfA3A, "PdfA3A"),
    (Standard::PdfA3B, "PdfA3B"),
    (Standard::PdfA3U, "PdfA3U"),
    (Standard::PdfA4, "PdfA4"),
    (Standard::PdfA4E, "PdfA4E"),
    (Standard::PdfA4F, "PdfA4F"),
    (Standard::PdfE1, "PdfE1"),
    (Standard::PdfUa1, "PdfUa1"),
    (Standard::PdfUa2, "PdfUa2"),
    (Standard::PdfVt, "PdfVt"),
    (Standard::PdfX1A, "PdfX1A"),
    (Standard::PdfX3, "PdfX3"),
    (Standard::PdfX4, "PdfX4"),
    (Standard::PdfX6, "PdfX6"),
];

/// A callback in the JavaScript table, by id: a sink's append or a save's progress.
struct Callback {
    /// Its id.
    id: u32,
}

impl Append for Callback {
    fn append(&self, bytes: &[u8]) -> io::Result<()> {
        append(self.id, bytes).map_err(|error| io_error(&error))
    }
}

impl Drop for Callback {
    fn drop(&mut self) {
        release(self.id);
    }
}

impl Progress for Callback {
    fn update(&self, done: u64, total: u64) -> bool {
        // One that throws cancels; the caller's thread rethrows its error.
        update(self.id, done, total).unwrap_or(false)
    }
}

/// A `Uint8Array` or `Blob` in the JavaScript table, by id; a `JsValue` is not thread-safe.
struct Input {
    /// Its id.
    id: u32,
    /// Its size in bytes.
    size: u64,
}

impl Drop for Input {
    fn drop(&mut self) {
        release(self.id);
    }
}

impl ReadAt for Input {
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        read(self.id, offset, buffer).map_err(|error| io_error(&error))
    }

    fn size(&self) -> io::Result<u64> {
        Ok(self.size)
    }
}

/// Input with a size and reads at an offset.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Source(input::Source);

#[wasm_bindgen]
impl Source {
    /// A source over input `id` of the JavaScript table, `size` bytes long.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    #[wasm_bindgen(js_name = fromTable)]
    pub fn from_table(id: u32, size: u64) -> Result<Self, JsValue> {
        let read_at: Arc<dyn ReadAt> = Arc::new(Input { id, size });
        input::Source::from_read_at(&read_at, &ReadAtSourceOptions::default())
            .map(Self)
            .map_err(|error| to_js(&error))
    }

    /// The same source, for `Page::draw_image` to take.
    #[must_use]
    pub fn share(&self) -> Self {
        Self(self.0.clone())
    }
}

/// A cursor over deviations.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Deviations(compliance::Deviations);

#[wasm_bindgen]
impl Deviations {
    /// The next deviation, `offset` a `BigInt`; `null` after the last.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    #[wasm_bindgen(
        unchecked_return_type = "{ clause: string; message: string; objectNumber: number | null; offset: bigint | null; page: number | null } | null"
    )]
    pub fn next(&self) -> Result<JsValue, JsValue> {
        self.0
            .next()
            .map_err(|error| to_js(&error))?
            .map_or(Ok(JsValue::NULL), |deviation| {
                record([
                    ("clause", deviation.clause.into()),
                    ("message", deviation.message.into()),
                    (
                        "objectNumber",
                        deviation.object_number.map_or(JsValue::NULL, JsValue::from),
                    ),
                    (
                        "offset",
                        deviation.offset.map_or(JsValue::NULL, JsValue::from),
                    ),
                    ("page", deviation.page.map_or(JsValue::NULL, JsValue::from)),
                ])
            })
    }
}

/// An open PDF.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Document(document::Document);

/// A cursor over documents.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Documents(document::Documents);

#[wasm_bindgen]
impl Documents {
    /// The next document; `undefined` after the last.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    pub fn next(&self) -> Result<Option<Document>, JsValue> {
        self.0
            .next()
            .map(|found| found.map(Document))
            .map_err(|error| to_js(&error))
    }
}

/// A layer.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Layer(layers::Layer);

#[wasm_bindgen]
impl Layer {
    /// The same layer, for `Page::draw_image` to take.
    #[must_use]
    pub fn share(&self) -> Self {
        Self(self.0.clone())
    }
}

#[wasm_bindgen]
impl Document {
    /// Adds a layer: `name`, `visible`.
    ///
    /// # Errors
    ///
    /// The core's, as details; a `TypeError` for a wrong option.
    #[wasm_bindgen(js_name = addLayer)]
    pub fn add_layer(&self, options: &JsValue) -> Result<Layer, JsValue> {
        let mut made = AddLayerOptions::default();
        made.name = text(options, "name")?.unwrap_or_default();
        made.visible = flag(options, "visible", true)?;
        self.0
            .add_layer(&made)
            .map(Layer)
            .map_err(|error| to_js(&error))
    }

    /// The catalog's id.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    #[wasm_bindgen(unchecked_return_type = "{ generation: number; number: number }")]
    pub fn catalog(&self) -> Result<JsValue, JsValue> {
        let id = self.0.catalog().map_err(|error| to_js(&error))?;
        record([
            ("generation", id.generation.into()),
            ("number", id.number.into()),
        ])
    }

    /// A new document with no pages: `version`, `[major, minor]`.
    ///
    /// # Errors
    ///
    /// The core's, as details; a `TypeError` or `RangeError` for a wrong option.
    pub fn create(options: &JsValue) -> Result<Self, JsValue> {
        let mut made = CreateOptions::default();
        made.version = version(options)?;
        document::Document::create(None, &made)
            .map(Self)
            .map_err(|error| to_js(&error))
    }

    /// The deviations not kept in `warnings`.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    #[wasm_bindgen(js_name = droppedWarnings)]
    pub fn dropped_warnings(&self) -> Result<u64, JsValue> {
        self.0.dropped_warnings().map_err(|error| to_js(&error))
    }

    /// How the document is protected; names as in the core.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    #[wasm_bindgen(
        unchecked_return_type = "{ certificate: boolean; cipher: string; restrictions: string[] }"
    )]
    pub fn encryption(&self) -> Result<JsValue, JsValue> {
        let found = self.0.encryption().map_err(|error| to_js(&error))?;
        let restrictions = found
            .restrictions
            .iter()
            .map(|&permission| named(&PERMISSIONS, permission, "permission").map(JsValue::from))
            .collect::<Result<Array, JsValue>>()?;
        record([
            ("certificate", found.certificate.into()),
            ("cipher", named(&CIPHERS, found.cipher, "cipher")?.into()),
            ("restrictions", restrictions.into()),
        ])
    }

    /// The document information dictionary; an absent entry empty.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    #[wasm_bindgen(
        js_name = infoDictionary,
        unchecked_return_type = "{ author: string; created: string; creator: string; keywords: string; modified: string; producer: string; subject: string; title: string }"
    )]
    pub fn info_dictionary(&self) -> Result<JsValue, JsValue> {
        let info = self.0.info_dictionary().map_err(|error| to_js(&error))?;
        record([
            ("author", info.author.into()),
            ("created", info.created.into()),
            ("creator", info.creator.into()),
            ("keywords", info.keywords.into()),
            ("modified", info.modified.into()),
            ("producer", info.producer.into()),
            ("subject", info.subject.into()),
            ("title", info.title.into()),
        ])
    }

    /// Inserts a blank page: `height`, `index`, `width`.
    ///
    /// # Errors
    ///
    /// The core's, as details; a `TypeError` or `RangeError` for a wrong option.
    #[wasm_bindgen(js_name = insertBlankPage)]
    pub fn insert_blank_page(&self, options: &JsValue) -> Result<Page, JsValue> {
        let mut made = InsertBlankPageOptions::default();
        made.height = real(options, "height")?;
        made.index = whole(options, "index")?;
        made.width = real(options, "width")?;
        self.0
            .insert_blank_page(&made)
            .map(Page)
            .map_err(|error| to_js(&error))
    }

    /// Object `number` `generation` as PDF syntax.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    pub fn object(&self, number: u32, generation: u16) -> Result<String, JsValue> {
        let mut options = ObjectOptions::default();
        options.id = object_id(number, generation);
        self.0.object(&options).map_err(|error| to_js(&error))
    }

    /// Opens `source` with `policy`, which it takes (see `ResourcePolicy::share`), `password` and `strict`.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    pub fn open(
        source: &Source,
        policy: Option<ResourcePolicy>,
        password: String,
        strict: bool,
    ) -> Result<Self, JsValue> {
        let mut options = OpenOptions::default();
        options.password = password;
        options.strict = strict;
        let core_policy = policy.map(|held| held.0);
        document::Document::open(&source.0, core_policy.as_ref(), &options)
            .map(Self)
            .map_err(|error| to_js(&error))
    }

    /// The page at `index`.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    pub fn page(&self, index: u32) -> Result<Page, JsValue> {
        let mut options = PageOptions::default();
        options.index = index;
        self.0
            .page(&options)
            .map(Page)
            .map_err(|error| to_js(&error))
    }

    /// The number of pages.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    #[wasm_bindgen(js_name = pageCount)]
    pub fn page_count(&self) -> Result<u32, JsValue> {
        self.0.page_count().map_err(|error| to_js(&error))
    }

    /// Saves to `sink`, reporting to callback `progress` of the JavaScript table, with the `SaveOptions` fields of
    /// `options`; `length` a `BigInt`.
    ///
    /// # Errors
    ///
    /// The core's, as details; a `TypeError` or `RangeError` for a wrong option.
    #[wasm_bindgen(unchecked_return_type = "{ brokenSignatures: string[]; length: bigint }")]
    pub fn save(
        &self,
        sink: &Sink,
        progress: Option<u32>,
        options: &JsValue,
    ) -> Result<JsValue, JsValue> {
        // First, so the table forgets the callback whatever fails.
        let callback = progress.map(|id| Callback { id });
        let settings = save_options(options)?;
        let report = self
            .0
            .save(
                &sink.0,
                callback.as_ref().map::<&dyn Progress, _>(|held| held),
                &settings,
            )
            .map_err(|error| to_js(&error))?;
        let broken: Array = report
            .broken_signatures
            .into_iter()
            .map(JsValue::from)
            .collect();
        record([
            ("brokenSignatures", broken.into()),
            ("length", report.length.into()),
        ])
    }

    /// Sets the given entries of the document information dictionary; empty removes one.
    ///
    /// # Errors
    ///
    /// The core's, as details; a `TypeError` for a wrong option.
    #[wasm_bindgen(js_name = setInfoDictionary)]
    pub fn set_info_dictionary(&self, options: &JsValue) -> Result<(), JsValue> {
        let mut made = SetInfoDictionaryOptions::default();
        made.author = text(options, "author")?;
        made.created = text(options, "created")?;
        made.creator = text(options, "creator")?;
        made.keywords = text(options, "keywords")?;
        made.modified = text(options, "modified")?;
        made.producer = text(options, "producer")?;
        made.subject = text(options, "subject")?;
        made.title = text(options, "title")?;
        self.0
            .set_info_dictionary(&made)
            .map_err(|error| to_js(&error))
    }

    /// Splits the pages into new documents: `allowLoss`, `by`, `count`, `starts`.
    ///
    /// # Errors
    ///
    /// The core's, as details; a `TypeError` or `RangeError` for a wrong option.
    pub fn split(&self, options: &JsValue) -> Result<Documents, JsValue> {
        let mut made = SplitOptions::default();
        made.allow_loss = flag(options, "allowLoss", false)?;
        made.by = choice(options, "by", &SPLITS, "split")?.unwrap_or_default();
        made.count = whole(options, "count")?.unwrap_or_default();
        made.starts = list(options, "starts")?
            .iter()
            .map(|start| integer(start, "starts"))
            .collect::<Result<_, _>>()?;
        self.0
            .split(&made)
            .map(Documents)
            .map_err(|error| to_js(&error))
    }

    /// Writes the data of stream `number` `generation` to `sink`, decoded if `decode`.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    #[wasm_bindgen(js_name = streamData)]
    pub fn stream_data(
        &self,
        sink: &Sink,
        number: u32,
        generation: u16,
        decode: bool,
    ) -> Result<(), JsValue> {
        let mut options = StreamDataOptions::default();
        options.decode = decode;
        options.id = object_id(number, generation);
        self.0
            .stream_data(&sink.0, &options)
            .map_err(|error| to_js(&error))
    }

    /// The deviations tolerated so far.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    pub fn warnings(&self) -> Result<Deviations, JsValue> {
        self.0
            .warnings()
            .map(Deviations)
            .map_err(|error| to_js(&error))
    }
}

/// A page.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Page(page::Page);

#[wasm_bindgen]
impl Page {
    /// Boundary `kind`, a `Boundary` name.
    ///
    /// # Errors
    ///
    /// The core's, as details; a `TypeError` for an unknown `kind`.
    #[wasm_bindgen(
        unchecked_return_type = "{ height: number; width: number; x: number; y: number }"
    )]
    pub fn boundary(&self, kind: &str) -> Result<JsValue, JsValue> {
        let mut options = BoundaryOptions::default();
        options.kind = match kind {
            "Art" => Boundary::Art,
            "Bleed" => Boundary::Bleed,
            "Crop" => Boundary::Crop,
            "Media" => Boundary::Media,
            "Trim" => Boundary::Trim,
            _ => return Err(TypeError::new(&format!("no boundary '{kind}'")).into()),
        };
        let rect = self.0.boundary(&options).map_err(|error| to_js(&error))?;
        record([
            ("height", rect.height.into()),
            ("width", rect.width.into()),
            ("x", rect.x.into()),
            ("y", rect.y.into()),
        ])
    }

    /// Draws `image`, with soft mask `mask` and in `layer`, which it takes (see `Source::share`), with the
    /// `DrawImageOptions` fields of `options`.
    ///
    /// # Errors
    ///
    /// The core's, as details; a `TypeError` or `RangeError` for a wrong option.
    #[wasm_bindgen(js_name = drawImage)]
    pub fn draw_image(
        &self,
        image: &Source,
        mask: Option<Source>,
        layer: Option<Layer>,
        options: &JsValue,
    ) -> Result<(), JsValue> {
        let mut made = DrawImageOptions::default();
        made.format = choice(options, "format", &IMAGE_FORMATS, "image format")?;
        made.frame = whole(options, "frame")?.unwrap_or_default();
        made.height = real(options, "height")?;
        made.ignore_orientation = flag(options, "ignoreOrientation", false)?;
        made.opacity = real(options, "opacity")?;
        made.width = real(options, "width")?;
        made.x = real(options, "x")?.unwrap_or_default();
        made.y = real(options, "y")?.unwrap_or_default();
        let core_mask = mask.map(|held| held.0);
        let core_layer = layer.map(|held| held.0);
        self.0
            .draw_image(&image.0, core_mask.as_ref(), core_layer.as_ref(), &made)
            .map_err(|error| to_js(&error))
    }

    /// The rotation in degrees.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    pub fn rotation(&self) -> Result<u16, JsValue> {
        self.0.rotation().map_err(|error| to_js(&error))
    }
}

/// Limits shared by the documents opened with it.
#[wasm_bindgen]
#[derive(Clone, Debug)]
pub struct ResourcePolicy(limits::ResourcePolicy);

#[wasm_bindgen]
impl ResourcePolicy {
    /// A policy with these limits; `undefined` for the default.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    pub fn create(
        decoded_limit: Option<u64>,
        object_limit: Option<u64>,
        warning_limit: Option<u32>,
    ) -> Result<Self, JsValue> {
        let mut options = ResourcePolicyOptions::default();
        options.decoded_limit = decoded_limit;
        options.object_limit = object_limit;
        options.warning_limit = warning_limit;
        limits::ResourcePolicy::new(&options)
            .map(Self)
            .map_err(|error| to_js(&error))
    }

    /// The same policy, for `Document::open` to take.
    #[must_use]
    pub fn share(&self) -> Self {
        self.clone()
    }
}

/// Output that only appends.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Sink(output::Sink);

#[wasm_bindgen]
impl Sink {
    /// A sink over callback `id` of the JavaScript table.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    #[wasm_bindgen(js_name = fromAppend)]
    pub fn from_append(id: u32) -> Result<Self, JsValue> {
        let append: Arc<dyn Append> = Arc::new(Callback { id });
        output::Sink::from_append(&append, &AppendSinkOptions::default())
            .map(Self)
            .map_err(|error| to_js(&error))
    }

    /// A sink that holds its bytes in memory.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    #[wasm_bindgen(js_name = inMemory)]
    pub fn in_memory() -> Result<Self, JsValue> {
        output::Sink::in_memory(&MemorySinkOptions::default())
            .map(Self)
            .map_err(|error| to_js(&error))
    }

    /// Takes the bytes appended so far, leaving the sink empty.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    #[wasm_bindgen(js_name = takeBytes)]
    pub fn take_bytes(&self) -> Result<Vec<u8>, JsValue> {
        self.0.take_bytes().map_err(|error| to_js(&error))
    }
}

/// Calls `starlightpdf::header_version`; `[major, minor]`.
///
/// # Errors
///
/// The core's, as details.
#[wasm_bindgen(js_name = headerVersion)]
pub fn header_version(bytes: &[u8]) -> Result<Vec<u8>, JsValue> {
    starlightpdf::header_version(bytes)
        .map(|version| vec![version.major, version.minor])
        .map_err(|error| to_js(&error))
}

/// Calls `starlightpdf::library_version`.
#[wasm_bindgen(js_name = libraryVersion)]
#[must_use]
pub fn library_version() -> String {
    starlightpdf::library_version().into()
}

/// String `key` of `options` as the value it names in `table`, a `kind`; `None` when undefined.
fn choice<T>(
    options: &JsValue,
    key: &str,
    table: &[(T, &str)],
    kind: &str,
) -> Result<Option<T>, JsValue>
where
    T: Copy,
{
    text(options, key)?
        .map(|name| parsed(table, kind, &name))
        .transpose()
}

/// `operation` in JavaScript's casing: `Document::page_count` as `Document.pageCount`.
fn camel_case(operation: &str) -> String {
    let mut upper = false;
    operation
        .replace("::", ".")
        .chars()
        .filter_map(|letter| {
            if letter == '_' {
                upper = true;
                return None;
            }
            let cased = if upper {
                letter.to_ascii_uppercase()
            } else {
                letter
            };
            upper = false;
            Some(cased)
        })
        .collect()
}

/// A plain object of `error`'s details, its operation in JavaScript's casing; `offset` a `BigInt`.
fn to_js(error: &starlightpdf::PdfError) -> JsValue {
    // This binding's reads fail with these two kinds only.
    let io_cause = error.io_cause().map(|kind| {
        if kind == ErrorKind::NotFound {
            "NotFound"
        } else {
            "Other"
        }
    });
    let fields: [(&str, JsValue); 7] = [
        ("code", error.code().name().into()),
        ("number", error.code().number().into()),
        ("message", error.message().into()),
        ("operation", camel_case(error.operation()).into()),
        (
            "offset",
            error.offset().map_or(JsValue::NULL, JsValue::from),
        ),
        (
            "objectNumber",
            error.object_number().map_or(JsValue::NULL, JsValue::from),
        ),
        ("ioCause", io_cause.map_or(JsValue::NULL, JsValue::from)),
    ];
    record(fields).unwrap_or_else(|failure| failure)
}

/// Option `key` of `options`; `None` when undefined.
fn field(options: &JsValue, key: &str) -> Result<Option<JsValue>, JsValue> {
    let value = Reflect::get(options, &key.into())?;
    Ok((!value.is_undefined()).then_some(value))
}

/// Flag `key` of `options`; `unset` when undefined.
fn flag(options: &JsValue, key: &str, unset: bool) -> Result<bool, JsValue> {
    field(options, key)?.map_or(Ok(unset), |value| {
        value.as_bool().ok_or_else(|| wrong(key, "a boolean"))
    })
}

/// `value`, option `key`, as an unsigned integer that fits `T`; a `RangeError` otherwise.
fn integer<T>(value: &JsValue, key: &str) -> Result<T, JsValue>
where
    T: TryFrom<u64>,
{
    Number::is_safe_integer(value)
        .then(|| BigInt::new(value).ok())
        .flatten()
        .and_then(|big| u64::try_from(big).ok())
        .and_then(|whole| T::try_from(whole).ok())
        .ok_or_else(|| RangeError::new(&format!("{key} must be an integer in range")).into())
}

/// The error a read or callback threw as an I/O error.
fn io_error(error: &JsValue) -> io::Error {
    let (name, message) = error.dyn_ref::<Error>().map_or_else(
        || (String::new(), String::from("a callback failed")),
        |thrown| (String::from(thrown.name()), String::from(thrown.message())),
    );
    let kind = if name == "NotFoundError" {
        ErrorKind::NotFound
    } else {
        ErrorKind::Other
    };
    io::Error::new(kind, message)
}

/// The items of array `key` of `options`; none when undefined.
fn list(options: &JsValue, key: &str) -> Result<Vec<JsValue>, JsValue> {
    field(options, key)?.map_or_else(
        || Ok(Vec::new()),
        |value| {
            value
                .dyn_ref::<Array>()
                .map(Array::to_vec)
                .ok_or_else(|| wrong(key, "an array"))
        },
    )
}

/// `value`'s name in `table`; an `Error` for a `kind` this binding does not know.
fn named<T>(table: &[(T, &'static str)], value: T, kind: &str) -> Result<&'static str, JsValue>
where
    T: Copy + PartialEq,
{
    table
        .iter()
        .find(|&&(known, _)| known == value)
        .map(|&(_, name)| name)
        .ok_or_else(|| Error::new(&format!("unknown {kind}")).into())
}

/// The values the strings of array `key` of `options` name in `table`, each a `kind`.
fn names<T>(
    options: &JsValue,
    key: &str,
    table: &[(T, &str)],
    kind: &str,
) -> Result<Vec<T>, JsValue>
where
    T: Copy,
{
    list(options, key)?
        .iter()
        .map(|item| {
            let name = item
                .as_string()
                .ok_or_else(|| wrong(key, "an array of strings"))?;
            parsed(table, kind, &name)
        })
        .collect()
}

/// Object `number` `generation`.
fn object_id(number: u32, generation: u16) -> ObjectId {
    let mut id = ObjectId::default();
    id.generation = generation;
    id.number = number;
    id
}

/// The value `name` names in `table`; a `TypeError` for no `kind` of that name.
fn parsed<T>(table: &[(T, &str)], kind: &str, name: &str) -> Result<T, JsValue>
where
    T: Copy,
{
    table
        .iter()
        .find(|&&(_, known)| known == name)
        .map(|&(value, _)| value)
        .ok_or_else(|| TypeError::new(&format!("no {kind} '{name}'")).into())
}

/// Number `key` of `options`; `None` when undefined.
fn real(options: &JsValue, key: &str) -> Result<Option<f64>, JsValue> {
    field(options, key)?
        .map(|value| value.as_f64().ok_or_else(|| wrong(key, "a number")))
        .transpose()
}

/// A plain object of `fields`.
fn record<const N: usize>(fields: [(&str, JsValue); N]) -> Result<JsValue, JsValue> {
    let object = Object::new();
    fields
        .into_iter()
        .try_for_each(|(key, value)| Reflect::set(&object, &key.into(), &value).map(drop))?;
    Ok(object.into())
}

/// The `SaveOptions` fields of `options`.
fn save_options(options: &JsValue) -> Result<SaveOptions, JsValue> {
    let mut made = SaveOptions::default();
    made.allow_loss = flag(options, "allowLoss", false)?;
    made.break_signatures = flag(options, "breakSignatures", false)?;
    made.cipher = choice(options, "cipher", &CIPHERS, "cipher")?;
    made.color_space = choice(options, "colorSpace", &COLOR_SPACES, "color space")?;
    made.image_quality = whole(options, "imageQuality")?;
    made.image_resolution = whole(options, "imageResolution")?;
    made.mode = choice(options, "mode", &MODES, "mode")?.unwrap_or_default();
    made.optimizations = names(options, "optimizations", &OPTIMIZATIONS, "optimization")?;
    made.owner_password = text(options, "ownerPassword")?.unwrap_or_default();
    made.recipients = field(options, "recipients")?
        .map(|value| {
            value
                .dyn_ref::<Uint8Array>()
                .map(Uint8Array::to_vec)
                .ok_or_else(|| wrong("recipients", "a Uint8Array"))
        })
        .transpose()?
        .unwrap_or_default();
    made.restrictions = names(options, "restrictions", &PERMISSIONS, "permission")?;
    made.standards = names(options, "standards", &STANDARDS, "standard")?;
    made.user_password = text(options, "userPassword")?.unwrap_or_default();
    made.version = version(options)?;
    Ok(made)
}

/// String `key` of `options`; `None` when undefined.
fn text(options: &JsValue, key: &str) -> Result<Option<String>, JsValue> {
    field(options, key)?
        .map(|value| value.as_string().ok_or_else(|| wrong(key, "a string")))
        .transpose()
}

/// Version `[major, minor]` of `options`; `None` when undefined.
fn version(options: &JsValue) -> Result<Option<Version>, JsValue> {
    field(options, "version")?
        .map(|value| {
            let parts = value
                .dyn_ref::<Array>()
                .filter(|parts| parts.length() == 2)
                .ok_or_else(|| wrong("version", "[major, minor]"))?;
            let mut made = Version::default();
            made.major = integer(&parts.get(0), "version")?;
            made.minor = integer(&parts.get(1), "version")?;
            Ok(made)
        })
        .transpose()
}

/// Unsigned integer `key` of `options`; `None` when undefined.
fn whole<T>(options: &JsValue, key: &str) -> Result<Option<T>, JsValue>
where
    T: TryFrom<u64>,
{
    field(options, key)?
        .map(|value| integer(&value, key))
        .transpose()
}

/// A `TypeError`: `key` must be `kind`.
fn wrong(key: &str, kind: &str) -> JsValue {
    TypeError::new(&format!("{key} must be {kind}")).into()
}
