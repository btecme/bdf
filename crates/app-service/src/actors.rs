//! Who is asking (BUILD-008).
//!
//! An [`ActorContext`] says whether a request is made by the person at the keyboard or by a client that person delegated
//! to, and which grant that client holds. **It cannot be written by a caller.** Its fields are private and there are two
//! ways to get one:
//!
//! * [`ActorContext::local_user`] takes no arguments: it reads the operating-system account the process runs as, so there
//!   is no name to claim. It is for the desktop and command-line applications' own bootstrap, and a test in this crate's
//!   suite fails if any other code in the workspace calls it.
//! * [`crate::GrantStore::authenticate`] turns a client's secret into an agent context, after checking the secret against
//!   the stored grant. Nothing in a request (a JSON field, a command-line argument, a tool argument) is ever read as an
//!   identity.
//!
//! What this does not do: tell two programs apart that run as the same operating-system account and both hold the same
//! secret, or defend against software running as the user that reads the user's private files. Local capability policy
//! is not a defence against that (`docs/bdf-build-spec.md` section 5); it is a defence against other accounts, against a
//! client doing more than it was granted, and against mistakes.

use std::fmt;
use std::io;

/// The operating-system account behind a request, as a stable string (a Windows SID, or `uid:<n>`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OsPrincipal(String);

impl OsPrincipal {
    /// The account this process runs as.
    pub fn current() -> io::Result<Self> {
        pdfcraft_platform::identity::current_user().map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// An account read back from stored data (a grant file).
    pub(crate) fn from_stored(text: &str) -> Option<Self> {
        let ok = !text.is_empty() && text.len() <= 128 && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | ':' | '_'));
        ok.then(|| Self(text.to_owned()))
    }
}

impl fmt::Display for OsPrincipal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The name a client goes by in a grant and in the audit log. Lowercase letters, digits, `-` and `_`, at most 64.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(String);

/// A client name that is not allowed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidClientId;

impl fmt::Display for InvalidClientId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a client name is 1 to 64 lowercase letters, digits, '-' or '_', starting with a letter or digit")
    }
}

impl std::error::Error for InvalidClientId {}

impl ClientId {
    pub fn parse(text: &str) -> Result<Self, InvalidClientId> {
        let mut chars = text.chars();
        let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'));
        if first_ok && rest_ok && text.len() <= 64 { Ok(Self(text.to_owned())) } else { Err(InvalidClientId) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClientId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One grant's identifier: `g-` and sixteen hex digits, chosen at random when the grant is issued.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GrantId(String);

impl GrantId {
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("g-")?;
        (digits.len() == 16 && digits.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())).then(|| Self(text.to_owned()))
    }

    pub(crate) fn from_random(bytes: [u8; 8]) -> Self {
        Self(format!("g-{}", crate::grants::hex(&bytes)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GrantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrincipalKind {
    /// The person at the keyboard: the operating-system user.
    Human,
    /// A client the person delegated to, acting under a grant.
    Agent,
}

impl PrincipalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PrincipalKind::Human => "human",
            PrincipalKind::Agent => "agent",
        }
    }
}

/// Who a request is made by. Not constructible by a caller: see the module documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActorContext {
    kind: PrincipalKind,
    os: OsPrincipal,
    client: Option<ClientId>,
    grant: Option<GrantId>,
}

impl ActorContext {
    /// The person at the keyboard, read from the operating system. Takes nothing a caller could claim.
    pub fn local_user() -> io::Result<Self> {
        Ok(Self { kind: PrincipalKind::Human, os: OsPrincipal::current()?, client: None, grant: None })
    }

    pub(crate) fn agent(os: OsPrincipal, client: ClientId, grant: GrantId) -> Self {
        Self { kind: PrincipalKind::Agent, os, client: Some(client), grant: Some(grant) }
    }

    pub fn kind(&self) -> PrincipalKind {
        self.kind
    }

    pub fn is_agent(&self) -> bool {
        self.kind == PrincipalKind::Agent
    }

    /// The operating-system account the request runs as (and, for an agent, that delegated it).
    pub fn os(&self) -> &OsPrincipal {
        &self.os
    }

    pub fn client(&self) -> Option<&ClientId> {
        self.client.as_ref()
    }

    pub fn grant(&self) -> Option<&GrantId> {
        self.grant.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_names_are_restricted() {
        for good in ["a", "claude-desktop", "agent_2", "0abc", &"a".repeat(64)] {
            assert!(ClientId::parse(good).is_ok(), "{good}");
        }
        for bad in ["", "-a", "_a", "A", "has space", "a/b", "a.b", "é", &"a".repeat(65), "a\u{0}", "a\nb"] {
            assert!(ClientId::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn grant_ids_round_trip_and_nothing_else_parses() {
        let id = GrantId::from_random([0xab, 0xcd, 0, 1, 2, 3, 4, 255]);
        assert_eq!(id.as_str(), "g-abcd0001020304ff");
        assert_eq!(GrantId::parse(id.as_str()), Some(id));
        for bad in ["", "g-", "g-abcd", "G-abcd0001020304ff", "g-ABCD0001020304FF", "g-abcd0001020304fg", "g-abcd0001020304ff0"] {
            assert_eq!(GrantId::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_local_user_is_a_human_with_no_client_or_grant() {
        let actor = ActorContext::local_user().expect("a user");
        assert_eq!(actor.kind(), PrincipalKind::Human);
        assert!(!actor.is_agent() && actor.client().is_none() && actor.grant().is_none());
        assert_eq!(actor.os(), &OsPrincipal::current().expect("a user"));
    }

    #[test]
    fn stored_accounts_are_validated() {
        assert!(OsPrincipal::from_stored("S-1-5-21-1-2-3-1001").is_some());
        assert!(OsPrincipal::from_stored("uid:1000").is_some());
        for bad in ["", "a b", "a\"b", "a/b", &"x".repeat(129)] {
            assert!(OsPrincipal::from_stored(bad).is_none(), "{bad:?}");
        }
    }
}
