//! Optional Windows integration for a portable copy of BDF (BUILD-018): make this copy an application Windows
//! offers for PDF files, and take that back out again.
//!
//! **What registering does.** On the person's explicit request it writes per-user (`HKEY_CURRENT_USER`)
//! entries, which need no administrator rights and affect only that account:
//!
//! * a `BDF.Portable.Document` file type whose open command is this copy's `bdf.exe`;
//! * one empty value naming it under `.pdf\OpenWithProgids` (this is what puts BDF in "Open with");
//! * an `Applications\bdf.exe` entry (the friendly name "BDF" in "Open with");
//! * Default Programs capabilities (`Software\BDF\PortableCapabilities`) and the `RegisteredApplications`
//!   value that makes BDF appear in Settings > Default apps;
//! * a marker (`Software\BDF\PortableRegistration`) saying which copy registered, so removal only ever
//!   removes what a copy owns.
//!
//! **What it never does.** It never changes who the default PDF app is: Windows protects that choice (the
//! `UserChoice` hash) and only the person can make it, in the Settings page [`open_default_apps`] opens. It
//! never writes `UserChoice`, never sets `.pdf`'s own default value, never removes another application's
//! values, and never touches the machine-wide (`HKEY_LOCAL_MACHINE`) side, which the MSI uses under different
//! names (`BDF.Document`, `RegisteredApplications\BDF`), so the two can coexist.
//!
//! A copy that was only extracted writes nothing: this module runs only when asked.
//!
//! Off Windows every operation says it is unsupported.

use std::fmt;
use std::path::Path;

/// The file type BDF registers for a portable copy.
pub const PROG_ID: &str = "BDF.Portable.Document";
/// The file type the Windows installer (MSI) registers machine-wide. Read only, to recognise a person who already chose
/// the installed BDF; this module never writes it.
pub const INSTALLED_PROG_ID: &str = "BDF.Document";
/// The name under `RegisteredApplications`, and the Settings page's `registeredAppUser` argument.
pub const REGISTERED_APP: &str = "BDF Portable";
/// The Settings page where the person confirms BDF as the default (falls back to the Default apps page).
pub const DEFAULT_APPS_URI: &str = "ms-settings:defaultapps?registeredAppUser=BDF%20Portable";

/// Where the registration stands, for this copy of BDF.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// Windows integration exists on this system (Windows only).
    pub supported: bool,
    /// Some copy of BDF is registered for this user.
    pub registered: bool,
    /// The registered copy is this one.
    pub this_copy: bool,
    /// The program the registration points at.
    pub registered_exe: Option<String>,
    /// The registration points at a program that no longer exists (the folder was moved or deleted).
    pub stale: bool,
    /// BDF is this user's default PDF app (read from Windows; never written).
    pub is_default: bool,
    /// The current default PDF app's file type (such as `AcroExch.Document.DC`), if Windows says.
    pub default_progid: Option<String>,
}

impl Status {
    /// Windows says BDF opens this user's PDF files: this copy, registered and chosen in Settings, or the installed BDF.
    /// Only what Windows reports counts. Registering, or opening the Settings page, is not the choice.
    pub fn bdf_is_default(&self) -> bool {
        (self.this_copy && self.is_default) || self.default_progid.as_deref() == Some(INSTALLED_PROG_ID)
    }

    /// What the person should be told, in one sentence.
    pub fn summary(&self) -> String {
        if !self.supported {
            "Windows integration is only available on Windows.".into()
        } else if !self.registered {
            "This copy is not registered with Windows.".into()
        } else if self.this_copy && self.is_default {
            "This copy is registered with Windows and is your default PDF app.".into()
        } else if self.this_copy {
            "This copy is registered with Windows. Choose it in Windows' Default apps to open PDFs with it.".into()
        } else if self.stale {
            format!(
                "The registration points to a copy that no longer exists ({}). Refresh it to register this copy.",
                self.registered_exe.as_deref().unwrap_or("unknown")
            )
        } else {
            format!(
                "Another copy of BDF is registered ({}). Refresh to register this copy instead.",
                self.registered_exe.as_deref().unwrap_or("unknown")
            )
        }
    }
}

/// What registering did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registered {
    /// The copy that was registered before, when it was a different one and has been replaced.
    pub replaced: Option<String>,
}

/// What removing did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Removed {
    /// Nothing was registered, so nothing was removed.
    pub nothing_to_remove: bool,
}

#[derive(Debug)]
pub enum IntegrationError {
    Unsupported,
    /// The program path cannot be registered (not absolute, not a file, characters that cannot be quoted).
    BadPath(String),
    /// Another copy of BDF, which still exists, owns the registration; this one will not remove it.
    NotOwner {
        registered: String,
    },
    Registry(String),
}

impl fmt::Display for IntegrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntegrationError::Unsupported => f.write_str("Windows integration is only available on Windows"),
            IntegrationError::BadPath(m) => write!(f, "this copy cannot be registered: {m}"),
            IntegrationError::NotOwner { registered } => {
                write!(f, "another copy of BDF owns the registration ({registered}); use Refresh to register this copy instead, then remove it")
            }
            IntegrationError::Registry(m) => write!(f, "Windows refused the change: {m}"),
        }
    }
}

impl std::error::Error for IntegrationError {}

/// The command Windows runs to open a file with the copy at `exe`.
pub fn open_command(exe: &str) -> String {
    format!("\"{exe}\" \"%1\"")
}

/// Check a program path before it is written into a command line in the registry.
pub fn check_exe(exe: &Path) -> Result<String, IntegrationError> {
    let text = exe.to_str().ok_or_else(|| IntegrationError::BadPath("the path is not valid text".into()))?;
    if !exe.is_absolute() {
        return Err(IntegrationError::BadPath("the path is not absolute".into()));
    }
    if text.len() > 1000 || text.chars().any(|c| c.is_control() || c == '"') {
        return Err(IntegrationError::BadPath("the path contains a quote or control character, or is too long".into()));
    }
    if !exe.is_file() {
        return Err(IntegrationError::BadPath("the program file does not exist".into()));
    }
    Ok(text.to_string())
}

/// Register the copy at `exe` for this user.
pub fn register(exe: &Path) -> Result<Registered, IntegrationError> {
    imp::register(&imp::Layout::real(), exe)
}

/// Remove the registration if `exe` owns it (or it is orphaned: its program no longer exists).
pub fn unregister(exe: &Path) -> Result<Removed, IntegrationError> {
    imp::unregister(&imp::Layout::real(), exe)
}

/// Where the registration stands for the copy at `exe`.
pub fn status(exe: &Path) -> Status {
    imp::status(&imp::Layout::real(), exe)
}

/// Open Windows' Default apps page for BDF, where the person confirms the choice. Changes nothing itself.
pub fn open_default_apps() -> std::io::Result<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe").arg(DEFAULT_APPS_URI).spawn().map(|_| ())
    }
    #[cfg(not(windows))]
    {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Windows integration is only available on Windows"))
    }
}

#[cfg(windows)]
mod imp {
    use std::path::Path;

    use winsafe::{HKEY, RegistryValue, co};

    use super::{IntegrationError, PROG_ID, REGISTERED_APP, Registered, Removed, Status, check_exe, open_command};

    /// Where under `HKEY_CURRENT_USER` things go. The real layout is Windows'; tests use a scratch one.
    pub struct Layout {
        classes: String,
        software: String,
        user_choice: String,
    }

    impl Layout {
        pub fn real() -> Layout {
            Layout {
                classes: r"Software\Classes".into(),
                software: "Software".into(),
                user_choice: r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.pdf\UserChoice".into(),
            }
        }

        /// A layout confined to `root`, so a test never touches the real associations.
        #[cfg(test)]
        pub fn scratch(root: &str) -> Layout {
            Layout { classes: format!(r"{root}\Classes"), software: root.to_string(), user_choice: format!(r"{root}\UserChoice") }
        }

        fn progid(&self) -> String {
            format!(r"{}\{PROG_ID}", self.classes)
        }
        fn open_with(&self) -> String {
            format!(r"{}\.pdf\OpenWithProgids", self.classes)
        }
        fn application(&self) -> String {
            format!(r"{}\Applications\bdf.exe", self.classes)
        }
        fn capabilities_rel(&self) -> String {
            format!(r"{}\BDF\PortableCapabilities", self.software)
        }
        fn marker(&self) -> String {
            format!(r"{}\BDF\PortableRegistration", self.software)
        }
        fn registered_apps(&self) -> String {
            format!(r"{}\RegisteredApplications", self.software)
        }
    }

    fn err(what: &str, e: co::ERROR) -> IntegrationError {
        IntegrationError::Registry(format!("{what} ({e})"))
    }

    fn set(sub: &str, name: Option<&str>, value: &str, what: &str) -> Result<(), IntegrationError> {
        let (key, _) =
            HKEY::CURRENT_USER.RegCreateKeyEx(sub, None, co::REG_OPTION::NON_VOLATILE, co::KEY::ALL_ACCESS, None).map_err(|e| err(what, e))?;
        // An empty marker value (as Windows writes under OpenWithProgids) is REG_NONE: winsafe cannot write an empty REG_SZ.
        let data = if value.is_empty() { RegistryValue::None } else { RegistryValue::Sz(value.to_string()) };
        key.RegSetValueEx(name, data).map_err(|e| err(what, e))
    }

    fn get(sub: &str, name: Option<&str>) -> Option<String> {
        let key = HKEY::CURRENT_USER.RegOpenKeyEx(Some(sub), co::REG_OPTION::default(), co::KEY::READ).ok()?;
        match key.RegQueryValueEx(name).ok()? {
            RegistryValue::Sz(s) | RegistryValue::ExpandSz(s) => Some(s),
            _ => None,
        }
    }

    fn exists(sub: &str) -> bool {
        HKEY::CURRENT_USER.RegOpenKeyEx(Some(sub), co::REG_OPTION::default(), co::KEY::READ).is_ok()
    }

    fn delete_tree(sub: &str) -> Result<(), IntegrationError> {
        match HKEY::CURRENT_USER.RegDeleteTree(Some(sub)) {
            Ok(()) => Ok(()),
            Err(e) if e == co::ERROR::FILE_NOT_FOUND => Ok(()),
            Err(e) => Err(err("couldn't remove a registry entry", e)),
        }
    }

    fn delete_value(sub: &str, name: &str) -> Result<(), IntegrationError> {
        let Ok(key) = HKEY::CURRENT_USER.RegOpenKeyEx(Some(sub), co::REG_OPTION::default(), co::KEY::SET_VALUE) else { return Ok(()) };
        match key.RegDeleteValue(Some(name)) {
            Ok(()) => Ok(()),
            Err(e) if e == co::ERROR::FILE_NOT_FOUND => Ok(()),
            Err(e) => Err(err("couldn't remove a registry value", e)),
        }
    }

    fn same_path(a: &str, b: &str) -> bool {
        let norm = |s: &str| std::fs::canonicalize(s).map(|p| p.to_string_lossy().to_lowercase()).unwrap_or_else(|_| s.to_lowercase());
        norm(a) == norm(b)
    }

    pub fn register(layout: &Layout, exe: &Path) -> Result<Registered, IntegrationError> {
        let exe = check_exe(exe)?;
        let previous = get(&layout.marker(), Some("Executable")).filter(|p| !same_path(p, &exe));
        let command = open_command(&exe);
        let progid = layout.progid();
        set(&progid, None, "PDF Document", "couldn't create the file type")?;
        set(&format!(r"{progid}\DefaultIcon"), None, &format!("\"{exe}\",-2"), "couldn't set the icon")?;
        set(&format!(r"{progid}\shell\open\command"), None, &command, "couldn't set the open command")?;
        // The one value that lists BDF in "Open with"; other programs' values beside it are not touched.
        set(&layout.open_with(), Some(PROG_ID), "", "couldn't list BDF in Open with")?;
        let app = layout.application();
        set(&app, Some("FriendlyAppName"), "BDF", "couldn't name the program")?;
        set(&format!(r"{app}\DefaultIcon"), None, &format!("\"{exe}\",-2"), "couldn't set the program icon")?;
        set(&format!(r"{app}\SupportedTypes"), Some(".pdf"), "", "couldn't declare the supported type")?;
        set(&format!(r"{app}\shell\open\command"), None, &command, "couldn't set the program's open command")?;
        let caps = layout.capabilities_rel();
        set(&caps, Some("ApplicationName"), "BDF", "couldn't write the capabilities")?;
        set(&caps, Some("ApplicationDescription"), "Open-source PDF workbench", "couldn't write the capabilities")?;
        set(&caps, Some("ApplicationIcon"), &format!("{exe},0"), "couldn't write the capabilities")?;
        set(&format!(r"{caps}\FileAssociations"), Some(".pdf"), PROG_ID, "couldn't associate .pdf with the capabilities")?;
        set(&layout.registered_apps(), Some(REGISTERED_APP), &caps, "couldn't register BDF as a Default Programs application")?;
        let marker = layout.marker();
        set(&marker, Some("Executable"), &exe, "couldn't record the registration")?;
        set(&marker, Some("Version"), env!("CARGO_PKG_VERSION"), "couldn't record the registration")?;
        Ok(Registered { replaced: previous })
    }

    pub fn status(layout: &Layout, exe: &Path) -> Status {
        let this = exe.to_str().unwrap_or_default();
        let registered_exe = get(&layout.marker(), Some("Executable"));
        let registered = registered_exe.is_some() && exists(&layout.progid());
        let default_progid = get(&layout.user_choice, Some("ProgId"));
        Status {
            supported: true,
            registered,
            this_copy: registered && registered_exe.as_deref().is_some_and(|r| same_path(r, this)),
            stale: registered && registered_exe.as_deref().is_some_and(|r| !Path::new(r).is_file()),
            is_default: default_progid.as_deref() == Some(PROG_ID),
            default_progid,
            registered_exe,
        }
    }

    pub fn unregister(layout: &Layout, exe: &Path) -> Result<Removed, IntegrationError> {
        let this = exe.to_str().ok_or_else(|| IntegrationError::BadPath("the path is not valid text".into()))?;
        let Some(owner) = get(&layout.marker(), Some("Executable")) else {
            return Ok(Removed { nothing_to_remove: true });
        };
        // Only the copy that registered removes the registration; an orphan (its program is gone) may be
        // removed by any copy, since nothing is left to break.
        if !same_path(&owner, this) && Path::new(&owner).is_file() {
            return Err(IntegrationError::NotOwner { registered: owner });
        }
        let command = open_command(&owner);
        delete_tree(&layout.progid())?;
        delete_value(&layout.open_with(), PROG_ID)?;
        // The program entry is shared by name (`bdf.exe`): remove it only when it is this registration's.
        let app = layout.application();
        if get(&format!(r"{app}\shell\open\command"), None).is_some_and(|c| c == command) {
            delete_tree(&app)?;
        }
        delete_tree(&layout.capabilities_rel())?;
        delete_value(&layout.registered_apps(), REGISTERED_APP)?;
        delete_tree(&layout.marker())?;
        Ok(Removed { nothing_to_remove: false })
    }

    #[cfg(test)]
    pub(super) mod tests {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use super::*;

        /// A scratch registry area removed when dropped.
        pub struct Scratch(pub String);

        impl Scratch {
            pub fn new() -> Scratch {
                static N: AtomicUsize = AtomicUsize::new(0);
                Scratch(format!(r"Software\BDFTest\integration-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
            }
            pub fn layout(&self) -> Layout {
                Layout::scratch(&self.0)
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = HKEY::CURRENT_USER.RegDeleteTree(Some(&self.0));
                // Remove the shared parent when this was the last scratch area.
                if let Ok(key) = HKEY::CURRENT_USER.RegOpenKeyEx(Some(r"Software\BDFTest"), co::REG_OPTION::default(), co::KEY::READ)
                    && key.RegEnumKeyEx().map(|mut i| i.next().is_none()).unwrap_or(false)
                {
                    drop(key);
                    let _ = HKEY::CURRENT_USER.RegDeleteKey(r"Software\BDFTest");
                }
            }
        }

        pub fn read(sub: &str, name: Option<&str>) -> Option<String> {
            get(sub, name)
        }
        pub fn put(sub: &str, name: Option<&str>, value: &str) {
            set(sub, name, value, "test setup").unwrap();
        }
        pub fn key_exists(sub: &str) -> bool {
            exists(sub)
        }
        pub fn values(sub: &str) -> Vec<String> {
            let Ok(key) = HKEY::CURRENT_USER.RegOpenKeyEx(Some(sub), co::REG_OPTION::default(), co::KEY::READ) else { return Vec::new() };
            let mut names: Vec<String> = key.RegEnumValue().unwrap().map(|v| v.unwrap().0).collect();
            names.sort();
            names
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    use super::{IntegrationError, Registered, Removed, Status};

    pub struct Layout;

    impl Layout {
        pub fn real() -> Layout {
            Layout
        }
    }

    pub fn register(_: &Layout, _: &Path) -> Result<Registered, IntegrationError> {
        Err(IntegrationError::Unsupported)
    }
    pub fn unregister(_: &Layout, _: &Path) -> Result<Removed, IntegrationError> {
        Err(IntegrationError::Unsupported)
    }
    pub fn status(_: &Layout, _: &Path) -> Status {
        Status::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_quotes_the_program_and_the_file() {
        assert_eq!(open_command(r"C:\Users\Someone\BDF\bdf.exe"), r#""C:\Users\Someone\BDF\bdf.exe" "%1""#);
    }

    #[test]
    fn program_paths_that_cannot_be_quoted_or_do_not_exist_are_refused() {
        let dir = std::env::temp_dir().join(format!("bdf-integration-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("bdf.exe");
        std::fs::write(&exe, b"x").unwrap();
        assert!(check_exe(&exe).is_ok());
        assert!(check_exe(Path::new("bdf.exe")).is_err(), "relative");
        assert!(check_exe(&dir.join("missing.exe")).is_err(), "missing");
        assert!(check_exe(&dir).is_err(), "a folder");
        let quoted = dir.join("a\"b.exe");
        assert!(check_exe(&quoted).is_err(), "a quote would end the command early");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_what_windows_reports_makes_bdf_the_default() {
        let base = Status { supported: true, registered: true, this_copy: true, ..Status::default() };
        assert!(!base.bdf_is_default(), "registered but not chosen (opening Settings is not the choice)");
        assert!(Status { is_default: true, default_progid: Some(PROG_ID.into()), ..base.clone() }.bdf_is_default());
        assert!(
            !Status { this_copy: false, is_default: true, default_progid: Some(PROG_ID.into()), ..base.clone() }.bdf_is_default(),
            "another copy's registration is the default, not this one"
        );
        assert!(Status { default_progid: Some(INSTALLED_PROG_ID.into()), ..Status::default() }.bdf_is_default(), "the installed BDF");
        assert!(!Status { default_progid: Some("AcroExch.Document.DC".into()), ..base }.bdf_is_default(), "another application");
        assert!(!Status::default().bdf_is_default());
    }

    #[test]
    fn the_status_sentences_say_what_to_do() {
        let mut s = Status { supported: true, ..Status::default() };
        assert!(s.summary().contains("not registered"));
        s.registered = true;
        s.this_copy = true;
        assert!(s.summary().contains("Default apps"));
        s.is_default = true;
        assert!(s.summary().contains("your default PDF app"));
        s.this_copy = false;
        s.is_default = false;
        s.registered_exe = Some(r"C:\Old\bdf.exe".into());
        assert!(s.summary().contains("Refresh"));
        s.stale = true;
        assert!(s.summary().contains("no longer exists"));
        assert!(!Status::default().supported && Status::default().summary().contains("only available on Windows"));
    }

    #[cfg(not(windows))]
    #[test]
    fn off_windows_everything_is_unsupported() {
        assert!(matches!(register(Path::new("/x")), Err(IntegrationError::Unsupported)));
        assert!(matches!(unregister(Path::new("/x")), Err(IntegrationError::Unsupported)));
        assert!(!status(Path::new("/x")).supported);
        assert!(open_default_apps().is_err());
    }

    #[cfg(windows)]
    mod registry {
        use super::super::imp::{self, tests::*};
        use super::*;

        fn exe(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
            let dir = std::env::temp_dir().join(format!("bdf-integration-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let exe = dir.join("bdf.exe");
            std::fs::write(&exe, b"x").unwrap();
            (dir, exe)
        }

        #[test]
        fn registering_writes_the_documented_entries_and_status_reads_them_back() {
            let (dir, exe) = exe("register");
            let scratch = Scratch::new();
            let layout = scratch.layout();
            let root = &scratch.0;
            assert!(!imp::status(&layout, &exe).registered);
            let done = imp::register(&layout, &exe).unwrap();
            assert_eq!(done, Registered { replaced: None });
            let exe_text = exe.to_str().unwrap();
            let progid = format!(r"{root}\Classes\{PROG_ID}");
            assert_eq!(read(&progid, None).as_deref(), Some("PDF Document"));
            assert_eq!(read(&format!(r"{progid}\shell\open\command"), None).unwrap(), open_command(exe_text));
            assert_eq!(values(&format!(r"{root}\Classes\.pdf\OpenWithProgids")), [PROG_ID]);
            assert_eq!(read(&format!(r"{root}\Classes\Applications\bdf.exe"), Some("FriendlyAppName")).as_deref(), Some("BDF"));
            assert_eq!(read(&format!(r"{root}\BDF\PortableCapabilities\FileAssociations"), Some(".pdf")).as_deref(), Some(PROG_ID));
            assert_eq!(read(&format!(r"{root}\RegisteredApplications"), Some(REGISTERED_APP)).unwrap(), format!(r"{root}\BDF\PortableCapabilities"));
            let s = imp::status(&layout, &exe);
            assert!(s.supported && s.registered && s.this_copy && !s.stale && !s.is_default, "{s:?}");
            // Registering again is a refresh: the same entries, nothing duplicated.
            assert_eq!(imp::register(&layout, &exe).unwrap(), Registered { replaced: None });
            assert_eq!(values(&format!(r"{root}\Classes\.pdf\OpenWithProgids")), [PROG_ID]);
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn removing_takes_out_only_what_registering_added_and_leaves_other_applications_alone() {
            let (dir, exe) = exe("others");
            let scratch = Scratch::new();
            let layout = scratch.layout();
            let root = &scratch.0;
            // Another application's registrations, and the user's current default, are already there.
            put(&format!(r"{root}\Classes\.pdf"), None, "Other.PDF.Document");
            put(&format!(r"{root}\Classes\.pdf\OpenWithProgids"), Some("Other.PDF.Document"), "");
            put(&format!(r"{root}\Classes\.pdf\OpenWithProgids"), Some("Third.Document"), "");
            put(&format!(r"{root}\Classes\Other.PDF.Document\shell\open\command"), None, r#""C:\Other\other.exe" "%1""#);
            put(&format!(r"{root}\RegisteredApplications"), Some("Other"), r"Software\Other\Capabilities");
            put(&format!(r"{root}\UserChoice"), Some("ProgId"), "Other.PDF.Document");
            imp::register(&layout, &exe).unwrap();
            let s = imp::status(&layout, &exe);
            assert_eq!(s.default_progid.as_deref(), Some("Other.PDF.Document"), "BDF only reads the default");
            assert!(!s.is_default);
            imp::unregister(&layout, &exe).unwrap();
            assert_eq!(read(&format!(r"{root}\Classes\.pdf"), None).as_deref(), Some("Other.PDF.Document"), ".pdf's default value is never touched");
            assert_eq!(values(&format!(r"{root}\Classes\.pdf\OpenWithProgids")), ["Other.PDF.Document", "Third.Document"]);
            assert_eq!(read(&format!(r"{root}\Classes\Other.PDF.Document\shell\open\command"), None).unwrap(), r#""C:\Other\other.exe" "%1""#);
            assert_eq!(values(&format!(r"{root}\RegisteredApplications")), ["Other"]);
            assert_eq!(read(&format!(r"{root}\UserChoice"), Some("ProgId")).as_deref(), Some("Other.PDF.Document"), "the user's choice is untouched");
            for gone in [
                format!(r"{root}\Classes\{PROG_ID}"),
                format!(r"{root}\Classes\Applications\bdf.exe"),
                format!(r"{root}\BDF\PortableCapabilities"),
                format!(r"{root}\BDF\PortableRegistration"),
            ] {
                assert!(!key_exists(&gone), "{gone} should be gone");
            }
            assert!(!imp::status(&layout, &exe).registered);
            // Removing again is a no-op, not an error.
            assert_eq!(imp::unregister(&layout, &exe).unwrap(), Removed { nothing_to_remove: true });
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn a_copy_that_does_not_own_the_registration_cannot_remove_it_but_can_refresh_it() {
            let (dir_a, exe_a) = exe("copy-a");
            let (dir_b, exe_b) = exe("copy-b");
            let scratch = Scratch::new();
            let layout = scratch.layout();
            imp::register(&layout, &exe_a).unwrap();
            let s = imp::status(&layout, &exe_b);
            assert!(s.registered && !s.this_copy && !s.stale, "{s:?}");
            assert!(s.summary().contains("Another copy"), "{}", s.summary());
            assert!(matches!(imp::unregister(&layout, &exe_b), Err(IntegrationError::NotOwner { .. })));
            assert!(imp::status(&layout, &exe_a).this_copy, "A's registration is intact");
            // Refresh from B re-points the registration, and says what it replaced.
            let done = imp::register(&layout, &exe_b).unwrap();
            assert_eq!(done.replaced.as_deref(), exe_a.to_str());
            assert!(imp::status(&layout, &exe_b).this_copy && !imp::status(&layout, &exe_a).this_copy);
            imp::unregister(&layout, &exe_b).unwrap();
            let _ = (std::fs::remove_dir_all(dir_a), std::fs::remove_dir_all(dir_b));
        }

        #[test]
        fn a_moved_folder_shows_as_stale_and_refreshes_or_clears_from_the_new_place() {
            let (dir_old, exe_old) = exe("moved-old");
            let (dir_new, exe_new) = exe("moved-new");
            let scratch = Scratch::new();
            let layout = scratch.layout();
            imp::register(&layout, &exe_old).unwrap();
            std::fs::remove_dir_all(&dir_old).unwrap();
            let s = imp::status(&layout, &exe_new);
            assert!(s.registered && s.stale && !s.this_copy, "{s:?}");
            assert!(s.summary().contains("no longer exists"));
            // The new location can refresh it...
            imp::register(&layout, &exe_new).unwrap();
            assert!(imp::status(&layout, &exe_new).this_copy);
            imp::unregister(&layout, &exe_new).unwrap();
            // ...or clear an orphan without refreshing.
            let (dir_gone, exe_gone) = exe("orphan");
            imp::register(&layout, &exe_gone).unwrap();
            std::fs::remove_dir_all(&dir_gone).unwrap();
            assert!(imp::unregister(&layout, &exe_new).is_ok());
            assert!(!imp::status(&layout, &exe_new).registered);
            let _ = std::fs::remove_dir_all(dir_new);
        }

        #[test]
        fn nothing_is_written_for_a_bad_program_path() {
            let scratch = Scratch::new();
            let layout = scratch.layout();
            assert!(imp::register(&layout, Path::new(r"C:\no\such\bdf.exe")).is_err());
            assert!(imp::register(&layout, Path::new("bdf.exe")).is_err());
            assert!(!key_exists(&scratch.0), "no registry entry was created");
        }

        #[test]
        fn the_real_registry_is_not_touched_by_these_tests() {
            use winsafe::{HKEY, co};
            let real = |sub: &str| HKEY::CURRENT_USER.RegOpenKeyEx(Some(sub), co::REG_OPTION::default(), co::KEY::READ).is_ok();
            let (dir, exe) = exe("real");
            let scratch = Scratch::new();
            let before = (real(&format!(r"Software\Classes\{PROG_ID}")), real(r"Software\BDF\PortableRegistration"));
            imp::register(&scratch.layout(), &exe).unwrap();
            assert_eq!(before, (real(&format!(r"Software\Classes\{PROG_ID}")), real(r"Software\BDF\PortableRegistration")));
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
