use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use zhs_crypto::{hex, random, sha256};

pub const PREFIX: &str = "zlk_";

pub const TOKEN_BYTES: usize = 32;

const DISPLAY_CHARS: usize = 8;

const MAX_TOKEN_CHARS: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewApiKey {
    pub name: String,

    pub subject: String,

    pub roles: Vec<String>,

    pub expires_at: Option<u64>,
}

impl NewApiKey {
    pub fn new(name: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            subject: subject.into(),
            roles: Vec::new(),
            expires_at: None,
        }
    }

    pub fn with_roles<I, S>(mut self, roles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.roles = roles.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_expires_at(mut self, seconds: u64) -> Self {
        self.expires_at = Some(seconds);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiKeyRecord {
    pub hash: String,

    pub prefix: String,

    pub name: String,

    pub subject: String,

    pub roles: Vec<String>,

    pub created_at: u64,

    pub expires_at: Option<u64>,

    pub revoked_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedApiKey {
    pub token: String,

    pub record: ApiKeyRecord,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApiKeyError {
    EmptyName,

    EmptySubject,

    Malformed,

    NotFound,

    Expired,

    Revoked,

    Entropy,

    Store(String),
}

impl ApiKeyError {
    pub fn store(message: impl Into<String>) -> Self {
        Self::Store(message.into())
    }
}

impl fmt::Display for ApiKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiKeyError::EmptyName => formatter.write_str("API key name is empty"),
            ApiKeyError::EmptySubject => formatter.write_str("API key subject is empty"),
            ApiKeyError::Malformed => formatter.write_str("malformed API key"),
            ApiKeyError::NotFound => formatter.write_str("API key was not found"),
            ApiKeyError::Expired => formatter.write_str("API key has expired"),
            ApiKeyError::Revoked => formatter.write_str("API key has been revoked"),
            ApiKeyError::Entropy => {
                formatter.write_str("failed to read secure random bytes for the API key")
            }
            ApiKeyError::Store(message) => write!(formatter, "API key store failed: {message}"),
        }
    }
}

impl StdError for ApiKeyError {}

pub trait ApiKeyStore: Send + Sync {
    fn save(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyError>;

    fn find(&self, hash: &str) -> Result<Option<ApiKeyRecord>, ApiKeyError>;

    fn revoke(&self, hash: &str, at: u64) -> Result<bool, ApiKeyError>;
}

#[derive(Clone)]
pub struct ApiKeys {
    store: Arc<dyn ApiKeyStore>,
}

impl ApiKeys {
    pub fn new(store: Arc<dyn ApiKeyStore>) -> Self {
        Self { store }
    }

    pub fn issue(&self, request: &NewApiKey) -> Result<IssuedApiKey, ApiKeyError> {
        self.issue_at(request, now())
    }

    pub fn issue_at(&self, request: &NewApiKey, now: u64) -> Result<IssuedApiKey, ApiKeyError> {
        if request.name.is_empty() {
            return Err(ApiKeyError::EmptyName);
        }
        if request.subject.is_empty() {
            return Err(ApiKeyError::EmptySubject);
        }

        let token = format!(
            "{PREFIX}{}",
            random::token(TOKEN_BYTES).map_err(|_| ApiKeyError::Entropy)?
        );
        let record = ApiKeyRecord {
            hash: hash_key(&token),
            prefix: display_prefix(&token),
            name: request.name.clone(),
            subject: request.subject.clone(),
            roles: request.roles.clone(),
            created_at: now,
            expires_at: request.expires_at,
            revoked_at: None,
        };
        self.store.save(&record)?;

        Ok(IssuedApiKey { token, record })
    }

    pub fn verify(&self, token: &str) -> Result<ApiKeyRecord, ApiKeyError> {
        self.verify_at(token, now())
    }

    pub fn verify_at(&self, token: &str, now: u64) -> Result<ApiKeyRecord, ApiKeyError> {
        let hash = self.hash_of(token)?;
        let record = self.store.find(&hash)?.ok_or(ApiKeyError::NotFound)?;

        if record.revoked_at.is_some() {
            return Err(ApiKeyError::Revoked);
        }
        if record.expires_at.is_some_and(|expires_at| now > expires_at) {
            return Err(ApiKeyError::Expired);
        }

        Ok(record)
    }

    pub fn revoke(&self, token: &str) -> Result<bool, ApiKeyError> {
        let hash = self.hash_of(token)?;
        self.store.revoke(&hash, now())
    }

    fn hash_of(&self, token: &str) -> Result<String, ApiKeyError> {
        if token.len() > MAX_TOKEN_CHARS || !token.starts_with(PREFIX) {
            return Err(ApiKeyError::Malformed);
        }
        Ok(hash_key(token))
    }
}

impl fmt::Debug for ApiKeys {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKeys(..)")
    }
}

pub fn hash_key(token: &str) -> String {
    hex::encode(&sha256::digest(token.as_bytes()))
}

pub fn display_prefix(token: &str) -> String {
    let keep = PREFIX.len() + DISPLAY_CHARS;
    match token.get(..keep) {
        Some(prefix) => prefix.to_owned(),
        None => token.to_owned(),
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Default)]
pub struct MemoryApiKeyStore {
    records: Mutex<HashMap<String, ApiKeyRecord>>,
}

const MAX_ENTRIES: usize = 10_000;

impl MemoryApiKeyStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, ApiKeyRecord>> {
        self.records
            .lock()
            .unwrap_or_else(crate::internal::recover_poison)
    }
}

impl ApiKeyStore for MemoryApiKeyStore {
    fn save(&self, record: &ApiKeyRecord) -> Result<(), ApiKeyError> {
        let mut records = self.lock();
        if !records.contains_key(&record.hash) && records.len() >= MAX_ENTRIES {
            // Bound memory: drop expired/revoked records first, then the
            // oldest remaining entries by creation timestamp.
            let now = now();
            records.retain(|_, existing| {
                existing.revoked_at.is_none()
                    && !existing
                        .expires_at
                        .is_some_and(|expires_at| now > expires_at)
            });
            while records.len() >= MAX_ENTRIES {
                let Some(oldest) = records
                    .iter()
                    .min_by_key(|(_, existing)| existing.created_at)
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

    fn find(&self, hash: &str) -> Result<Option<ApiKeyRecord>, ApiKeyError> {
        Ok(self.lock().get(hash).cloned())
    }

    fn revoke(&self, hash: &str, at: u64) -> Result<bool, ApiKeyError> {
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
}

#[cfg(test)]
mod tests {
    use super::{ApiKeyError, ApiKeys, MemoryApiKeyStore, NewApiKey, PREFIX, hash_key};
    use std::sync::Arc;

    fn keys() -> (ApiKeys, Arc<MemoryApiKeyStore>) {
        let store = Arc::new(MemoryApiKeyStore::new());
        (ApiKeys::new(store.clone()), store)
    }

    #[test]
    fn issues_and_verifies_keys() {
        let (keys, store) = keys();
        let issued = keys
            .issue(&NewApiKey::new("ci", "build-bot").with_roles(["ci", "deploy"]))
            .unwrap();

        assert!(issued.token.starts_with(PREFIX));
        assert_eq!(issued.token.len(), PREFIX.len() + 43);
        assert_eq!(issued.record.prefix, issued.token[..12]);
        assert_eq!(store.len(), 1);

        let record = keys.verify(&issued.token).unwrap();
        assert_eq!(record.subject, "build-bot");
        assert_eq!(record.name, "ci");
        assert_eq!(record.roles, ["ci", "deploy"]);
        assert_eq!(record.hash, hash_key(&issued.token));
        assert!(!record.hash.contains(issued.token.as_str()));
    }

    #[test]
    fn rejects_unknown_and_malformed_tokens() {
        let (keys, _) = keys();

        assert_eq!(keys.verify(""), Err(ApiKeyError::Malformed));
        assert_eq!(keys.verify("not-an-api-key"), Err(ApiKeyError::Malformed));
        assert_eq!(
            keys.verify(&format!("{PREFIX}unknown")),
            Err(ApiKeyError::NotFound)
        );
    }

    #[test]
    fn revokes_keys() {
        let (keys, _) = keys();
        let issued = keys.issue(&NewApiKey::new("ci", "build-bot")).unwrap();

        assert!(keys.revoke(&issued.token).unwrap());
        assert!(!keys.revoke(&issued.token).unwrap());
        assert_eq!(keys.verify(&issued.token), Err(ApiKeyError::Revoked));
    }

    #[test]
    fn expires_keys() {
        let (keys, _) = keys();
        let issued = keys
            .issue_at(
                &NewApiKey::new("ci", "build-bot").with_expires_at(2_000),
                1_000,
            )
            .unwrap();

        assert!(keys.verify_at(&issued.token, 2_000).is_ok());
        assert_eq!(
            keys.verify_at(&issued.token, 2_001),
            Err(ApiKeyError::Expired)
        );
    }

    #[test]
    fn requires_name_and_subject() {
        let (keys, _) = keys();

        assert_eq!(
            keys.issue(&NewApiKey::new("", "build-bot")),
            Err(ApiKeyError::EmptyName)
        );
        assert_eq!(
            keys.issue(&NewApiKey::new("ci", "")),
            Err(ApiKeyError::EmptySubject)
        );
    }

    #[test]
    fn memory_store_is_capped() {
        use super::{ApiKeyRecord, ApiKeyStore, MAX_ENTRIES};

        let store = MemoryApiKeyStore::new();
        for index in 0..(MAX_ENTRIES + 5) {
            let record = ApiKeyRecord {
                hash: format!("hash-{index}"),
                prefix: "zlk_test".to_owned(),
                name: format!("key-{index}"),
                subject: "build-bot".to_owned(),
                roles: Vec::new(),
                created_at: index as u64,
                expires_at: None,
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
}
