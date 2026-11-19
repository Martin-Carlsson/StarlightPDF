//! Input: sources.

use alloc::sync::Arc;
use core::fmt::{self, Debug, Formatter};
use std::fs::File;
use std::io::{self, ErrorKind, Read, Seek};
#[cfg(unix)]
use std::os::unix::fs::FileExt as _;
#[cfg(windows)]
use std::os::windows::fs::FileExt as _;
use std::path::PathBuf;

use crate::{ErrorCode, PdfError};

/// Bytes in memory.
struct Bytes(BytesSourceOptions);

/// Options for [`Source::from_bytes`].
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct BytesSourceOptions {
    /// The bytes, shared, not copied.
    pub bytes: Arc<Vec<u8>>,
}

/// A file.
struct FileReadAt(File);

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
    /// Any I/O error; its kind becomes [`PdfError::io_cause`](crate::PdfError::io_cause).
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize>;

    /// The size in bytes; a forward-only `ReadAt` may fail.
    ///
    /// # Errors
    ///
    /// Any I/O error; its kind becomes [`PdfError::io_cause`](crate::PdfError::io_cause).
    fn size(&self) -> io::Result<u64>;
}

/// Options for [`Source::from_read_at`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct ReadAtSourceOptions {
    /// The caller's `ReadAt` reads only forward from offset 0, like a pipe.
    pub forward_only: bool,
}

/// Options for [`Source::from_reader`].
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct ReaderSourceOptions;

/// Input with a size and reads at an offset.
#[derive(Clone)]
pub struct Source {
    /// Reads only forward.
    forward_only: bool,
    /// The input.
    read_at: Arc<dyn ReadAt>,
}

impl ReadAt for Bytes {
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let rest = usize::try_from(offset)
            .ok()
            .and_then(|start| self.0.bytes.get(start..))
            .unwrap_or_default();
        let count = rest.len().min(buffer.len());
        buffer
            .iter_mut()
            .zip(rest)
            .for_each(|(byte, source)| *byte = *source);
        Ok(count)
    }

    fn size(&self) -> io::Result<u64> {
        u64::try_from(self.0.bytes.len()).map_err(io::Error::other)
    }
}

impl ReadAt for FileReadAt {
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        // No OS reads past `i64::MAX`; no file is that large.
        let len = buffer.len();
        let Some(capped) = i64::MAX
            .cast_unsigned()
            .checked_sub(offset)
            .map(|room| usize::try_from(room).unwrap_or(usize::MAX).min(len))
            .and_then(|end| buffer.get_mut(..end))
        else {
            return Ok(0);
        };
        #[cfg(unix)]
        let count = self.0.read_at(capped, offset);
        #[cfg(windows)]
        let count = self.0.seek_read(capped, offset);
        #[cfg(not(any(unix, windows)))]
        let count = {
            let _: &mut [u8] = capped;
            Err(io::Error::from(ErrorKind::Unsupported))
        };
        count
    }

    fn size(&self) -> io::Result<u64> {
        Ok(self.0.metadata()?.len())
    }
}

impl Debug for Source {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Source")
            .field("forward_only", &self.forward_only)
            .finish_non_exhaustive()
    }
}

impl Source {
    /// Whether the source reads only forward.
    pub(crate) const fn forward_only(&self) -> bool {
        self.forward_only
    }

    /// A source over bytes in memory.
    ///
    /// # Errors
    ///
    /// None.
    pub fn from_bytes(options: &BytesSourceOptions) -> Result<Self, PdfError> {
        Ok(Self {
            forward_only: false,
            read_at: Arc::new(Bytes(options.clone())),
        })
    }

    /// A source over a file, opened now.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Io`].
    pub fn from_file(options: &FileSourceOptions) -> Result<Self, PdfError> {
        File::open(&options.file)
            .map(|file| Self {
                forward_only: false,
                read_at: Arc::new(FileReadAt(file)),
            })
            .map_err(|error| PdfError::io(&error, "Source::from_file"))
    }

    /// A source over a caller's `ReadAt`.
    ///
    /// # Errors
    ///
    /// None.
    pub fn from_read_at(
        read_at: &Arc<dyn ReadAt>,
        options: &ReadAtSourceOptions,
    ) -> Result<Self, PdfError> {
        Ok(Self {
            forward_only: options.forward_only,
            read_at: Arc::clone(read_at),
        })
    }

    /// A source over a `Read + Seek`, which serves one read at a time.
    ///
    /// Not implemented: fails with [`ErrorCode::NotImplemented`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NotImplemented`], [`ErrorCode::Io`].
    pub fn from_reader<R>(reader: R, options: &ReaderSourceOptions) -> Result<Self, PdfError>
    where
        R: Read + Seek + Send + 'static,
    {
        drop(reader);
        let _: &ReaderSourceOptions = options;
        Err(PdfError::new(
            ErrorCode::NotImplemented,
            "Source::from_reader",
        ))
    }

    /// Fills `buffer` from `offset`, stopping early at the end; returns the count read.
    pub(crate) fn read(
        &self,
        offset: u64,
        buffer: &mut [u8],
        operation: &'static str,
    ) -> Result<usize, PdfError> {
        let mut filled = 0;
        while let Some(rest) = buffer.get_mut(filled..).filter(|rest| !rest.is_empty())
            && let Some(position) = u64::try_from(filled)
                .ok()
                .and_then(|count| offset.checked_add(count))
        {
            match self.read_at.read_at(position, rest) {
                Ok(0) => break,
                Ok(count) => filled = filled.saturating_add(count),
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => return Err(PdfError::io(&error, operation)),
            }
        }
        Ok(filled.min(buffer.len()))
    }

    /// The size in bytes.
    pub(crate) fn size(&self, operation: &'static str) -> Result<u64, PdfError> {
        self.read_at
            .size()
            .map_err(|error| PdfError::io(&error, operation))
    }
}
