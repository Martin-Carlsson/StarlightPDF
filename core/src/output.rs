//! Output: sinks.

use alloc::sync::Arc;
use std::io;
use std::path::PathBuf;

use crate::{Error, ErrorCode};

/// Append-only output a caller implements.
pub trait Append: Send + Sync {
    /// Appends `bytes`.
    ///
    /// # Errors
    ///
    /// Any I/O error; its kind becomes [`Error::io_cause`](crate::Error::io_cause).
    fn append(&self, bytes: &[u8]) -> io::Result<()>;
}

/// Options for [`Sink::from_append`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct AppendSinkOptions;

/// Options for [`Sink::from_file`].
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FileSinkOptions {
    /// The file.
    pub file: PathBuf,
}

/// Options for [`Sink::in_memory`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct MemorySinkOptions;

/// Output that only appends.
#[derive(Clone, Debug)]
pub struct Sink {
    /// Reference to the shared state.
    _shared: Arc<()>,
}

impl Sink {
    /// A sink over a caller's `Append`.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`].
    pub fn from_append(
        append: &Arc<dyn Append>,
        options: &AppendSinkOptions,
    ) -> Result<Self, Error> {
        let _: &Arc<dyn Append> = append;
        let _: &AppendSinkOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Sink::from_append"))
    }

    /// A sink over a file.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::InvalidArgument`].
    pub fn from_file(options: &FileSinkOptions) -> Result<Self, Error> {
        let _: &FileSinkOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Sink::from_file"))
    }

    /// A sink that holds its bytes in memory, for [`Sink::take_bytes`].
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`].
    pub fn in_memory(options: &MemorySinkOptions) -> Result<Self, Error> {
        let _: &MemorySinkOptions = options;
        Err(Error::new(ErrorCode::NotImplemented, "Sink::in_memory"))
    }

    /// Takes the bytes appended so far, leaving the sink empty.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::InvalidArgument`] unless the sink is in memory.
    pub fn take_bytes(&self) -> Result<Vec<u8>, Error> {
        Err(Error::new(ErrorCode::NotImplemented, "Sink::take_bytes"))
    }
}
