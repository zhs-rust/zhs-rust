use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use zhs_crypto::{base64, constant_time, hmac};

use zhs_json::{Object, Value, parse_bytes, to_string};

pub const MAX_TOKEN_BYTES: usize = 8 * 1024;

pub const DEFAULT_LIFETIME: u64 = 15 * 60;

pub const DEFAULT_LEEWAY: u64 = 60;

const HEADER: &str = r#"{"alg":"HS256","typ":"JWT"}"#;

const REGISTERED: [&str; 8] = ["sub", "roles", "iss", "aud", "iat", "nbf", "exp", "jti"];

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum JwtError {
    SecretTooShort,

    TooLarge,

    Malformed,

    UnsupportedAlgorithm,

    InvalidSignature,

    MissingClaim(&'static str),

    InvalidClaim(&'static str),

    Expired,

    NotYetValid,

    WrongIssuer,

    WrongAudience,
}

impl fmt::Display for JwtError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JwtError::SecretTooShort => {
                formatter.write_str("JWT secret must be at least 32 bytes long")
            }
            JwtError::TooLarge => formatter.write_str("JWT is larger than the accepted maximum"),
            JwtError::Malformed => formatter.write_str("malformed JWT"),
            JwtError::UnsupportedAlgorithm => {
                formatter.write_str("unsupported JWT algorithm or extension")
            }
            JwtError::InvalidSignature => formatter.write_str("invalid JWT signature"),
            JwtError::MissingClaim(name) => write!(formatter, "missing JWT claim `{name}`"),
            JwtError::InvalidClaim(name) => write!(formatter, "invalid JWT claim `{name}`"),
            JwtError::Expired => formatter.write_str("JWT has expired"),
            JwtError::NotYetValid => formatter.write_str("JWT is not valid yet"),
            JwtError::WrongIssuer => formatter.write_str("JWT issuer does not match"),
            JwtError::WrongAudience => formatter.write_str("JWT audience does not match"),
        }
    }
}

impl std::error::Error for JwtError {}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Claims {
    pub subject: String,

    pub roles: Vec<String>,

    pub issuer: Option<String>,

    pub audience: Option<String>,

    pub issued_at: Option<u64>,

    pub not_before: Option<u64>,

    pub expires_at: Option<u64>,

    pub id: Option<String>,

    pub extra: Object,
}

impl Claims {
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            ..Self::default()
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

    pub fn with_issuer(mut self, issuer: impl Into<String>) -> Self {
        self.issuer = Some(issuer.into());
        self
    }

    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = Some(audience.into());
        self
    }

    pub fn with_issued_at(mut self, seconds: u64) -> Self {
        self.issued_at = Some(seconds);
        self
    }

    pub fn with_not_before(mut self, seconds: u64) -> Self {
        self.not_before = Some(seconds);
        self
    }

    pub fn with_expires_at(mut self, seconds: u64) -> Self {
        self.expires_at = Some(seconds);
        self
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn with_claim(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.extra.insert(key, value.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VerifiedClaims {
    pub subject: String,

    pub roles: Vec<String>,

    pub issuer: Option<String>,

    pub audience: Vec<String>,

    pub issued_at: u64,

    pub not_before: Option<u64>,

    pub expires_at: u64,

    pub id: Option<String>,

    pub extra: Object,
}

#[derive(Clone)]
pub struct JwtCodec {
    key: Vec<u8>,
    issuer: Option<String>,
    audience: Option<String>,
    lifetime: u64,
    leeway: u64,
}

impl JwtCodec {
    pub fn new(secret: impl AsRef<[u8]>) -> Result<Self, JwtError> {
        let key = secret.as_ref().to_vec();
        if key.len() < 32 {
            return Err(JwtError::SecretTooShort);
        }

        Ok(Self {
            key,
            issuer: None,
            audience: None,
            lifetime: DEFAULT_LIFETIME,
            leeway: DEFAULT_LEEWAY,
        })
    }

    pub fn with_issuer(mut self, issuer: impl Into<String>) -> Self {
        self.issuer = Some(issuer.into());
        self
    }

    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = Some(audience.into());
        self
    }

    pub fn with_lifetime(mut self, seconds: u64) -> Self {
        self.lifetime = seconds;
        self
    }

    pub fn with_leeway(mut self, seconds: u64) -> Self {
        self.leeway = seconds;
        self
    }

    /// 令牌有效期(秒),即 `with_lifetime` 配置的值。
    pub fn lifetime(&self) -> u64 {
        self.lifetime
    }

    pub fn issue(&self, claims: &Claims) -> Result<String, JwtError> {
        self.issue_at(claims, now())
    }

    pub fn issue_at(&self, claims: &Claims, now: u64) -> Result<String, JwtError> {
        if claims.subject.is_empty() {
            return Err(JwtError::MissingClaim("sub"));
        }

        let issued_at = claims.issued_at.unwrap_or(now);
        let expires_at = match claims.expires_at {
            Some(expires_at) => expires_at,
            None if self.lifetime > 0 => issued_at.saturating_add(self.lifetime),
            None => return Err(JwtError::MissingClaim("exp")),
        };

        if expires_at <= issued_at {
            return Err(JwtError::InvalidClaim("exp"));
        }
        if let Some(not_before) = claims.not_before
            && not_before > expires_at
        {
            return Err(JwtError::InvalidClaim("nbf"));
        }

        let mut payload = Object::new();
        payload.insert("sub", Value::from(claims.subject.as_str()));
        if !claims.roles.is_empty() {
            let roles: Vec<Value> = claims
                .roles
                .iter()
                .map(|role| Value::from(role.as_str()))
                .collect();
            payload.insert("roles", Value::from(roles));
        }
        // iss/aud:claims 显式指定的优先;否则用 codec 端配置补齐,
        // 避免"校验端 with_issuer(x) 签发端没写 iss"导致自己签的令牌自己验不过。
        let issuer = claims.issuer.as_ref().or(self.issuer.as_ref());
        if let Some(issuer) = issuer {
            payload.insert("iss", Value::from(issuer.as_str()));
        }
        let audience = claims.audience.as_ref().or(self.audience.as_ref());
        if let Some(audience) = audience {
            payload.insert("aud", Value::from(audience.as_str()));
        }
        payload.insert("iat", Value::from(issued_at));
        if let Some(not_before) = claims.not_before {
            payload.insert("nbf", Value::from(not_before));
        }
        payload.insert("exp", Value::from(expires_at));
        if let Some(id) = &claims.id {
            payload.insert("jti", Value::from(id.as_str()));
        }
        for (key, value) in &claims.extra {
            if !is_registered(key) {
                payload.insert(key, value.clone());
            }
        }

        let header = base64::encode_url(HEADER.as_bytes());
        let body = base64::encode_url(to_string(&Value::Object(payload)).as_bytes());
        let signature = self.sign(&header, &body);

        Ok(format!("{header}.{body}.{signature}"))
    }

    pub fn verify(&self, token: &str) -> Result<VerifiedClaims, JwtError> {
        self.verify_at(token, now())
    }

    pub fn verify_at(&self, token: &str, now: u64) -> Result<VerifiedClaims, JwtError> {
        if token.len() > MAX_TOKEN_BYTES {
            return Err(JwtError::TooLarge);
        }

        let mut segments = token.split('.');
        let (Some(header), Some(payload), Some(signature)) =
            (segments.next(), segments.next(), segments.next())
        else {
            return Err(JwtError::Malformed);
        };
        if segments.next().is_some() {
            return Err(JwtError::Malformed);
        }

        self.check_header(header)?;
        self.check_signature(header, payload, signature)?;

        let bytes = base64::decode_url(payload).map_err(|_| JwtError::Malformed)?;
        let value = parse_bytes(&bytes).map_err(|_| JwtError::Malformed)?;
        let payload = value.as_object().ok_or(JwtError::Malformed)?;

        let subject = required_string(payload, "sub")?;
        let roles = claims_roles(payload)?;
        let issuer = optional_string(payload, "iss")?;
        let audience = claims_audience(payload)?;
        let issued_at = optional_timestamp(payload, "iat")?.unwrap_or(0);
        let not_before = optional_timestamp(payload, "nbf")?;
        let expires_at = required_timestamp(payload, "exp")?;
        let id = optional_string(payload, "jti")?;

        if now > expires_at.saturating_add(self.leeway) {
            return Err(JwtError::Expired);
        }
        if let Some(not_before) = not_before
            && now.saturating_add(self.leeway) < not_before
        {
            return Err(JwtError::NotYetValid);
        }
        if issued_at > now.saturating_add(self.leeway) {
            return Err(JwtError::NotYetValid);
        }

        if let Some(expected) = &self.issuer
            && issuer.as_deref() != Some(expected.as_str())
        {
            return Err(JwtError::WrongIssuer);
        }
        if let Some(expected) = &self.audience
            && !audience.iter().any(|value| value == expected)
        {
            return Err(JwtError::WrongAudience);
        }

        let mut extra = Object::new();
        for (key, value) in payload {
            if !is_registered(key) {
                extra.insert(key, value.clone());
            }
        }

        Ok(VerifiedClaims {
            subject,
            roles,
            issuer,
            audience,
            issued_at,
            not_before,
            expires_at,
            id,
            extra,
        })
    }

    fn check_header(&self, header: &str) -> Result<(), JwtError> {
        let bytes = base64::decode_url(header).map_err(|_| JwtError::Malformed)?;
        let value = parse_bytes(&bytes).map_err(|_| JwtError::Malformed)?;
        let header = value.as_object().ok_or(JwtError::Malformed)?;

        match header.get("alg").and_then(Value::as_str) {
            Some("HS256") => {}
            Some(_) => return Err(JwtError::UnsupportedAlgorithm),
            None => return Err(JwtError::Malformed),
        }

        if header.contains_key("crit")
            || header.contains_key("enc")
            || header.contains_key("jku")
            || header.contains_key("x5u")
            || header.contains_key("jwk")
        {
            return Err(JwtError::UnsupportedAlgorithm);
        }

        match header.get("typ") {
            None => Ok(()),
            Some(value) => match value.as_str() {
                Some(typ) if typ.eq_ignore_ascii_case("JWT") => Ok(()),
                _ => Err(JwtError::Malformed),
            },
        }
    }

    fn check_signature(
        &self,
        header: &str,
        payload: &str,
        signature: &str,
    ) -> Result<(), JwtError> {
        let signature = base64::decode_url(signature).map_err(|_| JwtError::Malformed)?;
        if signature.len() != 32 {
            return Err(JwtError::InvalidSignature);
        }

        let mut message = Vec::with_capacity(header.len() + payload.len() + 1);
        message.extend_from_slice(header.as_bytes());
        message.push(b'.');
        message.extend_from_slice(payload.as_bytes());

        let expected = hmac::sha256(&self.key, &message);
        if constant_time::eq(&expected, &signature) {
            Ok(())
        } else {
            Err(JwtError::InvalidSignature)
        }
    }

    fn sign(&self, header: &str, payload: &str) -> String {
        let mut message = Vec::with_capacity(header.len() + payload.len() + 1);
        message.extend_from_slice(header.as_bytes());
        message.push(b'.');
        message.extend_from_slice(payload.as_bytes());

        base64::encode_url(&hmac::sha256(&self.key, &message))
    }
}

impl fmt::Debug for JwtCodec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JwtCodec")
            .field("key", &"<redacted>")
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("lifetime", &self.lifetime)
            .field("leeway", &self.leeway)
            .finish()
    }
}

fn is_registered(name: &str) -> bool {
    REGISTERED.contains(&name)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn required_string(payload: &Object, name: &'static str) -> Result<String, JwtError> {
    match payload.get(name) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(_) => Err(JwtError::InvalidClaim(name)),
        None => Err(JwtError::MissingClaim(name)),
    }
}

fn optional_string(payload: &Object, name: &'static str) -> Result<Option<String>, JwtError> {
    match payload.get(name) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(JwtError::InvalidClaim(name)),
    }
}

fn required_timestamp(payload: &Object, name: &'static str) -> Result<u64, JwtError> {
    match payload.get(name) {
        Some(value) => value.as_u64().ok_or(JwtError::InvalidClaim(name)),
        None => Err(JwtError::MissingClaim(name)),
    }
}

fn optional_timestamp(payload: &Object, name: &'static str) -> Result<Option<u64>, JwtError> {
    match payload.get(name) {
        None => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or(JwtError::InvalidClaim(name)),
    }
}

fn claims_roles(payload: &Object) -> Result<Vec<String>, JwtError> {
    match payload.get("roles") {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or(JwtError::InvalidClaim("roles"))
            })
            .collect(),
        Some(_) => Err(JwtError::InvalidClaim("roles")),
    }
}

fn claims_audience(payload: &Object) -> Result<Vec<String>, JwtError> {
    match payload.get("aud") {
        None => Ok(Vec::new()),
        Some(Value::String(value)) => Ok(vec![value.clone()]),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or(JwtError::InvalidClaim("aud"))
            })
            .collect(),
        Some(_) => Err(JwtError::InvalidClaim("aud")),
    }
}

#[cfg(test)]
mod tests {
    use zhs_crypto::{base64, hmac};

    use super::{Claims, JwtCodec, JwtError, MAX_TOKEN_BYTES};

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn codec() -> JwtCodec {
        JwtCodec::new(SECRET).unwrap()
    }

    fn raw(header: &str, payload: &str, secret: &str) -> String {
        let header = base64::encode_url(header.as_bytes());
        let body = base64::encode_url(payload.as_bytes());
        let mut message = header.clone();
        message.push('.');
        message.push_str(&body);
        let signature = base64::encode_url(&hmac::sha256(secret.as_bytes(), message.as_bytes()));
        format!("{header}.{body}.{signature}")
    }

    #[test]
    fn round_trips_claims() {
        let token = codec()
            .issue(
                &Claims::new("ada")
                    .with_roles(["admin", "reader"])
                    .with_issuer("zhs")
                    .with_audience("api")
                    .with_id("token-1")
                    .with_claim("tenant", "acme"),
            )
            .unwrap();

        let claims = codec().verify(&token).unwrap();
        assert_eq!(claims.subject, "ada");
        assert_eq!(claims.roles, ["admin", "reader"]);
        assert_eq!(claims.issuer.as_deref(), Some("zhs"));
        assert_eq!(claims.audience, ["api"]);
        assert_eq!(claims.id.as_deref(), Some("token-1"));
        assert_eq!(
            claims.extra.get("tenant").and_then(|v| v.as_str()),
            Some("acme")
        );
        assert_eq!(claims.expires_at, claims.issued_at + 15 * 60);
    }

    #[test]
    fn codec_fills_missing_issuer_and_audience() {
        // 校验端配了 with_issuer/with_audience,签发端不显式设置:
        // codec 自动补齐,自己签的令牌自己能验过。
        let codec = JwtCodec::new(SECRET)
            .unwrap()
            .with_issuer("zhs")
            .with_audience("api");

        let token = codec.issue(&Claims::new("ada")).unwrap();
        let claims = codec.verify(&token).unwrap();

        assert_eq!(claims.issuer.as_deref(), Some("zhs"));
        assert_eq!(claims.audience, ["api"]);
    }

    #[test]
    fn claims_issuer_wins_over_codec_default() {
        // claims 显式指定的 iss/aud 优先于 codec 的默认值
        let codec = JwtCodec::new(SECRET).unwrap().with_issuer("zhs");

        let token = codec
            .issue(&Claims::new("ada").with_issuer("other"))
            .unwrap();

        assert_eq!(codec.verify(&token), Err(JwtError::WrongIssuer));
    }

    #[test]
    fn lifetime_getter_returns_configuration() {
        assert_eq!(codec().lifetime(), 15 * 60);
        assert_eq!(codec().with_lifetime(3600).lifetime(), 3600);
    }

    #[test]
    fn accepts_tokens_issued_elsewhere() {
        let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJib2IiLCJleHAiOjE3MDAwMDM2MDAsImlhdCI6MTcwMDAwMDAwMCwicm9sZXMiOlsicmVhZGVyIiwid3JpdGVyIl19.yH8IdS9mo9_7Ci8zJcYVct_h9lTzkd8nsv00UjW5w3M";

        let claims = codec().verify_at(token, 1_700_000_100).unwrap();
        assert_eq!(claims.subject, "bob");
        assert_eq!(claims.roles, ["reader", "writer"]);
        assert_eq!(claims.issued_at, 1_700_000_000);
        assert_eq!(claims.expires_at, 1_700_003_600);
    }

    #[test]
    fn applies_lifetime_and_leeway() {
        let token = codec().issue_at(&Claims::new("ada"), 1_000).unwrap();
        assert_eq!(codec().verify_at(&token, 1_900).unwrap().subject, "ada");

        assert!(codec().verify_at(&token, 1_960).is_ok());
        assert_eq!(codec().verify_at(&token, 1_961), Err(JwtError::Expired));

        let strict = codec().with_leeway(0);
        assert!(strict.verify_at(&token, 1_900).is_ok());
        assert_eq!(strict.verify_at(&token, 1_901), Err(JwtError::Expired));
    }

    #[test]
    fn rejects_tampered_tokens() {
        let token = codec().issue(&Claims::new("ada")).unwrap();
        let mut parts: Vec<&str> = token.split('.').collect();

        let payload = parts[1].to_owned();
        let mut tampered = payload[..payload.len() - 1].to_owned();
        tampered.push(if payload.ends_with('A') { 'B' } else { 'A' });
        parts[1] = &tampered;
        let tampered = parts.join(".");
        assert_eq!(codec().verify(&tampered), Err(JwtError::InvalidSignature));

        let mut signature = parts[2].to_owned();
        signature.push('A');
        let tampered = format!("{}.{}.{}", parts[0], parts[1], signature);
        assert_eq!(codec().verify(&tampered), Err(JwtError::InvalidSignature));

        let other = JwtCodec::new("ffffffffffffffffffffffffffffffff").unwrap();
        assert_eq!(other.verify(&token), Err(JwtError::InvalidSignature));
    }

    #[test]
    fn rejects_unsupported_algorithms_and_extensions() {
        let payload = r#"{"sub":"ada","iat":1000,"exp":2000}"#;

        for header in [
            r#"{"alg":"none"}"#,
            r#"{"alg":"HS512","typ":"JWT"}"#,
            r#"{"alg":"RS256","typ":"JWT"}"#,
            r#"{"alg":"HS256","typ":"JWT","crit":["exp"]}"#,
            r#"{"alg":"HS256","typ":"JWT","jku":"https://evil.example/keys"}"#,
        ] {
            let token = raw(header, payload, SECRET);
            assert_eq!(
                codec().verify_at(&token, 1_500),
                Err(JwtError::UnsupportedAlgorithm),
                "header {header} must be rejected"
            );
        }

        let token = raw(r#"{"alg":"HS256","typ":"JOSE"}"#, payload, SECRET);
        assert_eq!(codec().verify_at(&token, 1_500), Err(JwtError::Malformed));

        let token = raw(r#"{"typ":"JWT"}"#, payload, SECRET);
        assert_eq!(codec().verify_at(&token, 1_500), Err(JwtError::Malformed));
    }

    #[test]
    fn rejects_broken_structure() {
        for token in ["", "abc", "a.b", "a.b.c.d", "..", "a..b"] {
            assert_eq!(codec().verify_at(token, 1_500), Err(JwtError::Malformed));
        }

        let oversized = "a".repeat(MAX_TOKEN_BYTES + 1);
        assert_eq!(
            codec().verify_at(&oversized, 1_500),
            Err(JwtError::TooLarge)
        );
    }

    #[test]
    fn rejects_tokens_without_required_claims() {
        let tokens = [
            (r#"{"iat":1000,"exp":2000}"#, JwtError::MissingClaim("sub")),
            (
                r#"{"sub":7,"iat":1000,"exp":2000}"#,
                JwtError::InvalidClaim("sub"),
            ),
            (r#"{"sub":"ada","iat":1000}"#, JwtError::MissingClaim("exp")),
            (
                r#"{"sub":"ada","iat":1000,"exp":"2000"}"#,
                JwtError::InvalidClaim("exp"),
            ),
            (
                r#"{"sub":"ada","iat":1000,"exp":2000,"roles":"admin"}"#,
                JwtError::InvalidClaim("roles"),
            ),
            (
                r#"{"sub":"ada","iat":1000,"exp":2000,"roles":[1]}"#,
                JwtError::InvalidClaim("roles"),
            ),
        ];

        for (payload, expected) in tokens {
            let token = raw(r#"{"alg":"HS256","typ":"JWT"}"#, payload, SECRET);
            assert_eq!(codec().verify_at(&token, 1_500), Err(expected));
        }
    }

    #[test]
    fn enforces_issuer_and_audience() {
        let issuer = codec().with_issuer("zhs");
        let token = codec()
            .issue_at(&Claims::new("ada").with_issuer("zhs"), 1_000)
            .unwrap();
        assert!(issuer.verify_at(&token, 1_500).is_ok());

        let token = codec().issue_at(&Claims::new("ada"), 1_000).unwrap();
        assert_eq!(issuer.verify_at(&token, 1_500), Err(JwtError::WrongIssuer));

        let token = codec()
            .issue_at(&Claims::new("ada").with_issuer("other"), 1_000)
            .unwrap();
        assert_eq!(issuer.verify_at(&token, 1_500), Err(JwtError::WrongIssuer));

        let audience = codec().with_audience("api");
        let token = codec()
            .issue_at(&Claims::new("ada").with_audience("web"), 1_000)
            .unwrap();
        assert_eq!(
            audience.verify_at(&token, 1_500),
            Err(JwtError::WrongAudience)
        );

        let token = raw(
            r#"{"alg":"HS256","typ":"JWT"}"#,
            r#"{"sub":"ada","iat":1000,"exp":2000,"aud":["web","api"]}"#,
            SECRET,
        );
        assert_eq!(
            audience.verify_at(&token, 1_500).unwrap().audience,
            ["web", "api"]
        );
    }

    #[test]
    fn enforces_not_before_and_issued_at() {
        let token = codec()
            .issue_at(
                &Claims::new("ada")
                    .with_not_before(2_000)
                    .with_expires_at(3_000),
                1_000,
            )
            .unwrap();
        assert_eq!(codec().verify_at(&token, 1_900), Err(JwtError::NotYetValid));
        assert!(codec().verify_at(&token, 1_940).is_ok());

        let token = codec()
            .issue_at(
                &Claims::new("ada")
                    .with_issued_at(2_000)
                    .with_expires_at(3_000),
                1_000,
            )
            .unwrap();
        assert_eq!(codec().verify_at(&token, 1_000), Err(JwtError::NotYetValid));
    }

    #[test]
    fn registered_claims_cannot_be_overridden_by_extra() {
        let token = codec()
            .issue_at(
                &Claims::new("ada")
                    .with_claim("exp", 1_u64)
                    .with_claim("sub", "eve"),
                1_000,
            )
            .unwrap();

        let claims = codec().verify_at(&token, 1_500).unwrap();
        assert_eq!(claims.subject, "ada");
        assert_eq!(claims.expires_at, 1_900);
    }

    #[test]
    fn issue_validates_input() {
        assert_eq!(
            JwtCodec::new("too-short").unwrap_err(),
            JwtError::SecretTooShort
        );

        assert_eq!(
            codec().issue(&Claims::default()),
            Err(JwtError::MissingClaim("sub"))
        );

        assert_eq!(
            codec().issue(
                &Claims::new("ada")
                    .with_issued_at(2_000)
                    .with_expires_at(2_000)
            ),
            Err(JwtError::InvalidClaim("exp"))
        );

        assert_eq!(
            codec()
                .with_lifetime(0)
                .issue(&Claims::new("ada").with_issued_at(1_000)),
            Err(JwtError::MissingClaim("exp"))
        );

        assert_eq!(
            codec().issue(
                &Claims::new("ada")
                    .with_issued_at(1_000)
                    .with_not_before(3_000)
                    .with_expires_at(2_000)
            ),
            Err(JwtError::InvalidClaim("nbf"))
        );
    }
}
