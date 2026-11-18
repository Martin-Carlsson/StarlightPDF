//! Python binding.

extern crate alloc;

use alloc::sync::Arc;
use std::io;
use std::path::PathBuf;

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::pybacked::PyBackedBytes;
use pyo3::types::PyBytes;
use starlightpdf::document::OpenOptions;
use starlightpdf::input::{FileSourceOptions, ReadAt, ReadAtSourceOptions};
use starlightpdf::{document, input};

create_exception!(
    starlightpdf,
    PdfError,
    PyException,
    "Every failure; `code` and `number` give the error code."
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

/// An open PDF.
#[pyclass(frozen, module = "starlightpdf")]
struct Document {
    /// The open document.
    _document: document::Document,
}

#[pymethods]
impl Document {
    /// Opens `source`, checking only its header.
    #[staticmethod]
    fn open(source: &Source) -> PyResult<Self> {
        document::Document::open(&source.0, None, &OpenOptions::default())
            .map(|opened| Self { _document: opened })
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

/// A `PdfError` carrying `error`'s code name and number.
fn to_py(error: &starlightpdf::PdfError) -> PyErr {
    Python::attach(|py| {
        let raised = PdfError::new_err(error.to_string());
        let value = raised.value(py);
        value
            .setattr("code", error.code().name())
            .and_then(|()| value.setattr("number", error.code().number()))
            .map_or_else(|failure| failure, |()| raised)
    })
}

/// The `starlightpdf` module.
#[pymodule(name = "starlightpdf")]
fn init(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("PdfError", module.py().get_type::<PdfError>())?;
    module.add_class::<Source>()?;
    module.add_class::<Document>()?;
    module.add_function(wrap_pyfunction!(header_version, module)?)
}
