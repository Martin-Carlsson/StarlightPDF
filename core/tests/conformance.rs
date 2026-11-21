//! Runs the conformance suite in `tests/conformance/*.tsv` through the core crate.

extern crate alloc;

use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use core::fmt::Write as _;
use core::mem;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use starlightpdf::arrange::{InsertBlankPageOptions, SplitBy, SplitOptions};
use starlightpdf::compliance::Standard;
use starlightpdf::document::{
    CreateOptions, Document, OpenOptions, Optimization, PageOptions, SaveMode, SaveOptions,
};
use starlightpdf::drawing::DrawImageOptions;
use starlightpdf::images::ImageFormat;
use starlightpdf::input::{BytesSourceOptions, FileSourceOptions, Source};
use starlightpdf::limits::{ResourcePolicy, ResourcePolicyOptions};
use starlightpdf::metadata::SetInfoDictionaryOptions;
use starlightpdf::objects::{ObjectId, ObjectOptions, StreamDataOptions};
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf::page::{Boundary, BoundaryOptions, Rect};
use starlightpdf::progress::Progress;
use starlightpdf::rendering::ColorSpace;
use starlightpdf::security::{Cipher, Permission};
use starlightpdf::{ErrorCode, PdfError, Version};

/// Each `box=` value's boundary.
const BOXES: [(&str, Boundary); 5] = [
    ("Art", Boundary::Art),
    ("Bleed", Boundary::Bleed),
    ("Crop", Boundary::Crop),
    ("Media", Boundary::Media),
    ("Trim", Boundary::Trim),
];

/// Each cipher's name.
const CIPHERS: [(Cipher, &str); 4] = [
    (Cipher::Aes128, "Aes128"),
    (Cipher::Aes256, "Aes256"),
    (Cipher::Rc4, "Rc4"),
    (Cipher::Unencrypted, "Unencrypted"),
];

/// Each `color_space=` value.
const COLOR_SPACES: [(ColorSpace, &str); 1] = [(ColorSpace::Gray, "Gray")];

/// The keys without a value.
const FLAGS: [&str; 7] = [
    "allow_loss",
    "break_signatures",
    "cancel",
    "decode",
    "ignore_orientation",
    "saved",
    "strict",
];

/// Each `format=` value.
const FORMATS: [(ImageFormat, &str); 3] = [
    (ImageFormat::Gif, "Gif"),
    (ImageFormat::Jpeg, "Jpeg"),
    (ImageFormat::Png, "Png"),
];

/// SHA-256's initial hash value (FIPS 180-4, 5.3.3).
const INITIAL: [u32; 8] = [
    0x6A09_E667,
    0xBB67_AE85,
    0x3C6E_F372,
    0xA54F_F53A,
    0x510E_527F,
    0x9B05_688C,
    0x1F83_D9AB,
    0x5BE0_CD19,
];

/// Each `mode=` value.
const MODES: [(SaveMode, &str); 3] = [
    (SaveMode::InPlace, "InPlace"),
    (SaveMode::Incremental, "Incremental"),
    (SaveMode::Rewrite, "Rewrite"),
];

/// The keys every family that opens a file takes.
const OPEN: [&str; 5] = [
    "decoded_limit",
    "object_limit",
    "password",
    "strict",
    "warning_limit",
];

/// Each `optimizations=` value.
const OPTIMIZATIONS: [(Optimization, &str); 1] =
    [(Optimization::CompressStreams, "CompressStreams")];

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

/// Each `read=` value of `draw_image`.
const READS: [&str; 10] = [
    "bits",
    "color_space",
    "decode",
    "filter",
    "icc_sha256",
    "mask_samples_sha256",
    "matrix",
    "raw_sha256",
    "samples_sha256",
    "size",
];

/// The repository root, against which case paths resolve.
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

/// SHA-256's constants (FIPS 180-4, 4.2.2).
const ROUNDS: [u32; 64] = [
    0x428A_2F98,
    0x7137_4491,
    0xB5C0_FBCF,
    0xE9B5_DBA5,
    0x3956_C25B,
    0x59F1_11F1,
    0x923F_82A4,
    0xAB1C_5ED5,
    0xD807_AA98,
    0x1283_5B01,
    0x2431_85BE,
    0x550C_7DC3,
    0x72BE_5D74,
    0x80DE_B1FE,
    0x9BDC_06A7,
    0xC19B_F174,
    0xE49B_69C1,
    0xEFBE_4786,
    0x0FC1_9DC6,
    0x240C_A1CC,
    0x2DE9_2C6F,
    0x4A74_84AA,
    0x5CB0_A9DC,
    0x76F9_88DA,
    0x983E_5152,
    0xA831_C66D,
    0xB003_27C8,
    0xBF59_7FC7,
    0xC6E0_0BF3,
    0xD5A7_9147,
    0x06CA_6351,
    0x1429_2967,
    0x27B7_0A85,
    0x2E1B_2138,
    0x4D2C_6DFC,
    0x5338_0D13,
    0x650A_7354,
    0x766A_0ABB,
    0x81C2_C92E,
    0x9272_2C85,
    0xA2BF_E8A1,
    0xA81A_664B,
    0xC24B_8B70,
    0xC76C_51A3,
    0xD192_E819,
    0xD699_0624,
    0xF40E_3585,
    0x106A_A070,
    0x19A4_C116,
    0x1E37_6C08,
    0x2748_774C,
    0x34B0_BCB5,
    0x391C_0CB3,
    0x4ED8_AA4A,
    0x5B9C_CA4F,
    0x682E_6FF3,
    0x748F_82EE,
    0x78A5_636F,
    0x84C8_7814,
    0x8CC7_0208,
    0x90BE_FFFA,
    0xA450_6CEB,
    0xBEF9_A3F7,
    0xC671_78F2,
];

/// The save options' keys.
const SAVE: [&str; 15] = [
    "allow_loss",
    "break_signatures",
    "cancel",
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
];

/// The split options' keys.
const SPLIT: [&str; 4] = ["allow_loss", "by", "count", "starts"];

/// Each `by=` value.
const SPLITS: [(SplitBy, &str); 4] = [
    (SplitBy::Bookmarks, "Bookmarks"),
    (SplitBy::Pages, "Pages"),
    (SplitBy::Ranges, "Ranges"),
    (SplitBy::Size, "Size"),
];

/// Each `standards=` value.
const STANDARDS: [(Standard, &str); 1] = [(Standard::PdfA2B, "PdfA2B")];

/// A case's arguments after its path.
#[derive(Debug, Default)]
struct Case {
    /// `decode`.
    decode: bool,
    /// `x=`, `y=`, `width=`, `height=`, `ignore_orientation`, `format=`, `frame=`, `opacity=`.
    draw: DrawImageOptions,
    /// `draws=`: how many times to draw; 0 means 1.
    draws: u32,
    /// `id=`.
    id: ObjectId,
    /// `index=`.
    index: u32,
    /// `set_<key>=`.
    info: SetInfoDictionaryOptions,
    /// `key=`.
    key: String,
    /// `box=`.
    kind: Boundary,
    /// `mask=`.
    mask: Option<String>,
    /// `on=`.
    on: Option<String>,
    /// `page_width=`, `page_height=`.
    page: InsertBlankPageOptions,
    /// `page=`, in order.
    pages: Vec<InsertBlankPageOptions>,
    /// `part=`.
    part: Option<u32>,
    /// `password=`.
    password: String,
    /// The limits, when any is given.
    policy: Option<ResourcePolicyOptions>,
    /// `read=`.
    read: String,
    /// The save options.
    save: SaveOptions,
    /// The keys given.
    seen: Vec<String>,
    /// The split options.
    split: SplitOptions,
    /// `strict`.
    strict: bool,
}

/// A progress callback that cancels at its first call.
struct Cancel;

/// Why a case gives no value: a library error, whose code is the value, or a fault of the case.
#[derive(Debug)]
enum Failure {
    /// The case is wrong or the library broke a promise the runner checks.
    Case(String),
    /// The library failed.
    Pdf(PdfError),
}

/// An object parsed from its syntax (ISO 32000-2:2020, 7.3), as far as the runner reads it.
#[derive(Clone, Debug)]
enum Object {
    /// An array.
    Array(Vec<Self>),
    /// A dictionary's entries, keys without their slash.
    Dictionary(Vec<(String, Self)>),
    /// A name without its slash.
    Name(String),
    /// Another token: a number, string, boolean or null.
    Other(String),
    /// An indirect reference.
    Reference(ObjectId),
}

impl From<PdfError> for Failure {
    fn from(error: PdfError) -> Self {
        Self::Pdf(error)
    }
}

impl Progress for Cancel {
    fn update(&self, done: u64, total: u64) -> bool {
        let _: (u64, u64) = (done, total);
        false
    }
}

/// Fails unless `case` combines its keys as `family` allows: save options only with `saved`, split options only
/// with `part=`, and not both.
fn combine(operation: &str, family: &str, case: &Case) -> Result<(), String> {
    let has = |key: &str| case.seen.iter().any(|seen| seen == key);
    if !matches!(
        family,
        "save" | "split" | "draw_image" | "insert_blank_page"
    ) {
        let saved = has("saved");
        let part = has("part");
        let lone = |keys: &[&str]| keys.iter().any(|&key| key != "allow_loss" && has(key));
        if saved && part
            || lone(&SAVE) && !saved
            || lone(&SPLIT) && !part
            || has("allow_loss") && !saved && !part
        {
            return Err(format!(
                "{operation}: save options need saved, split options part=, and not both"
            ));
        }
    }
    Ok(())
}

/// Calls `operation`; only operations listed here are known.
fn call(operation: &str, args: &[&str]) -> Result<String, String> {
    if operation == "header_version" {
        return Ok(
            starlightpdf::header_version(&read(operation, args)?).map_or_else(
                |error| String::from(error.code().name()),
                |version| format!("{}.{}", version.major, version.minor),
            ),
        );
    }
    let unknown = || format!("unknown operation '{operation}'");
    let (family, file) = match operation {
        "draw_image" | "insert_blank_page" => (operation, false),
        _ => match operation.strip_suffix("_bytes") {
            Some(family) => (family, false),
            None => (operation.strip_suffix("_file").ok_or_else(unknown)?, true),
        },
    };
    let (takes, needs) = keys(family).ok_or_else(unknown)?;
    let (&relative, rest) = args
        .split_first()
        .ok_or_else(|| format!("{operation} takes a path"))?;
    let case = parse(operation, &takes, rest)?;
    if let Some(need) = needs
        .iter()
        .find(|&&need| !case.seen.iter().any(|key| key == need))
    {
        return Err(format!("{operation} needs {need}="));
    }
    combine(operation, family, &case)?;
    let result = match family {
        "insert_blank_page" if relative == "-" => inserted(&case),
        "insert_blank_page" => return Err(format!("{operation} takes the path -")),
        "draw_image" => drawn(relative, &case),
        _ => {
            let file_path = format!("{ROOT}/{relative}");
            let source = if file {
                let mut options = FileSourceOptions::default();
                options.file = PathBuf::from(file_path);
                Source::from_file(&options)
            } else {
                let mut options = BytesSourceOptions::default();
                options.bytes =
                    Arc::new(fs::read(&file_path).map_err(|err| format!("{file_path}: {err}"))?);
                Source::from_bytes(&options)
            };
            perform(family, &case, source)
        }
    };
    match result {
        Ok(value) => Ok(value),
        Err(Failure::Pdf(error)) => Ok(String::from(error.code().name())),
        Err(Failure::Case(why)) => Err(format!("{operation}: {why}")),
    }
}

/// The color space `value` names: `DeviceGray`, `DeviceRGB`, `DeviceCMYK`, `ICCBased N`, or
/// `Indexed <base> <hival> <lookup as hex>` (ISO 32000-2:2020, 8.6).
fn color_space(document: &Document, value: Option<Object>) -> Result<String, Failure> {
    let resolved = resolve(document, value)?;
    let Object::Array(items) = resolved else {
        return text(Some(resolved));
    };
    let mut operands = items.into_iter();
    let family = text(operands.next())?;
    match family.as_str() {
        "ICCBased" => {
            let profile = resolve(document, operands.next())?;
            Ok(format!("ICCBased {}", number(entry(profile, "N"))?))
        }
        "Indexed" => {
            let (Some(base), Some(high), Some(lookup)) =
                (operands.next(), operands.next(), operands.next())
            else {
                return Err(Failure::Case(String::from("Indexed takes 3 operands")));
            };
            let bytes = match lookup {
                Object::Reference(id) => stream_bytes(document, id, true)?,
                other @ (Object::Array(_)
                | Object::Dictionary(_)
                | Object::Name(_)
                | Object::Other(_)) => string_bytes(&text(Some(other))?)?,
            };
            Ok(format!(
                "Indexed {} {} {}",
                color_space(document, Some(base))?,
                number(Some(high))?,
                hex(&bytes)
            ))
        }
        _ => Ok(family),
    }
}

/// Draws `path`'s image as `case` says, `draws=` times, on page `index=` of the PDF `on=` or else on a new
/// document's one page, saves, reopens and reads `read=`.
fn drawn(path: &str, case: &Case) -> Result<String, Failure> {
    let (document, page) = if let Some(on) = case.on.as_deref() {
        let document = open(&from_bytes(on)?, case)?;
        let mut options = PageOptions::default();
        options.index = case.index;
        let page = document.page(&options)?;
        (document, page)
    } else {
        let policy = case.policy.as_ref().map(ResourcePolicy::new).transpose()?;
        let document = Document::create(policy.as_ref(), &CreateOptions::default())?;
        let page = document.insert_blank_page(&case.page)?;
        (document, page)
    };
    let image = from_bytes(path)?;
    let mask = case.mask.as_deref().map(from_bytes).transpose()?;
    for _ in 0..case.draws.max(1) {
        page.draw_image(&image, mask.as_ref(), None, &case.draw)?;
    }
    let saved = reopen(twice(&document, &SaveOptions::default(), false)?, case)?;
    image_read(&saved, case.index, &case.read)
}

/// The value of `key` in `dictionary`, if it is a dictionary holding it.
fn entry(dictionary: Object, key: &str) -> Option<Object> {
    let Object::Dictionary(entries) = dictionary else {
        return None;
    };
    entries
        .into_iter()
        .find(|pair| pair.0 == key)
        .map(|pair| pair.1)
}

/// The `Filter` names space-separated, or `none` (ISO 32000-2:2020, 7.4).
fn filter(value: Option<Object>) -> Result<String, Failure> {
    match value {
        None => Ok(String::from("none")),
        Some(Object::Array(names)) => Ok(names
            .into_iter()
            .map(|item| text(Some(item)))
            .collect::<Result<Vec<String>, Failure>>()?
            .join(" ")),
        Some(
            other @ (Object::Dictionary(_)
            | Object::Name(_)
            | Object::Other(_)
            | Object::Reference(_)),
        ) => text(Some(other)),
    }
}

/// A source over the bytes of the file at `path`.
fn from_bytes(path: &str) -> Result<Source, Failure> {
    let file = format!("{ROOT}/{path}");
    let mut options = BytesSourceOptions::default();
    options.bytes =
        Arc::new(fs::read(&file).map_err(|err| Failure::Case(format!("{file}: {err}")))?);
    Ok(Source::from_bytes(&options)?)
}

/// Whether `case` gives `key`.
fn has(case: &Case, key: &str) -> bool {
    case.seen.iter().any(|seen| seen == key)
}

/// `bytes` as lowercase hex.
fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .try_fold(String::new(), |mut text, byte| {
            write!(text, "{byte:02x}").map(|()| text)
        })
        .unwrap_or_default()
}

/// The ICC profile `value`'s color space holds, directly or as an `Indexed` base, if any.
fn icc(document: &Document, value: Option<Object>) -> Result<Option<ObjectId>, Failure> {
    let Object::Array(items) = resolve(document, value)? else {
        return Ok(None);
    };
    let mut operands = items.into_iter();
    let family = operands.next().map(|item| text(Some(item))).transpose()?;
    match (family.as_deref(), operands.next()) {
        (Some("ICCBased"), Some(Object::Reference(id))) => Ok(Some(id)),
        (Some("Indexed"), base) => icc(document, base),
        _ => Ok(None),
    }
}

/// What `read` gives of the image the last `Do` paints on kid `index` of `document`'s root page tree node.
fn image_read(document: &Document, index: u32, read: &str) -> Result<String, Failure> {
    let catalog = object_at(document, document.catalog()?)?;
    let pages = resolve(document, entry(catalog, "Pages"))?;
    let kid = match resolve(document, entry(pages, "Kids"))? {
        Object::Array(items) => items
            .into_iter()
            .nth(usize::try_from(index).unwrap_or(usize::MAX)),
        Object::Dictionary(_) | Object::Name(_) | Object::Other(_) | Object::Reference(_) => None,
    };
    let page = resolve(document, kid)?;
    let (label, ctm) = painted(document, page.clone())?;
    let resources = resolve(document, entry(page, "Resources"))?;
    let id = match entry(resolve(document, entry(resources, "XObject"))?, &label) {
        Some(Object::Reference(id)) => Some(id),
        Some(Object::Array(_) | Object::Dictionary(_) | Object::Name(_) | Object::Other(_))
        | None => None,
    }
    .ok_or_else(|| Failure::Case(format!("no XObject /{label} on the page")))?;
    let image = object_at(document, id)?;
    if !matches!(entry(image.clone(), "Subtype"), Some(Object::Name(subtype)) if subtype == "Image")
    {
        return Err(Failure::Case(format!("/{label} is not an image XObject")));
    }
    let field = |key: &str| entry(image.clone(), key);
    let digest = |found: Option<Object>| -> Result<String, Failure> {
        match found {
            Some(Object::Reference(at)) => Ok(sha256(&stream_bytes(document, at, true)?)),
            Some(Object::Array(_) | Object::Dictionary(_) | Object::Name(_) | Object::Other(_))
            | None => Ok(String::from("none")),
        }
    };
    match read {
        "matrix" => Ok(ctm),
        "decode" => match field("Decode") {
            Some(Object::Array(items)) => Ok(items
                .into_iter()
                .map(|item| number(Some(item)))
                .collect::<Result<Vec<String>, Failure>>()?
                .join(" ")),
            Some(
                Object::Dictionary(_) | Object::Name(_) | Object::Other(_) | Object::Reference(_),
            ) => Err(Failure::Case(String::from("Decode is not an array"))),
            None => Ok(String::from("none")),
        },
        "size" => Ok(format!(
            "{} {}",
            number(field("Width"))?,
            number(field("Height"))?
        )),
        "bits" => number(field("BitsPerComponent")),
        "color_space" => color_space(document, field("ColorSpace")),
        "filter" => filter(field("Filter")),
        "raw_sha256" => Ok(sha256(&stream_bytes(document, id, false)?)),
        "samples_sha256" => Ok(sha256(&stream_bytes(document, id, true)?)),
        "mask_samples_sha256" => digest(field("SMask")),
        _ => digest(icc(document, field("ColorSpace"))?.map(Object::Reference)),
    }
}

/// Creates a document, inserts `case`'s pages, saves, reopens and gives each page's `MediaBox`.
fn inserted(case: &Case) -> Result<String, Failure> {
    let document = Document::create(None, &CreateOptions::default())?;
    for options in &case.pages {
        drop(document.insert_blank_page(options)?);
    }
    let saved = reopen(twice(&document, &SaveOptions::default(), false)?, case)?;
    let mut boundary = BoundaryOptions::default();
    boundary.kind = Boundary::Media;
    let mut boxes = Vec::new();
    for index in 0..saved.page_count()? {
        let mut page = PageOptions::default();
        page.index = index;
        boxes.push(rect(saved.page(&page)?.boundary(&boundary)?));
    }
    Ok(boxes.join(";"))
}

/// The keys `family` takes, then those it needs; `None` for an unknown family.
fn keys(family: &str) -> Option<(Vec<&'static str>, &'static [&'static str])> {
    let (own, needs): (&[&str], &[&str]) = match family {
        "open" | "page_count" | "catalog" | "encryption" => (&[], &[]),
        "object" => (&["id"], &["id"]),
        "stream_data" => (&["id", "decode"], &["id"]),
        "page" | "rotation" => (&["index"], &["index"]),
        "boundary" => (&["index", "box"], &["index", "box"]),
        "warnings" | "dropped_warnings" => (&["id"], &[]),
        "info" => (
            &[
                "key",
                "set_author",
                "set_created",
                "set_creator",
                "set_keywords",
                "set_modified",
                "set_producer",
                "set_subject",
                "set_title",
            ],
            &["key"],
        ),
        "save" => return Some(([OPEN.as_slice(), &SAVE].concat(), &[])),
        "split" => return Some(([OPEN.as_slice(), &SPLIT].concat(), &[])),
        "insert_blank_page" => return Some((vec!["page"], &[])),
        "draw_image" => {
            return Some((
                vec![
                    "decoded_limit",
                    "draws",
                    "format",
                    "frame",
                    "height",
                    "ignore_orientation",
                    "index",
                    "mask",
                    "on",
                    "opacity",
                    "page_height",
                    "page_width",
                    "read",
                    "strict",
                    "width",
                    "x",
                    "y",
                ],
                &["read"],
            ));
        }
        _ => return None,
    };
    Some((
        [OPEN.as_slice(), own, &["saved", "part"], &SAVE, &SPLIT].concat(),
        needs,
    ))
}

/// The value `names` gives `text`.
fn lookup<T>(names: &[(T, &str)], text: &str) -> Option<T>
where
    T: Copy,
{
    names.iter().find(|pair| pair.1 == text).map(|pair| pair.0)
}

/// The values `names` gives the comma-separated `text`; empty for empty text.
fn lookups<T>(names: &[(T, &str)], text: &str) -> Option<Vec<T>>
where
    T: Copy,
{
    if text.is_empty() {
        return Some(Vec::new());
    }
    text.split(',').map(|item| lookup(names, item)).collect()
}

/// The name the last `Do` in `page`'s content paints, and the CTM in effect at each `Do` relative to default user space,
/// `;`-separated: the product of every `cm` in effect (ISO 32000-2:2020, 8.3.4, 8.4.4), each number rounded to 4
/// fraction digits.
fn painted(document: &Document, page: Object) -> Result<(String, String), Failure> {
    let streams = match entry(page, "Contents") {
        Some(Object::Reference(id)) => vec![id],
        Some(Object::Array(items)) => items
            .into_iter()
            .filter_map(|item| match item {
                Object::Reference(id) => Some(id),
                Object::Array(_) | Object::Dictionary(_) | Object::Name(_) | Object::Other(_) => {
                    None
                }
            })
            .collect(),
        Some(Object::Dictionary(_) | Object::Name(_) | Object::Other(_)) | None => Vec::new(),
    };
    let mut content = Vec::new();
    for id in streams {
        content.extend(stream_bytes(document, id, true)?);
        content.push(b'\n');
    }
    let text = String::from_utf8_lossy(&content);
    let mut states: Vec<[f64; 6]> = Vec::new();
    let mut current = [1.0_f64, 0.0_f64, 0.0_f64, 1.0_f64, 0.0_f64, 0.0_f64];
    let mut operands: Vec<String> = Vec::new();
    let mut last = None;
    let mut ctms = Vec::new();
    for token in tokens(&text) {
        match token.as_str() {
            "q" => states.push(current),
            "Q" => current = states.pop().unwrap_or(current),
            "cm" => {
                let values: Vec<f64> = operands
                    .iter()
                    .rev()
                    .take(6)
                    .rev()
                    .filter_map(|operand| operand.parse().ok())
                    .collect();
                let [ma, mb, mc, md, me, mf] = *values.as_slice() else {
                    return Err(Failure::Case(String::from("cm takes 6 numbers")));
                };
                let [ca, cb, cc, cd, ce, cf] = current;
                current = [
                    ma.mul_add(ca, mb.mul_add(cc, 0.0)),
                    ma.mul_add(cb, mb.mul_add(cd, 0.0)),
                    mc.mul_add(ca, md.mul_add(cc, 0.0)),
                    mc.mul_add(cb, md.mul_add(cd, 0.0)),
                    me.mul_add(ca, mf.mul_add(cc, ce)),
                    me.mul_add(cb, mf.mul_add(cd, cf)),
                ];
            }
            "Do" => {
                let label = operands
                    .last()
                    .and_then(|operand| operand.strip_prefix('/'));
                last =
                    Some(String::from(label.ok_or_else(|| {
                        Failure::Case(String::from("Do takes a name"))
                    })?));
                ctms.push(current.map(round).join(" "));
            }
            _ => {}
        }
        let first = token.chars().next().unwrap_or(' ');
        if first.is_ascii_alphabetic() || first == '\'' || first == '"' {
            operands.clear();
        } else {
            operands.push(token);
        }
    }
    let label = last.ok_or_else(|| Failure::Case(String::from("no Do in the page's content")))?;
    Ok((label, ctms.join(";")))
}

/// The value `names` gives `value`, or `unknown`.
fn name<T>(names: &[(T, &'static str)], value: T) -> &'static str
where
    T: Copy + PartialEq,
{
    names
        .iter()
        .find(|pair| pair.0 == value)
        .map_or("unknown", |pair| pair.1)
}

/// The number `value` holds, as the shortest decimal that round-trips.
fn number(value: Option<Object>) -> Result<String, Failure> {
    let token = text(value)?;
    token
        .parse::<f64>()
        .map(|parsed| parsed.to_string())
        .map_err(|_err| Failure::Case(format!("not a number: {token}")))
}

/// Options for reading `case`'s object.
fn object(case: &Case) -> ObjectOptions {
    let mut options = ObjectOptions::default();
    options.id = case.id;
    options
}

/// The object `id`, parsed.
fn object_at(document: &Document, id: ObjectId) -> Result<Object, Failure> {
    let mut options = ObjectOptions::default();
    options.id = id;
    let syntax = document.object(&options)?;
    let words = tokens(&syntax);
    let refs: Vec<&str> = words.iter().map(String::as_str).collect();
    parse_object(&refs)
        .map(|pair| pair.0)
        .ok_or_else(|| Failure::Case(format!("cannot parse {syntax}")))
}

/// Opens `source` with `case`'s options.
fn open(source: &Source, case: &Case) -> Result<Document, PdfError> {
    let policy = case.policy.as_ref().map(ResourcePolicy::new).transpose()?;
    let mut options = OpenOptions::default();
    options.password.clone_from(&case.password);
    options.strict = case.strict;
    Document::open(source, policy.as_ref(), &options)
}

/// Parses `args`, accepting only the keys in `takes`, each once but `page=`.
fn parse(operation: &str, takes: &[&str], args: &[&str]) -> Result<Case, String> {
    let mut case = Case::default();
    for &arg in args {
        let (key, value) = arg.split_once('=').unwrap_or((arg, ""));
        let bad = || format!("{operation}: bad argument '{arg}'");
        if !takes.contains(&key)
            || FLAGS.contains(&key) == arg.contains('=')
            || key != "page" && case.seen.iter().any(|seen| seen == key)
        {
            return Err(bad());
        }
        case.seen.push(String::from(key));
        set(&mut case, key, value).ok_or_else(bad)?;
    }
    Ok(case)
}

/// The first object of `tokens` and the tokens after it.
fn parse_object<'tokens>(
    tokens: &'tokens [&'tokens str],
) -> Option<(Object, &'tokens [&'tokens str])> {
    let (&first, mut rest) = tokens.split_first()?;
    match first {
        "<<" => {
            let mut entries = Vec::new();
            loop {
                let (&key, after) = rest.split_first()?;
                if key == ">>" {
                    return Some((Object::Dictionary(entries), after));
                }
                let (value, next) = parse_object(after)?;
                entries.push((String::from(key.strip_prefix('/')?), value));
                rest = next;
            }
        }
        "[" => {
            let mut items = Vec::new();
            loop {
                if let Some((&"]", after)) = rest.split_first() {
                    return Some((Object::Array(items), after));
                }
                let (value, next) = parse_object(rest)?;
                items.push(value);
                rest = next;
            }
        }
        _ => {
            if let Some(name) = first.strip_prefix('/') {
                return Some((Object::Name(String::from(name)), rest));
            }
            if let Some((&generation, tail)) = rest.split_first()
                && let Some((&"R", after)) = tail.split_first()
                && let (Ok(number), Ok(parsed)) = (first.parse(), generation.parse())
            {
                let mut id = ObjectId::default();
                id.number = number;
                id.generation = parsed;
                return Some((Object::Reference(id), after));
            }
            Some((Object::Other(String::from(first)), rest))
        }
    }
}

/// Opens `source` with `case`'s options, performs `family`, after `saved` or `part=` on the saved document.
fn perform(family: &str, case: &Case, source: Result<Source, PdfError>) -> Result<String, Failure> {
    let document = open(&source?, case)?;
    if case.seen.iter().any(|key| key.starts_with("set_")) {
        document.set_info_dictionary(&case.info)?;
    }
    match family {
        "save" => {
            let sink = Sink::in_memory(&MemorySinkOptions::default())?;
            let stop: &dyn Progress = &Cancel;
            let report = document.save(&sink, has(case, "cancel").then_some(stop), &case.save)?;
            return Ok(if report.broken_signatures.is_empty() {
                String::from("ok")
            } else {
                report.broken_signatures.join(",")
            });
        }
        "split" => {
            let parts = document.split(&case.split)?;
            let mut counts = Vec::new();
            while let Some(part) = parts.next()? {
                counts.push(part.page_count()?.to_string());
            }
            return Ok(if counts.is_empty() {
                String::from("none")
            } else {
                counts.join(" ")
            });
        }
        _ => {}
    }
    if has(case, "saved") {
        let bytes = twice(&document, &case.save, has(case, "cancel"))?;
        return inspect(family, case, &reopen(bytes, case)?);
    }
    if let Some(index) = case.part {
        let parts = document.split(&case.split)?;
        let missing = || Failure::Case(format!("no part {index}"));
        for _ in 0..index {
            drop(parts.next()?.ok_or_else(missing)?);
        }
        let part = parts.next()?.ok_or_else(missing)?;
        let bytes = twice(&part, &SaveOptions::default(), false)?;
        return inspect(family, case, &reopen(bytes, case)?);
    }
    inspect(family, case, &document)
}

/// Reads `family`'s value of `document`.
fn inspect(family: &str, case: &Case, document: &Document) -> Result<String, Failure> {
    let mut page = PageOptions::default();
    page.index = case.index;
    match family {
        "page_count" => Ok(document.page_count()?.to_string()),
        "catalog" => {
            let id = document.catalog()?;
            Ok(format!("{} {}", id.number, id.generation))
        }
        "object" => Ok(document.object(&object(case))?),
        "stream_data" => {
            let bytes = stream_data(document, case, case.decode)?;
            Ok(if bytes.is_empty() {
                String::from("empty")
            } else {
                hex(&bytes)
            })
        }
        "page" => document
            .page(&page)
            .map(|_page| String::from("ok"))
            .map_err(Failure::Pdf),
        "boundary" => {
            let mut boundary = BoundaryOptions::default();
            boundary.kind = case.kind;
            Ok(rect(document.page(&page)?.boundary(&boundary)?))
        }
        "rotation" => Ok(document.page(&page)?.rotation()?.to_string()),
        "encryption" => {
            let encryption = document.encryption()?;
            let denied: Vec<&str> = encryption
                .restrictions
                .iter()
                .map(|&permission| name(&PERMISSIONS, permission))
                .collect();
            Ok(format!(
                "{} {} {}",
                name(&CIPHERS, encryption.cipher),
                encryption.certificate,
                if denied.is_empty() {
                    String::from("none")
                } else {
                    denied.join(",")
                }
            ))
        }
        "info" => {
            let info = document.info_dictionary()?;
            let value = match case.key.as_str() {
                "author" => info.author,
                "created" => info.created,
                "creator" => info.creator,
                "keywords" => info.keywords,
                "modified" => info.modified,
                "producer" => info.producer,
                "subject" => info.subject,
                _ => info.title,
            };
            Ok(if value.is_empty() {
                String::from("none")
            } else {
                value
            })
        }
        "warnings" | "dropped_warnings" => warnings(family, case, document),
        // `open`.
        _ => Ok(String::from("ok")),
    }
}

/// The one argument, a path from the repository root, resolved.
fn path(operation: &str, args: &[&str]) -> Result<String, String> {
    let [relative] = *args else {
        return Err(format!("{operation} takes 1 argument, got {}", args.len()));
    };
    Ok(format!("{ROOT}/{relative}"))
}

/// The bytes of the file the one argument names.
fn read(operation: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let file = path(operation, args)?;
    fs::read(&file).map_err(|err| format!("{file}: {err}"))
}

/// `rect` as `x y width height`.
fn rect(rect: Rect) -> String {
    format!("{} {} {} {}", rect.x, rect.y, rect.width, rect.height)
}

/// Opens `bytes` with `case`'s options.
fn reopen(bytes: Vec<u8>, case: &Case) -> Result<Document, PdfError> {
    let mut options = BytesSourceOptions::default();
    options.bytes = Arc::new(bytes);
    open(&Source::from_bytes(&options)?, case)
}

/// The object `value` is, read when it is a reference.
fn resolve(document: &Document, value: Option<Object>) -> Result<Object, Failure> {
    match value {
        Some(Object::Reference(id)) => object_at(document, id),
        Some(
            other @ (Object::Array(_) | Object::Dictionary(_) | Object::Name(_) | Object::Other(_)),
        ) => Ok(other),
        None => Err(Failure::Case(String::from("a needed entry is missing"))),
    }
}

/// `value` rounded to 4 fraction digits, trailing zeros and point dropped, -0 as 0.
fn round(value: f64) -> String {
    let fixed = format!("{value:.4}");
    let trimmed = fixed.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        String::from("0")
    } else {
        String::from(trimmed)
    }
}

/// Saves `document` to memory with `options`, cancelling at the first progress when `cancel`.
fn save_bytes(
    document: &Document,
    options: &SaveOptions,
    cancel: bool,
) -> Result<Vec<u8>, PdfError> {
    let sink = Sink::in_memory(&MemorySinkOptions::default())?;
    let stop: &dyn Progress = &Cancel;
    drop(document.save(&sink, cancel.then_some(stop), options)?);
    sink.take_bytes()
}

/// Sets `key`'s `value` in `case`; `None` when the value is bad.
fn set(case: &mut Case, key: &str, value: &str) -> Option<()> {
    let text = || Some(String::from(value));
    let real = || value.parse::<f64>().ok();
    match key {
        "strict" => case.strict = true,
        "decode" => case.decode = true,
        "password" => case.password = String::from(value),
        "decoded_limit" => {
            case.policy.get_or_insert_default().decoded_limit = Some(value.parse().ok()?);
        }
        "object_limit" => {
            case.policy.get_or_insert_default().object_limit = Some(value.parse().ok()?);
        }
        "warning_limit" => {
            case.policy.get_or_insert_default().warning_limit = Some(value.parse().ok()?);
        }
        "id" => {
            let (number, generation) = value.split_once(' ')?;
            case.id.number = number.parse().ok()?;
            case.id.generation = generation.parse().ok()?;
        }
        "index" => case.index = value.parse().ok()?,
        "box" => case.kind = BOXES.iter().find(|pair| pair.0 == value)?.1,
        "key" => {
            [
                "author", "created", "creator", "keywords", "modified", "producer", "subject",
                "title",
            ]
            .contains(&value)
            .then_some(())?;
            case.key = String::from(value);
        }
        "set_author" => case.info.author = text(),
        "set_created" => case.info.created = text(),
        "set_creator" => case.info.creator = text(),
        "set_keywords" => case.info.keywords = text(),
        "set_modified" => case.info.modified = text(),
        "set_producer" => case.info.producer = text(),
        "set_subject" => case.info.subject = text(),
        "set_title" => case.info.title = text(),
        "allow_loss" => {
            case.save.allow_loss = true;
            case.split.allow_loss = true;
        }
        "by" => case.split.by = lookup(&SPLITS, value)?,
        "count" => case.split.count = value.parse().ok()?,
        "starts" => {
            case.split.starts = if value.is_empty() {
                Vec::new()
            } else {
                value
                    .split(',')
                    .map(|start| start.parse().ok())
                    .collect::<Option<Vec<u32>>>()?
            };
        }
        "part" => case.part = Some(value.parse().ok()?),
        "page" => case.pages.push(sheet(value)?),
        "page_width" => case.page.width = Some(real()?),
        "page_height" => case.page.height = Some(real()?),
        "x" => case.draw.x = real()?,
        "y" => case.draw.y = real()?,
        "width" => case.draw.width = Some(real()?),
        "height" => case.draw.height = Some(real()?),
        "opacity" => case.draw.opacity = Some(real()?),
        "ignore_orientation" => case.draw.ignore_orientation = true,
        "format" => case.draw.format = Some(lookup(&FORMATS, value)?),
        "frame" => case.draw.frame = value.parse().ok()?,
        "draws" => case.draws = value.parse().ok()?,
        "mask" => case.mask = text(),
        "on" => case.on = text(),
        "read" => {
            READS.contains(&value).then_some(())?;
            case.read = String::from(value);
        }
        // `saved`, `cancel`: their presence in `seen`.
        "saved" | "cancel" => {}
        _ => set_save(&mut case.save, key, value)?,
    }
    Some(())
}

/// Sets the save option `key`'s `value`; `None` when the key or value is bad.
fn set_save(save: &mut SaveOptions, key: &str, value: &str) -> Option<()> {
    match key {
        "break_signatures" => save.break_signatures = true,
        "cipher" => save.cipher = Some(lookup(&CIPHERS, value)?),
        "color_space" => save.color_space = Some(lookup(&COLOR_SPACES, value)?),
        "image_quality" => save.image_quality = Some(value.parse().ok()?),
        "image_resolution" => save.image_resolution = Some(value.parse().ok()?),
        "mode" => save.mode = lookup(&MODES, value)?,
        "optimizations" => save.optimizations = lookups(&OPTIMIZATIONS, value)?,
        "owner_password" => save.owner_password = String::from(value),
        "recipients" => save.recipients = unhex(value)?,
        "restrictions" => save.restrictions = lookups(&PERMISSIONS, value)?,
        "standards" => save.standards = lookups(&STANDARDS, value)?,
        "user_password" => save.user_password = String::from(value),
        "version" => {
            let (major, minor) = value.split_once('.')?;
            let mut version = Version::default();
            version.major = major.parse().ok()?;
            version.minor = minor.parse().ok()?;
            save.version = Some(version);
        }
        _ => return None,
    }
    Some(())
}

/// SHA-256 of `data` as lowercase hex (FIPS 180-4, 6.2).
fn sha256(data: &[u8]) -> String {
    let bits = u64::try_from(data.len())
        .unwrap_or(u64::MAX)
        .wrapping_mul(8);
    let mut message = data.to_vec();
    message.push(0x80);
    while !message.len().wrapping_add(8).is_multiple_of(64) {
        message.push(0);
    }
    message.extend(
        (0_u32..8)
            .rev()
            .map(|at| u8::try_from(bits.wrapping_shr(at.wrapping_mul(8)) & 0xFF).unwrap_or(0)),
    );
    let mut hash = INITIAL;
    for block in message.as_chunks::<64>().0 {
        let mut schedule: Vec<u32> = block
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| {
                word.iter()
                    .fold(0, |acc: u32, &byte| acc.wrapping_shl(8) | u32::from(byte))
            })
            .collect();
        while schedule.len() < 64 {
            let back = |distance: usize| {
                schedule
                    .len()
                    .checked_sub(distance)
                    .and_then(|at| schedule.get(at))
                    .copied()
                    .unwrap_or(0)
            };
            let (two, fifteen) = (back(2), back(15));
            let next = (two.rotate_right(17) ^ two.rotate_right(19) ^ two.wrapping_shr(10))
                .wrapping_add(back(7))
                .wrapping_add(
                    fifteen.rotate_right(7) ^ fifteen.rotate_right(18) ^ fifteen.wrapping_shr(3),
                )
                .wrapping_add(back(16));
            schedule.push(next);
        }
        let mut state = hash;
        for (&constant, &word) in ROUNDS.iter().zip(&schedule) {
            let [va, vb, vc, vd, ve, vf, vg, vh] = state;
            let first = vh
                .wrapping_add(ve.rotate_right(6) ^ ve.rotate_right(11) ^ ve.rotate_right(25))
                .wrapping_add((ve & vf) ^ (!ve & vg))
                .wrapping_add(constant)
                .wrapping_add(word);
            let second = (va.rotate_right(2) ^ va.rotate_right(13) ^ va.rotate_right(22))
                .wrapping_add((va & vb) ^ (va & vc) ^ (vb & vc));
            state = [
                first.wrapping_add(second),
                va,
                vb,
                vc,
                vd.wrapping_add(first),
                ve,
                vf,
                vg,
            ];
        }
        for (value, add) in hash.iter_mut().zip(state) {
            *value = value.wrapping_add(add);
        }
    }
    hash.iter()
        .try_fold(String::new(), |mut text, word| {
            write!(text, "{word:08x}").map(|()| text)
        })
        .unwrap_or_default()
}

/// The insertion `page=` gives: `[WxH][@I]`, A4 and the end when left out.
fn sheet(value: &str) -> Option<InsertBlankPageOptions> {
    let (size, index) = value.split_once('@').unwrap_or((value, ""));
    let mut options = InsertBlankPageOptions::default();
    if !size.is_empty() {
        let (width, height) = size.split_once('x')?;
        options.width = Some(width.parse().ok()?);
        options.height = Some(height.parse().ok()?);
    }
    if !index.is_empty() || value.contains('@') {
        options.index = Some(index.parse().ok()?);
    }
    Some(options)
}

/// The data of the stream `id`, decoded when `decode`.
fn stream_bytes(document: &Document, id: ObjectId, decode: bool) -> Result<Vec<u8>, PdfError> {
    let sink = Sink::in_memory(&MemorySinkOptions::default())?;
    let mut options = StreamDataOptions::default();
    options.decode = decode;
    options.id = id;
    document.stream_data(&sink, &options)?;
    sink.take_bytes()
}

/// The data of `case`'s stream, decoded when `decode`, through an in-memory sink.
fn stream_data(document: &Document, case: &Case, decode: bool) -> Result<Vec<u8>, PdfError> {
    stream_bytes(document, case.id, decode)
}

/// The bytes of a string token: hexadecimal or literal (ISO 32000-2:2020, 7.3.4).
fn string_bytes(token: &str) -> Result<Vec<u8>, Failure> {
    let bad = || Failure::Case(format!("not a string: {token}"));
    if let Some(inner) = token
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
    {
        let mut digits: Vec<u8> = inner
            .chars()
            .filter(|character| !character.is_ascii_whitespace())
            .map(|character| u8::try_from(character.to_digit(16)?).ok())
            .collect::<Option<Vec<u8>>>()
            .ok_or_else(bad)?;
        if !digits.len().is_multiple_of(2) {
            digits.push(0);
        }
        return Ok(digits
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                pair.iter()
                    .fold(0_u8, |acc, &digit| acc.wrapping_shl(4) | digit)
            })
            .collect());
    }
    let inner = token
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(bad)?;
    let mut bytes = Vec::new();
    let mut characters = inner.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '\\' {
            let mut buffer = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
            continue;
        }
        let Some(escaped) = characters.next() else {
            break;
        };
        match escaped {
            'n' => bytes.push(b'\n'),
            'r' => bytes.push(b'\r'),
            't' => bytes.push(b'\t'),
            'b' => bytes.push(0x08),
            'f' => bytes.push(0x0C),
            '\r' => {
                let _: Option<char> = characters.next_if_eq(&'\n');
            }
            '\n' => {}
            '0'..='7' => {
                let mut octal = escaped.to_digit(8).unwrap_or(0);
                for _ in 0_u8..2 {
                    let Some(digit) = characters.peek().and_then(|next| next.to_digit(8)) else {
                        break;
                    };
                    octal = octal.wrapping_mul(8).wrapping_add(digit);
                    let _: Option<char> = characters.next();
                }
                bytes.push(u8::try_from(octal & 0xFF).unwrap_or(0));
            }
            other => {
                let mut buffer = [0; 4];
                bytes.extend_from_slice(other.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
    Ok(bytes)
}

/// The token `value` is, when it is a name or another token.
fn text(value: Option<Object>) -> Result<String, Failure> {
    match value {
        Some(Object::Name(token) | Object::Other(token)) => Ok(token),
        Some(Object::Array(_) | Object::Dictionary(_) | Object::Reference(_)) | None => Err(
            Failure::Case(String::from("a needed name or number is missing")),
        ),
    }
}

/// PDF syntax split into tokens (ISO 32000-2:2020, 7.2): delimiters, names, strings whole, comments dropped.
fn tokens(syntax: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut current = String::new();
    let mut characters = syntax.chars().peekable();
    while let Some(character) = characters.next() {
        let delimiter = "()<>[]{}/%".contains(character);
        if (character.is_ascii_whitespace() || character == '\0' || delimiter)
            && !current.is_empty()
        {
            found.push(mem::take(&mut current));
        }
        match character {
            '(' => {
                let mut depth = 1_u32;
                let mut string = String::from('(');
                while let Some(next) = characters.next() {
                    string.push(next);
                    match next {
                        '\\' => {
                            if let Some(escaped) = characters.next() {
                                string.push(escaped);
                            }
                        }
                        '(' => depth = depth.saturating_add(1),
                        ')' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                found.push(string);
            }
            '<' | '>' if characters.next_if_eq(&character).is_some() => {
                found.push(format!("{character}{character}"));
            }
            '<' => {
                let mut string = String::from('<');
                for next in characters.by_ref() {
                    string.push(next);
                    if next == '>' {
                        break;
                    }
                }
                found.push(string);
            }
            '%' => {
                while characters
                    .next_if(|&next| next != '\r' && next != '\n')
                    .is_some()
                {}
            }
            '/' => current.push('/'),
            '[' | ']' | '{' | '}' | '>' | ')' => found.push(String::from(character)),
            _ if character.is_ascii_whitespace() || character == '\0' => {}
            _ => current.push(character),
        }
    }
    if !current.is_empty() {
        found.push(current);
    }
    found
}

/// Saves `document` twice to memory; the bytes, or a failure unless both saves give the same.
fn twice(document: &Document, options: &SaveOptions, cancel: bool) -> Result<Vec<u8>, Failure> {
    let first = save_bytes(document, options, cancel)?;
    if save_bytes(document, options, cancel)? == first {
        Ok(first)
    } else {
        Err(Failure::Case(String::from(
            "two saves give different bytes",
        )))
    }
}

/// The bytes of hexadecimal `text`.
fn unhex(text: &str) -> Option<Vec<u8>> {
    string_bytes(&format!("<{text}>")).ok()
}

/// The warnings, or their dropped count, after counting the pages and, with `id=`, reading that object and any
/// stored stream data.
fn warnings(family: &str, case: &Case, document: &Document) -> Result<String, Failure> {
    let _: u32 = document.page_count()?;
    if has(case, "id") {
        drop(document.object(&object(case))?);
        match stream_data(document, case, false) {
            Err(error) if error.code() == ErrorCode::InvalidArgument => {}
            read => drop(read?),
        }
    }
    if family == "dropped_warnings" {
        return Ok(document.dropped_warnings()?.to_string());
    }
    let found = document.warnings()?;
    let mut clauses = Vec::new();
    while let Some(deviation) = found.next()? {
        clauses.push(deviation.clause);
    }
    Ok(if clauses.is_empty() {
        String::from("none")
    } else {
        clauses.join(";")
    })
}

/// Fails unless `operation` on `args` gives `expected`.
fn check(operation: &str, args: &[&str], expected: &str) -> Result<(), String> {
    let actual = call(operation, args)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{operation}: expected '{expected}', got '{actual}'"
        ))
    }
}

/// Runs every case; fails naming each bad case and each disagreeing pair by `file:line`.
fn run(suites: &[(&str, &str)]) -> Result<(), String> {
    let mut failures = Vec::new();
    let mut first: HashMap<String, (String, &str)> = HashMap::new();
    let mut any = false;
    for &(name, text) in suites {
        for (raw, number) in text.split_terminator('\n').zip(1_u32..) {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if line.starts_with('#') {
                continue;
            }
            any = true;
            let position = format!("{name}:{number}");
            let mut fields = line.split('\t');
            let (Some(operation), Some(expected)) = (fields.next(), fields.next_back()) else {
                failures.push(format!("{position}: malformed line '{line}'"));
                continue;
            };
            let args: Vec<&str> = fields.collect();
            let family = operation
                .strip_suffix("_bytes")
                .or_else(|| operation.strip_suffix("_file"))
                .unwrap_or(operation);
            let earlier = first
                .entry(format!("{family}\t{}", args.join("\t")))
                .or_insert_with(|| (position.clone(), expected));
            if earlier.1 != expected {
                failures.push(format!(
                    "{position}: expects '{expected}', {} expects '{}'",
                    earlier.0, earlier.1
                ));
            }
            if let Err(error) = check(operation, &args, expected) {
                failures.push(format!("{position}: {error}"));
            }
        }
    }
    if !any {
        return Err(String::from("no cases"));
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

/// Succeeds only when `suite` fails.
fn rejects(suite: &str) -> Result<(), String> {
    match run(&[("s.tsv", suite)]) {
        Ok(()) => Err(format!("accepted {suite}")),
        Err(_) => Ok(()),
    }
}

/// Fails unless `result` is an error naming exactly `positions`.
fn names(result: Result<(), String>, positions: &[&str]) -> Result<(), String> {
    let error = result.err().ok_or("accepted")?;
    let named: BTreeSet<&str> = error
        .split(|character: char| !(character.is_alphanumeric() || "._-:".contains(character)))
        .map(|token| token.trim_end_matches(':'))
        .filter(|token| {
            token.rsplit_once(':').is_some_and(|(file, line)| {
                Path::new(file).extension() == Some(OsStr::new("tsv"))
                    && !line.is_empty()
                    && line.bytes().all(|byte| byte.is_ascii_digit())
            })
        })
        .collect();
    if named == positions.iter().copied().collect() {
        Ok(())
    } else {
        Err(format!("named {named:?}, not {positions:?}, in: {error}"))
    }
}

/// Every suite file passes.
#[test]
fn suite_passes() -> Result<(), String> {
    let dir = format!("{ROOT}/tests/conformance");
    let mut paths = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|err| format!("{dir}: {err}"))? {
        let path = entry.map_err(|err| format!("{dir}: {err}"))?.path();
        if path.extension() == Some(OsStr::new("tsv")) {
            paths.push(path);
        }
    }
    paths.sort();
    let mut suites = Vec::new();
    for path in &paths {
        let text = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| format!("{}: no file name", path.display()))?;
        suites.push((String::from(name), text));
    }
    let pairs: Vec<(&str, &str)> = suites
        .iter()
        .map(|pair| (pair.0.as_str(), pair.1.as_str()))
        .collect();
    run(&pairs)
}

/// A case with an unknown operation fails.
#[test]
fn unknown_operation_fails() -> Result<(), String> {
    rejects("nope\tx\ty\n")
}

/// A case with an unknown argument fails.
#[test]
fn unknown_argument_fails() -> Result<(), String> {
    rejects("open_bytes\ttests/corpus/header/version-1.7.pdf\tnope\tok\n")
}

/// A suite with only a comment fails.
#[test]
fn suite_without_cases_fails() -> Result<(), String> {
    rejects("# operation\targuments\texpected\n")
}

/// Each failing case is named by its suite file and line, comments counted.
#[test]
fn names_every_failing_case() -> Result<(), String> {
    names(
        run(&[
            (
                "a.tsv",
                "# c\nheader_version\ttests/corpus/header/version-1.7.pdf\t1.7\nheader_version\ttests/corpus/header/version-2.9.pdf\t1.7\n",
            ),
            ("b.tsv", "open_bytes\ttests/corpus/header/empty.pdf\tok\n"),
        ]),
        &["a.tsv:3", "b.tsv:1"],
    )
}

/// Bytes and file expectations for one corpus file must agree; both lines are named.
#[test]
fn rejects_expectations_that_differ_by_source() -> Result<(), String> {
    names(
        run(&[
            (
                "open.tsv",
                "open_bytes\ttests/corpus/header/version-1.7.pdf\tok\nopen_file\ttests/corpus/header/version-1.7.pdf\tInvalidHeader\n",
            ),
            (
                "page_count.tsv",
                "page_count_bytes\ttests/corpus/page-count/one-page.pdf\t1\npage_count_file\ttests/corpus/page-count/one-page.pdf\t2\n",
            ),
        ]),
        &[
            "open.tsv:1",
            "open.tsv:2",
            "page_count.tsv:1",
            "page_count.tsv:2",
        ],
    )
}

/// A CR LF suite passes as its LF form does.
#[test]
fn crlf_suite_matches_lf() -> Result<(), String> {
    run(&[(
        "s.tsv",
        "# c\r\nheader_version\ttests/corpus/header/version-1.7.pdf\t1.7\r\n",
    )])
}
