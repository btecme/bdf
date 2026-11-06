//! What a managed copy looks like on disk, and where the updater keeps its files.
//!
//! ```text
//! <app dir>/                 the extracted portable package (any folder the user can write to)
//!   bdf.exe, bdf-cli.exe, bdf-worker.exe, bdf-updater.exe, portable.txt, licenses, ...
//!   bdf-package.json         the list of files the package installed: makes this copy "managed"
//!   BDFData/                 the user's settings and recovery files: never touched
//!   BDFUpdate/               the updater's working files (downloads, staging, journal, helper copy, result)
//!   BDFPrevious/             the version before the last update, complete, for rollback
//! ```

use std::path::{Path, PathBuf};

use crate::package::{PACKAGE_FILE, PackageInfo};
use crate::{Result, UpdateError, Version, io_err};

/// The updater's working folder inside the app folder.
pub const UPDATE_DIR: &str = "BDFUpdate";
/// The previous version, kept after an update.
pub const PREVIOUS_DIR: &str = "BDFPrevious";

/// A managed copy of BDF.
#[derive(Clone, Debug)]
pub struct Install {
    pub app_dir: PathBuf,
    pub info: PackageInfo,
    pub version: Version,
}

/// Read the package list beside the program. A copy without one (a build from source, an MSI install, a
/// hand-assembled folder) is not managed and does not update itself.
pub fn detect(app_dir: &Path) -> Result<Install> {
    let file = app_dir.join(PACKAGE_FILE);
    let bytes = match std::fs::read(&file) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(UpdateError::Unmanaged(format!(
                "this copy of BDF was not installed from a portable package ({PACKAGE_FILE} is missing), so it cannot update itself; download the portable ZIP and extract it"
            )));
        }
        Err(e) => return Err(io_err(format!("couldn't read {PACKAGE_FILE}"))(e)),
    };
    let info = PackageInfo::parse(&bytes)
        .map_err(|e| UpdateError::Unmanaged(format!("{PACKAGE_FILE} is unusable, so this copy cannot update itself ({e})")))?;
    let version = info.parsed_version()?;
    Ok(Install { app_dir: app_dir.to_path_buf(), info, version })
}

/// `<app>/BDFUpdate`.
pub fn update_dir(app_dir: &Path) -> PathBuf {
    app_dir.join(UPDATE_DIR)
}

/// `<app>/BDFUpdate/downloads`: packages being downloaded and the verified package that is ready.
pub fn downloads_dir(app_dir: &Path) -> PathBuf {
    update_dir(app_dir).join("downloads")
}

/// `<app>/BDFPrevious`.
pub fn previous_dir(app_dir: &Path) -> PathBuf {
    app_dir.join(PREVIOUS_DIR)
}

/// Prove the app folder can be written (a folder can exist yet be read-only, or full) before anything large
/// is downloaded. Creates `BDFUpdate` if needed.
pub fn ensure_writable(app_dir: &Path) -> Result<()> {
    let dir = update_dir(app_dir);
    std::fs::create_dir_all(&dir).map_err(io_err(format!("couldn't create {}", dir.display())))?;
    let probe = dir.join(".write-test");
    std::fs::write(&probe, b"x").map_err(io_err(format!("couldn't write to {}", dir.display())))?;
    let _ = std::fs::remove_file(&probe);
    // The program files themselves are renamed, which needs the same permission on the app folder.
    let rename_probe = app_dir.join(".bdf-rename-test");
    std::fs::write(&rename_probe, b"x").map_err(io_err(format!("couldn't write to {}", app_dir.display())))?;
    let moved = dir.join(".rename-test");
    let result = std::fs::rename(&rename_probe, &moved);
    let _ = std::fs::remove_file(&rename_probe);
    let _ = std::fs::remove_file(&moved);
    result.map_err(io_err(format!("couldn't rename files in {}", app_dir.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_without_a_package_list_is_not_managed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(detect(dir.path()), Err(UpdateError::Unmanaged(_))));
        std::fs::write(dir.path().join(PACKAGE_FILE), b"not json").unwrap();
        assert!(matches!(detect(dir.path()), Err(UpdateError::Unmanaged(_))));
    }

    #[test]
    fn a_writable_folder_passes_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        ensure_writable(dir.path()).unwrap();
        let mut names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, [UPDATE_DIR]);
        assert_eq!(std::fs::read_dir(update_dir(dir.path())).unwrap().count(), 0);
    }
}
