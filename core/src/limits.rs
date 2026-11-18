//! Resource policies.

use alloc::sync::Arc;
use std::path::PathBuf;

use crate::{ErrorCode, PdfError};

/// Nesting depth of arrays and dictionaries when a policy sets none.
pub(crate) const DEFAULT_DEPTH_LIMIT: u32 = 256;

/// Limits shared by the documents opened with it.
#[derive(Clone, Debug)]
pub struct ResourcePolicy {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl ResourcePolicy {
    /// A policy; documents opened with it share its budget.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::InvalidArgument`], [`ErrorCode::Io`].
    pub fn new(options: &ResourcePolicyOptions) -> Result<Self, PdfError> {
        let _: &ResourcePolicyOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "ResourcePolicy::new",
        ))
    }
}

/// Options for [`ResourcePolicy::new`]; `None` means a safe library default.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ResourcePolicyOptions {
    /// Bytes of decoded data the cache keeps.
    pub cache_limit: Option<u64>,
    /// Threads one operation may use.
    pub concurrency: Option<u32>,
    /// Bytes one stream may decode to.
    pub decoded_limit: Option<u64>,
    /// Nesting depth of objects.
    pub depth_limit: Option<u32>,
    /// Bytes of held work (overlay, bookkeeping, buffers) before spilling or failing.
    pub memory_budget: Option<u64>,
    /// Objects per document.
    pub object_limit: Option<u64>,
    /// The directory for temporary storage; `None`: none.
    pub temporary_directory: Option<PathBuf>,
    /// Bytes the temporary directory may hold.
    pub temporary_quota: Option<u64>,
    /// Warnings recorded per document.
    pub warning_limit: Option<u32>,
}
