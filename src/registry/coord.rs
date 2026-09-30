//! Cross-replica coordination.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use redis::AsyncCommands;
use redis::aio::ConnectionManager;

use crate::config::RedisConfig;

#[derive(Clone)]
pub enum Coordinator {
    Local(Arc<Local>),
    Redis(Arc<Redis>),
}

pub struct Local {
    round_robin: AtomicU64,
}

pub struct Redis {
    conn: ConnectionManager,
    prefix: String,
    fail_open: bool,
    response_timeout: Duration,
    round_robin: AtomicU64,
}

impl Coordinator {
    pub async fn connect(config: &RedisConfig) -> anyhow::Result<Self> {
        let Some(url) = &config.url else {
            tracing::info!("redis not configured; coordinating in-process");
            return Ok(Self::Local(Arc::new(Local {
                round_robin: AtomicU64::new(0),
            })));
        };

        let client = redis::Client::open(url.as_str())?;
        let connect = redis::aio::ConnectionManager::new(client);
        let conn = tokio::time::timeout(config.connect_timeout.get(), connect)
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "redis connection to {url} timed out after {:?}",
                    config.connect_timeout.get()
                )
            })??;

        tracing::info!(prefix = %config.key_prefix, "coordinating through redis");
        Ok(Self::Redis(Arc::new(Redis {
            conn,
            prefix: config.key_prefix.clone(),
            fail_open: config.fail_open,
            response_timeout: config.response_timeout.get(),
            round_robin: AtomicU64::new(0),
        })))
    }

    pub fn is_shared(&self) -> bool {
        matches!(self, Self::Redis(_))
    }

    pub async fn cache_get(&self, namespace: &str, key: &str) -> Option<String> {
        let Self::Redis(redis) = self else {
            return None;
        };
        let full = redis.key(namespace, key);
        let mut conn = redis.conn.clone();
        match redis.guard(conn.get::<_, Option<String>>(&full)).await {
            Ok(value) => value,
            Err(e) => {
                redis.report("get", &full, &e);
                None
            }
        }
    }

    pub async fn cache_put(&self, namespace: &str, key: &str, value: &str, ttl: Duration) {
        let Self::Redis(redis) = self else {
            return;
        };
        let full = redis.key(namespace, key);
        let mut conn = redis.conn.clone();
        let seconds = ttl.as_secs().max(1);
        if let Err(e) = redis
            .guard(conn.set_ex::<_, _, ()>(&full, value, seconds))
            .await
        {
            redis.report("set_ex", &full, &e);
        }
    }

    pub async fn cache_del(&self, namespace: &str, key: &str) {
        let Self::Redis(redis) = self else {
            return;
        };
        let full = redis.key(namespace, key);
        let mut conn = redis.conn.clone();
        if let Err(e) = redis.guard(conn.del::<_, ()>(&full)).await {
            redis.report("del", &full, &e);
        }
    }

    pub async fn try_lock_handle(&self, handle: &str, owner: &str, ttl: Duration) -> bool {
        let Self::Redis(redis) = self else {
            return true;
        };
        let full = redis.key("handle-lock", handle);
        let mut conn = redis.conn.clone();
        let options = redis::SetOptions::default()
            .conditional_set(redis::ExistenceCheck::NX)
            .with_expiration(redis::SetExpiry::EX(ttl.as_secs().max(1)));

        match redis
            .guard(conn.set_options::<_, _, Option<String>>(&full, owner, options))
            .await
        {
            // Redis replies OK on success and nil when NX rejected the write.
            Ok(reply) => reply.is_some(),
            Err(e) => {
                redis.report("set_nx", &full, &e);
                redis.fail_open
            }
        }
    }

    pub async fn unlock_handle(&self, handle: &str) {
        self.cache_del("handle-lock", handle).await;
    }

    /// Monotonic counter for round-robin placement. Shared through Redis when
    /// available so replicas do not all start from the same node.
    pub async fn next_rotation(&self) -> u64 {
        match self {
            Self::Local(local) => local.round_robin.fetch_add(1, Ordering::Relaxed),
            Self::Redis(redis) => {
                let full = redis.key("rotation", "placement");
                let mut conn = redis.conn.clone();
                match redis.guard(conn.incr::<_, _, u64>(&full, 1_u64)).await {
                    Ok(n) => n,
                    Err(e) => {
                        redis.report("incr", &full, &e);
                        redis.round_robin.fetch_add(1, Ordering::Relaxed)
                    }
                }
            }
        }
    }
}

impl Redis {
    fn key(&self, namespace: &str, key: &str) -> String {
        format!("{}:{namespace}:{key}", self.prefix)
    }

    async fn guard<T>(
        &self,
        op: impl Future<Output = redis::RedisResult<T>>,
    ) -> redis::RedisResult<T> {
        match tokio::time::timeout(self.response_timeout, op).await {
            Ok(result) => result,
            Err(_) => Err(redis::RedisError::from((
                redis::ErrorKind::Io,
                "redis command timed out",
            ))),
        }
    }

    fn report(&self, op: &str, key: &str, error: &redis::RedisError) {
        crate::metrics::REDIS_ERRORS.incr();
        if self.fail_open {
            tracing::warn!(op, key, error = %error, "redis unavailable; continuing without it");
        } else {
            tracing::error!(op, key, error = %error, "redis command failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_coordination_needs_no_redis() {
        let coord = Coordinator::connect(&RedisConfig::default()).await.unwrap();
        assert!(!coord.is_shared());

        // Locks always succeed locally; SQLite is the real arbiter.
        assert!(
            coord
                .try_lock_handle("alice.rocksky.social", "me", Duration::from_secs(60))
                .await
        );
        assert!(
            coord
                .cache_get("h2d", "alice.rocksky.social")
                .await
                .is_none()
        );

        assert_eq!(coord.next_rotation().await, 0);
        assert_eq!(coord.next_rotation().await, 1);
        assert_eq!(coord.next_rotation().await, 2);
    }

    #[tokio::test]
    async fn a_bad_redis_url_fails_to_connect() {
        let config = RedisConfig {
            url: Some("redis://127.0.0.1:1".to_owned()),
            connect_timeout: Duration::from_millis(250).into(),
            ..RedisConfig::default()
        };
        assert!(Coordinator::connect(&config).await.is_err());
    }
}
