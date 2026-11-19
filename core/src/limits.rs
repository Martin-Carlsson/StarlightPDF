//! Resource policies.

use alloc::sync::Arc;
use std::path::PathBuf;

use crate::{ErrorCode, PdfError};

/// Nesting depth of arrays and dictionaries unless a policy sets one.
pub(crate) const DEFAULT_DEPTH_LIMIT: u32 = 256;

/// Limits shared by the documents opened with it.
#[derive(Clone, Debug)]
pub struct ResourcePolicy {
    /// Nesting depth of arrays and dictionaries.
    depth_limit: Arc<u32>,
}

impl ResourcePolicy {
    /// Nesting depth of arrays and dictionaries.
    pub(crate) fn depth_limit(&self) -> u32 {
        *self.depth_limit
    }

    /// A policy; documents opened with it share its budget.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::InvalidArgument`].
    pub fn new(options: &ResourcePolicyOptions) -> Result<Self, PdfError> {
        /// The operation.
        const NEW: &str = "ResourcePolicy::new";
        if options.cache_limit.is_some()
            || options.concurrency.is_some()
            || options.decoded_limit.is_some()
            || options.memory_budget.is_some()
            || options.object_limit.is_some()
            || options.temporary_directory.is_some()
            || options.temporary_quota.is_some()
            || options.warning_limit.is_some()
        {
            return Err(PdfError::new(ErrorCode::NotImplemented, NEW));
        }
        match options.depth_limit {
            Some(0) => Err(PdfError::new(ErrorCode::InvalidArgument, NEW)),
            limit => Ok(Self {
                depth_limit: Arc::new(limit.unwrap_or(DEFAULT_DEPTH_LIMIT)),
            }),
        }
    }
}

/// Options for [`ResourcePolicy::new`]; `None` means a safe library default.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ResourcePolicyOptions {
    /// Bytes of decoded data the cache keeps.
    ///
    /// Not implemented: [`ResourcePolicy::new`] fails with [`ErrorCode::NotImplemented`] when set.
    pub cache_limit: Option<u64>,
    /// Threads one operation may use.
    ///
    /// Not implemented: [`ResourcePolicy::new`] fails with [`ErrorCode::NotImplemented`] when set.
    pub concurrency: Option<u32>,
    /// Bytes one stream may decode to.
    ///
    /// Not implemented: [`ResourcePolicy::new`] fails with [`ErrorCode::NotImplemented`] when set.
    pub decoded_limit: Option<u64>,
    /// Nesting depth of objects; `None`: 256.
    pub depth_limit: Option<u32>,
    /// Bytes of held work (overlay, bookkeeping, buffers) before spilling or failing.
    ///
    /// Not implemented: [`ResourcePolicy::new`] fails with [`ErrorCode::NotImplemented`] when set.
    pub memory_budget: Option<u64>,
    /// Objects per document.
    ///
    /// Not implemented: [`ResourcePolicy::new`] fails with [`ErrorCode::NotImplemented`] when set.
    pub object_limit: Option<u64>,
    /// The directory for temporary storage; `None`: none.
    ///
    /// Not implemented: [`ResourcePolicy::new`] fails with [`ErrorCode::NotImplemented`] when set.
    pub temporary_directory: Option<PathBuf>,
    /// Bytes the temporary directory may hold.
    ///
    /// Not implemented: [`ResourcePolicy::new`] fails with [`ErrorCode::NotImplemented`] when set.
    pub temporary_quota: Option<u64>,
    /// Warnings recorded per document.
    ///
    /// Not implemented: [`ResourcePolicy::new`] fails with [`ErrorCode::NotImplemented`] when set.
    pub warning_limit: Option<u32>,
}
