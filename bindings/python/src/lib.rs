//! Python binding.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// Calls `starlightpdf::header_version`, raising `ValueError` on error.
#[pyfunction]
fn header_version(data: &[u8]) -> PyResult<(u8, u8)> {
    starlightpdf::header_version(data)
        .map(|version| (version.major, version.minor))
        .map_err(|error| PyValueError::new_err(error.to_string()))
}

/// The `starlightpdf` module.
#[pymodule(name = "starlightpdf")]
fn init(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(header_version, module)?)
}
