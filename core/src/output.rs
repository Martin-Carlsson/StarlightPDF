//! Output: sinks.

use alloc::sync::Arc;
use core::fmt::{self, Debug, Formatter};
use core::mem::take;
use std::io::{self, ErrorKind};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use crate::{ErrorCode, PdfError};

/// Append-only output a caller implements.
pub trait Append: Send + Sync {
    /// Appends `bytes`.
    ///
    /// # Errors
    ///
    /// Any I/O error; its kind becomes [`PdfError::io_cause`](crate::PdfError::io_cause).
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

/// Bytes held in memory.
struct Memory(Mutex<Vec<u8>>);

/// Options for [`Sink::in_memory`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct MemorySinkOptions;

/// Output that only appends.
#[derive(Clone)]
pub struct Sink {
    /// The output.
    append: Arc<dyn Append>,
    /// The output, when it is in memory.
    memory: Option<Arc<Memory>>,
}

impl Append for Memory {
    fn append(&self, bytes: &[u8]) -> io::Result<()> {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        held.try_reserve(bytes.len())
            .map_err(|error| io::Error::new(ErrorKind::OutOfMemory, error))?;
        held.extend_from_slice(bytes);
        drop(held);
        Ok(())
    }
}

impl Debug for Sink {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sink").finish_non_exhaustive()
    }
}

impl Sink {
    /// A sink over a caller's `Append`, which gets each write as it happens.
    ///
    /// # Errors
    ///
    /// None.
    pub fn from_append(
        append: &Arc<dyn Append>,
        options: &AppendSinkOptions,
    ) -> Result<Self, PdfError> {
        let _: &AppendSinkOptions = options;
        Ok(Self {
            append: Arc::clone(append),
            memory: None,
        })
    }

    /// A sink over a file.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::InvalidArgument`].
    pub fn from_file(options: &FileSinkOptions) -> Result<Self, PdfError> {
        let _: &FileSinkOptions = options;
        Err(PdfError::new(ErrorCode::NotImplemented, "Sink::from_file"))
    }

    /// A sink that holds its bytes in memory, for [`Sink::take_bytes`].
    ///
    /// # Errors
    ///
    /// None.
    pub fn in_memory(options: &MemorySinkOptions) -> Result<Self, PdfError> {
        let _: &MemorySinkOptions = options;
        let memory = Arc::new(Memory(Mutex::new(Vec::new())));
        Ok(Self {
            append: Arc::<Memory>::clone(&memory),
            memory: Some(memory),
        })
    }

    /// Takes the bytes appended so far, leaving the sink empty.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidArgument`] unless the sink is in memory.
    pub fn take_bytes(&self) -> Result<Vec<u8>, PdfError> {
        let memory = self
            .memory
            .as_ref()
            .ok_or_else(|| PdfError::new(ErrorCode::InvalidArgument, "Sink::take_bytes"))?;
        Ok(take(
            &mut *memory.0.lock().unwrap_or_else(PoisonError::into_inner),
        ))
    }

    /// Appends `bytes`.
    pub(crate) fn write(&self, bytes: &[u8], operation: &'static str) -> Result<(), PdfError> {
        self.append
            .append(bytes)
            .map_err(|error| PdfError::io(&error, operation))
    }
}
