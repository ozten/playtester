//! Production `FileSystem`: thin `std::fs` wrapper.
//!
//! Creates parent directories on write/append so callers don't need to
//! `mkdir -p` themselves. That matches the port contract documented on
//! the trait.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use playtest_ports::{FileSystem, FsError};

#[derive(Debug, Default, Clone, Copy)]
pub struct ProductionFileSystem;

impl ProductionFileSystem {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

fn io_err(path: &Path, source: std::io::Error) -> FsError {
    FsError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn ensure_parent(path: &Path) -> Result<(), FsError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| io_err(path, source))?;
    }
    Ok(())
}

impl FileSystem for ProductionFileSystem {
    fn read(&self, path: &Path) -> Result<Vec<u8>, FsError> {
        match fs::read(path) {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(FsError::NotFound {
                path: path.display().to_string(),
            }),
            Err(e) => Err(io_err(path, e)),
        }
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> Result<(), FsError> {
        ensure_parent(path)?;
        fs::write(path, bytes).map_err(|e| io_err(path, e))
    }

    fn append_line(&mut self, path: &Path, line: &str) -> Result<(), FsError> {
        ensure_parent(path)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| io_err(path, e))?;
        file.write_all(line.as_bytes())
            .map_err(|e| io_err(path, e))?;
        if !line.ends_with('\n') {
            file.write_all(b"\n").map_err(|e| io_err(path, e))?;
        }
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_creates_parents_and_reads_back() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a/b/c.txt");
        let mut fs = ProductionFileSystem::new();
        fs.write(&path, b"hi").unwrap();
        assert_eq!(fs.read(&path).unwrap(), b"hi");
        assert!(fs.exists(&path));
    }

    #[test]
    fn read_missing_yields_not_found() {
        let dir = tempdir().unwrap();
        let fs = ProductionFileSystem::new();
        let err = fs.read(&dir.path().join("missing.txt")).unwrap_err();
        assert!(matches!(err, FsError::NotFound { .. }));
    }

    #[test]
    fn append_line_adds_newline_if_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let mut fs = ProductionFileSystem::new();
        fs.append_line(&path, "a").unwrap();
        fs.append_line(&path, "b\n").unwrap();
        assert_eq!(fs.read(&path).unwrap(), b"a\nb\n");
    }
}
