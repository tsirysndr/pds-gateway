//! Handle syntax per the atproto handle spec.

use crate::error::GatewayError;

pub const MAX_HANDLE_LEN: usize = 253;

/// TLDs the spec forbids for handles.
const DISALLOWED_TLDS: [&str; 8] = [
    "local",
    "arpa",
    "invalid",
    "localhost",
    "internal",
    "example",
    "alt",
    "onion",
];

/// A syntactically valid, lowercased handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(String);

impl Handle {
    pub fn parse(raw: &str) -> Result<Self, GatewayError> {
        let handle = raw.trim().trim_end_matches('.').to_ascii_lowercase();
        let invalid = |msg: &str| GatewayError::InvalidHandle(format!("`{raw}` {msg}"));

        if handle.is_empty() {
            return Err(invalid("is empty"));
        }
        if handle.len() > MAX_HANDLE_LEN {
            return Err(invalid(&format!(
                "is {} characters; the limit is {MAX_HANDLE_LEN}",
                handle.len()
            )));
        }
        if !handle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        {
            return Err(invalid(
                "may only contain ASCII letters, digits, hyphens and dots",
            ));
        }

        let segments: Vec<&str> = handle.split('.').collect();
        if segments.len() < 2 {
            return Err(invalid("needs at least two dot-separated segments"));
        }
        for segment in &segments {
            if segment.is_empty() {
                return Err(invalid("has an empty segment"));
            }
            if segment.len() > 63 {
                return Err(invalid("has a segment longer than 63 characters"));
            }
            if segment.starts_with('-') || segment.ends_with('-') {
                return Err(invalid("has a segment starting or ending with a hyphen"));
            }
        }

        let tld = segments[segments.len() - 1];
        if tld.starts_with(|c: char| c.is_ascii_digit()) {
            return Err(invalid("has a final segment starting with a digit"));
        }
        if DISALLOWED_TLDS.contains(&tld) {
            return Err(invalid(&format!("uses the reserved TLD `.{tld}`")));
        }

        Ok(Self(handle))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn label_under(&self, domain: &str) -> Option<&str> {
        let suffix = format!(".{}", domain.trim_start_matches('.').to_ascii_lowercase());
        self.0
            .strip_suffix(&suffix)
            .filter(|label| !label.is_empty())
    }

    /// The label under whichever of `domains` this handle sits under.
    pub fn label_under_any<I>(&self, domains: I) -> Option<&str>
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        domains
            .into_iter()
            .find_map(|domain| self.label_under(domain.as_ref()))
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Cheap check used to tell handles from DIDs and emails when classifying a
/// request's `identifier` parameter.
pub fn looks_like_handle(value: &str) -> bool {
    !value.starts_with("did:") && !value.contains('@') && value.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_handles() {
        for raw in [
            "alice.rocksky.social",
            "a-b.radxa.rocksky.social",
            "xn--90a3ac.example.com",
            "ALICE.ROCKSKY.SOCIAL",
            "alice.rocksky.social.",
        ] {
            Handle::parse(raw).unwrap_or_else(|e| panic!("{raw} should parse: {e}"));
        }
        assert_eq!(
            Handle::parse("ALICE.Rocksky.Social").unwrap().as_str(),
            "alice.rocksky.social"
        );
    }

    #[test]
    fn rejects_malformed_handles() {
        for raw in [
            "",
            "alice",
            "alice..social",
            "-alice.social",
            "alice-.social",
            "alice.rocksky.social/extra",
            "alice_b.social",
            "alice.123",
            "alice.local",
            "alice.onion",
        ] {
            assert!(
                Handle::parse(raw).is_err(),
                "`{raw}` should have been rejected"
            );
        }
    }

    #[test]
    fn rejects_over_length_handles() {
        let long = format!("{}.social", "a".repeat(MAX_HANDLE_LEN));
        assert!(Handle::parse(&long).is_err());
    }

    #[test]
    fn extracts_the_label_under_a_domain() {
        let handle = Handle::parse("alice.rocksky.social").unwrap();
        assert_eq!(handle.label_under("rocksky.social"), Some("alice"));
        assert_eq!(handle.label_under("bsky.social"), None);

        let nested = Handle::parse("a.b.rocksky.social").unwrap();
        assert_eq!(nested.label_under("rocksky.social"), Some("a.b"));

        // The bare domain has no label under itself.
        let bare = Handle::parse("rocksky.social").unwrap();
        assert_eq!(bare.label_under("rocksky.social"), None);
    }

    #[test]
    fn distinguishes_handles_from_dids_and_emails() {
        assert!(looks_like_handle("alice.rocksky.social"));
        assert!(!looks_like_handle("did:plc:abc123"));
        assert!(!looks_like_handle("alice@example.com"));
        assert!(!looks_like_handle("alice"));
    }
}
