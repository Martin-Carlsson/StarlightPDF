//! Resource policies.

use alloc::sync::Arc;
use std::path::PathBuf;

use crate::{ErrorCode, PdfError};

/// Bytes one stream may decode to unless a policy sets it: 1 GiB.
const DEFAULT_DECODED_LIMIT: u64 = 0x4000_0000;

/// Nesting depth of arrays and dictionaries unless a policy sets one.
const DEFAULT_DEPTH_LIMIT: u32 = 256;

/// Objects per document unless a policy sets it: 16,777,216.
const DEFAULT_OBJECT_LIMIT: u64 = 0x0100_0000;

/// Warnings kept per document unless a policy sets it.
const DEFAULT_WARNING_LIMIT: u32 = 1_000;

/// The limits of a policy.
#[derive(Debug)]
struct Limits {
    /// Bytes one stream may decode to.
    decoded: u64,
    /// Nesting depth of arrays and dictionaries.
    depth: u32,
    /// Objects per document.
    objects: u64,
    /// Warnings kept per document.
    warnings: u32,
}

/// Limits shared by the documents opened with it.
#[derive(Clone, Debug)]
pub struct ResourcePolicy {
    /// The limits.
    limits: Arc<Limits>,
}

impl ResourcePolicy {
    /// Bytes one stream may decode to.
    pub(crate) fn decoded_limit(&self) -> u64 {
        self.limits.decoded
    }

    /// The policy of a document opened without one.
    pub(crate) fn defaults() -> Self {
        Self {
            limits: Arc::new(Limits {
                decoded: DEFAULT_DECODED_LIMIT,
                depth: DEFAULT_DEPTH_LIMIT,
                objects: DEFAULT_OBJECT_LIMIT,
                warnings: DEFAULT_WARNING_LIMIT,
            }),
        }
    }

    /// Nesting depth of arrays and dictionaries.
    pub(crate) fn depth_limit(&self) -> u32 {
        self.limits.depth
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
            || options.memory_budget.is_some()
            || options.temporary_directory.is_some()
            || options.temporary_quota.is_some()
        {
            return Err(PdfError::new(ErrorCode::NotImplemented, NEW));
        }
        if [
            options.decoded_limit,
            options.depth_limit.map(u64::from),
            options.object_limit,
        ]
        .contains(&Some(0))
        {
            return Err(PdfError::new(ErrorCode::InvalidArgument, NEW));
        }
        Ok(Self {
            limits: Arc::new(Limits {
                decoded: options.decoded_limit.unwrap_or(DEFAULT_DECODED_LIMIT),
                depth: options.depth_limit.unwrap_or(DEFAULT_DEPTH_LIMIT),
                objects: options.object_limit.unwrap_or(DEFAULT_OBJECT_LIMIT),
                warnings: options.warning_limit.unwrap_or(DEFAULT_WARNING_LIMIT),
            }),
        })
    }

    /// Objects per document.
    pub(crate) fn object_limit(&self) -> u64 {
        self.limits.objects
    }

    /// Warnings kept per document.
    pub(crate) fn warning_limit(&self) -> u32 {
        self.limits.warnings
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
    /// Bytes one stream may decode to; `None`: 1 GiB.
    pub decoded_limit: Option<u64>,
    /// Nesting depth of objects; `None`: 256.
    pub depth_limit: Option<u32>,
    /// Bytes of held work (overlay, bookkeeping, buffers) before spilling or failing.
    ///
    /// Not implemented: [`ResourcePolicy::new`] fails with [`ErrorCode::NotImplemented`] when set.
    pub memory_budget: Option<u64>,
    /// Objects in use per document; `None`: 16,777,216.
    pub object_limit: Option<u64>,
    /// The directory for temporary storage; `None`: none.
    ///
    /// Not implemented: [`ResourcePolicy::new`] fails with [`ErrorCode::NotImplemented`] when set.
    pub temporary_directory: Option<PathBuf>,
    /// Bytes the temporary directory may hold.
    ///
    /// Not implemented: [`ResourcePolicy::new`] fails with [`ErrorCode::NotImplemented`] when set.
    pub temporary_quota: Option<u64>,
    /// Warnings recorded per document; `None`: 1,000.
    pub warning_limit: Option<u32>,
}
