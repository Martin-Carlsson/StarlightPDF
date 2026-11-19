//! Resource policies: their options and the depth limit.

extern crate alloc;

use alloc::sync::Arc;
use std::path::PathBuf;

use starlightpdf::document::{Document, OpenOptions};
use starlightpdf::input::{BytesSourceOptions, Source};
use starlightpdf::limits::{ResourcePolicy, ResourcePolicyOptions};
use starlightpdf::{ErrorCode, PdfError};

/// Sets one option.
type Setter = fn(&mut ResourcePolicyOptions);

/// A policy setting `depth_limit` only.
fn policy(depth_limit: Option<u32>) -> Result<ResourcePolicy, PdfError> {
    let mut options = ResourcePolicyOptions::default();
    options.depth_limit = depth_limit;
    ResourcePolicy::new(&options)
}

/// A one-page PDF whose catalog holds a value inside `depth` arrays and dictionaries, the
/// catalog included, alternating (ISO 32000-2:2020, 7.3.6, 7.3.7).
fn nested(depth: usize) -> Result<Source, PdfError> {
    let mut value = String::from("0");
    let mut array = depth.is_multiple_of(2);
    for _ in 1..depth {
        value = if array {
            format!("[{value}]")
        } else {
            format!("<< /K {value} >>")
        };
        array = !array;
    }
    let objects = [
        format!("<< /Type /Catalog /Pages 2 0 R /Deep {value} >>"),
        String::from("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        String::from("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>"),
    ];
    let mut pdf = b"%PDF-2.0\n".to_vec();
    let mut entries = b"0000000000 65535 f\r\n".to_vec();
    for (number, object) in (1_u32..).zip(&objects) {
        entries.extend_from_slice(format!("{:010} 00000 n\r\n", pdf.len()).as_bytes());
        pdf.extend_from_slice(format!("{number} 0 obj\n{object}\nendobj\n").as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 4\n");
    pdf.extend_from_slice(&entries);
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let mut options = BytesSourceOptions::default();
    options.bytes = Arc::new(pdf);
    Source::from_bytes(&options)
}

/// Opens a PDF nested `depth` levels with `policy` and counts its pages.
fn page_count(depth: usize, policy: Option<&ResourcePolicy>) -> Result<u32, ErrorCode> {
    nested(depth)
        .and_then(|source| Document::open(&source, policy, &OpenOptions::default()))
        .and_then(|document| document.page_count())
        .map_err(|error| error.code())
}

#[test]
fn new_accepts_no_options_or_only_depth_limit() {
    for depth_limit in [None, Some(1), Some(8), Some(u32::MAX)] {
        assert_eq!(
            policy(depth_limit)
                .map(|_| ())
                .map_err(|error| error.code()),
            Ok(()),
            "depth_limit {depth_limit:?}"
        );
    }
}

#[test]
fn new_rejects_zero_depth_and_other_options() {
    assert_eq!(
        policy(Some(0)).map(|_| ()).map_err(|error| error.code()),
        Err(ErrorCode::InvalidArgument),
        "depth_limit 0"
    );
    let others: [(&str, Setter); 8] = [
        ("cache_limit", |options| options.cache_limit = Some(1)),
        ("concurrency", |options| options.concurrency = Some(1)),
        ("decoded_limit", |options| options.decoded_limit = Some(1)),
        ("memory_budget", |options| options.memory_budget = Some(1)),
        ("object_limit", |options| options.object_limit = Some(1)),
        ("temporary_directory", |options| {
            options.temporary_directory = Some(PathBuf::new());
        }),
        ("temporary_quota", |options| {
            options.temporary_quota = Some(1);
        }),
        ("warning_limit", |options| options.warning_limit = Some(1)),
    ];
    for (name, set) in others {
        for depth_limit in [None, Some(8)] {
            let mut options = ResourcePolicyOptions::default();
            options.depth_limit = depth_limit;
            set(&mut options);
            assert_eq!(
                ResourcePolicy::new(&options)
                    .map(|_| ())
                    .map_err(|error| error.code()),
                Err(ErrorCode::NotImplemented),
                "{name} with depth_limit {depth_limit:?}"
            );
        }
    }
}

#[test]
fn depth_limit_eight_reads_eight_levels_and_fails_at_nine() {
    assert_eq!(
        policy(Some(8))
            .map(|eight| [page_count(8, Some(&eight)), page_count(9, Some(&eight))])
            .map_err(|error| error.code()),
        Ok([Ok(1), Err(ErrorCode::LimitExceeded)]),
        "8 and 9 levels"
    );
}

#[test]
fn default_depth_reads_256_levels_and_fails_at_257() {
    let expected = [Ok(1), Err(ErrorCode::LimitExceeded)];
    assert_eq!(
        [page_count(256, None), page_count(257, None)],
        expected,
        "256 and 257 levels without a policy"
    );
    assert_eq!(
        policy(None)
            .map(|unset| [page_count(256, Some(&unset)), page_count(257, Some(&unset))])
            .map_err(|error| error.code()),
        Ok(expected),
        "256 and 257 levels with depth_limit unset"
    );
}

#[test]
fn documents_follow_their_own_policies() {
    let counts = nested(12).and_then(|source| {
        let eight = policy(Some(8))?;
        let sixteen = policy(Some(16))?;
        let shallow = Document::open(&source, Some(&eight), &OpenOptions::default())?;
        let deep = Document::open(&source, Some(&sixteen), &OpenOptions::default())?;
        Ok([shallow.page_count(), deep.page_count()]
            .map(|count| count.map_err(|error| error.code())))
    });
    assert_eq!(
        counts.map_err(|error| error.code()),
        Ok([Err(ErrorCode::LimitExceeded), Ok(1)]),
        "12 levels under limits 8 and 16"
    );
}
