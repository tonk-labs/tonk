use std::fs::{File, OpenOptions};
use std::io::{Error, Read, Result, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::MutexGuard;
use std::sync::{Arc, Mutex};

/// A cloneable wrapper around file operations with better performance
/// Uses a shared file handle with synchronization to avoid reopening files
#[derive(Clone, Debug)]
pub struct CloneableFile {
    inner: Arc<Mutex<CloneableFileInner>>,
}

#[derive(Debug)]
struct CloneableFileInner {
    file: File,
    path: PathBuf,
}

impl CloneableFile {
    /// Create a new CloneableFile from a path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new().read(true).write(true).open(&path)?;

        Ok(Self {
            inner: Arc::new(Mutex::new(CloneableFileInner { file, path })),
        })
    }

    /// Get the path of this file
    pub fn path(&self) -> Result<PathBuf> {
        let inner = self.lock_inner()?;
        Ok(inner.path.clone())
    }

    /// Helper function to acquire the inner lock
    fn lock_inner(&self) -> Result<MutexGuard<'_, CloneableFileInner>> {
        self.inner
            .lock()
            .map_err(|_| Error::other("Failed to acquire file lock"))
    }
}

impl Read for CloneableFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let mut inner = self.lock_inner()?;
        inner.file.read(buf)
    }
}

impl Write for CloneableFile {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let mut inner = self.lock_inner()?;
        inner.file.write(buf)
    }

    fn flush(&mut self) -> Result<()> {
        let mut inner = self.lock_inner()?;
        inner.file.flush()
    }
}

impl Seek for CloneableFile {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let mut inner = self.lock_inner()?;
        inner.file.seek(pos)
    }
}
