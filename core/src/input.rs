//! Input: sources.

use alloc::sync::Arc;
use std::io;
use std::path::PathBuf;

use crate::{Error, ErrorCode};

/// Options for [`Source::from_bytes`].
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct BytesSourceOptions {
    /// The bytes, shared, not copied.
    pub bytes: Arc<Vec<u8>>,
}

/// Options for [`Source::from_file`].
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FileSourceOptions {
    /// The file.
    pub file: PathBuf,
}

/// Random-access input a caller implements.
pub trait ReadAt: Send + Sync {
    /// Reads into `buffer` from `offset`; returns the count read, 0 at the end.
    ///
    /// # Errors
    ///
    /// Any I/O error; its kind becomes [`Error::io_cause`](crate::Error::io_cause).
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize>;

    /// The size in bytes; a forward-only `ReadAt` may fail.
    ///
    /// # Errors
    ///
    /// Any I/O error; its kind becomes [`Error::io_cause`](crate::Error::io_cause).
    fn size(&self) -> io::Result<u64>;
}

/// Options for [`Source::from_read_at`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct ReadAtSourceOptions {
    /// The caller's `ReadAt` reads only forward from offset 0, like a pipe.
    pub forward_only: bool,
}

/// Input with a size and reads at an offset.
#[derive(Clone, Debug)]
pub struct Source {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Source {
    /// A source over bytes in memory.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`].
    pub fn from_bytes(options: &BytesSourceOptions) -> Result<Self, Error> {
        let _: &BytesSourceOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Source::from_bytes"))
    }

    /// A source over a file, opened now.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::Io`].
    pub fn from_file(options: &FileSourceOptions) -> Result<Self, Error> {
        let _: &FileSourceOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Source::from_file"))
    }

    /// A source over a caller's `ReadAt`.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`].
    pub fn from_read_at(
        read_at: &Arc<dyn ReadAt>,
        options: &ReadAtSourceOptions,
    ) -> Result<Self, Error> {
        let _: &Arc<dyn ReadAt> = read_at;
        let _: &ReadAtSourceOptions = options;
        Err(Error::new(
            ErrorCode::NotImplemented,
            "Source::from_read_at",
        ))
    }
}
