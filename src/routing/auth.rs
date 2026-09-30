//! Reading routing hints out of a bearer token.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenClaims {
    /// The account the token speaks for.
    pub sub: Option<String>,
    /// The service the token was issued for: a PDS's own DID for a session
    /// token, or the target service for a service-auth token.
    pub aud: Option<String>,
    /// `com.atproto.access`, `com.atproto.refresh`, an app-password scope, or
    /// an OAuth scope string.
    pub scope: Option<String>,
    /// Lexicon method a service-auth token is bound to.
    pub lxm: Option<String>,
}

impl TokenClaims {
    /// Reads the claim set from a JWT payload without verifying anything.
    pub fn parse_unverified(token: &str) -> Option<Self> {
        let mut parts = token.split('.');
        let _header = parts.next()?;
        let payload = parts.next()?;
        // A JWT has exactly three segments; anything else is not one.
        if parts.next().is_none() || parts.next().is_some() {
            return None;
        }

        // Refuse absurd payloads rather than allocating for them.
        if payload.len() > 8192 {
            return None;
        }

        let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
        let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;

        let string = |key: &str| {
            claims
                .get(key)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };

        Some(Self {
            sub: string("sub"),
            aud: string("aud"),
            scope: string("scope"),
            lxm: string("lxm"),
        })
    }

    /// Extracts the token from an `Authorization: Bearer` header value.
    pub fn from_authorization(value: &str) -> Option<Self> {
        let token = value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "))
            .or_else(|| value.strip_prefix("DPoP "))
            .or_else(|| value.strip_prefix("dpop "))?;
        Self::parse_unverified(token.trim())
    }

    pub fn is_refresh(&self) -> bool {
        self.scope.as_deref() == Some("com.atproto.refresh")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(payload: serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"at+jwt"}"#);
        let body = URL_SAFE_NO_PAD.encode(payload.to_string());
        format!("{header}.{body}.signature-we-never-check")
    }

    #[test]
    fn reads_the_routing_claims_from_a_session_token() {
        let token = jwt(serde_json::json!({
            "sub": "did:plc:alice",
            "aud": "did:web:radxa.rocksky.social",
            "scope": "com.atproto.access",
            "iat": 1_700_000_000,
        }));

        let claims = TokenClaims::parse_unverified(&token).unwrap();
        assert_eq!(claims.sub.as_deref(), Some("did:plc:alice"));
        assert_eq!(claims.aud.as_deref(), Some("did:web:radxa.rocksky.social"));
        assert!(!claims.is_refresh());
    }

    #[test]
    fn recognises_a_refresh_token() {
        let token = jwt(serde_json::json!({
            "sub": "did:plc:alice",
            "scope": "com.atproto.refresh",
        }));
        assert!(TokenClaims::parse_unverified(&token).unwrap().is_refresh());
    }

    #[test]
    fn accepts_both_bearer_and_dpop_schemes() {
        let token = jwt(serde_json::json!({"sub": "did:plc:alice"}));
        for header in [
            format!("Bearer {token}"),
            format!("bearer {token}"),
            format!("DPoP {token}"),
        ] {
            let claims = TokenClaims::from_authorization(&header).unwrap();
            assert_eq!(claims.sub.as_deref(), Some("did:plc:alice"));
        }
    }

    #[test]
    fn rejects_things_that_are_not_tokens() {
        assert!(TokenClaims::parse_unverified("").is_none());
        assert!(TokenClaims::parse_unverified("not.a").is_none());
        assert!(TokenClaims::parse_unverified("a.b.c.d").is_none());
        assert!(TokenClaims::parse_unverified("a.!!!notbase64!!!.c").is_none());
        assert!(TokenClaims::from_authorization("Basic abc123").is_none());
        // A valid-looking token whose payload is not JSON.
        let bad = format!("h.{}.s", URL_SAFE_NO_PAD.encode(b"plain text"));
        assert!(TokenClaims::parse_unverified(&bad).is_none());
    }

    #[test]
    fn treats_missing_and_empty_claims_alike() {
        let token = jwt(serde_json::json!({"sub": "", "aud": "did:web:x.example"}));
        let claims = TokenClaims::parse_unverified(&token).unwrap();
        assert!(claims.sub.is_none());
        assert_eq!(claims.aud.as_deref(), Some("did:web:x.example"));
    }

    #[test]
    fn refuses_an_oversized_payload() {
        let token = format!("h.{}.s", "A".repeat(9000));
        assert!(TokenClaims::parse_unverified(&token).is_none());
    }

    #[test]
    fn keeps_the_service_auth_method_binding() {
        let token = jwt(serde_json::json!({
            "sub": "did:plc:alice",
            "aud": "did:web:api.bsky.app",
            "lxm": "app.bsky.feed.getTimeline",
        }));
        let claims = TokenClaims::parse_unverified(&token).unwrap();
        assert_eq!(claims.lxm.as_deref(), Some("app.bsky.feed.getTimeline"));
    }
}
