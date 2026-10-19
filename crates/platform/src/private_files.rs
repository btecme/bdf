//! Files that hold a secret (the UI control channel's port and token) and must be readable only by the
//! current user (BUILD-005, finding F01).
//!
//! A secret is written only into a file that has been checked to be private, and the check happens before
//! the first byte of it exists on disk:
//!
//! 1. A new file with an unpredictable name is created beside the destination ([`crate::staging`]: exclusive,
//!    `0600` on Unix).
//! 2. It is verified private while it is still empty. Unix: owned by this user and with no group or other
//!    access. Windows: its access list is read and every entry that grants access must belong to this user, the
//!    local system, the built-in Administrators, or the creator-owner placeholders. A folder that hands its
//!    contents to ordinary users (inherited entries) therefore refuses the save, naming the account.
//! 3. Only then are the bytes written, flushed, and the file put in place of the destination.
//!
//! The destination is replaced, never opened: a link planted there is replaced and its target left alone.
//! If the check cannot be made, or fails, nothing secret was written and nothing is left behind.
//!
//! On Windows the access list is read by running `icacls.exe` from the system folder with the file's path as
//! its only argument; no safe API for reading one is available to this workspace (it forbids `unsafe`). The
//! secret never appears in a command line, a log line or an error.
//!
//! What this does not prove: that a *second Windows account* really cannot read the file. That needs two
//! accounts; the check reads the access list that governs it, which is what decides the question.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::staging::{StagingName, create_staging, staging_suffixes};

/// The operations a private write performs, so a test can watch or fail them. [`Steps::real`] is the only set
/// the product uses.
pub struct Steps<'a> {
    /// Verify that the (still empty) file at this path is private to the current user.
    pub verify: &'a dyn Fn(&Path) -> io::Result<()>,
    pub write: &'a dyn Fn(&mut File, &[u8]) -> io::Result<()>,
}

impl Steps<'static> {
    pub fn real() -> Self {
        Steps { verify: &verify_private, write: &|file, bytes| file.write_all(bytes) }
    }
}

/// Write `bytes` to `path` so that only the current user can read them, or fail without having written them.
pub fn write_private_file(path: &Path, bytes: &[u8], form: StagingName) -> io::Result<()> {
    write_private_file_with(path, bytes, form, staging_suffixes(), &Steps::real())
}

/// [`write_private_file`], trying the staging names `suffixes` give and performing the operations `steps` give.
pub fn write_private_file_with(
    path: &Path,
    bytes: &[u8],
    form: StagingName,
    suffixes: impl IntoIterator<Item = u64>,
    steps: &Steps,
) -> io::Result<()> {
    let name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a file name"))?;
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (staging, file) = create_staging(dir, name, form, suffixes)?;
    let remove = |e: io::Error| {
        let _ = std::fs::remove_file(&staging);
        e
    };
    // The handle is closed when this ends, before the file is removed or moved (Windows keeps an open file).
    let written = (|| {
        let mut file = file;
        (steps.verify)(&staging)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: not private to you, so nothing was written ({e})", dir.display())))?;
        (steps.write)(&mut file, bytes)?;
        file.sync_all()
    })();
    written.map_err(remove)?;
    std::fs::rename(&staging, path).map_err(remove)
}

/// Whether the file at `path` can be read, written or listed by anyone but the current user (and the system
/// accounts that can always override it). `Ok` means it cannot.
pub fn verify_private(path: &Path) -> io::Result<()> {
    imp::verify_private(path)
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    pub fn verify_private(path: &Path) -> io::Result<()> {
        let meta = std::fs::symlink_metadata(path)?;
        if !meta.is_file() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "not a regular file"));
        }
        let me = rustix::process::geteuid().as_raw();
        if meta.uid() != me {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the file is owned by another user"));
        }
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("its mode is {:o}: group or others have access", mode & 0o7777)));
        }
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use super::*;

    pub fn verify_private(path: &Path) -> io::Result<()> {
        let me = current_user_sid()?;
        let sddl = read_dacl_sddl(path)?;
        check_dacl(&sddl, &me).map_err(|why| io::Error::new(io::ErrorKind::PermissionDenied, why))
    }

    /// The current user's SID as a string such as `S-1-5-21-...`.
    fn current_user_sid() -> io::Result<String> {
        let os = |e: winsafe::co::ERROR| io::Error::from_raw_os_error(e.raw() as i32);
        let token = winsafe::HPROCESS::GetCurrentProcess().OpenProcessToken(winsafe::co::TOKEN::QUERY).map_err(os)?;
        match token.GetTokenInformation(winsafe::co::TOKEN_INFORMATION_CLASS::User).map_err(os)? {
            winsafe::TokenInfo::User(user) => {
                user.User.Sid().map(|sid| sid.to_string()).ok_or_else(|| io::Error::other("the current user has no SID"))
            }
            _ => Err(io::Error::other("unexpected token information")),
        }
    }

    /// The file's access list in SDDL form, from `icacls /save`, which is locale-independent (unlike its normal
    /// output). The tool is run from the system folder, with the path as its only argument.
    fn read_dacl_sddl(path: &Path) -> io::Result<String> {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let icacls = root.join("System32").join("icacls.exe");
        let out_dir = std::env::temp_dir();
        let out = out_dir.join(format!(
            "pdfcraft-acl-{}-{:016x}.txt",
            std::process::id(),
            std::hash::BuildHasher::hash_one(&std::hash::RandomState::new(), 0u8)
        ));
        let status = std::process::Command::new(&icacls)
            .arg(path)
            .arg("/save")
            .arg(&out)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let text = match status {
            Ok(s) if s.success() => std::fs::read(&out),
            Ok(s) => Err(io::Error::other(format!("icacls exited with {s}"))),
            Err(e) => Err(e),
        };
        let _ = std::fs::remove_file(&out);
        decode(&text?).ok_or_else(|| io::Error::other("couldn't read the file's access list"))
    }

    /// `icacls /save` writes UTF-16LE with a byte-order mark.
    fn decode(bytes: &[u8]) -> Option<String> {
        let body = bytes.strip_prefix(&[0xff, 0xfe]).unwrap_or(bytes);
        if !body.len().is_multiple_of(2) {
            return None;
        }
        let units: Vec<u16> = body.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        String::from_utf16(&units).ok()
    }

    pub(super) fn check_dacl(saved: &str, me: &str) -> Result<(), String> {
        super::sddl::check_dacl(saved, me)
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::*;

    pub fn verify_private(_: &Path) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "private files can't be verified on this platform"))
    }
}

/// Reading a Windows access list from its SDDL text. Pure text processing, so it is compiled and tested everywhere.
#[cfg_attr(not(windows), allow(dead_code))]
mod sddl {
    /// Principals whose access is expected on a private file besides its owner: the local system, the built-in
    /// administrators (who can take any file) and the creator-owner placeholders. SDDL writes the well-known ones
    /// as two-letter aliases.
    const TRUSTED: &[&str] = &["SY", "BA", "CO", "OW", "S-1-5-18", "S-1-5-32-544", "S-1-3-0", "S-1-3-4"];

    /// Why the access list in `saved` (the text `icacls /save` wrote: the file's name, then its SDDL) is not private
    /// to `me`, a SID string; `Ok` when every entry that grants access belongs to a trusted principal.
    pub fn check_dacl(saved: &str, me: &str) -> Result<(), String> {
        // The text is the file's name, then its security descriptor (`D:` is the access list; owner and group, if
        // present, come before it on the same line).
        let dacl = saved
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with(['D', 'O', 'G']) && l[1..].starts_with(':'))
            .find_map(|l| l.find("D:").map(|i| &l[i + 2..]))
            .ok_or("no access list was found, so privacy can't be shown")?;
        // A DACL with no entries at all and no protection flag still lists entries; "NO_ACCESS_CONTROL" or an
        // absent list means everyone has access.
        if dacl.contains("NO_ACCESS_CONTROL") || dacl.is_empty() {
            return Err("the file has no access list, so everyone can read it".into());
        }
        let mut aces = 0;
        let mut rest = dacl;
        while let Some(open) = rest.find('(') {
            let after = &rest[open + 1..];
            let Some(close) = after.find(')') else { return Err("the access list is malformed".into()) };
            let ace = &after[..close];
            rest = &after[close + 1..];
            aces += 1;
            let fields: Vec<&str> = ace.split(';').collect();
            if fields.len() < 6 {
                return Err(format!("an access entry is malformed: ({ace})"));
            }
            let (kind, rights, who) = (fields[0], fields[2], fields[5]);
            // Allow entries (plain, object, callback). Deny and audit entries cannot widen access.
            if !matches!(kind, "A" | "OA" | "XA" | "ZA") || rights.is_empty() {
                continue;
            }
            if who != me && !TRUSTED.contains(&who) {
                return Err(format!("it grants access to {}", describe(who)));
            }
        }
        if aces == 0 {
            // An empty, non-null list grants nothing: private.
            return Ok(());
        }
        Ok(())
    }

    /// A name for a principal in an error: the well-known aliases people recognise, otherwise the SID.
    fn describe(who: &str) -> String {
        match who {
            "WD" => "Everyone".into(),
            "BU" => "all Users".into(),
            "AU" => "all authenticated users".into(),
            "IU" => "interactive users".into(),
            "NU" => "network users".into(),
            "LG" => "guests".into(),
            other => format!("another account ({other})"),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::check_dacl;

        const ME: &str = "S-1-5-21-111-222-333-1001";

        fn saved(dacl: &str) -> String {
            format!("secret.json\r\nD:{dacl}\r\n")
        }

        #[test]
        fn only_the_owner_the_system_and_administrators_are_private() {
            assert!(check_dacl(&saved(&format!("PAI(A;;FA;;;{ME})")), ME).is_ok());
            assert!(check_dacl(&saved(&format!("PAI(A;;FA;;;{ME})(A;;FA;;;SY)(A;;FA;;;BA)")), ME).is_ok());
            assert!(check_dacl(&saved("AI(A;;FA;;;S-1-5-18)(A;;FA;;;S-1-5-32-544)(A;ID;FA;;;CO)"), ME).is_ok());
            assert!(check_dacl(&saved("P"), ME).is_ok(), "an empty protected list grants nothing");
        }

        /// The inherited entries a folder hands to ordinary users are exactly what must be refused.
        #[test]
        fn a_grant_to_anyone_else_is_refused_and_named() {
            for (dacl, who) in [
                ("AI(A;ID;0x1200a9;;;BU)(A;;FA;;;S-1-5-21-111-222-333-1001)", "all Users"),
                ("AI(A;ID;FR;;;WD)", "Everyone"),
                ("AI(A;ID;FA;;;AU)", "authenticated users"),
                ("AI(A;ID;FR;;;S-1-5-21-111-222-333-1002)", "S-1-5-21-111-222-333-1002"),
                ("AI(A;;FA;;;S-1-5-21-111-222-333-1001)(A;ID;FR;;;IU)", "interactive users"),
                ("AI(OA;;CCDC;bf967aba-0de6-11d0-a285-00aa003049e2;;S-1-5-21-111-222-333-1002)", "S-1-5-21-111-222-333-1002"),
                ("AI(XA;;FA;;;S-1-5-21-111-222-333-1002;(@User.x==1))", "S-1-5-21-111-222-333-1002"),
            ] {
                let why = check_dacl(&saved(dacl), ME).unwrap_err();
                assert!(why.contains(who), "{dacl}: {why}");
            }
        }

        #[test]
        fn deny_and_audit_entries_cannot_widen_access() {
            assert!(check_dacl(&saved(&format!("PAI(D;;FR;;;WD)(A;;FA;;;{ME})")), ME).is_ok());
            assert!(check_dacl(&saved(&format!("PAI(A;;FA;;;{ME})(AU;SA;FA;;;WD)")), ME).is_ok());
        }

        /// What cannot be read cannot be shown private.
        #[test]
        fn a_missing_null_or_malformed_list_is_refused() {
            assert!(check_dacl("secret.json\r\n", ME).is_err(), "no list");
            assert!(check_dacl(&saved(""), ME).is_err(), "empty text");
            assert!(check_dacl(&saved("NO_ACCESS_CONTROL"), ME).is_err(), "null DACL means everyone");
            assert!(check_dacl(&saved(&format!("PAI(A;;FA;;;{ME}")), ME).is_err(), "unclosed entry");
            assert!(check_dacl(&saved("PAI(A;;FA)"), ME).is_err(), "short entry");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn test_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pdfcraft-private-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    fn save_with(path: &Path, bytes: &[u8], steps: &Steps) -> io::Result<()> {
        write_private_file_with(path, bytes, StagingName::TagThenSuffix, [1, 2, 3], steps)
    }

    /// The verification comes first and a failed one means no byte of the secret was written and nothing is left.
    #[test]
    fn a_file_that_cannot_be_shown_private_gets_no_secret_and_leaves_nothing() {
        let dir = test_dir("refuse");
        let target = dir.join("ctl.json");
        std::fs::write(&target, b"OLD").unwrap();
        let written = Cell::new(false);
        let steps = Steps {
            verify: &|_| Err(io::Error::new(io::ErrorKind::PermissionDenied, "it grants access to all Users")),
            write: &|_, _| {
                written.set(true);
                Ok(())
            },
        };
        let e = save_with(&target, b"SECRET TOKEN", &steps).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
        assert!(e.to_string().contains("nothing was written") && e.to_string().contains("all Users"), "{e}");
        assert!(!e.to_string().contains("SECRET TOKEN"), "the secret is never in an error");
        assert!(!written.get(), "no byte of the secret was written");
        assert_eq!(std::fs::read(&target).unwrap(), b"OLD");
        assert_eq!(entries(&dir), ["ctl.json"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The file is checked while it is still empty, then written, and replaces the destination.
    #[test]
    fn the_check_sees_an_empty_file_and_the_secret_replaces_the_destination() {
        let dir = test_dir("order");
        let target = dir.join("ctl.json");
        let seen = Cell::new(None);
        let steps = Steps {
            verify: &|p| {
                seen.set(Some(std::fs::metadata(p)?.len()));
                Ok(())
            },
            ..Steps::real()
        };
        save_with(&target, b"SECRET TOKEN", &steps).unwrap();
        assert_eq!(seen.get(), Some(0), "verified before any byte was written");
        assert_eq!(std::fs::read(&target).unwrap(), b"SECRET TOKEN");
        assert_eq!(entries(&dir), ["ctl.json"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_leaves_no_temporary_file_and_the_old_destination() {
        let dir = test_dir("write-fails");
        let target = dir.join("ctl.json");
        std::fs::write(&target, b"OLD").unwrap();
        let steps = Steps { verify: &|_| Ok(()), write: &|_, _| Err(io::Error::other("disk full")) };
        assert!(save_with(&target, b"SECRET", &steps).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"OLD");
        assert_eq!(entries(&dir), ["ctl.json"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        #[test]
        fn a_new_file_is_owner_only_and_a_wider_one_is_refused() {
            let dir = test_dir("unix-modes");
            let target = dir.join("ctl.json");
            write_private_file(&target, b"SECRET", StagingName::TagThenSuffix).unwrap();
            assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o600);
            assert!(verify_private(&target).is_ok());
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
            let e = verify_private(&target).unwrap_err();
            assert!(e.to_string().contains("group or others"), "{e}");
            let link = dir.join("link.json");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(verify_private(&link).is_err(), "a link is not a regular file");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[cfg(windows)]
    mod windows {
        use super::*;
        use std::process::Command;

        fn icacls(path: &Path, args: &[&str]) -> bool {
            Command::new("icacls").arg(path).args(args).output().map(|o| o.status.success()).unwrap_or(false)
        }

        fn user() -> String {
            std::env::var("USERNAME").unwrap_or_default()
        }

        /// A folder whose entries are inherited by what is created in it: private (the current user only) or one
        /// that also hands ordinary users read access, as a shared or permissively configured folder does.
        fn folder(test: &str, users_can_read: bool) -> PathBuf {
            let dir = test_dir(test);
            assert!(icacls(&dir, &["/inheritance:r", "/grant:r", &format!("{}:(OI)(CI)F", user())]), "icacls must be available to run this test");
            if users_can_read {
                assert!(icacls(&dir, &["/grant", "*S-1-5-32-545:(OI)(CI)R"]));
            }
            dir
        }

        /// F01: a folder that grants ordinary users read access hands the token to them through inheritance. The
        /// write must be refused, naming the group, with no token on disk and nothing left behind.
        #[test]
        fn a_folder_that_grants_users_read_access_is_refused_before_the_secret_is_written() {
            let dir = folder("shared", true);
            let target = dir.join("ctl.json");
            let e = write_private_file(&target, b"SECRET TOKEN", StagingName::TagThenSuffix).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");
            assert!(e.to_string().contains("all Users") && e.to_string().contains("nothing was written"), "{e}");
            assert!(!e.to_string().contains("SECRET TOKEN"));
            assert!(entries(&dir).is_empty(), "no file, not even an empty one: {:?}", entries(&dir));
            let _ = icacls(&dir, &["/reset"]);
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_folder_only_the_current_user_can_use_is_accepted_and_the_file_is_private() {
            let dir = folder("private", false);
            let target = dir.join("ctl.json");
            write_private_file(&target, b"SECRET TOKEN", StagingName::TagThenSuffix).unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"SECRET TOKEN");
            assert!(verify_private(&target).is_ok());
            assert_eq!(entries(&dir), ["ctl.json"]);
            // And the check is not a rubber stamp: the same file in a shared folder is refused.
            let shared = folder("private-then-shared", true);
            let copy = shared.join("copy.json");
            std::fs::write(&copy, b"x").unwrap();
            assert!(verify_private(&copy).is_err());
            let _ = icacls(&shared, &["/reset"]);
            let _ = std::fs::remove_dir_all(&shared);
            let _ = std::fs::remove_dir_all(&dir);
        }

        /// A planted link or hard link at the destination is replaced, not written through.
        #[test]
        fn a_hard_link_planted_at_the_destination_is_replaced_not_written_through() {
            let dir = folder("hardlink", false);
            std::fs::write(dir.join("victim.txt"), "keep me").unwrap();
            std::fs::hard_link(dir.join("victim.txt"), dir.join("ctl.json")).unwrap();
            write_private_file(&dir.join("ctl.json"), b"SECRET TOKEN", StagingName::TagThenSuffix).unwrap();
            assert_eq!(std::fs::read_to_string(dir.join("victim.txt")).unwrap(), "keep me");
            assert_eq!(std::fs::read(dir.join("ctl.json")).unwrap(), b"SECRET TOKEN");
            assert_eq!(entries(&dir), ["ctl.json", "victim.txt"]);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
