//! Release versions: `MAJOR.MINOR.PATCH`, numbers only.
//!
//! Updates are strict: a version with a pre-release or build suffix is not a release this updater
//! will install, so a malformed tag can never compare as newer by accident.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    /// `0.8.0` or `v0.8.0`; anything else (suffixes, extra parts, signs, spaces) is `None`.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
        let mut parts = text.split('.');
        let mut next = || {
            let p = parts.next()?;
            if p.is_empty() || p.len() > 9 || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            p.parse::<u64>().ok()
        };
        let version = Version(next()?, next()?, next()?);
        parts.next().is_none().then_some(version)
    }

    /// The version this program was built as.
    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version(0, 0, 0))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_versions_parse_and_order() {
        assert_eq!(Version::parse("0.8.0"), Some(Version(0, 8, 0)));
        assert_eq!(Version::parse("v1.2.3"), Some(Version(1, 2, 3)));
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
        assert_eq!(Version(0, 8, 1).to_string(), "0.8.1");
    }

    #[test]
    fn anything_that_is_not_three_plain_numbers_is_refused() {
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "1.2.3-rc1",
            "1.2.3+b",
            "a.b.c",
            "1..3",
            " 1.2.3",
            "1.2.3 ",
            "-1.2.3",
            "+1.2.3",
            "1.2.\u{663}",
            "1.2.99999999999",
            "vv1.2.3",
            "1.2.3\n",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
    }
}
