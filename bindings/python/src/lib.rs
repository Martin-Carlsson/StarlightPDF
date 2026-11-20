//! Python binding.

#![forbid(unsafe_code)]

extern crate alloc;

use alloc::sync::Arc;
use std::io;
use std::path::PathBuf;

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::pybacked::PyBackedBytes;
use pyo3::types::PyBytes;
use starlightpdf::document::{OpenOptions, PageOptions};
use starlightpdf::input::{FileSourceOptions, ReadAt, ReadAtSourceOptions};
use starlightpdf::limits::ResourcePolicyOptions;
use starlightpdf::objects::{ObjectOptions, StreamDataOptions};
use starlightpdf::output::MemorySinkOptions;
use starlightpdf::page::{Boundary, BoundaryOptions};
use starlightpdf::security::{Cipher, Permission};
use starlightpdf::{compliance, document, input, limits, objects, output, page};

/// Each boundary's name.
const BOUNDARIES: [(Boundary, &str); 5] = [
    (Boundary::Art, "Art"),
    (Boundary::Bleed, "Bleed"),
    (Boundary::Crop, "Crop"),
    (Boundary::Media, "Media"),
    (Boundary::Trim, "Trim"),
];

/// Each cipher's name.
const CIPHERS: [(Cipher, &str); 4] = [
    (Cipher::Aes128, "Aes128"),
    (Cipher::Aes256, "Aes256"),
    (Cipher::Rc4, "Rc4"),
    (Cipher::Unencrypted, "Unencrypted"),
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
struct Sink(output::Sink);

#[pymethods]
impl Sink {
    /// A sink that holds its bytes in memory, for `take_bytes`.
    #[staticmethod]
    fn in_memory() -> PyResult<Self> {
        output::Sink::in_memory(&MemorySinkOptions::default())
            .map(Self)
            .map_err(|error| to_py(&error))
    }

    /// Takes the bytes appended so far, copied into `bytes`, leaving the sink empty.
    fn take_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        self.0
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
        options.kind = BOUNDARIES
            .iter()
            .find(|entry| entry.1 == kind)
            .map(|entry| entry.0)
            .ok_or_else(|| PyValueError::new_err(format!("unknown boundary '{kind}'")))?;
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

    /// Writes stream `object_id`'s stored bytes to `sink`, or with `decode` its decoded data.
    #[pyo3(signature = (sink, object_id, /, *, decode = false))]
    fn stream_data(&self, sink: &Sink, object_id: &ObjectId, decode: bool) -> PyResult<()> {
        let mut options = StreamDataOptions::default();
        options.decode = decode;
        options.id = object_id.0;
        self.document
            .stream_data(&sink.0, &options)
            .map_err(|error| to_py(&error))
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

/// A `PdfError` carrying `error`'s details, its operation spelled as in Python.
fn to_py(error: &starlightpdf::PdfError) -> PyErr {
    let operation = error.operation().replace("::", ".");
    Python::attach(|py| {
        let raised = PdfError::new_err(format!("{operation}: {}", error.message()));
        let value = raised.value(py);
        let io_cause = error
            .io_cause()
            .map(|kind| PyErr::from(io::Error::from(kind)).into_value(py));
        value
            .setattr("code", error.code().name())
            .and_then(|()| value.setattr("number", error.code().number()))
            .and_then(|()| value.setattr("message", error.message()))
            .and_then(|()| value.setattr("operation", operation))
            .and_then(|()| value.setattr("offset", error.offset()))
            .and_then(|()| value.setattr("object_number", error.object_number()))
            .and_then(|()| value.setattr("io_cause", io_cause))
            .map_or_else(|failure| failure, |()| raised)
    })
}

/// The `starlightpdf` module.
#[pymodule(name = "starlightpdf")]
fn init(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("PdfError", module.py().get_type::<PdfError>())?;
    module.add_class::<Deviation>()?;
    module.add_class::<Deviations>()?;
    module.add_class::<Document>()?;
    module.add_class::<Encryption>()?;
    module.add_class::<ObjectId>()?;
    module.add_class::<Page>()?;
    module.add_class::<Rect>()?;
    module.add_class::<ResourcePolicy>()?;
    module.add_class::<Sink>()?;
    module.add_class::<Source>()?;
    module.add_function(wrap_pyfunction!(header_version, module)?)?;
    module.add_function(wrap_pyfunction!(library_version, module)?)
}
