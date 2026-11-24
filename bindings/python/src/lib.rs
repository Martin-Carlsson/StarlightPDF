//! Python binding.

#![forbid(unsafe_code)]

extern crate alloc;

use alloc::sync::Arc;
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::pybacked::PyBackedBytes;
use pyo3::types::{PyBytes, PyDict, PyList, PyNone, PyTuple};
use starlightpdf::arrange::{InsertBlankPageOptions, SplitBy, SplitOptions};
use starlightpdf::compliance::Standard;
use starlightpdf::document::{
    CreateOptions, OpenOptions, Optimization, PageOptions, SaveMode, SaveOptions,
};
use starlightpdf::drawing::DrawImageOptions;
use starlightpdf::images::{ImageFormat, RasterFormat};
use starlightpdf::input::{FileSourceOptions, ReadAt, ReadAtSourceOptions};
use starlightpdf::limits::ResourcePolicyOptions;
use starlightpdf::metadata::SetInfoDictionaryOptions;
use starlightpdf::objects::{ObjectOptions, StreamDataOptions};
use starlightpdf::output::{Append, AppendSinkOptions, FileSinkOptions, MemorySinkOptions};
use starlightpdf::page::{Boundary, BoundaryOptions};
use starlightpdf::progress::Progress;
use starlightpdf::rendering::{ColorSpace, RenderOptions, RenderUsage};
use starlightpdf::security::{Cipher, Permission};
use starlightpdf::{
    Capability, ErrorCode, Version, compliance, document, input, limits, objects, output, page,
};

/// Each boundary's name.
const BOUNDARIES: [(Boundary, &str); 5] = [
    (Boundary::Art, "Art"),
    (Boundary::Bleed, "Bleed"),
    (Boundary::Crop, "Crop"),
    (Boundary::Media, "Media"),
    (Boundary::Trim, "Trim"),
];

/// Each capability's name.
const CAPABILITIES: [(Capability, &str); 5] = [
    (Capability::Conversion, "Conversion"),
    (Capability::Ocr, "Ocr"),
    (Capability::Printing, "Printing"),
    (Capability::Rendering, "Rendering"),
    (Capability::Signing, "Signing"),
];

/// Each cipher's name.
const CIPHERS: [(Cipher, &str); 4] = [
    (Cipher::Aes128, "Aes128"),
    (Cipher::Aes256, "Aes256"),
    (Cipher::Rc4, "Rc4"),
    (Cipher::Unencrypted, "Unencrypted"),
];

/// Each color space's name.
const COLOR_SPACES: [(ColorSpace, &str); 3] = [
    (ColorSpace::Cmyk, "Cmyk"),
    (ColorSpace::Gray, "Gray"),
    (ColorSpace::Rgb, "Rgb"),
];

/// Each image format's name.
const IMAGE_FORMATS: [(ImageFormat, &str); 37] = [
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
    (ImageFormat::Xpm, "Xpm"),
];

/// Each optimization's name.
const OPTIMIZATIONS: [(Optimization, &str); 5] = [
    (Optimization::CompressStreams, "CompressStreams"),
    (Optimization::Deduplicate, "Deduplicate"),
    (Optimization::Linearize, "Linearize"),
    (Optimization::ObjectStreams, "ObjectStreams"),
    (Optimization::SubsetFonts, "SubsetFonts"),
];

/// Each permission's name.
const PERMISSIONS: [(Permission, &str); 7] = [
    (Permission::Annotate, "Annotate"),
    (Permission::Assemble, "Assemble"),
    (Permission::Copy, "Copy"),
    (Permission::Edit, "Edit"),
    (Permission::FillForms, "FillForms"),
    (Permission::Print, "Print"),
    (Permission::PrintHighQuality, "PrintHighQuality"),
];

/// Each raster format's name.
const RASTER_FORMATS: [(RasterFormat, &str); 4] = [
    (RasterFormat::Heif, "Heif"),
    (RasterFormat::Jpeg, "Jpeg"),
    (RasterFormat::Png, "Png"),
    (RasterFormat::Tiff, "Tiff"),
];

/// Each render usage's name.
const RENDER_USAGES: [(RenderUsage, &str); 2] = [
    (RenderUsage::Display, "Display"),
    (RenderUsage::Print, "Print"),
];

/// Each save mode's name.
const SAVE_MODES: [(SaveMode, &str); 3] = [
    (SaveMode::InPlace, "InPlace"),
    (SaveMode::Incremental, "Incremental"),
    (SaveMode::Rewrite, "Rewrite"),
];

/// Each split's name.
const SPLITS: [(SplitBy, &str); 4] = [
    (SplitBy::Bookmarks, "Bookmarks"),
    (SplitBy::Pages, "Pages"),
    (SplitBy::Ranges, "Ranges"),
    (SplitBy::Size, "Size"),
];

/// Each standard's name.
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

create_exception!(
    starlightpdf,
    PdfError,
    PyException,
    "Every failure; carries `code`, `number`, `message`, `operation`, `offset`, `object_number` and `io_cause`, `None` when unknown."
);

/// A caller's `bytes`.
struct Bytes(PyBackedBytes);

impl ReadAt for Bytes {
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let rest = usize::try_from(offset)
            .ok()
            .and_then(|start| self.0.get(start..))
            .unwrap_or_default();
        let count = rest.len().min(buffer.len());
        buffer
            .iter_mut()
            .zip(rest)
            .for_each(|(byte, source)| *byte = *source);
        Ok(count)
    }

    fn size(&self) -> io::Result<u64> {
        u64::try_from(self.0.len()).map_err(io::Error::other)
    }
}

/// A caller's callable, and what it raised.
struct Callback {
    /// The callable.
    callable: Py<PyAny>,
    /// What it raised, for the operation that called it to raise instead.
    raised: Mutex<Option<PyErr>>,
}

impl Append for Callback {
    fn append(&self, bytes: &[u8]) -> io::Result<()> {
        Python::attach(|py| {
            self.callable
                .call1(py, (PyBytes::new(py, bytes),))
                .map(drop)
        })
        .map_err(|error| {
            self.keep(error);
            io::Error::other("the callable raised")
        })
    }
}

impl Callback {
    /// Keeps `error` for [`Self::take`].
    fn keep(&self, error: PyErr) {
        *self.raised.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
    }

    /// A callback over `callable`.
    const fn new(callable: Py<PyAny>) -> Self {
        Self {
            callable,
            raised: Mutex::new(None),
        }
    }

    /// Takes what it raised.
    fn take(&self) -> Option<PyErr> {
        self.raised
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

impl Progress for Callback {
    fn update(&self, done: u64, total: u64) -> bool {
        Python::attach(|py| self.callable.call1(py, (done, total))?.extract::<bool>(py))
            .unwrap_or_else(|error| {
                self.keep(error);
                false
            })
    }
}

/// Keyword arguments a method takes as `**options`.
struct Keywords<'py>(Option<Bound<'py, PyDict>>);

impl<'py> Keywords<'py> {
    /// The value of `key`, or `default` when not given.
    fn get<T>(&self, key: &str, default: T) -> PyResult<T>
    where
        T: FromPyObjectOwned<'py>,
    {
        self.0
            .as_ref()
            .map(|given| given.get_item(key))
            .transpose()?
            .flatten()
            .map_or(Ok(default), |found| found.extract().map_err(Into::into))
    }

    /// `given`, failing with `TypeError` on a key not in `known`.
    fn new(given: Option<Bound<'py, PyDict>>, known: &[&str]) -> PyResult<Self> {
        for key in given.iter().flat_map(PyDictMethods::keys) {
            let name: String = key.extract()?;
            if !known.contains(&name.as_str()) {
                return Err(PyTypeError::new_err(format!(
                    "unexpected keyword argument '{name}'"
                )));
            }
        }
        Ok(Self(given))
    }
}

/// Input with a size and reads at an offset.
#[pyclass(frozen, module = "starlightpdf")]
struct Source(input::Source);

#[pymethods]
impl Source {
    /// A source over `bytes`, shared, not copied.
    #[staticmethod]
    fn from_bytes(bytes: Bound<'_, PyBytes>) -> PyResult<Self> {
        let read_at: Arc<dyn ReadAt> = Arc::new(Bytes(PyBackedBytes::from(bytes)));
        input::Source::from_read_at(&read_at, &ReadAtSourceOptions::default())
            .map(Self)
            .map_err(|error| to_py(&error))
    }

    /// A source over a file, opened now.
    #[staticmethod]
    fn from_file(file: PathBuf) -> PyResult<Self> {
        let mut options = FileSourceOptions::default();
        options.file = file;
        input::Source::from_file(&options)
            .map(Self)
            .map_err(|error| to_py(&error))
    }
}

/// Output that only appends.
#[pyclass(frozen, module = "starlightpdf")]
struct Sink {
    /// The caller's callable, when it has one.
    callback: Option<Arc<Callback>>,
    /// The sink.
    sink: output::Sink,
}

#[pymethods]
impl Sink {
    /// A sink that calls `append` with each write's bytes.
    #[staticmethod]
    fn from_append(append: Py<PyAny>) -> PyResult<Self> {
        let callback = Arc::new(Callback::new(append));
        let shared: Arc<dyn Append> = Arc::<Callback>::clone(&callback);
        output::Sink::from_append(&shared, &AppendSinkOptions::default())
            .map(|sink| Self {
                callback: Some(callback),
                sink,
            })
            .map_err(|error| to_py(&error))
    }

    /// A sink over a file, replaced only when a save succeeds.
    #[staticmethod]
    fn from_file(file: PathBuf) -> PyResult<Self> {
        let mut options = FileSinkOptions::default();
        options.file = file;
        output::Sink::from_file(&options)
            .map(|sink| Self {
                callback: None,
                sink,
            })
            .map_err(|error| to_py(&error))
    }

    /// A sink that holds its bytes in memory, for `take_bytes`.
    #[staticmethod]
    fn in_memory() -> PyResult<Self> {
        output::Sink::in_memory(&MemorySinkOptions::default())
            .map(|sink| Self {
                callback: None,
                sink,
            })
            .map_err(|error| to_py(&error))
    }

    /// Takes the bytes appended so far, copied into `bytes`, leaving the sink empty.
    fn take_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        self.sink
            .take_bytes()
            .map(|bytes| PyBytes::new(py, &bytes))
            .map_err(|error| to_py(&error))
    }
}

/// Limits shared by the documents opened with it.
#[pyclass(frozen, module = "starlightpdf")]
struct ResourcePolicy(limits::ResourcePolicy);

#[pymethods]
impl ResourcePolicy {
    /// A policy; `None` means a safe library default.
    #[new]
    #[pyo3(signature = (*, decoded_limit = None, object_limit = None, warning_limit = None))]
    fn new(
        decoded_limit: Option<u64>,
        object_limit: Option<u64>,
        warning_limit: Option<u32>,
    ) -> PyResult<Self> {
        let mut options = ResourcePolicyOptions::default();
        options.decoded_limit = decoded_limit;
        options.object_limit = object_limit;
        options.warning_limit = warning_limit;
        limits::ResourcePolicy::new(&options)
            .map(Self)
            .map_err(|error| to_py(&error))
    }
}

/// An indirect object's number and generation.
#[pyclass(frozen, module = "starlightpdf")]
struct ObjectId(objects::ObjectId);

#[pymethods]
impl ObjectId {
    /// The generation number.
    #[getter]
    const fn generation(&self) -> u16 {
        self.0.generation
    }

    /// An object id.
    #[new]
    fn new(number: u32, generation: u16) -> Self {
        let mut id = objects::ObjectId::default();
        id.number = number;
        id.generation = generation;
        Self(id)
    }

    /// The object number.
    #[getter]
    const fn number(&self) -> u32 {
        self.0.number
    }
}

/// A rectangle in default user space, y up.
#[pyclass(frozen, module = "starlightpdf")]
struct Rect {
    /// The height.
    #[pyo3(get)]
    height: f64,
    /// The width.
    #[pyo3(get)]
    width: f64,
    /// The left edge.
    #[pyo3(get)]
    x: f64,
    /// The bottom edge.
    #[pyo3(get)]
    y: f64,
}

/// A page.
#[pyclass(frozen, module = "starlightpdf")]
struct Page(page::Page);

#[pymethods]
impl Page {
    /// The boundary `kind`: `"Art"`, `"Bleed"`, `"Crop"`, `"Media"` or `"Trim"`.
    #[pyo3(signature = (*, kind = "Crop"))]
    fn boundary(&self, kind: &str) -> PyResult<Rect> {
        let mut options = BoundaryOptions::default();
        options.kind = value(&BOUNDARIES, kind)?;
        self.0
            .boundary(&options)
            .map(|rect| Rect {
                height: rect.height,
                width: rect.width,
                x: rect.x,
                y: rect.y,
            })
            .map_err(|error| to_py(&error))
    }

    /// Draws `image`, `mask` its soft mask, in `layer`, of which none can exist yet; `x`, `y` in points from the
    /// top-left.
    #[pyo3(signature = (image, mask = None, layer = None, **keywords))]
    fn draw_image(
        &self,
        image: &Source,
        mask: Option<&Source>,
        layer: Option<&Bound<'_, PyNone>>,
        keywords: Option<Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let _: Option<&Bound<'_, PyNone>> = layer;
        let given = Keywords::new(
            keywords,
            &[
                "format",
                "frame",
                "height",
                "ignore_orientation",
                "opacity",
                "width",
                "x",
                "y",
            ],
        )?;
        let mut options = DrawImageOptions::default();
        options.format = given
            .get::<Option<String>>("format", None)?
            .map(|format| value(&IMAGE_FORMATS, &format))
            .transpose()?;
        options.frame = given.get("frame", 0)?;
        options.height = given.get("height", None)?;
        options.ignore_orientation = given.get("ignore_orientation", false)?;
        options.opacity = given.get("opacity", None)?;
        options.width = given.get("width", None)?;
        options.x = given.get("x", 0.0_f64)?;
        options.y = given.get("y", 0.0_f64)?;
        self.0
            .draw_image(&image.0, mask.map(|each| &each.0), None, &options)
            .map_err(|error| to_py(&error))
    }

    /// Writes the page as displayed to `sink` as an image; `progress(done, total)`, a `total` of 0
    /// meaning not yet known, returns false to cancel.
    /// `hidden_layers` must be empty, as no layer can exist yet.
    #[pyo3(signature = (sink, progress = None, **keywords))]
    fn render(
        &self,
        sink: &Sink,
        progress: Option<Py<PyAny>>,
        keywords: Option<Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let given = Keywords::new(
            keywords,
            &[
                "color_space",
                "fit",
                "format",
                "hidden_layers",
                "quality",
                "resolution",
                "usage",
            ],
        )?;
        let mut options = RenderOptions::default();
        options.color_space = value(
            &COLOR_SPACES,
            &given.get("color_space", String::from("Rgb"))?,
        )?;
        options.fit = given.get("fit", None)?;
        options.format = value(&RASTER_FORMATS, &given.get("format", String::from("Png"))?)?;
        options.quality = given.get("quality", None)?;
        options.resolution = given.get("resolution", None)?;
        options.usage = value(
            &RENDER_USAGES,
            &given.get("usage", String::from("Display"))?,
        )?;
        // Counted, not copied; a list or tuple, as in JavaScript an array.
        let hidden: Option<Bound<'_, PyAny>> = given.get("hidden_layers", None)?;
        let count = match hidden {
            None => 0,
            Some(layers) => {
                if let Ok(list) = layers.cast::<PyList>() {
                    list.len()
                } else if let Ok(tuple) = layers.cast::<PyTuple>() {
                    tuple.len()
                } else {
                    return Err(PyTypeError::new_err(
                        "hidden_layers must be a list or tuple",
                    ));
                }
            }
        };
        if count > 0 {
            return Err(pdf_error(
                ErrorCode::NotImplemented,
                "Page.render",
                "no implementation yet",
                None,
                None,
                None,
            ));
        }
        let callback = progress.map(Callback::new);
        let result = self.0.render(
            &sink.sink,
            callback.as_ref().map::<&dyn Progress, _>(|each| each),
            &options,
        );
        checked(result, &[callback.as_ref(), sink.callback.as_deref()])
    }

    /// The rotation in degrees clockwise: 0, 90, 180 or 270.
    #[getter]
    fn rotation(&self) -> PyResult<u16> {
        self.0.rotation().map_err(|error| to_py(&error))
    }
}

/// How a document is protected.
#[pyclass(frozen, module = "starlightpdf")]
struct Encryption {
    /// Certificate-based rather than password-based.
    #[pyo3(get)]
    certificate: bool,
    /// The algorithm: `"Aes128"`, `"Aes256"`, `"Rc4"` or `"Unencrypted"`.
    #[pyo3(get)]
    cipher: &'static str,
    /// What is denied, by permission name.
    #[pyo3(get)]
    restrictions: Vec<&'static str>,
}

/// A deviation from a standard.
#[pyclass(frozen, module = "starlightpdf")]
struct Deviation {
    /// Standard and clause, e.g. `ISO 32000-2:2020, 7.5.4`.
    #[pyo3(get)]
    clause: String,
    /// What deviates.
    #[pyo3(get)]
    message: String,
    /// The object number, when known.
    #[pyo3(get)]
    object_number: Option<u32>,
    /// The byte offset in the source, when known.
    #[pyo3(get)]
    offset: Option<u64>,
    /// The page index, when known.
    #[pyo3(get)]
    page: Option<u32>,
}

/// Deviations, iterated.
#[pyclass(frozen, module = "starlightpdf")]
struct Deviations(compliance::Deviations);

#[pymethods]
impl Deviations {
    /// Itself.
    const fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// The next deviation.
    fn __next__(&self) -> PyResult<Option<Deviation>> {
        self.0
            .next()
            .map(|found| {
                found.map(|deviation| Deviation {
                    clause: deviation.clause,
                    message: deviation.message,
                    object_number: deviation.object_number,
                    offset: deviation.offset,
                    page: deviation.page,
                })
            })
            .map_err(|error| to_py(&error))
    }
}

/// The document information dictionary; an absent entry is empty.
#[pyclass(frozen, module = "starlightpdf")]
struct InfoDictionary {
    /// The author.
    #[pyo3(get)]
    author: String,
    /// The creation date, ISO 8601.
    #[pyo3(get)]
    created: String,
    /// The application that created the original.
    #[pyo3(get)]
    creator: String,
    /// Keywords.
    #[pyo3(get)]
    keywords: String,
    /// The last modification date, ISO 8601.
    #[pyo3(get)]
    modified: String,
    /// The application that wrote the PDF.
    #[pyo3(get)]
    producer: String,
    /// The subject.
    #[pyo3(get)]
    subject: String,
    /// The title.
    #[pyo3(get)]
    title: String,
}

/// What a save did.
#[pyclass(frozen, module = "starlightpdf")]
struct SaveReport {
    /// Fields of signatures the save invalidated.
    #[pyo3(get)]
    broken_signatures: Vec<String>,
    /// Bytes written.
    #[pyo3(get)]
    length: u64,
}

/// Documents, iterated.
#[pyclass(frozen, module = "starlightpdf")]
struct Documents(document::Documents);

#[pymethods]
impl Documents {
    /// Itself.
    const fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// The next document.
    fn __next__(&self) -> PyResult<Option<Document>> {
        self.0
            .next()
            .map(|found| found.map(|document| Document { document }))
            .map_err(|error| to_py(&error))
    }
}

/// An open PDF.
#[pyclass(frozen, module = "starlightpdf")]
struct Document {
    /// The open document.
    document: document::Document,
}

#[pymethods]
impl Document {
    /// The catalog's id.
    #[getter]
    fn catalog(&self) -> PyResult<ObjectId> {
        self.document
            .catalog()
            .map(ObjectId)
            .map_err(|error| to_py(&error))
    }

    /// An empty document; `version` is `(major, minor)`, `None` 2.0.
    #[staticmethod]
    #[pyo3(signature = (policy = None, *, version = None))]
    fn create(policy: Option<&ResourcePolicy>, version: Option<(u8, u8)>) -> PyResult<Self> {
        let mut options = CreateOptions::default();
        options.version = version.map(to_version);
        document::Document::create(policy.map(|given| &given.0), &options)
            .map(|document| Self { document })
            .map_err(|error| to_py(&error))
    }

    /// The number of warnings beyond the policy's limit, not kept.
    #[getter]
    fn dropped_warnings(&self) -> PyResult<u64> {
        self.document
            .dropped_warnings()
            .map_err(|error| to_py(&error))
    }

    /// How the document is protected.
    #[getter]
    fn encryption(&self) -> PyResult<Encryption> {
        let found = self.document.encryption().map_err(|error| to_py(&error))?;
        Ok(Encryption {
            certificate: found.certificate,
            cipher: name(&CIPHERS, found.cipher)?,
            restrictions: found
                .restrictions
                .iter()
                .map(|&permission| name(&PERMISSIONS, permission))
                .collect::<PyResult<_>>()?,
        })
    }

    /// The document information dictionary.
    #[getter]
    fn info_dictionary(&self) -> PyResult<InfoDictionary> {
        self.document
            .info_dictionary()
            .map(|found| InfoDictionary {
                author: found.author,
                created: found.created,
                creator: found.creator,
                keywords: found.keywords,
                modified: found.modified,
                producer: found.producer,
                subject: found.subject,
                title: found.title,
            })
            .map_err(|error| to_py(&error))
    }

    /// Inserts a page of `width` by `height` points, `None` A4, at `index`, `None` the end.
    #[pyo3(signature = (*, width = None, height = None, index = None))]
    fn insert_blank_page(
        &self,
        width: Option<f64>,
        height: Option<f64>,
        index: Option<u32>,
    ) -> PyResult<Page> {
        let mut options = InsertBlankPageOptions::default();
        options.height = height;
        options.index = index;
        options.width = width;
        self.document
            .insert_blank_page(&options)
            .map(Page)
            .map_err(|error| to_py(&error))
    }

    /// The object `object_id` as PDF syntax; a stream's dictionary only.
    #[pyo3(signature = (object_id, /))]
    fn object(&self, object_id: &ObjectId) -> PyResult<String> {
        let mut options = ObjectOptions::default();
        options.id = object_id.0;
        self.document
            .object(&options)
            .map_err(|error| to_py(&error))
    }

    /// Opens `source`.
    #[staticmethod]
    #[pyo3(signature = (source, policy = None, *, strict = false, password = ""))]
    fn open(
        source: &Source,
        policy: Option<&ResourcePolicy>,
        strict: bool,
        password: &str,
    ) -> PyResult<Self> {
        let mut options = OpenOptions::default();
        password.clone_into(&mut options.password);
        options.strict = strict;
        document::Document::open(&source.0, policy.map(|given| &given.0), &options)
            .map(|document| Self { document })
            .map_err(|error| to_py(&error))
    }

    /// The page at `index`, from 0.
    #[pyo3(signature = (*, index))]
    fn page(&self, index: u32) -> PyResult<Page> {
        let mut options = PageOptions::default();
        options.index = index;
        self.document
            .page(&options)
            .map(Page)
            .map_err(|error| to_py(&error))
    }

    /// The number of pages.
    #[getter]
    fn page_count(&self) -> PyResult<u32> {
        self.document.page_count().map_err(|error| to_py(&error))
    }

    /// Writes the document to `sink`; `progress(done, total)` returns false to cancel.
    #[pyo3(signature = (sink, progress = None, **keywords))]
    fn save(
        &self,
        sink: &Sink,
        progress: Option<Py<PyAny>>,
        keywords: Option<Bound<'_, PyDict>>,
    ) -> PyResult<SaveReport> {
        let given = Keywords::new(
            keywords,
            &[
                "allow_loss",
                "break_signatures",
                "cipher",
                "color_space",
                "image_quality",
                "image_resolution",
                "mode",
                "optimizations",
                "owner_password",
                "recipients",
                "restrictions",
                "standards",
                "user_password",
                "version",
            ],
        )?;
        let mut options = SaveOptions::default();
        options.allow_loss = given.get("allow_loss", false)?;
        options.break_signatures = given.get("break_signatures", false)?;
        options.cipher = given
            .get::<Option<String>>("cipher", None)?
            .map(|cipher| value(&CIPHERS, &cipher))
            .transpose()?;
        options.color_space = given
            .get::<Option<String>>("color_space", None)?
            .map(|space| value(&COLOR_SPACES, &space))
            .transpose()?;
        options.image_quality = given.get("image_quality", None)?;
        options.image_resolution = given.get("image_resolution", None)?;
        options.mode = value(&SAVE_MODES, &given.get("mode", String::from("Rewrite"))?)?;
        options.optimizations = values(&OPTIMIZATIONS, &given.get("optimizations", Vec::new())?)?;
        options.owner_password = given.get("owner_password", String::new())?;
        options.recipients = given.get("recipients", Vec::new())?;
        options.restrictions = values(&PERMISSIONS, &given.get("restrictions", Vec::new())?)?;
        options.standards = values(&STANDARDS, &given.get("standards", Vec::new())?)?;
        options.user_password = given.get("user_password", String::new())?;
        options.version = given
            .get::<Option<(u8, u8)>>("version", None)?
            .map(to_version);
        let callback = progress.map(Callback::new);
        let result = self.document.save(
            &sink.sink,
            callback.as_ref().map::<&dyn Progress, _>(|each| each),
            &options,
        );
        checked(result, &[callback.as_ref(), sink.callback.as_deref()]).map(|report| SaveReport {
            broken_signatures: report.broken_signatures,
            length: report.length,
        })
    }

    /// Sets the entries given; `None` keeps an entry, empty removes it.
    #[pyo3(signature = (**entries))]
    fn set_info_dictionary(&self, entries: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        let given = Keywords::new(
            entries,
            &[
                "author", "created", "creator", "keywords", "modified", "producer", "subject",
                "title",
            ],
        )?;
        let mut options = SetInfoDictionaryOptions::default();
        options.author = given.get("author", None)?;
        options.created = given.get("created", None)?;
        options.creator = given.get("creator", None)?;
        options.keywords = given.get("keywords", None)?;
        options.modified = given.get("modified", None)?;
        options.producer = given.get("producer", None)?;
        options.subject = given.get("subject", None)?;
        options.title = given.get("title", None)?;
        self.document
            .set_info_dictionary(&options)
            .map_err(|error| to_py(&error))
    }

    /// The parts: `by` `"Pages"`, `count` each, or `"Ranges"`, one from each of `starts`; after
    /// this document's next edit, they and the iterator fail `StaleHandle`.
    #[pyo3(signature = (*, by = "Pages", count = 0, starts = Vec::new(), allow_loss = false))]
    fn split(
        &self,
        by: &str,
        count: u32,
        starts: Vec<u32>,
        allow_loss: bool,
    ) -> PyResult<Documents> {
        let mut options = SplitOptions::default();
        options.allow_loss = allow_loss;
        options.by = value(&SPLITS, by)?;
        options.count = count;
        options.starts = starts;
        self.document
            .split(&options)
            .map(Documents)
            .map_err(|error| to_py(&error))
    }

    /// Writes stream `object_id`'s stored bytes to `sink`, or with `decode` its decoded data.
    #[pyo3(signature = (sink, object_id, /, *, decode = false))]
    fn stream_data(&self, sink: &Sink, object_id: &ObjectId, decode: bool) -> PyResult<()> {
        let mut options = StreamDataOptions::default();
        options.decode = decode;
        options.id = object_id.0;
        checked(
            self.document.stream_data(&sink.sink, &options),
            &[sink.callback.as_deref()],
        )
    }

    /// The warnings kept, at most the policy's limit.
    #[getter]
    fn warnings(&self) -> PyResult<Deviations> {
        self.document
            .warnings()
            .map(Deviations)
            .map_err(|error| to_py(&error))
    }
}

/// Calls `starlightpdf::capabilities`; each capability's name.
#[pyfunction]
fn capabilities() -> PyResult<Vec<&'static str>> {
    starlightpdf::capabilities()
        .map_err(|error| to_py(&error))?
        .into_iter()
        .map(|capability| name(&CAPABILITIES, capability))
        .collect()
}

/// `result`, unless a callback raised; then the first exception raised.
fn checked<T>(
    result: Result<T, starlightpdf::PdfError>,
    callbacks: &[Option<&Callback>],
) -> PyResult<T> {
    callbacks
        .iter()
        .flatten()
        .fold(None, |first: Option<PyErr>, callback| {
            let taken = callback.take();
            first.or(taken)
        })
        .map_or_else(|| result.map_err(|error| to_py(&error)), Err)
}

/// Calls `starlightpdf::header_version`.
#[pyfunction]
fn header_version(data: &[u8]) -> PyResult<(u8, u8)> {
    starlightpdf::header_version(data)
        .map(|version| (version.major, version.minor))
        .map_err(|error| to_py(&error))
}

/// Calls `starlightpdf::library_version`.
#[pyfunction]
const fn library_version() -> &'static str {
    starlightpdf::library_version()
}

/// `value`'s name in `names`.
fn name<T>(names: &[(T, &'static str)], value: T) -> PyResult<&'static str>
where
    T: Copy + PartialEq,
{
    names
        .iter()
        .find(|entry| entry.0 == value)
        .map(|entry| entry.1)
        .ok_or_else(|| PyRuntimeError::new_err("a value this binding does not name"))
}

/// A `PdfError` with these details.
fn pdf_error(
    code: ErrorCode,
    operation: &str,
    message: &str,
    offset: Option<u64>,
    object_number: Option<u32>,
    io_cause: Option<io::ErrorKind>,
) -> PyErr {
    Python::attach(|py| {
        let raised = PdfError::new_err(format!("{operation}: {message}"));
        let value = raised.value(py);
        let cause = io_cause.map(|kind| PyErr::from(io::Error::from(kind)).into_value(py));
        value
            .setattr("code", code.name())
            .and_then(|()| value.setattr("number", code.number()))
            .and_then(|()| value.setattr("message", message))
            .and_then(|()| value.setattr("operation", operation))
            .and_then(|()| value.setattr("offset", offset))
            .and_then(|()| value.setattr("object_number", object_number))
            .and_then(|()| value.setattr("io_cause", cause))
            .map_or_else(|failure| failure, |()| raised)
    })
}

/// A `PdfError` carrying `error`'s details, its operation spelled as in Python.
fn to_py(error: &starlightpdf::PdfError) -> PyErr {
    pdf_error(
        error.code(),
        &error.operation().replace("::", "."),
        error.message(),
        error.offset(),
        error.object_number(),
        error.io_cause(),
    )
}

/// `(major, minor)` as a version.
fn to_version(given: (u8, u8)) -> Version {
    let mut version = Version::default();
    version.major = given.0;
    version.minor = given.1;
    version
}

/// The value `given` names in `names`.
fn value<T>(names: &[(T, &'static str)], given: &str) -> PyResult<T>
where
    T: Copy,
{
    names
        .iter()
        .find(|entry| entry.1 == given)
        .map(|entry| entry.0)
        .ok_or_else(|| PyValueError::new_err(format!("unknown name '{given}'")))
}

/// The values `given` names in `names`.
fn values<T>(names: &[(T, &'static str)], given: &[String]) -> PyResult<Vec<T>>
where
    T: Copy,
{
    given.iter().map(|each| value(names, each)).collect()
}

/// The `starlightpdf` module.
#[pymodule(name = "starlightpdf")]
fn init(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("PdfError", module.py().get_type::<PdfError>())?;
    module.add_class::<Deviation>()?;
    module.add_class::<Deviations>()?;
    module.add_class::<Document>()?;
    module.add_class::<Documents>()?;
    module.add_class::<Encryption>()?;
    module.add_class::<InfoDictionary>()?;
    module.add_class::<ObjectId>()?;
    module.add_class::<Page>()?;
    module.add_class::<Rect>()?;
    module.add_class::<ResourcePolicy>()?;
    module.add_class::<SaveReport>()?;
    module.add_class::<Sink>()?;
    module.add_class::<Source>()?;
    module.add_function(wrap_pyfunction!(capabilities, module)?)?;
    module.add_function(wrap_pyfunction!(header_version, module)?)?;
    module.add_function(wrap_pyfunction!(library_version, module)?)
}
