//! Runs the conformance suite in `tests/conformance/*.tsv` through the core crate.

extern crate alloc;

use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use core::fmt::Write as _;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use starlightpdf::document::{Document, OpenOptions, PageOptions};
use starlightpdf::input::{BytesSourceOptions, FileSourceOptions, Source};
use starlightpdf::limits::{ResourcePolicy, ResourcePolicyOptions};
use starlightpdf::objects::{ObjectId, ObjectOptions, StreamDataOptions};
use starlightpdf::output::{MemorySinkOptions, Sink};
use starlightpdf::page::{Boundary, BoundaryOptions};
use starlightpdf::security::{Cipher, Permission};
use starlightpdf::{ErrorCode, PdfError};

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

/// The repository root, against which case paths resolve.
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

/// A case's arguments after its path.
#[derive(Debug, Default)]
struct Case {
    /// `decode`.
    decode: bool,
    /// `id=`.
    id: ObjectId,
    /// `index=`.
    index: u32,
    /// `box=`.
    kind: Boundary,
    /// `password=`.
    password: String,
    /// The limits, when any is given.
    policy: Option<ResourcePolicyOptions>,
    /// The keys given.
    seen: Vec<String>,
    /// `strict`.
    strict: bool,
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
    let (family, file) = match operation.strip_suffix("_bytes") {
        Some(family) => (family, false),
        None => (operation.strip_suffix("_file").ok_or_else(unknown)?, true),
    };
    // The keys each operation takes, then those it needs.
    let (takes, needs): (&[&str], &[&str]) = match family {
        "open" | "page_count" | "catalog" | "encryption" => (&[], &[]),
        "object" => (&["id"], &["id"]),
        "stream_data" => (&["id", "decode"], &["id"]),
        "page" | "rotation" => (&["index"], &["index"]),
        "boundary" => (&["index", "box"], &["index", "box"]),
        "warnings" | "dropped_warnings" => (&["id"], &[]),
        _ => return Err(unknown()),
    };
    let (&relative, rest) = args
        .split_first()
        .ok_or_else(|| format!("{operation} takes a path"))?;
    let case = parse(operation, takes, rest)?;
    if let Some(need) = needs
        .iter()
        .find(|&&need| !case.seen.iter().any(|key| key == need))
    {
        return Err(format!("{operation} needs {need}="));
    }
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
    Ok(perform(family, &case, source).unwrap_or_else(|error| String::from(error.code().name())))
}

/// Parses `args`, accepting only the open options and the keys in `takes`.
fn parse(operation: &str, takes: &[&str], args: &[&str]) -> Result<Case, String> {
    let mut case = Case::default();
    for &arg in args {
        let (key, value) = arg.split_once('=').unwrap_or((arg, ""));
        let bad = || format!("{operation}: bad argument '{arg}'");
        let flag = !arg.contains('=');
        let known = match key {
            "strict" | "decode" => flag,
            "password" | "decoded_limit" | "object_limit" | "warning_limit" | "id" | "index"
            | "box" => !flag,
            _ => false,
        };
        if !known
            || ["decode", "id", "index", "box"].contains(&key) && !takes.contains(&key)
            || case.seen.iter().any(|seen| seen == key)
        {
            return Err(bad());
        }
        case.seen.push(String::from(key));
        match key {
            "strict" => case.strict = true,
            "decode" => case.decode = true,
            "password" => case.password = String::from(value),
            "decoded_limit" => {
                case.policy.get_or_insert_default().decoded_limit =
                    Some(value.parse().map_err(|_err| bad())?);
            }
            "object_limit" => {
                case.policy.get_or_insert_default().object_limit =
                    Some(value.parse().map_err(|_err| bad())?);
            }
            "warning_limit" => {
                case.policy.get_or_insert_default().warning_limit =
                    Some(value.parse().map_err(|_err| bad())?);
            }
            "id" => {
                let (number, generation) = value.split_once(' ').ok_or_else(bad)?;
                case.id.number = number.parse().map_err(|_err| bad())?;
                case.id.generation = generation.parse().map_err(|_err| bad())?;
            }
            "index" => case.index = value.parse().map_err(|_err| bad())?,
            _ => {
                case.kind = BOXES.iter().find(|pair| pair.0 == value).ok_or_else(bad)?.1;
            }
        }
    }
    Ok(case)
}

/// The name `names` gives `value`, or `unknown`.
fn name<T>(names: &[(T, &'static str)], value: T) -> &'static str
where
    T: Copy + PartialEq,
{
    names
        .iter()
        .find(|pair| pair.0 == value)
        .map_or("unknown", |pair| pair.1)
}

/// Options for reading `case`'s object.
fn object(case: &Case) -> ObjectOptions {
    let mut options = ObjectOptions::default();
    options.id = case.id;
    options
}

/// Opens `source` with `case`'s options and performs `family`: its result, or the error code's name.
fn perform(
    family: &str,
    case: &Case,
    source: Result<Source, PdfError>,
) -> Result<String, PdfError> {
    let made = source?;
    let policy = case.policy.as_ref().map(ResourcePolicy::new).transpose()?;
    let mut options = OpenOptions::default();
    options.password.clone_from(&case.password);
    options.strict = case.strict;
    let document = Document::open(&made, policy.as_ref(), &options)?;
    let mut page = PageOptions::default();
    page.index = case.index;
    match family {
        "page_count" => Ok(document.page_count()?.to_string()),
        "catalog" => {
            let id = document.catalog()?;
            Ok(format!("{} {}", id.number, id.generation))
        }
        "object" => document.object(&object(case)),
        "stream_data" => {
            let bytes = stream_data(&document, case, case.decode)?;
            Ok(if bytes.is_empty() {
                String::from("empty")
            } else {
                bytes
                    .iter()
                    .try_fold(String::new(), |mut hex, byte| {
                        write!(hex, "{byte:02x}").map(|()| hex)
                    })
                    .unwrap_or_default()
            })
        }
        "page" => document.page(&page).map(|_page| String::from("ok")),
        "boundary" => {
            let mut boundary = BoundaryOptions::default();
            boundary.kind = case.kind;
            let rect = document.page(&page)?.boundary(&boundary)?;
            Ok(format!(
                "{} {} {} {}",
                rect.x, rect.y, rect.width, rect.height
            ))
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
        "warnings" | "dropped_warnings" => {
            let _: u32 = document.page_count()?;
            if case.seen.iter().any(|key| key == "id") {
                drop(document.object(&object(case))?);
                match stream_data(&document, case, false) {
                    Err(error) if error.code() == ErrorCode::InvalidArgument => {}
                    read => drop(read?),
                }
            }
            if family == "dropped_warnings" {
                return Ok(document.dropped_warnings()?.to_string());
            }
            let warnings = document.warnings()?;
            let mut clauses = Vec::new();
            while let Some(deviation) = warnings.next()? {
                clauses.push(deviation.clause);
            }
            Ok(if clauses.is_empty() {
                String::from("none")
            } else {
                clauses.join(";")
            })
        }
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

/// The data of `case`'s stream, decoded when `decode`, through an in-memory sink.
fn stream_data(document: &Document, case: &Case, decode: bool) -> Result<Vec<u8>, PdfError> {
    let sink = Sink::in_memory(&MemorySinkOptions::default())?;
    let mut options = StreamDataOptions::default();
    options.decode = decode;
    options.id = case.id;
    document.stream_data(&sink, &options)?;
    sink.take_bytes()
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
