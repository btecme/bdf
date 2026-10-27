//! Who the current operating-system user is, as a stable string (BUILD-008).
//!
//! The application service decides what an actor may do; for the person at the keyboard that starts from the account the
//! process runs as. Nothing a client says about itself is used: this reads the process's own access token (Windows) or
//! effective user id (Unix), which a program running under the same account cannot be told apart from, and which a program
//! running under another account cannot claim.

use std::io;

/// The current user: a Windows SID such as `S-1-5-21-...`, or `uid:<number>` on Unix.
pub fn current_user() -> io::Result<String> {
    imp::current_user()
}

#[cfg(windows)]
mod imp {
    use std::io;

    pub fn current_user() -> io::Result<String> {
        let os = |e: winsafe::co::ERROR| io::Error::from_raw_os_error(e.raw() as i32);
        let token = winsafe::HPROCESS::GetCurrentProcess().OpenProcessToken(winsafe::co::TOKEN::QUERY).map_err(os)?;
        match token.GetTokenInformation(winsafe::co::TOKEN_INFORMATION_CLASS::User).map_err(os)? {
            winsafe::TokenInfo::User(user) => {
                user.User.Sid().map(|sid| sid.to_string()).ok_or_else(|| io::Error::other("the current user has no SID"))
            }
            _ => Err(io::Error::other("unexpected token information")),
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::io;

    pub fn current_user() -> io::Result<String> {
        Ok(format!("uid:{}", rustix::process::geteuid().as_raw()))
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::io;

    pub fn current_user() -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "the current user can't be identified on this platform"))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(any(unix, windows))]
    #[test]
    fn the_current_user_is_stable_and_never_empty() {
        let first = super::current_user().expect("a user");
        assert!(!first.is_empty());
        assert_eq!(first, super::current_user().expect("a user"));
        #[cfg(windows)]
        assert!(first.starts_with("S-1-5-"), "{first}");
        #[cfg(unix)]
        assert!(first.starts_with("uid:"), "{first}");
    }
}
