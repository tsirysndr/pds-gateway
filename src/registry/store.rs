//! Durable state: which node hosts which account, and the handle reservations

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};

use crate::config::StoreConfig;
use crate::error::{GatewayError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub did: String,
    pub handle: String,
    pub node: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct Reservation {
    pub handle: String,
    pub node: String,
    pub owner: String,
    pub expires_at: i64,
}

pub struct Store {
    pool: SqlitePool,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

impl Store {
    pub async fn open(config: &StoreConfig) -> anyhow::Result<Self> {
        if let Some(parent) = config.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }

        let options = SqliteConnectOptions::new()
            .filename(&config.path)
            .create_if_missing(true)
            // WAL plus NORMAL keeps concurrent readers fast on the SD cards
            // these nodes tend to run from.
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(10))
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(config.max_connections.max(1))
            .connect_with(options)
            .await?;

        sqlx::migrate!("./migrations").run(&pool).await?;
        tracing::info!(path = %config.path.display(), "opened gateway store");

        Ok(Self { pool })
    }

    #[cfg(test)]
    pub async fn open_in_memory() -> anyhow::Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn account_by_did(&self, did: &str) -> Result<Option<Account>> {
        let row = sqlx::query(
            "SELECT did, handle, node, status, created_at, updated_at \
             FROM accounts WHERE did = ?",
        )
        .bind(did)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(to_account))
    }

    pub async fn account_by_handle(&self, handle: &str) -> Result<Option<Account>> {
        let row = sqlx::query(
            "SELECT did, handle, node, status, created_at, updated_at \
             FROM accounts WHERE handle = ? COLLATE NOCASE",
        )
        .bind(handle)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(to_account))
    }

    pub async fn upsert_account(&self, did: &str, handle: &str, node: &str) -> Result<()> {
        let ts = now();
        sqlx::query(
            "INSERT INTO accounts (did, handle, node, status, created_at, updated_at) \
             VALUES (?, ?, ?, 'active', ?, ?) \
             ON CONFLICT(did) DO UPDATE SET \
               handle = excluded.handle, node = excluded.node, updated_at = excluded.updated_at",
        )
        .bind(did)
        .bind(handle)
        .bind(node)
        .bind(ts)
        .bind(ts)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_handle(&self, did: &str, handle: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM accounts WHERE handle = ? COLLATE NOCASE AND did <> ?")
            .bind(handle)
            .bind(did)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE accounts SET handle = ?, updated_at = ? WHERE did = ?")
            .bind(handle)
            .bind(now())
            .bind(did)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn delete_account(&self, did: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM accounts WHERE did = ?")
            .bind(did)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn reserve_handle(
        &self,
        handle: &str,
        node: &str,
        owner: &str,
        ttl: Duration,
    ) -> Result<Reservation> {
        let ts = now();
        let expires_at = ts + ttl.as_secs().max(1) as i64;
        let mut tx = self.pool.begin().await?;

        // Expired reservations are cleared inline so a crashed creation does
        // not hold a handle until the sweeper next runs.
        sqlx::query("DELETE FROM handle_reservations WHERE expires_at <= ?")
            .bind(ts)
            .execute(&mut *tx)
            .await?;

        let taken: Option<String> =
            sqlx::query("SELECT did FROM accounts WHERE handle = ? COLLATE NOCASE")
                .bind(handle)
                .fetch_optional(&mut *tx)
                .await?
                .map(|r| r.get::<String, _>("did"));
        if taken.is_some() {
            return Err(GatewayError::HandleTaken(handle.to_owned()));
        }

        let existing: Option<(String, String)> = sqlx::query(
            "SELECT owner, node FROM handle_reservations WHERE handle = ? COLLATE NOCASE",
        )
        .bind(handle)
        .fetch_optional(&mut *tx)
        .await?
        .map(|r| (r.get("owner"), r.get("node")));

        let node = match existing {
            Some((existing_owner, existing_node)) if existing_owner == owner => existing_node,
            Some(_) => return Err(GatewayError::HandleTaken(handle.to_owned())),
            None => node.to_owned(),
        };

        sqlx::query(
            "INSERT INTO handle_reservations (handle, node, owner, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(handle) DO UPDATE SET expires_at = excluded.expires_at",
        )
        .bind(handle)
        .bind(&node)
        .bind(owner)
        .bind(ts)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(Reservation {
            handle: handle.to_owned(),
            node,
            owner: owner.to_owned(),
            expires_at,
        })
    }

    /// Releases a reservation, but only if `owner` still holds it.
    pub async fn release_handle(&self, handle: &str, owner: &str) -> Result<()> {
        sqlx::query(
            "DELETE FROM handle_reservations WHERE handle = ? COLLATE NOCASE AND owner = ?",
        )
        .bind(handle)
        .bind(owner)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Turns a reservation into an account row in one transaction, so a handle
    /// is never simultaneously reserved and assigned.
    pub async fn commit_reservation(
        &self,
        handle: &str,
        owner: &str,
        did: &str,
        node: &str,
    ) -> Result<()> {
        let ts = now();
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "DELETE FROM handle_reservations WHERE handle = ? COLLATE NOCASE AND owner = ?",
        )
        .bind(handle)
        .bind(owner)
        .execute(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM accounts WHERE handle = ? COLLATE NOCASE AND did <> ?")
            .bind(handle)
            .bind(did)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "INSERT INTO accounts (did, handle, node, status, created_at, updated_at) \
             VALUES (?, ?, ?, 'active', ?, ?) \
             ON CONFLICT(did) DO UPDATE SET \
               handle = excluded.handle, node = excluded.node, updated_at = excluded.updated_at",
        )
        .bind(did)
        .bind(handle)
        .bind(node)
        .bind(ts)
        .bind(ts)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn sweep_expired(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM handle_reservations WHERE expires_at <= ?")
            .bind(now())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Account count per node, including reservations in flight so placement
    /// does not pile concurrent signups onto the same node.
    pub async fn counts_by_node(&self) -> Result<Vec<(String, u64)>> {
        let rows = sqlx::query(
            "SELECT node, COUNT(*) AS n FROM ( \
                 SELECT node FROM accounts \
                 UNION ALL \
                 SELECT node FROM handle_reservations WHERE expires_at > ? \
             ) GROUP BY node",
        )
        .bind(now())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| (r.get::<String, _>("node"), r.get::<i64, _>("n") as u64))
            .collect())
    }

    pub async fn hint(&self, identifier: &str) -> Result<Option<String>> {
        let row =
            sqlx::query("SELECT node FROM identifier_hints WHERE identifier = ? COLLATE NOCASE")
                .bind(identifier)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|r| r.get::<String, _>("node")))
    }

    pub async fn put_hint(&self, identifier: &str, node: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO identifier_hints (identifier, node, updated_at) VALUES (?, ?, ?) \
             ON CONFLICT(identifier) DO UPDATE SET \
               node = excluded.node, updated_at = excluded.updated_at",
        )
        .bind(identifier)
        .bind(node)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn forget_hints_for_node(&self, node: &str) -> Result<u64> {
        let result = sqlx::query("DELETE FROM identifier_hints WHERE node = ?")
            .bind(node)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Keyset pagination over accounts, ordered by DID so the cursor is stable.
    pub async fn list_accounts(
        &self,
        node: Option<&str>,
        after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Account>> {
        let rows = sqlx::query(
            "SELECT did, handle, node, status, created_at, updated_at FROM accounts \
             WHERE (?1 IS NULL OR node = ?1) AND (?2 IS NULL OR did > ?2) \
             ORDER BY did LIMIT ?3",
        )
        .bind(node)
        .bind(after)
        .bind(limit.clamp(1, 1000))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(to_account).collect())
    }

    pub async fn list_reservations(&self) -> Result<Vec<Reservation>> {
        let rows = sqlx::query(
            "SELECT handle, node, owner, expires_at FROM handle_reservations \
             WHERE expires_at > ? ORDER BY expires_at",
        )
        .bind(now())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| Reservation {
                handle: r.get("handle"),
                node: r.get("node"),
                owner: r.get("owner"),
                expires_at: r.get("expires_at"),
            })
            .collect())
    }

    pub async fn total_accounts(&self) -> Result<u64> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM accounts")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get::<i64, _>("n") as u64)
    }

    /// Reassigns every account on `from` to `to`, for draining a node.
    pub async fn move_node(&self, from: &str, to: &str) -> Result<u64> {
        let result = sqlx::query("UPDATE accounts SET node = ?, updated_at = ? WHERE node = ?")
            .bind(to)
            .bind(now())
            .bind(from)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}

fn to_account(row: sqlx::sqlite::SqliteRow) -> Account {
    Account {
        did: row.get("did"),
        handle: row.get("handle"),
        node: row.get("node"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(600);

    async fn store() -> Store {
        Store::open_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn records_and_finds_accounts_either_way() {
        let store = store().await;
        store
            .upsert_account("did:plc:alice", "alice.rocksky.social", "radxa")
            .await
            .unwrap();

        let by_did = store
            .account_by_did("did:plc:alice")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_did.node, "radxa");

        // Handle lookup is case-insensitive.
        let by_handle = store
            .account_by_handle("ALICE.ROCKSKY.SOCIAL")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_handle.did, "did:plc:alice");

        assert!(
            store
                .account_by_did("did:plc:nobody")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_reservation_blocks_a_different_owner() {
        let store = store().await;
        store
            .reserve_handle("alice.rocksky.social", "radxa", "req-1", TTL)
            .await
            .unwrap();

        let err = store
            .reserve_handle("alice.rocksky.social", "primary", "req-2", TTL)
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::HandleTaken(_)));

        // Case differences must not slip past the reservation.
        let err = store
            .reserve_handle("ALICE.rocksky.social", "primary", "req-3", TTL)
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::HandleTaken(_)));
    }

    #[tokio::test]
    async fn the_same_owner_may_retry_and_keeps_its_node() {
        let store = store().await;
        let first = store
            .reserve_handle("alice.rocksky.social", "radxa", "req-1", TTL)
            .await
            .unwrap();
        let retry = store
            .reserve_handle("alice.rocksky.social", "primary", "req-1", TTL)
            .await
            .unwrap();

        // A retry must not silently move the account to another node.
        assert_eq!(first.node, "radxa");
        assert_eq!(retry.node, "radxa");
    }

    #[tokio::test]
    async fn an_existing_account_blocks_reservation() {
        let store = store().await;
        store
            .upsert_account("did:plc:alice", "alice.rocksky.social", "radxa")
            .await
            .unwrap();

        let err = store
            .reserve_handle("alice.rocksky.social", "primary", "req-1", TTL)
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::HandleTaken(_)));
    }

    #[tokio::test]
    async fn an_expired_reservation_stops_blocking() {
        let store = store().await;
        store
            .reserve_handle(
                "alice.rocksky.social",
                "radxa",
                "req-1",
                Duration::from_secs(1),
            )
            .await
            .unwrap();

        // Age the reservation rather than sleeping.
        sqlx::query("UPDATE handle_reservations SET expires_at = ?")
            .bind(now() - 10)
            .execute(&store.pool)
            .await
            .unwrap();

        store
            .reserve_handle("alice.rocksky.social", "primary", "req-2", TTL)
            .await
            .unwrap();
        assert_eq!(store.sweep_expired().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn releasing_requires_the_right_owner() {
        let store = store().await;
        store
            .reserve_handle("alice.rocksky.social", "radxa", "req-1", TTL)
            .await
            .unwrap();

        store
            .release_handle("alice.rocksky.social", "someone-else")
            .await
            .unwrap();
        assert!(
            store
                .reserve_handle("alice.rocksky.social", "primary", "req-2", TTL)
                .await
                .is_err(),
            "a foreign release must not free the handle"
        );

        store
            .release_handle("alice.rocksky.social", "req-1")
            .await
            .unwrap();
        store
            .reserve_handle("alice.rocksky.social", "primary", "req-2", TTL)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn committing_a_reservation_creates_the_account() {
        let store = store().await;
        store
            .reserve_handle("alice.rocksky.social", "radxa", "req-1", TTL)
            .await
            .unwrap();
        store
            .commit_reservation("alice.rocksky.social", "req-1", "did:plc:alice", "radxa")
            .await
            .unwrap();

        assert!(store.list_reservations().await.unwrap().is_empty());
        let account = store
            .account_by_handle("alice.rocksky.social")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(account.did, "did:plc:alice");
        assert_eq!(account.node, "radxa");
    }

    #[tokio::test]
    async fn counts_include_accounts_and_live_reservations() {
        let store = store().await;
        store
            .upsert_account("did:plc:a", "a.rocksky.social", "radxa")
            .await
            .unwrap();
        store
            .upsert_account("did:plc:b", "b.rocksky.social", "radxa")
            .await
            .unwrap();
        store
            .reserve_handle("c.rocksky.social", "primary", "req-1", TTL)
            .await
            .unwrap();

        let counts: std::collections::HashMap<_, _> =
            store.counts_by_node().await.unwrap().into_iter().collect();
        assert_eq!(counts.get("radxa"), Some(&2));
        // An in-flight signup counts, so concurrent placements spread out.
        assert_eq!(counts.get("primary"), Some(&1));
        assert_eq!(store.total_accounts().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn renaming_frees_the_old_handle() {
        let store = store().await;
        store
            .upsert_account("did:plc:alice", "alice.rocksky.social", "radxa")
            .await
            .unwrap();
        store
            .set_handle("did:plc:alice", "alicia.rocksky.social")
            .await
            .unwrap();

        assert!(
            store
                .account_by_handle("alice.rocksky.social")
                .await
                .unwrap()
                .is_none()
        );
        let moved = store
            .account_by_handle("alicia.rocksky.social")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(moved.did, "did:plc:alice");

        // The freed handle can be taken by someone else.
        store
            .reserve_handle("alice.rocksky.social", "primary", "req-1", TTL)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_rename_onto_a_taken_handle_evicts_the_stale_row() {
        let store = store().await;
        store
            .upsert_account("did:plc:old", "shared.rocksky.social", "radxa")
            .await
            .unwrap();
        store
            .upsert_account("did:plc:new", "new.rocksky.social", "primary")
            .await
            .unwrap();

        store
            .set_handle("did:plc:new", "shared.rocksky.social")
            .await
            .unwrap();

        let holder = store
            .account_by_handle("shared.rocksky.social")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(holder.did, "did:plc:new");
        assert!(store.account_by_did("did:plc:old").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remembers_identifier_hints() {
        let store = store().await;
        assert!(store.hint("alice@example.com").await.unwrap().is_none());

        store.put_hint("alice@example.com", "radxa").await.unwrap();
        assert_eq!(
            store.hint("ALICE@EXAMPLE.COM").await.unwrap().as_deref(),
            Some("radxa")
        );

        store
            .put_hint("alice@example.com", "primary")
            .await
            .unwrap();
        assert_eq!(
            store.hint("alice@example.com").await.unwrap().as_deref(),
            Some("primary")
        );

        assert_eq!(store.forget_hints_for_node("primary").await.unwrap(), 1);
        assert!(store.hint("alice@example.com").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn lists_accounts_with_a_stable_cursor() {
        let store = store().await;
        for name in ["a", "b", "c", "d"] {
            store
                .upsert_account(
                    &format!("did:plc:{name}"),
                    &format!("{name}.rocksky.social"),
                    if name < "c" { "radxa" } else { "primary" },
                )
                .await
                .unwrap();
        }

        let first = store.list_accounts(None, None, 2).await.unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].did, "did:plc:a");

        let next = store
            .list_accounts(None, Some(&first[1].did), 10)
            .await
            .unwrap();
        assert_eq!(next.len(), 2);
        assert_eq!(next[0].did, "did:plc:c");

        let filtered = store
            .list_accounts(Some("primary"), None, 10)
            .await
            .unwrap();
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|a| a.node == "primary"));
    }

    #[tokio::test]
    async fn drains_a_node() {
        let store = store().await;
        store
            .upsert_account("did:plc:a", "a.rocksky.social", "radxa")
            .await
            .unwrap();
        store
            .upsert_account("did:plc:b", "b.rocksky.social", "radxa")
            .await
            .unwrap();

        assert_eq!(store.move_node("radxa", "primary").await.unwrap(), 2);
        assert_eq!(
            store
                .account_by_did("did:plc:a")
                .await
                .unwrap()
                .unwrap()
                .node,
            "primary"
        );
    }

    #[tokio::test]
    async fn deletes_accounts() {
        let store = store().await;
        store
            .upsert_account("did:plc:alice", "alice.rocksky.social", "radxa")
            .await
            .unwrap();

        assert!(store.delete_account("did:plc:alice").await.unwrap());
        assert!(!store.delete_account("did:plc:alice").await.unwrap());
        assert!(
            store
                .account_by_did("did:plc:alice")
                .await
                .unwrap()
                .is_none()
        );
    }
}

impl Store {
    pub async fn firehose_cursor(&self, node: &str) -> Result<Option<i64>> {
        let row = sqlx::query("SELECT upstream_seq FROM firehose_cursors WHERE node = ?")
            .bind(node)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<i64, _>("upstream_seq")))
    }

    pub async fn put_firehose_cursor(
        &self,
        node: &str,
        upstream_seq: i64,
        next_seq: i64,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO firehose_cursors (node, upstream_seq, updated_at) VALUES (?, ?, ?) \
             ON CONFLICT(node) DO UPDATE SET \
               upstream_seq = excluded.upstream_seq, updated_at = excluded.updated_at",
        )
        .bind(node)
        .bind(upstream_seq)
        .bind(now())
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO firehose_sequence (id, next_seq) VALUES (1, ?) \
             ON CONFLICT(id) DO UPDATE SET next_seq = MAX(next_seq, excluded.next_seq)",
        )
        .bind(next_seq)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn firehose_next_seq(&self) -> Result<i64> {
        let row = sqlx::query("SELECT next_seq FROM firehose_sequence WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<i64, _>("next_seq")).unwrap_or(1))
    }
}

#[cfg(test)]
mod firehose_tests {
    use super::*;

    #[tokio::test]
    async fn cursors_start_empty_and_persist() {
        let store = Store::open_in_memory().await.unwrap();
        assert!(store.firehose_cursor("radxa").await.unwrap().is_none());
        assert_eq!(store.firehose_next_seq().await.unwrap(), 1);

        store.put_firehose_cursor("radxa", 42, 100).await.unwrap();
        assert_eq!(store.firehose_cursor("radxa").await.unwrap(), Some(42));
        assert_eq!(store.firehose_next_seq().await.unwrap(), 100);
    }

    #[tokio::test]
    async fn the_gateway_sequence_never_goes_backwards() {
        let store = Store::open_in_memory().await.unwrap();
        store.put_firehose_cursor("radxa", 10, 500).await.unwrap();
        // A lagging node must not rewind the shared high-water mark.
        store.put_firehose_cursor("primary", 3, 200).await.unwrap();

        assert_eq!(store.firehose_next_seq().await.unwrap(), 500);
        assert_eq!(store.firehose_cursor("primary").await.unwrap(), Some(3));
    }
}
