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
}
