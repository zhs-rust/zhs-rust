use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use zhs_crypto::{hex, random, sha256};

pub const DEFAULT_LIFETIME: u64 = 30 * 24 * 60 * 60;

pub const TOKEN_BYTES: usize = 32;

const MAX_TOKEN_CHARS: usize = 512;

/// A revocation within this window is treated as a benign concurrent
/// duplicate rotation instead of genuine token reuse, so the losing
/// request does not revoke every token of the subject.
const REUSE_GRACE_SECONDS: u64 = 10;

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub fn hash_token(token: &str) -> String {
    hex::encode(&sha256::digest(token.as_bytes()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshTokenRecord {
    pub hash: String,

    pub subject: String,

    pub issued_at: u64,

    pub expires_at: u64,

    pub revoked_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedRefreshToken {
    pub token: String,

    pub subject: String,

    pub expires_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefreshError {
    EmptySubject,

    Malformed,

    NotFound,

    Expired,

    Revoked,

    ReuseDetected,

    Entropy,

    Store(String),
}

impl RefreshError {
    pub fn store(message: impl Into<String>) -> Self {
        Self::Store(message.into())
    }
}

impl fmt::Display for RefreshError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RefreshError::EmptySubject => formatter.write_str("refresh token subject is empty"),
            RefreshError::Malformed => formatter.write_str("malformed refresh token"),
            RefreshError::NotFound => formatter.write_str("refresh token was not found"),
            RefreshError::Expired => formatter.write_str("refresh token has expired"),
            RefreshError::Revoked => formatter.write_str("refresh token has been revoked"),
            RefreshError::ReuseDetected => formatter.write_str("revoked refresh token was reused"),
            RefreshError::Entropy => {
                formatter.write_str("failed to read secure random bytes for the refresh token")
            }
            RefreshError::Store(message) => {
                write!(formatter, "refresh token store failed: {message}")
            }
        }
    }
}

impl StdError for RefreshError {}

pub trait RefreshTokenStore: Send + Sync {
    fn save(&self, record: &RefreshTokenRecord) -> Result<(), RefreshError>;

    fn find(&self, hash: &str) -> Result<Option<RefreshTokenRecord>, RefreshError>;

    fn revoke(&self, hash: &str, at: u64) -> Result<bool, RefreshError>;

    fn revoke_subject(&self, subject: &str, at: u64) -> Result<usize, RefreshError>;
}

#[derive(Clone)]
pub struct RefreshTokens {
    store: Arc<dyn RefreshTokenStore>,
    lifetime: u64,
}

impl RefreshTokens {
    pub fn new(store: Arc<dyn RefreshTokenStore>) -> Self {
        Self {
            store,
            lifetime: DEFAULT_LIFETIME,
        }
    }

    pub fn with_lifetime(mut self, seconds: u64) -> Self {
        self.lifetime = seconds;
        self
    }

    pub fn lifetime(&self) -> u64 {
        self.lifetime
    }

    pub fn issue(&self, subject: &str, now: u64) -> Result<IssuedRefreshToken, RefreshError> {
        if subject.is_empty() {
            return Err(RefreshError::EmptySubject);
        }

        let token = random::token(TOKEN_BYTES).map_err(|_| RefreshError::Entropy)?;
        let record = RefreshTokenRecord {
            hash: hash_token(&token),
            subject: subject.to_owned(),
            issued_at: now,
            expires_at: now.saturating_add(self.lifetime),
            revoked_at: None,
        };
        self.store.save(&record)?;

        Ok(IssuedRefreshToken {
            token,
            subject: record.subject,
            expires_at: record.expires_at,
        })
    }

    pub fn verify(&self, token: &str, now: u64) -> Result<RefreshTokenRecord, RefreshError> {
        let record = self.lookup(token)?;
        self.check(&record, now)?;
        Ok(record)
    }

    pub fn rotate(&self, token: &str, now: u64) -> Result<IssuedRefreshToken, RefreshError> {
        let record = self.lookup(token)?;

        if let Some(revoked_at) = record.revoked_at {
            if Self::within_reuse_grace(revoked_at, now) {
                // Benign concurrent duplicate rotation: the winner revoked
                // this token moments ago. Do not revoke the whole subject.
                return Err(RefreshError::ReuseDetected);
            }
            self.store.revoke_subject(&record.subject, now)?;
            return Err(RefreshError::ReuseDetected);
        }
        if now > record.expires_at {
            return Err(RefreshError::Expired);
        }
        if !self.store.revoke(&record.hash, now)? {
            // Lost a race against a concurrent rotation. If the winner
            // revoked the token within the grace window, treat this as a
            // benign duplicate instead of revoking the whole subject.
            let benign = self
                .store
                .find(&record.hash)?
                .and_then(|fresh| fresh.revoked_at)
                .is_some_and(|revoked_at| Self::within_reuse_grace(revoked_at, now));
            if !benign {
                self.store.revoke_subject(&record.subject, now)?;
            }
            return Err(RefreshError::ReuseDetected);
        }

        self.issue(&record.subject, now)
    }

    fn within_reuse_grace(revoked_at: u64, now: u64) -> bool {
        now.saturating_sub(revoked_at) <= REUSE_GRACE_SECONDS
    }

    pub fn revoke(&self, token: &str, now: u64) -> Result<bool, RefreshError> {
        self.store.revoke(&self.hash_of(token)?, now)
    }

    pub fn revoke_subject(&self, subject: &str, now: u64) -> Result<usize, RefreshError> {
        if subject.is_empty() {
            return Err(RefreshError::EmptySubject);
        }
        self.store.revoke_subject(subject, now)
    }

    fn hash_of(&self, token: &str) -> Result<String, RefreshError> {
        if token.is_empty() || token.len() > MAX_TOKEN_CHARS {
            return Err(RefreshError::Malformed);
        }
        Ok(hash_token(token))
    }

    fn lookup(&self, token: &str) -> Result<RefreshTokenRecord, RefreshError> {
        let hash = self.hash_of(token)?;
        self.store.find(&hash)?.ok_or(RefreshError::NotFound)
    }

    fn check(&self, record: &RefreshTokenRecord, now: u64) -> Result<(), RefreshError> {
        if record.revoked_at.is_some() {
            return Err(RefreshError::Revoked);
        }
        if now > record.expires_at {
            return Err(RefreshError::Expired);
        }
        Ok(())
    }
}

impl fmt::Debug for RefreshTokens {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RefreshTokens")
            .field("lifetime", &self.lifetime)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
pub struct MemoryRefreshTokenStore {
    records: Mutex<HashMap<String, RefreshTokenRecord>>,
}

const MAX_ENTRIES: usize = 10_000;

impl MemoryRefreshTokenStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, RefreshTokenRecord>> {
        self.records
            .lock()
            .unwrap_or_else(crate::internal::recover_poison)
    }
}

impl RefreshTokenStore for MemoryRefreshTokenStore {
    fn save(&self, record: &RefreshTokenRecord) -> Result<(), RefreshError> {
        let mut records = self.lock();
        if !records.contains_key(&record.hash) && records.len() >= MAX_ENTRIES {
            // Bound memory: drop expired/revoked records first, then the
            // oldest remaining entries by issuance timestamp.
            let now = now();
            records.retain(|_, existing| existing.revoked_at.is_none() && now <= existing.expires_at);
            while records.len() >= MAX_ENTRIES {
                let Some(oldest) = records
                    .iter()
                    .min_by_key(|(_, existing)| existing.issued_at)
                    .map(|(hash, _)| hash.clone())
                else {
                    break;
                };
                records.remove(&oldest);
            }
        }
        records.insert(record.hash.clone(), record.clone());
        Ok(())
    }

    fn find(&self, hash: &str) -> Result<Option<RefreshTokenRecord>, RefreshError> {
        Ok(self.lock().get(hash).cloned())
    }

    fn revoke(&self, hash: &str, at: u64) -> Result<bool, RefreshError> {
        let mut records = self.lock();
        let Some(record) = records.get_mut(hash) else {
            return Ok(false);
        };
        if record.revoked_at.is_some() {
            return Ok(false);
        }
        record.revoked_at = Some(at);
        Ok(true)
    }

    fn revoke_subject(&self, subject: &str, at: u64) -> Result<usize, RefreshError> {
        let mut revoked = 0;
        for record in self.lock().values_mut() {
            if record.subject == subject && record.revoked_at.is_none() {
                record.revoked_at = Some(at);
                revoked += 1;
            }
        }
        Ok(revoked)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_LIFETIME, MemoryRefreshTokenStore, RefreshError, RefreshTokens, hash_token,
    };
    use std::sync::Arc;

    fn service(lifetime: u64) -> (RefreshTokens, Arc<MemoryRefreshTokenStore>) {
        let store = Arc::new(MemoryRefreshTokenStore::new());
        let tokens = RefreshTokens::new(store.clone()).with_lifetime(lifetime);
        (tokens, store)
    }

    #[test]
    fn issues_and_verifies_tokens() {
        let (tokens, store) = service(DEFAULT_LIFETIME);
        let issued = tokens.issue("ada", 1_000).unwrap();

        assert_eq!(issued.expires_at, 1_000 + DEFAULT_LIFETIME);
        assert_eq!(issued.subject, "ada");
        assert_eq!(store.len(), 1);

        let record = tokens.verify(&issued.token, 1_000).unwrap();
        assert_eq!(record.subject, "ada");
        assert_eq!(record.hash, hash_token(&issued.token));
        assert!(!record.hash.contains(issued.token.as_str()));
    }

    #[test]
    fn rotates_and_revokes_the_old_token() {
        let (tokens, _) = service(60);
        let first = tokens.issue("ada", 1_000).unwrap();
        let second = tokens.rotate(&first.token, 1_010).unwrap();

        assert_ne!(first.token, second.token);
        assert_eq!(
            tokens.verify(&first.token, 1_010),
            Err(RefreshError::Revoked)
        );
        assert_eq!(tokens.verify(&second.token, 1_010).unwrap().subject, "ada");
    }

    #[test]
    fn reusing_a_rotated_token_revokes_the_whole_subject() {
        let (tokens, _) = service(60);
        let first = tokens.issue("ada", 1_000).unwrap();
        let second = tokens.rotate(&first.token, 1_010).unwrap();

        // Beyond the reuse grace window this is genuine reuse.
        assert_eq!(
            tokens.rotate(&first.token, 1_021),
            Err(RefreshError::ReuseDetected)
        );
        assert_eq!(
            tokens.verify(&second.token, 1_021),
            Err(RefreshError::Revoked)
        );
    }

    #[test]
    fn concurrent_duplicate_rotation_inside_grace_window_keeps_the_subject() {
        let (tokens, _) = service(60);
        let first = tokens.issue("ada", 1_000).unwrap();
        let second = tokens.rotate(&first.token, 1_010).unwrap();

        // The losing concurrent rotation (within the grace window) reports
        // the reuse error but must NOT revoke the subject's other tokens.
        assert_eq!(
            tokens.rotate(&first.token, 1_015),
            Err(RefreshError::ReuseDetected)
        );
        assert_eq!(
            tokens.rotate(&first.token, 1_020),
            Err(RefreshError::ReuseDetected)
        );
        assert!(tokens.verify(&second.token, 1_020).is_ok());
    }

    #[test]
    fn memory_store_is_capped() {
        use super::{MAX_ENTRIES, RefreshTokenRecord, RefreshTokenStore};

        let store = MemoryRefreshTokenStore::new();
        for index in 0..(MAX_ENTRIES + 5) {
            let record = RefreshTokenRecord {
                hash: format!("hash-{index}"),
                subject: "ada".to_owned(),
                issued_at: index as u64,
                expires_at: u64::MAX,
                revoked_at: None,
            };
            store.save(&record).unwrap();
        }

        assert_eq!(store.len(), MAX_ENTRIES);
        assert!(store.find("hash-0").unwrap().is_none());
        assert!(store
            .find(&format!("hash-{}", MAX_ENTRIES + 4))
            .unwrap()
            .is_some());
    }

    #[test]
    fn expires_tokens() {
        let (tokens, _) = service(10);
        let issued = tokens.issue("ada", 1_000).unwrap();

        assert!(tokens.verify(&issued.token, 1_010).is_ok());
        assert_eq!(
            tokens.verify(&issued.token, 1_011),
            Err(RefreshError::Expired)
        );
        assert_eq!(
            tokens.rotate(&issued.token, 1_011),
            Err(RefreshError::Expired)
        );
    }

    #[test]
    fn rejects_bad_tokens() {
        let (tokens, _) = service(60);

        assert_eq!(tokens.verify("", 1_000), Err(RefreshError::Malformed));
        assert_eq!(
            tokens.verify(&"x".repeat(513), 1_000),
            Err(RefreshError::Malformed)
        );
        assert_eq!(
            tokens.verify("not-a-real-token", 1_000),
            Err(RefreshError::NotFound)
        );
        assert_eq!(tokens.issue("", 1_000), Err(RefreshError::EmptySubject));
        assert_eq!(
            tokens.revoke_subject("", 1_000),
            Err(RefreshError::EmptySubject)
        );
    }

    #[test]
    fn revoke_is_idempotent() {
        let (tokens, _) = service(60);
        let issued = tokens.issue("ada", 1_000).unwrap();

        assert!(tokens.revoke(&issued.token, 1_000).unwrap());
        assert!(!tokens.revoke(&issued.token, 1_001).unwrap());
        assert!(!tokens.revoke("unknown-token", 1_001).unwrap());
        assert_eq!(
            tokens.verify(&issued.token, 1_001),
            Err(RefreshError::Revoked)
        );
    }

    #[test]
    fn revoke_subject_reports_the_count() {
        let (tokens, store) = service(60);
        tokens.issue("ada", 1_000).unwrap();
        tokens.issue("ada", 1_001).unwrap();
        tokens.issue("bob", 1_002).unwrap();

        assert_eq!(tokens.revoke_subject("ada", 1_010).unwrap(), 2);
        assert_eq!(tokens.revoke_subject("ada", 1_011).unwrap(), 0);
        assert_eq!(store.len(), 3);
    }
}
