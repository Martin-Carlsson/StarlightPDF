//! JavaScript binding.

#![forbid(unsafe_code)]

extern crate alloc;

use alloc::sync::Arc;
use std::io::{self, ErrorKind};

use js_sys::{Error, Object, Reflect};
use starlightpdf::document::OpenOptions;
use starlightpdf::input::{ReadAt, ReadAtSourceOptions};
use starlightpdf::{document, input};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::{JsValue, wasm_bindgen};

#[wasm_bindgen(raw_module = "../table.js")]
extern "C" {
    /// Reads input `id` from `offset` into `buffer`; the count read.
    #[wasm_bindgen(catch)]
    fn read(id: u32, offset: u64, buffer: &mut [u8]) -> Result<usize, JsValue>;

    /// Forgets input `id`.
    fn release(id: u32);
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
        read(self.id, offset, buffer).map_err(|error| {
            let (name, message) = error.dyn_ref::<Error>().map_or_else(
                || (String::new(), String::from("read failed")),
                |thrown| (String::from(thrown.name()), String::from(thrown.message())),
            );
            let kind = if name == "NotFoundError" {
                ErrorKind::NotFound
            } else {
                ErrorKind::Other
            };
            io::Error::new(kind, message)
        })
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
}

/// An open PDF.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Document(document::Document);

#[wasm_bindgen]
impl Document {
    /// Opens `source`, checking only its header.
    ///
    /// # Errors
    ///
    /// The core's, as details.
    pub fn open(source: &Source) -> Result<Self, JsValue> {
        document::Document::open(&source.0, None, &OpenOptions::default())
            .map(Self)
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
    let details = Object::new();
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
    fields
        .into_iter()
        .try_for_each(|(key, value)| Reflect::set(&details, &key.into(), &value).map(drop))
        .map_or_else(|failure| failure, |()| details.into())
}
