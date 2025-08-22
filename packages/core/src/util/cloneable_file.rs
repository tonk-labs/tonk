use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A cloneable wrapper around file operations that uses sync operations
/// This reopens the file for each operation to maintain position tracking
#[derive(Clone, Debug)]
pub struct CloneableFile {
    path: PathBuf,
    position: Arc<std::sync::Mutex<u64>>,
}

impl CloneableFile {
    pub fn new<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        // Verify file exists and is readable/writable
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)?;

        Ok(Self {
            path,
            position: Arc::new(std::sync::Mutex::new(0)),
        })
    }

    fn open_file(&self) -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)
    }
}

impl Read for CloneableFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut file = self.open_file()?;
        let pos = *self.position.lock().unwrap();
        file.seek(SeekFrom::Start(pos))?;
        let bytes_read = file.read(buf)?;
        *self.position.lock().unwrap() += bytes_read as u64;
        Ok(bytes_read)
    }
}

impl Write for CloneableFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut file = self.open_file()?;
        let pos = *self.position.lock().unwrap();
        file.seek(SeekFrom::Start(pos))?;
        let bytes_written = file.write(buf)?;
        *self.position.lock().unwrap() += bytes_written as u64;
        Ok(bytes_written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut file = self.open_file()?;
        std::io::Write::flush(&mut file)
    }
}

impl Seek for CloneableFile {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let file = self.open_file()?;
        let file_size = file.metadata()?.len();

        let new_pos = match pos {
            SeekFrom::Start(offset) => offset,
            SeekFrom::End(offset) => (file_size as i64 + offset) as u64,
            SeekFrom::Current(offset) => {
                let current = *self.position.lock().unwrap();
                (current as i64 + offset) as u64
            }
        };

        *self.position.lock().unwrap() = new_pos;
        Ok(new_pos)
    }
}
