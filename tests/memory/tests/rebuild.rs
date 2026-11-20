//! Peak allocation of rebuilding the cross-reference table of a file full of `trailer` keywords,
//! `/ObjStm` names or objects.

extern crate alloc;

use alloc::sync::Arc;
use std::io;

use starlightpdf::ErrorCode;
use starlightpdf::document::{Document, OpenOptions};
use starlightpdf::input::{ReadAt, ReadAtSourceOptions, Source};
use starlightpdf::limits::{ResourcePolicy, ResourcePolicyOptions};
use starlightpdf_memory::peak;

/// Most bytes opening and counting the pages may hold allocated at once.
const BOUND: usize = 67_584;

/// Most bytes failing to open past the object limit may hold allocated at once.
const LIMITED_BOUND: usize = 73_728;

/// The objects, before the repeated bytes.
const HEAD: &[u8] = b"%PDF-2.0\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\
3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n";

/// The keyword, repeated.
const KEYWORD: &[u8] = b"trailer\n";

/// An object, repeated.
const OBJECT: &[u8] = b"4 0 obj\nnull\nendobj\n";

/// An object stream's type name, repeated.
const OBJECT_STREAM: &[u8] = b"/ObjStm\n";

/// Times the bytes are repeated.
const REPEATS: u64 = 1 << 20;

/// The trailer, after the repeated bytes; no `startxref`.
const TAIL: &[u8] = b"trailer\n<< /Size 4 /Root 1 0 R >>\n%%EOF\n";

/// [`HEAD`], `unit` [`REPEATS`] times, then [`TAIL`].
struct Repeated {
    /// The bytes repeated.
    unit: &'static [u8],
}

impl ReadAt for Repeated {
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let size = self.size()?;
        let head = u64::try_from(HEAD.len()).map_err(io::Error::other)?;
        let width = u64::try_from(self.unit.len()).map_err(io::Error::other)?;
        let tail = head.saturating_add(REPEATS.saturating_mul(width));
        let mut at = offset;
        for slot in buffer.iter_mut() {
            if at >= size {
                break;
            }
            let (bytes, within) = if at < head {
                (HEAD, at)
            } else if at < tail {
                (
                    self.unit,
                    at.saturating_sub(head).checked_rem(width).unwrap_or(0),
                )
            } else {
                (TAIL, at.saturating_sub(tail))
            };
            *slot = *usize::try_from(within)
                .ok()
                .and_then(|index| bytes.get(index))
                .ok_or_else(|| io::Error::other("offset"))?;
            at = at.saturating_add(1);
        }
        usize::try_from(at.saturating_sub(offset)).map_err(io::Error::other)
    }

    fn size(&self) -> io::Result<u64> {
        let fixed =
            u64::try_from(HEAD.len().saturating_add(TAIL.len())).map_err(io::Error::other)?;
        let width = u64::try_from(self.unit.len()).map_err(io::Error::other)?;
        Ok(fixed.saturating_add(REPEATS.saturating_mul(width)))
    }
}

/// A rebuild keeps few trailer candidates, however many `trailer` keywords the file holds
/// (ISO 32000-2:2020, 7.5.4 and 7.5.5).
#[test]
fn trailer_keywords_are_bounded() {
    let read_at: Arc<dyn ReadAt> = Arc::new(Repeated { unit: KEYWORD });
    let (count, bytes) = peak(|| {
        Source::from_read_at(&read_at, &ReadAtSourceOptions::default())
            .and_then(|opened| Document::open(&opened, None, &OpenOptions::default()))
            .and_then(|document| document.page_count())
            .map_err(|error| error.code())
    });
    assert_eq!(count, Ok(1), "page count");
    assert!((1..=BOUND).contains(&bytes), "peak {bytes} bytes");
}

/// A rebuild keeps one object stream candidate per object, however many `/ObjStm` names it holds
/// (ISO 32000-2:2020, 7.5.4 and 7.5.7).
#[test]
fn objstm_names_are_bounded() {
    let read_at: Arc<dyn ReadAt> = Arc::new(Repeated {
        unit: OBJECT_STREAM,
    });
    let (count, bytes) = peak(|| {
        Source::from_read_at(&read_at, &ReadAtSourceOptions::default())
            .and_then(|opened| Document::open(&opened, None, &OpenOptions::default()))
            .and_then(|document| document.page_count())
            .map_err(|error| error.code())
    });
    assert_eq!(count, Ok(1), "page count");
    assert!((1..=BOUND).contains(&bytes), "peak {bytes} bytes");
}

/// A rebuild fails at the object limit, before holding the objects past it
/// (ISO 32000-2:2020, 7.5.4).
#[test]
fn objects_past_the_limit_are_bounded() {
    let read_at: Arc<dyn ReadAt> = Arc::new(Repeated { unit: OBJECT });
    let mut options = ResourcePolicyOptions::default();
    options.object_limit = Some(100);
    let policy = ResourcePolicy::new(&options);
    assert!(policy.is_ok(), "policy");
    let (opened, bytes) = peak(|| {
        Source::from_read_at(&read_at, &ReadAtSourceOptions::default())
            .and_then(|source| {
                Document::open(&source, policy.as_ref().ok(), &OpenOptions::default())
            })
            .map(drop)
            .map_err(|error| error.code())
    });
    assert_eq!(opened, Err(ErrorCode::LimitExceeded), "open");
    assert!((1..=LIMITED_BOUND).contains(&bytes), "peak {bytes} bytes");
}
