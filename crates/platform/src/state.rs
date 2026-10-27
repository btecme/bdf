//! Where the per-user access files live: the grants a person has issued and the record of what clients did (BUILD-008).
//!
//! One folder per operating-system user, in the place that user's applications keep their data, so another account does not
//! see it and a roaming profile does not carry it away:
//!
//! * Windows: `%LOCALAPPDATA%\PdfCraft\Access`
//! * macOS: `~/Library/Application Support/PdfCraft/Access`
//! * elsewhere: `$XDG_DATA_HOME/pdfcraft/access`, or `~/.local/share/pdfcraft/access`
//!
//! The product folder name follows the applications' existing data folders (the recovery store and the clipboard cache use
//! the same `PdfCraft` name); it is one constant, so renaming the product is one edit. Nothing here creates the folder or
//! checks that it is private: the files written into it do that (`crate::private_files`).

use std::ffi::OsString;
use std::path::PathBuf;

/// The folder name the applications use under the user's data folder.
const PRODUCT_DIR: &str = "PdfCraft";

/// The per-user folder for access files, or `None` if the environment does not say where the user's data goes.
pub fn access_dir() -> Option<PathBuf> {
    access_dir_with(&|name| std::env::var_os(name))
}

/// [`access_dir`], reading the environment through `env` so a test can supply one. A value that is empty or not an absolute
/// path is not used: a relative path would put the files wherever the process happened to be started.
pub fn access_dir_with(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let absolute = |name: &str| env(name).map(PathBuf::from).filter(|p| p.is_absolute());
    #[cfg(windows)]
    {
        absolute("LOCALAPPDATA").map(|d| d.join(PRODUCT_DIR).join("Access"))
    }
    #[cfg(target_os = "macos")]
    {
        absolute("HOME").map(|d| d.join("Library").join("Application Support").join(PRODUCT_DIR).join("Access"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let lower = PRODUCT_DIR.to_lowercase();
        absolute("XDG_DATA_HOME")
            .map(|d| d.join(&lower).join("access"))
            .or_else(|| absolute("HOME").map(|d| d.join(".local").join("share").join(&lower).join("access")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| OsString::from(*v))
    }

    #[cfg(windows)]
    #[test]
    fn the_folder_is_under_the_users_local_data() {
        let dir = access_dir_with(&env(&[("LOCALAPPDATA", r"C:\Users\someone\AppData\Local")]));
        assert_eq!(dir.as_deref(), Some(Path::new(r"C:\Users\someone\AppData\Local\PdfCraft\Access")));
        assert_eq!(access_dir_with(&env(&[])), None);
        assert_eq!(access_dir_with(&env(&[("LOCALAPPDATA", "")])), None);
        assert_eq!(access_dir_with(&env(&[("LOCALAPPDATA", r"relative\path")])), None, "a relative folder is not used");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_folder_is_under_application_support() {
        let dir = access_dir_with(&env(&[("HOME", "/Users/someone")]));
        assert_eq!(dir.as_deref(), Some(Path::new("/Users/someone/Library/Application Support/PdfCraft/Access")));
        assert_eq!(access_dir_with(&env(&[("HOME", "relative")])), None);
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn the_folder_follows_the_xdg_data_home_then_home() {
        assert_eq!(
            access_dir_with(&env(&[("XDG_DATA_HOME", "/data"), ("HOME", "/home/someone")])).as_deref(),
            Some(Path::new("/data/pdfcraft/access"))
        );
        assert_eq!(access_dir_with(&env(&[("HOME", "/home/someone")])).as_deref(), Some(Path::new("/home/someone/.local/share/pdfcraft/access")));
        assert_eq!(
            access_dir_with(&env(&[("XDG_DATA_HOME", "relative"), ("HOME", "/home/someone")])).as_deref(),
            Some(Path::new("/home/someone/.local/share/pdfcraft/access"))
        );
        assert_eq!(access_dir_with(&env(&[])), None);
        assert_eq!(access_dir_with(&env(&[("HOME", "")])), None);
    }
}
