//! The library's version.

use starlightpdf::library_version;

/// The workspace manifest, read as text.
const MANIFEST: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../Cargo.toml"));

/// The `version` under `[workspace.package]`.
fn workspace_version() -> Option<&'static str> {
    MANIFEST
        .split_once("\n[workspace.package]\n")?
        .1
        .lines()
        .take_while(|line| !line.starts_with('['))
        .find_map(|line| line.strip_prefix("version = \"")?.strip_suffix('"'))
}

/// The version is the workspace's.
#[test]
fn library_version_is_workspace_version() {
    assert_eq!(
        workspace_version(),
        Some(library_version()),
        "not the workspace version"
    );
}
