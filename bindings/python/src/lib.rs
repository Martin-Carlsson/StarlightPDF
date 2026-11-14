//! Python binding.

use pyo3::prelude::*;

/// Calls `starlightpdf::greet`.
#[pyfunction]
fn greet(name: &str) -> String {
    starlightpdf::greet(name)
}

/// The `starlightpdf` module.
#[pymodule(name = "starlightpdf")]
fn init(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(greet, module)?)
}
