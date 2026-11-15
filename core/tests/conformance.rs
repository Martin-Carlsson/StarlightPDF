//! Runs the conformance suite in `tests/conformance/*.tsv` through the core crate.

use std::ffi::OsStr;
use std::fs;

/// Calls `operation`; only operations listed here are known.
fn call(operation: &str, args: &[&str]) -> Result<String, String> {
    match operation {
        "greet" => {
            let [name] = *args else {
                return Err(format!("greet takes 1 argument, got {}", args.len()));
            };
            Ok(starlightpdf::greet(name))
        }
        _ => Err(format!("unknown operation '{operation}'")),
    }
}

/// Runs one case: operation, arguments, expected value, tab-separated.
fn run_case(line: &str) -> Result<(), String> {
    let fields: Vec<&str> = line.split('\t').collect();
    let malformed = || format!("malformed line '{line}'");
    let (expected, rest) = fields.split_last().ok_or_else(malformed)?;
    let (operation, args) = rest.split_first().ok_or_else(malformed)?;
    let actual = call(operation, args)?;
    if actual == *expected {
        Ok(())
    } else {
        Err(format!(
            "{operation}: expected '{expected}', got '{actual}'"
        ))
    }
}

/// Runs every case of `suite`; fails on the first bad case or when there is none.
fn run(suite: &str) -> Result<(), String> {
    let cases: Vec<&str> = suite
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    if cases.is_empty() {
        return Err(String::from("no cases"));
    }
    cases.into_iter().try_for_each(run_case)
}

/// Succeeds only when `suite` fails.
fn rejects(suite: &str) -> Result<(), String> {
    match run(suite) {
        Ok(()) => Err(format!("accepted {suite}")),
        Err(_) => Ok(()),
    }
}

/// Every suite file passes.
#[test]
fn suite_passes() -> Result<(), String> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/conformance");
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir).map_err(|err| format!("{dir}: {err}"))? {
        let path = entry.map_err(|err| format!("{dir}: {err}"))?.path();
        if path.extension() == Some(OsStr::new("tsv")) {
            paths.push(path);
        }
    }
    paths.sort();
    let mut suite = String::new();
    for path in &paths {
        let text = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
        suite.push_str(&text);
        if !text.ends_with('\n') {
            suite.push('\n');
        }
    }
    run(&suite)
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
