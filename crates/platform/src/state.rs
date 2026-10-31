//! Where the per-user access files live: the grants a person has issued and the record of what clients did (BUILD-008).
//!
//! One folder per operating-system user, in the place that user's applications keep their data, so another account does not
//! see it and a roaming profile does not carry it away:
//!
//! * Windows: `%LOCALAPPDATA%\BDF\Access`
//! * macOS: `~/Library/Application Support/BDF/Access`
//! * elsewhere: `$XDG_DATA_HOME/bdf/access`, or `~/.local/share/bdf/access`
//!
//! The folder is BDF's own (the owner's decision, 2026-10-31; the build specification requires a BDF-specific directory).
//! It is deliberately **not** the inherited `PdfCraft` folder: access grants and their secrets are BDF's, and nothing
//! here reads, imports or shares anything the upstream application left under that name. The applications' older data
//! folders (settings, the recovery store, the clipboard cache) still use the `PdfCraft` name; moving them is a separate,
//! recorded decision and is not done here. The name is one constant. Nothing here creates the folder or checks that it
//! is private: the files written into it do that (`crate::private_files`).

use std::ffi::OsString;
use std::path::PathBuf;

/// The folder name BDF uses for its own per-user files, under the user's data folder.
const PRODUCT_DIR: &str = "BDF";

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
        assert_eq!(dir.as_deref(), Some(Path::new(r"C:\Users\someone\AppData\Local\BDF\Access")));
        assert_eq!(access_dir_with(&env(&[])), None);
        assert_eq!(access_dir_with(&env(&[("LOCALAPPDATA", "")])), None);
        assert_eq!(access_dir_with(&env(&[("LOCALAPPDATA", r"relative\path")])), None, "a relative folder is not used");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_folder_is_under_application_support() {
        let dir = access_dir_with(&env(&[("HOME", "/Users/someone")]));
        assert_eq!(dir.as_deref(), Some(Path::new("/Users/someone/Library/Application Support/BDF/Access")));
        assert_eq!(access_dir_with(&env(&[("HOME", "relative")])), None);
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn the_folder_follows_the_xdg_data_home_then_home() {
        assert_eq!(access_dir_with(&env(&[("XDG_DATA_HOME", "/data"), ("HOME", "/home/someone")])).as_deref(), Some(Path::new("/data/bdf/access")));
        assert_eq!(access_dir_with(&env(&[("HOME", "/home/someone")])).as_deref(), Some(Path::new("/home/someone/.local/share/bdf/access")));
        assert_eq!(
            access_dir_with(&env(&[("XDG_DATA_HOME", "relative"), ("HOME", "/home/someone")])).as_deref(),
            Some(Path::new("/home/someone/.local/share/bdf/access"))
        );
        assert_eq!(access_dir_with(&env(&[])), None);
        assert_eq!(access_dir_with(&env(&[("HOME", "")])), None);
    }
}
