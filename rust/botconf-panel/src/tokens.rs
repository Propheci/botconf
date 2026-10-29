//! Button tokens: an action's stateless encoding when it fits the front end's
//! limit, and otherwise a short handle kept in a [`TokenStore`].
//!
//! A handle is `~` followed by 10 base62 characters (11 bytes, which fits
//! every front end). They come from a 64-bit FNV-1a hash of the encoded
//! action, so the same action always gets the same handle: redrawing a
//! screen renews the stored entry instead of adding one. `~` never starts a
//! stateless token, whose action codes are letters.

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicI64, Ordering},
    },
};

use botconf::StorageError;
use futures::future::BoxFuture;

use crate::token::Action;

#[cfg(feature = "sea-orm")]
mod sea_orm;
#[cfg(feature = "sea-orm")]
pub use self::sea_orm::{DEFAULT_TOKENS_TABLE, SeaOrmTokenStore, create_tokens_table};

/// Starts every handle.
pub(crate) const HANDLE_MARK: char = '~';

/// The length of a handle, in bytes.
pub const HANDLE_LEN: usize = 11;

/// How many handles are tried for one action before its button is left out.
const ATTEMPTS: u8 = 3;

/// Expired handles are purged at most this often, on a `put`.
pub(crate) const PURGE_INTERVAL_MS: i64 = 60 * 60 * 1000;

/// Keeps the actions of buttons whose stateless token would be too long.
/// The host provides it; [`MemoryTokenStore`] is the default.
pub trait TokenStore: Send + Sync + 'static {
    /// Stores `action` under `key` until `expires_at_ms`. If `key` holds the
    /// same action, or an action that expired before `now_ms`, it is replaced
    /// (renewing the expiry). Returns `false`, changing nothing, if `key`
    /// holds a different unexpired action (a collision).
    fn put<'a>(
        &'a self,
        key: &'a str,
        action: &'a str,
        now_ms: i64,
        expires_at_ms: i64,
    ) -> BoxFuture<'a, Result<bool, StorageError>>;

    /// The action stored under `key`, unless it is absent or expired at
    /// `now_ms`.
    fn get<'a>(
        &'a self,
        key: &'a str,
        now_ms: i64,
    ) -> BoxFuture<'a, Result<Option<String>, StorageError>>;

    /// Deletes the entries expired at `now_ms`; returns how many.
    fn purge_expired(&self, now_ms: i64) -> BoxFuture<'_, Result<u64, StorageError>>;
}

/// Handles in memory: they last until the process stops.
#[derive(Debug, Default)]
pub struct MemoryTokenStore {
    entries: Mutex<HashMap<String, (String, i64)>>,
}

impl MemoryTokenStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many entries are kept, expired ones included.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (String, i64)>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl TokenStore for MemoryTokenStore {
    fn put<'a>(
        &'a self,
        key: &'a str,
        action: &'a str,
        now_ms: i64,
        expires_at_ms: i64,
    ) -> BoxFuture<'a, Result<bool, StorageError>> {
        let mut entries = self.lock();
        let stored = match entries.get(key) {
            Some((existing, expires)) if existing != action && *expires > now_ms => false,
            _ => {
                entries.insert(key.to_string(), (action.to_string(), expires_at_ms));
                true
            }
        };
        Box::pin(async move { Ok(stored) })
    }

    fn get<'a>(
        &'a self,
        key: &'a str,
        now_ms: i64,
    ) -> BoxFuture<'a, Result<Option<String>, StorageError>> {
        let found = self
            .lock()
            .get(key)
            .filter(|(_, expires)| *expires > now_ms)
            .map(|(action, _)| action.clone());
        Box::pin(async move { Ok(found) })
    }

    fn purge_expired(&self, now_ms: i64) -> BoxFuture<'_, Result<u64, StorageError>> {
        let mut entries = self.lock();
        let before = entries.len();
        entries.retain(|_, (_, expires)| *expires > now_ms);
        let purged = (before - entries.len()) as u64;
        Box::pin(async move { Ok(purged) })
    }
}

impl<T: TokenStore> TokenStore for std::sync::Arc<T> {
    fn put<'a>(
        &'a self,
        key: &'a str,
        action: &'a str,
        now_ms: i64,
        expires_at_ms: i64,
    ) -> BoxFuture<'a, Result<bool, StorageError>> {
        (**self).put(key, action, now_ms, expires_at_ms)
    }

    fn get<'a>(
        &'a self,
        key: &'a str,
        now_ms: i64,
    ) -> BoxFuture<'a, Result<Option<String>, StorageError>> {
        (**self).get(key, now_ms)
    }

    fn purge_expired(&self, now_ms: i64) -> BoxFuture<'_, Result<u64, StorageError>> {
        (**self).purge_expired(now_ms)
    }
}

/// The store with its policy: the TTL and when it was last purged.
pub(crate) struct Tokens {
    pub(crate) store: Box<dyn TokenStore>,
    pub(crate) ttl_ms: i64,
    last_purge: AtomicI64,
}

impl Tokens {
    pub(crate) fn new(store: Box<dyn TokenStore>, ttl_ms: i64) -> Self {
        Self {
            store,
            ttl_ms,
            last_purge: AtomicI64::new(i64::MIN),
        }
    }

    /// The token of `action`: its stateless encoding if it fits
    /// `token_bytes`, else a stored handle. `None` when no handle could be
    /// stored (after 3 collisions) or a handle does not fit either.
    pub(crate) async fn issue(
        &self,
        action: &Action,
        token_bytes: usize,
        now_ms: i64,
    ) -> Result<Option<String>, StorageError> {
        let encoded = action.encode();
        if encoded.len() <= token_bytes {
            return Ok(Some(encoded));
        }
        if token_bytes < HANDLE_LEN {
            return Ok(None);
        }

        self.purge_if_due(now_ms).await?;
        let expires = now_ms.saturating_add(self.ttl_ms);
        for attempt in 0..ATTEMPTS {
            let key = handle(&encoded, attempt);
            if self.store.put(&key, &encoded, now_ms, expires).await? {
                return Ok(Some(key));
            }
        }
        tracing::warn!(
            action = encoded,
            "no free handle for a long button; it is left out"
        );
        Ok(None)
    }

    /// The action `token` stands for: parsed, or looked up for a handle.
    /// `Ok(None)` for an unknown or expired handle; `Err` for a token that is
    /// neither.
    pub(crate) async fn redeem(&self, token: &str, now_ms: i64) -> Result<Redeemed, StorageError> {
        if !token.starts_with(HANDLE_MARK) {
            return Ok(Action::parse(token).map_or(Redeemed::Malformed, Redeemed::Action));
        }
        Ok(match self.store.get(token, now_ms).await? {
            Some(encoded) => Action::parse(&encoded).map_or(Redeemed::Malformed, Redeemed::Action),
            None => Redeemed::Expired,
        })
    }

    /// Deletes the expired handles now.
    pub(crate) async fn purge(&self, now_ms: i64) -> Result<u64, StorageError> {
        self.last_purge.store(now_ms, Ordering::Relaxed);
        self.store.purge_expired(now_ms).await
    }

    async fn purge_if_due(&self, now_ms: i64) -> Result<(), StorageError> {
        let last = self.last_purge.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last) >= PURGE_INTERVAL_MS
            && self
                .last_purge
                .compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            self.store.purge_expired(now_ms).await?;
        }
        Ok(())
    }
}

/// What a token stood for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Redeemed {
    Action(Action),
    /// An unknown or expired handle.
    Expired,
    /// Not a token of this panel.
    Malformed,
}

/// The `attempt`th handle of an encoded action.
fn handle(encoded: &str, attempt: u8) -> String {
    let mut bytes = encoded.as_bytes().to_vec();
    if attempt > 0 {
        bytes.push(0);
        bytes.push(attempt);
    }
    let mut hash = fnv1a(&bytes);
    let mut key = String::with_capacity(HANDLE_LEN);
    key.push(HANDLE_MARK);
    for _ in 0..HANDLE_LEN - 1 {
        key.push(BASE62[(hash % 62) as usize] as char);
        hash /= 62;
    }
    key
}

const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// 64-bit FNV-1a: fixed, unlike `DefaultHasher`, whose output may change
/// between Rust releases (stored handles must stay valid).
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;
    bytes.iter().fold(OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use crate::token::{Page, Target};

    const DAY: i64 = 24 * 60 * 60 * 1000;

    fn tokens(store: impl TokenStore) -> Tokens {
        Tokens::new(Box::new(store), DAY)
    }

    #[test]
    fn handles_are_stable_and_short() {
        let encoded = "d:3.some-long-entry-name";
        assert_eq!(handle(encoded, 0), handle(encoded, 0));
        assert_ne!(handle(encoded, 0), handle(encoded, 1));
        assert_eq!(handle(encoded, 0).len(), HANDLE_LEN);
        assert!(handle(encoded, 0).starts_with(HANDLE_MARK));
        // FNV-1a's reference value, so the derivation can't drift.
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[tokio::test]
    async fn data_that_does_not_fit_is_refused() {
        // Without room for a handle, a long action gets no token...
        let tokens = tokens(MemoryTokenStore::new());
        let long = Action::Delete(Target::setting(1).entry(&"x".repeat(60)));
        assert_eq!(tokens.issue(&long, 8, 0).await.unwrap(), None);

        // ...and with room, a handle that leads back to it.
        let token = tokens.issue(&long, 20, 0).await.unwrap().unwrap();
        assert!(token.starts_with(HANDLE_MARK) && token.len() == HANDLE_LEN);
        assert_eq!(
            tokens.redeem(&token, 1).await.unwrap(),
            Redeemed::Action(long)
        );

        // Short actions stay stateless.
        let home = Action::open(Page::Home);
        assert_eq!(tokens.issue(&home, 20, 0).await.unwrap().unwrap(), "o:h");
    }

    #[tokio::test]
    async fn expired_and_unknown_handles_are_expired() {
        let tokens = tokens(MemoryTokenStore::new());
        let long = Action::Delete(Target::setting(1).entry(&"y".repeat(60)));
        let token = tokens.issue(&long, 20, 0).await.unwrap().unwrap();
        assert_eq!(
            tokens.redeem(&token, DAY + 1).await.unwrap(),
            Redeemed::Expired
        );
        assert_eq!(
            tokens.redeem("~0000000000", 0).await.unwrap(),
            Redeemed::Expired
        );
        assert_eq!(tokens.redeem("q:1", 0).await.unwrap(), Redeemed::Malformed);
    }

    /// Counts the purges of a memory store.
    #[derive(Default)]
    struct Counting {
        inner: MemoryTokenStore,
        purges: AtomicUsize,
    }

    impl TokenStore for Counting {
        fn put<'a>(
            &'a self,
            key: &'a str,
            action: &'a str,
            now_ms: i64,
            expires_at_ms: i64,
        ) -> BoxFuture<'a, Result<bool, StorageError>> {
            self.inner.put(key, action, now_ms, expires_at_ms)
        }

        fn get<'a>(
            &'a self,
            key: &'a str,
            now_ms: i64,
        ) -> BoxFuture<'a, Result<Option<String>, StorageError>> {
            self.inner.get(key, now_ms)
        }

        fn purge_expired(&self, now_ms: i64) -> BoxFuture<'_, Result<u64, StorageError>> {
            self.purges.fetch_add(1, Ordering::Relaxed);
            self.inner.purge_expired(now_ms)
        }
    }

    #[tokio::test]
    async fn expired_handles_are_purged_hourly() {
        let store = Arc::new(Counting::default());
        let tokens = tokens(Arc::clone(&store));
        let long = |n: usize| Action::Delete(Target::setting(n).entry(&"z".repeat(60)));
        let purges = || store.purges.load(Ordering::Relaxed);

        tokens.issue(&long(1), 20, 0).await.unwrap();
        tokens
            .issue(&long(2), 20, PURGE_INTERVAL_MS - 1)
            .await
            .unwrap();
        assert_eq!(purges(), 1);
        tokens.issue(&long(3), 20, PURGE_INTERVAL_MS).await.unwrap();
        assert_eq!(purges(), 2);
        // Stateless tokens never touch the store.
        tokens
            .issue(&Action::Reload, 20, 10 * PURGE_INTERVAL_MS)
            .await
            .unwrap();
        assert_eq!(purges(), 2);
    }

    #[tokio::test]
    async fn colliding_handles_take_the_next_key() {
        let store = Arc::new(MemoryTokenStore::new());
        let tokens = tokens(Arc::clone(&store));
        let long = Action::Delete(Target::setting(1).entry(&"w".repeat(60)));
        let first = handle(&long.encode(), 0);
        assert!(store.put(&first, "x", 0, DAY).await.unwrap());

        let token = tokens.issue(&long, 20, 0).await.unwrap().unwrap();
        assert_eq!(token, handle(&long.encode(), 1));
        assert_eq!(
            tokens.redeem(&token, 0).await.unwrap(),
            Redeemed::Action(long)
        );
    }

    #[tokio::test]
    async fn redrawing_reuses_handles() {
        let store = Arc::new(MemoryTokenStore::new());
        let tokens = tokens(Arc::clone(&store));
        let long = Action::Delete(Target::setting(1).entry(&"v".repeat(60)));
        let first = tokens.issue(&long, 20, 0).await.unwrap();
        let again = tokens.issue(&long, 20, 1000).await.unwrap();
        assert_eq!(first, again);
        assert_eq!(store.len(), 1);
    }
}
