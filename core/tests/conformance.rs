//! Runs the conformance suite in `tests/conformance/*.tsv` through the core crate.

extern crate alloc;

use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use starlightpdf::PdfError;
use starlightpdf::document::{Document, OpenOptions};
use starlightpdf::input::{BytesSourceOptions, FileSourceOptions, Source};

/// The repository root, against which case paths resolve.
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

/// Calls `operation`; only operations listed here are known.
fn call(operation: &str, args: &[&str]) -> Result<String, String> {
    match operation {
        "header_version" => Ok(
            starlightpdf::header_version(&read(operation, args)?).map_or_else(
                |error| String::from(error.code().name()),
                |version| format!("{}.{}", version.major, version.minor),
            ),
        ),
        "open_bytes" => {
            let mut options = BytesSourceOptions::default();
            options.bytes = Arc::new(read(operation, args)?);
            Ok(open(Source::from_bytes(&options)))
        }
        "open_file" => {
            let mut options = FileSourceOptions::default();
            options.file = PathBuf::from(path(operation, args)?);
            Ok(open(Source::from_file(&options)))
        }
        "page_count_bytes" => {
            let mut options = BytesSourceOptions::default();
            options.bytes = Arc::new(read(operation, args)?);
            Ok(page_count(Source::from_bytes(&options)))
        }
        "page_count_file" => {
            let mut options = FileSourceOptions::default();
            options.file = PathBuf::from(path(operation, args)?);
            Ok(page_count(Source::from_file(&options)))
        }
        _ => Err(format!("unknown operation '{operation}'")),
    }
}

/// Opens a made source without a policy: `ok`, or the error code's name.
fn open(made: Result<Source, PdfError>) -> String {
    made.and_then(|source| Document::open(&source, None, &OpenOptions::default()))
        .map_or_else(
            |error| String::from(error.code().name()),
            |_| String::from("ok"),
        )
}

/// Opens a made source without a policy and counts its pages: the count, or the error code's name.
fn page_count(made: Result<Source, PdfError>) -> String {
    made.and_then(|source| Document::open(&source, None, &OpenOptions::default()))
        .and_then(|document| document.page_count())
        .map_or_else(
            |error| String::from(error.code().name()),
            |count| count.to_string(),
        )
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
