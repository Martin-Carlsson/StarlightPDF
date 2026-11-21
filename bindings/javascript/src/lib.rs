//! JavaScript binding.

#![forbid(unsafe_code)]

extern crate alloc;

use alloc::sync::Arc;
use std::io::{self, ErrorKind};

use js_sys::{Array, Error, Object, Reflect, TypeError};
use starlightpdf::document::{OpenOptions, PageOptions};
use starlightpdf::input::{ReadAt, ReadAtSourceOptions};
use starlightpdf::limits::ResourcePolicyOptions;
use starlightpdf::objects::{ObjectId, ObjectOptions, StreamDataOptions};
use starlightpdf::output::MemorySinkOptions;
use starlightpdf::page::{Boundary, BoundaryOptions};
use starlightpdf::security::{Cipher, Permission};
use starlightpdf::{compliance, document, input, limits, output, page};
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

#[wasm_bindgen]
impl Document {
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
            .map(|&permission| permission_name(permission).map(JsValue::from))
            .collect::<Result<Array, JsValue>>()?;
        record([
            ("certificate", found.certificate.into()),
            ("cipher", cipher_name(found.cipher)?.into()),
            ("restrictions", restrictions.into()),
        ])
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

/// `cipher`'s name; an `Error` for one this binding does not know.
fn cipher_name(cipher: Cipher) -> Result<&'static str, JsValue> {
    match cipher {
        Cipher::Aes128 => Ok("Aes128"),
        Cipher::Aes256 => Ok("Aes256"),
        Cipher::Rc4 => Ok("Rc4"),
        Cipher::Unencrypted => Ok("Unencrypted"),
        _ => Err(Error::new("unknown cipher").into()),
    }
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

/// Object `number` `generation`.
fn object_id(number: u32, generation: u16) -> ObjectId {
    let mut id = ObjectId::default();
    id.generation = generation;
    id.number = number;
    id
}

/// `permission`'s name; an `Error` for one this binding does not know.
fn permission_name(permission: Permission) -> Result<&'static str, JsValue> {
    match permission {
        Permission::Annotate => Ok("Annotate"),
        Permission::Assemble => Ok("Assemble"),
        Permission::Copy => Ok("Copy"),
        Permission::Edit => Ok("Edit"),
        Permission::FillForms => Ok("FillForms"),
        Permission::Print => Ok("Print"),
        Permission::PrintHighQuality => Ok("PrintHighQuality"),
        _ => Err(Error::new("unknown permission").into()),
    }
}

/// A plain object of `fields`.
fn record<const N: usize>(fields: [(&str, JsValue); N]) -> Result<JsValue, JsValue> {
    let object = Object::new();
    fields
        .into_iter()
        .try_for_each(|(key, value)| Reflect::set(&object, &key.into(), &value).map(drop))?;
    Ok(object.into())
}
