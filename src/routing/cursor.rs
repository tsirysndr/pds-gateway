//! Opaque cursors for merged fan-out reads.
//!
//! Each node paginates in its own cursor space, so a merged listing cannot
//! return any single node's cursor. This packs one cursor per node into one
//! opaque string. Nodes that have run out are dropped, so when every node is
//! exhausted there is no cursor left to return and the client stops.

use std::collections::BTreeMap;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergedCursor {
    per_node: BTreeMap<String, String>,
}

impl MergedCursor {
    pub fn is_empty(&self) -> bool {
        self.per_node.is_empty()
    }

    pub fn get(&self, node: &str) -> Option<&str> {
        self.per_node.get(node).map(String::as_str)
    }

    pub fn set(&mut self, node: &str, cursor: impl Into<String>) {
        self.per_node.insert(node.to_owned(), cursor.into());
    }

    pub fn decode(raw: &str) -> Option<Self> {
        let bytes = URL_SAFE_NO_PAD.decode(raw.trim()).ok()?;
        if bytes.len() > 64 * 1024 {
            return None;
        }
        let per_node: BTreeMap<String, String> = serde_json::from_slice(&bytes).ok()?;
        Some(Self { per_node })
    }

    pub fn encode(&self) -> Option<String> {
        if self.per_node.is_empty() {
            return None;
        }
        let json = serde_json::to_vec(&self.per_node).ok()?;
        Some(URL_SAFE_NO_PAD.encode(json))
    }

    /// Splits a requested page size across the nodes that are still producing,
    /// leaving every one of them at least 1 so no node is starved.
    pub fn split_limit(limit: Option<i64>, nodes: usize) -> Option<i64> {
        let limit = limit?;
        let nodes = nodes.max(1) as i64;
        Some((limit / nodes).max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_an_opaque_string() {
        let mut cursor = MergedCursor::default();
        cursor.set("primary", "abc");
        cursor.set("radxa", "123");

        let encoded = cursor.encode().unwrap();
        // The client must not be able to read node names out of it by accident.
        assert!(!encoded.contains("radxa"));

        let decoded = MergedCursor::decode(&encoded).unwrap();
        assert_eq!(decoded, cursor);
        assert_eq!(decoded.get("radxa"), Some("123"));
        assert_eq!(decoded.get("absent"), None);
    }

    #[test]
    fn an_empty_cursor_encodes_to_nothing() {
        // Nothing left to page through means no cursor, which is how a client
        // knows to stop.
        assert!(MergedCursor::default().encode().is_none());
        assert!(MergedCursor::default().is_empty());
    }

    #[test]
    fn rejects_junk_without_panicking() {
        assert!(MergedCursor::decode("not base64!!").is_none());
        assert!(MergedCursor::decode(&URL_SAFE_NO_PAD.encode(b"[1,2,3]")).is_none());
        assert!(MergedCursor::decode("").is_none());
    }

    #[test]
    fn splits_a_page_across_the_nodes() {
        assert_eq!(MergedCursor::split_limit(Some(100), 4), Some(25));
        // Never zero, or a node would return nothing and stall the merge.
        assert_eq!(MergedCursor::split_limit(Some(2), 4), Some(1));
        assert_eq!(MergedCursor::split_limit(Some(1), 1), Some(1));
        assert_eq!(MergedCursor::split_limit(None, 4), None);
    }

    #[test]
    fn dropping_an_exhausted_node_shrinks_the_cursor() {
        let mut cursor = MergedCursor::default();
        cursor.set("a", "1");
        cursor.set("b", "2");
        let encoded = cursor.encode().unwrap();

        // Next round only `b` still has a cursor.
        let mut next = MergedCursor::default();
        if let Some(c) = MergedCursor::decode(&encoded).unwrap().get("b") {
            next.set("b", c);
        }
        assert_eq!(next.get("a"), None);
        assert_eq!(next.get("b"), Some("2"));
    }
}
