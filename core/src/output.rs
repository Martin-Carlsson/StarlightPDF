//! Output: sinks.

use alloc::sync::Arc;
use core::fmt::{self, Debug, Formatter};
use core::mem::take;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Write as _};
use std::path::{Path, PathBuf};
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

/// A file written beside its target, then renamed over it.
struct FileTarget {
    /// The file written, while a write is under way, and its path.
    pending: Mutex<Option<(File, PathBuf)>>,
    /// The target, symbolic links resolved.
    target: PathBuf,
}

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
    /// The output, when it is a file.
    file: Option<Arc<FileTarget>>,
    /// The output, when it is in memory.
    memory: Option<Arc<Memory>>,
}

impl Append for FileTarget {
    fn append(&self, bytes: &[u8]) -> io::Result<()> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if pending.is_none() {
            *pending = Some(self.temporary()?);
        }
        let written = pending
            .as_mut()
            .map_or(Ok(()), |&mut (ref mut file, _)| file.write_all(bytes));
        drop(pending);
        written
    }
}

impl FileTarget {
    /// Makes `file` at `path` durable and renames it over `target`, with its permissions.
    fn commit(file: &File, path: &Path, target: &Path) -> io::Result<()> {
        file.sync_all()?;
        match fs::metadata(target) {
            Ok(metadata) => file.set_permissions(metadata.permissions())?,
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::rename(path, target)
    }

    /// Ends the write under way: on success renames the file over the target, given the
    /// target's permissions, else removes it.
    fn finish(&self, success: bool) -> io::Result<()> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if success && pending.is_none() {
            *pending = Some(self.temporary()?);
        }
        // The file closes at the end of this block, before it is removed.
        let (committed, path) = {
            let Some((file, path)) = pending.take() else {
                return Ok(());
            };
            drop(pending);
            let committed = if success {
                Self::commit(&file, &path, &self.target)
            } else {
                Ok(())
            };
            (committed, path)
        };
        if !success || committed.is_err() {
            drop(fs::remove_file(&path));
        }
        committed
    }

    /// A new file beside the target, named after it.
    fn temporary(&self) -> io::Result<(File, PathBuf)> {
        let name = self
            .target
            .file_name()
            .map_or_default(|found| found.to_string_lossy().into_owned());
        let mut attempt = 0_u32;
        loop {
            let path = self.target.with_file_name(format!(".{name}.{attempt}.tmp"));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((file, path)),
                Err(error) if error.kind() == ErrorKind::AlreadyExists && attempt < 1000 => {
                    attempt = attempt.saturating_add(1);
                }
                Err(error) => return Err(error),
            }
        }
    }
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
    /// Ends an operation's writes with its `result`: a file sink's file replaces its target on
    /// success and is removed on failure.
    pub(crate) fn finish<T>(
        &self,
        result: Result<T, PdfError>,
        operation: &'static str,
    ) -> Result<T, PdfError> {
        let Some(file) = self.file.as_ref() else {
            return result;
        };
        let ended = file.finish(result.is_ok());
        let value = result?;
        ended.map_err(|error| PdfError::io(&error, operation))?;
        Ok(value)
    }

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
            file: None,
            memory: None,
        })
    }

    /// A sink over a file; a symbolic link is resolved now. An operation writes a file beside it
    /// and, on success, renames that over it with its permissions; on failure the file stays.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidArgument`] for an empty path, [`ErrorCode::Io`].
    pub fn from_file(options: &FileSinkOptions) -> Result<Self, PdfError> {
        /// The operation.
        const FROM_FILE: &str = "Sink::from_file";
        if options.file.as_os_str().is_empty() {
            return Err(PdfError::new(ErrorCode::InvalidArgument, FROM_FILE));
        }
        let target = resolve(&options.file).map_err(|error| PdfError::io(&error, FROM_FILE))?;
        let file = Arc::new(FileTarget {
            pending: Mutex::new(None),
            target,
        });
        Ok(Self {
            append: Arc::<FileTarget>::clone(&file),
            file: Some(file),
            memory: None,
        })
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
            file: None,
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

/// `path` with its symbolic links resolved, a dangling one followed to where it leads.
fn resolve(path: &Path) -> io::Result<PathBuf> {
    /// Links followed before giving up, as Linux's `SYMLOOP_MAX`.
    const LINKS: u32 = 40;
    let mut current = path.to_path_buf();
    for _ in 0..LINKS {
        match fs::canonicalize(&current) {
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            other => return other,
        }
        if !fs::symlink_metadata(&current).is_ok_and(|found| found.file_type().is_symlink()) {
            return Ok(current);
        }
        let to = fs::read_link(&current)?;
        current = current
            .parent()
            .map_or_else(|| to.clone(), |dir| dir.join(&to));
    }
    Err(io::Error::from(ErrorKind::InvalidInput))
}
