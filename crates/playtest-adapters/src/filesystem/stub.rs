//! Stub `FileSystem`: in-memory `HashMap` of path → bytes.
//!
//! No real filesystem access at all. Unit tests construct a fake tree,
//! inject it, and assert on its contents afterwards.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use playtest_ports::{FileSystem, FsError};

#[derive(Debug, Default, Clone)]
pub struct StubFileSystem {
    files: HashMap<PathBuf, Vec<u8>>,
}

impl StubFileSystem {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-populate the fake tree with a file.
    pub fn insert(&mut self, path: impl Into<PathBuf>, bytes: impl Into<Vec<u8>>) {
        self.files.insert(path.into(), bytes.into());
    }

    /// Directly read the in-memory contents; handy for assertions.
    #[must_use]
    pub fn snapshot(&self, path: &Path) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }
}

impl FileSystem for StubFileSystem {
    fn read(&self, path: &Path) -> Result<Vec<u8>, FsError> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| FsError::NotFound {
                path: path.display().to_string(),
            })
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> Result<(), FsError> {
        self.files.insert(path.to_path_buf(), bytes.to_vec());
        Ok(())
    }

    fn append_line(&mut self, path: &Path, line: &str) -> Result<(), FsError> {
        let entry = self.files.entry(path.to_path_buf()).or_default();
        entry.extend_from_slice(line.as_bytes());
        if !line.ends_with('\n') {
            entry.push(b'\n');
        }
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_roundtrips() {
        let mut fs = StubFileSystem::new();
        fs.write(Path::new("/a/b.txt"), b"hello").unwrap();
        assert_eq!(fs.read(Path::new("/a/b.txt")).unwrap(), b"hello");
    }

    #[test]
    fn read_missing_path_returns_not_found() {
        let fs = StubFileSystem::new();
        let err = fs.read(Path::new("/nope")).unwrap_err();
        assert!(matches!(err, FsError::NotFound { .. }));
    }

    #[test]
    fn append_line_creates_file_and_adds_newline() {
        let mut fs = StubFileSystem::new();
        fs.append_line(Path::new("/log"), "one").unwrap();
        fs.append_line(Path::new("/log"), "two\n").unwrap();
        assert_eq!(fs.read(Path::new("/log")).unwrap(), b"one\ntwo\n");
    }

    #[test]
    fn exists_reports_presence() {
        let mut fs = StubFileSystem::new();
        assert!(!fs.exists(Path::new("/x")));
        fs.write(Path::new("/x"), b"").unwrap();
        assert!(fs.exists(Path::new("/x")));
    }
}
