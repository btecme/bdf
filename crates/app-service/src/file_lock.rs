//! An exclusive lock on a file, held across processes: the grant file and the audit log each take turns between the
//! processes that write them with one of these. It is released when the value is dropped, or when the process ends,
//! however it ends.

use std::io;
use std::path::Path;
use std::time::Duration;

/// Why a lock was not taken.
#[derive(Debug)]
pub(crate) enum LockError {
    /// Another process held it for the whole wait (the browser build has no files to wait on, so it never says this).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Busy,
    Io(io::Error),
}

pub(crate) struct FileLock {
    _file: std::fs::File,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileLock {
    /// Lock the file at `path` (created, empty, if it is not there), waiting up to `wait` for another holder.
    pub(crate) fn acquire(path: &Path, wait: Duration) -> Result<Self, LockError> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(LockError::Io)?;
        }
        let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).map_err(LockError::Io)?;
        let deadline = std::time::Instant::now() + wait;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(std::fs::TryLockError::WouldBlock) => {}
                Err(std::fs::TryLockError::Error(e)) => return Err(LockError::Io(e)),
            }
            if std::time::Instant::now() >= deadline {
                return Err(LockError::Busy);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl FileLock {
    pub(crate) fn acquire(_path: &Path, _wait: Duration) -> Result<Self, LockError> {
        Err(LockError::Io(io::Error::new(io::ErrorKind::Unsupported, "file locks are not available in the browser build")))
    }
}
