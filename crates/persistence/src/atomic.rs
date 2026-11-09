//! Crash-safe file writing and size-limited reading.
//!
//! [`atomic_write`] writes to a temporary file in the destination folder,
//! flushes it to disk and renames it over the destination. A rename within
//! one file system is atomic on every supported platform, so readers (and a
//! crashed or killed application) see either the complete old file or the
//! complete new one, never a partial write.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::PersistenceError;

/// Distinguishes temporary files created by concurrent writers of one process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Suffix of temporary files (`.<name>.<pid>.<n>.tmp`), so leftovers of a
/// crash are recognisable.
const TEMP_SUFFIX: &str = ".tmp";

/// Longest part of the destination name copied into a temporary name, so the
/// temporary name stays within the usual 255-byte file-name limit even for
/// destinations whose own name is close to it.
const MAX_TEMP_STEM_BYTES: usize = 120;

fn temp_path_for(path: &Path) -> Result<PathBuf, PersistenceError> {
    let name = path
        .file_name()
        .ok_or_else(|| PersistenceError::InvalidData("the destination has no file name".to_string()))?
        .to_string_lossy();
    let mut stem_end = name.len().min(MAX_TEMP_STEM_BYTES);
    while !name.is_char_boundary(stem_end) {
        stem_end -= 1;
    }
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_name = format!(".{}.{}.{n}{TEMP_SUFFIX}", &name[..stem_end], std::process::id());
    Ok(path.with_file_name(temp_name))
}

/// Writes `bytes` to `path` atomically (temporary file + flush + rename).
/// The parent folder must exist. On failure the destination is unchanged and
/// the temporary file is removed.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), PersistenceError> {
    write_inner(path, bytes).map_err(|e| PersistenceError::at(path, e))
}

fn write_inner(path: &Path, bytes: &[u8]) -> Result<(), PersistenceError> {
    let temp = temp_path_for(path)?;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| PersistenceError::io("create a temporary file next to the destination", e))?;
        file.write_all(bytes).map_err(|e| PersistenceError::io("write the file", e))?;
        file.sync_all().map_err(|e| PersistenceError::io("flush the file to disk", e))?;
        drop(file);
        fs::rename(&temp, path).map_err(|e| PersistenceError::io("replace the file", e))
    })();
    if result.is_err() {
        // Best-effort cleanup; the original error is what the user needs.
        if temp.exists() {
            let _ignored = fs::remove_file(&temp);
        }
        return result;
    }
    sync_parent(path);
    Ok(())
}

/// Flushes the directory entry of a renamed file on Unix so the rename
/// survives a power loss. Failure only weakens durability, never
/// consistency, so it is not reported.
fn sync_parent(path: &Path) {
    #[cfg(unix)]
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
        && let Ok(dir) = File::open(parent)
    {
        let _ignored = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Reads a whole file, refusing files larger than `max_bytes` before
/// allocating memory for them.
pub fn read_limited(path: &Path, max_bytes: u64) -> Result<Vec<u8>, PersistenceError> {
    let read = || -> Result<Vec<u8>, PersistenceError> {
        let file = File::open(path).map_err(|e| PersistenceError::io("open the file", e))?;
        let size = file.metadata().map_err(|e| PersistenceError::io("read the file size", e))?.len();
        if size > max_bytes {
            return Err(PersistenceError::TooLarge { what: "the file", size, limit: max_bytes });
        }
        let mut bytes = Vec::with_capacity(size as usize);
        // `take` guards against a file that grows while it is being read.
        file.take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| PersistenceError::io("read the file", e))?;
        if bytes.len() as u64 > max_bytes {
            return Err(PersistenceError::TooLarge { what: "the file", size: bytes.len() as u64, limit: max_bytes });
        }
        Ok(bytes)
    };
    read().map_err(|e| PersistenceError::at(path, e))
}

#[cfg(test)]
pub(crate) mod test_dir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Unique temporary folder removed on drop.
    pub(crate) struct TestDir(PathBuf);

    impl TestDir {
        pub(crate) fn new(name: &str) -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("emergence-lab-persistence-{}-{name}-{n}", std::process::id()));
            if path.exists() {
                std::fs::remove_dir_all(&path).unwrap();
            }
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }

        pub(crate) fn file_names(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ignored = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_dir::TestDir;
    use super::*;

    #[test]
    fn writes_and_replaces_without_leftovers() {
        let dir = TestDir::new("atomic");
        let path = dir.path().join("data.json");
        atomic_write(&path, b"first").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");
        atomic_write(&path, b"second, longer content").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second, longer content");
        assert_eq!(dir.file_names(), vec!["data.json"]);
    }

    #[test]
    fn failure_keeps_destination_and_removes_temp() {
        let dir = TestDir::new("atomic-fail");
        // Destination is a folder: the rename fails after the temporary file
        // was written; the folder is untouched and the temporary file removed.
        let folder = dir.path().join("folder.json");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("inside"), b"x").unwrap();
        let err = atomic_write(&folder, b"data").unwrap_err();
        assert_eq!(err.path(), Some(folder.as_path()));
        assert!(folder.join("inside").exists());
        // Missing parent folder: clear I/O error naming the path.
        let orphan = dir.path().join("missing").join("file.ron");
        let err = atomic_write(&orphan, b"data").unwrap_err();
        assert!(matches!(err.root(), PersistenceError::Io { .. }));
        assert!(err.to_string().contains("file.ron"), "{err}");
        assert_eq!(dir.file_names(), vec!["folder.json"]);
    }

    #[test]
    fn concurrent_writers_never_leave_partial_files() {
        let dir = TestDir::new("atomic-concurrent");
        let path = dir.path().join("shared.bin");
        let contents: Vec<Vec<u8>> = (0..8u8).map(|i| vec![i; 64 * 1024 + i as usize]).collect();
        std::thread::scope(|s| {
            for c in &contents {
                let path = &path;
                s.spawn(move || {
                    for _ in 0..5 {
                        atomic_write(path, c).unwrap();
                    }
                });
            }
            // Readers only ever see one complete version.
            s.spawn(|| {
                for _ in 0..50 {
                    if let Ok(bytes) = fs::read(&path) {
                        assert!(contents.contains(&bytes), "partial file of {} bytes observed", bytes.len());
                    }
                }
            });
        });
        assert!(contents.contains(&fs::read(&path).unwrap()));
        assert_eq!(dir.file_names(), vec!["shared.bin"]);
    }

    #[test]
    fn long_destination_names_get_short_temporary_names() {
        let dir = TestDir::new("atomic-long");
        // 250 bytes of multi-byte characters: the temporary name must be cut
        // on a character boundary and stay below the file-name limit.
        let name = format!("{}.json", "é".repeat(122));
        let path = dir.path().join(&name);
        atomic_write(&path, b"{}").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"{}");
        assert_eq!(dir.file_names(), vec![name]);
        let temp = temp_path_for(&path).unwrap();
        let temp_name = temp.file_name().unwrap().to_str().unwrap();
        assert!(temp_name.len() < 160 && temp_name.starts_with(".é") && temp_name.ends_with(TEMP_SUFFIX));
    }

    #[test]
    fn read_limit_is_enforced() {
        let dir = TestDir::new("read-limit");
        let path = dir.path().join("big.json");
        fs::write(&path, vec![b' '; 1000]).unwrap();
        assert_eq!(read_limited(&path, 1000).unwrap().len(), 1000);
        let err = read_limited(&path, 999).unwrap_err();
        assert!(matches!(err.root(), PersistenceError::TooLarge { size: 1000, limit: 999, .. }));
        // An unlimited read does not overflow the limit arithmetic.
        assert_eq!(read_limited(&path, u64::MAX).unwrap().len(), 1000);
        assert!(matches!(
            read_limited(&dir.path().join("absent.json"), 10).unwrap_err().root(),
            PersistenceError::Io { .. }
        ));
    }
}
