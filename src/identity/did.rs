//! DID syntax per the atproto DID spec.

use crate::error::GatewayError;

pub const MAX_DID_LEN: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Did(String);

impl Did {
    pub fn parse(raw: &str) -> Result<Self, GatewayError> {
        let did = raw.trim();
        let invalid = |msg: &str| GatewayError::InvalidRequest(format!("DID `{raw}` {msg}"));

        if did.len() > MAX_DID_LEN {
            return Err(invalid("is longer than 2048 characters"));
        }
        let rest = did
            .strip_prefix("did:")
            .ok_or_else(|| invalid("does not start with `did:`"))?;
        let (method, identifier) = rest
            .split_once(':')
            .ok_or_else(|| invalid("has no method-specific identifier"))?;

        if method.is_empty() || !method.bytes().all(|b| b.is_ascii_lowercase()) {
            return Err(invalid("has a method that is not lowercase ASCII letters"));
        }
        if identifier.is_empty() {
            return Err(invalid("has an empty identifier"));
        }
        if identifier.ends_with(':') {
            return Err(invalid("ends with a colon"));
        }
        if !identifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'%'))
        {
            return Err(invalid("has an identifier with disallowed characters"));
        }

        Ok(Self(did.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn method(&self) -> &str {
        self.0
            .split(':')
            .nth(1)
            .expect("a parsed DID always has a method")
    }

    pub fn identifier(&self) -> &str {
        let prefix_len = "did:".len() + self.method().len() + 1;
        &self.0[prefix_len..]
    }

    /// For `did:web`, the hostname the document is served from.
    pub fn web_host(&self) -> Option<String> {
        if self.method() != "web" {
            return None;
        }
        let first = self.identifier().split(':').next()?;
        // did:web percent-encodes the port separator.
        Some(first.replace("%3A", ":").replace("%3a", ":"))
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for Did {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plc_and_web_dids() {
        assert_eq!(
            Did::parse("did:plc:7iza6de2dwap2sbkpav7c6c6")
                .unwrap()
                .method(),
            "plc"
        );
        let web = Did::parse("did:web:radxa.rocksky.social").unwrap();
        assert_eq!(web.method(), "web");
        assert_eq!(web.identifier(), "radxa.rocksky.social");
        assert_eq!(web.web_host().as_deref(), Some("radxa.rocksky.social"));
    }

    #[test]
    fn decodes_a_port_in_a_did_web() {
        let did = Did::parse("did:web:localhost%3A2583").unwrap();
        assert_eq!(did.web_host().as_deref(), Some("localhost:2583"));
    }

    #[test]
    fn ignores_a_path_when_reading_the_did_web_host() {
        let did = Did::parse("did:web:example.com:user:alice").unwrap();
        assert_eq!(did.web_host().as_deref(), Some("example.com"));
    }

    #[test]
    fn rejects_malformed_dids() {
        for raw in [
            "",
            "plc:abc",
            "did:plc",
            "did:plc:",
            "did:PLC:abc",
            "did:plc:abc:",
            "did:plc:abc def",
            "did:1plc:abc",
        ] {
            assert!(Did::parse(raw).is_err(), "`{raw}` should be rejected");
        }
    }

    #[test]
    fn has_no_web_host_for_other_methods() {
        assert!(Did::parse("did:plc:abc").unwrap().web_host().is_none());
    }
}
