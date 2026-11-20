use std::fmt;
use std::sync::Arc;

use super::context::Context;
use super::headers::HeaderMap;
use super::pipeline::{Middleware, Next, problem_response};
use super::response::Response;
use super::service::ServiceFuture;
use super::status::StatusCode;
use crate::internal::api_key::ApiKeys;
use crate::internal::jwt::JwtCodec;
use zhs_log::runtime::{LogLevel, LogRecord, Logger};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    name: String,
    roles: Vec<String>,
}

impl Principal {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            roles: Vec::new(),
        }
    }

    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.roles.push(role.into());
        self
    }

    pub fn with_roles<I, S>(mut self, roles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.roles.extend(roles.into_iter().map(Into::into));
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn roles(&self) -> &[String] {
        &self.roles
    }

    pub fn is_in_role(&self, role: &str) -> bool {
        self.roles.iter().any(|existing| existing == role)
    }
}

pub trait Authenticator: Send + Sync {
    fn authenticate(&self, context: &Context) -> Option<Principal>;
}

type Lookup = Arc<dyn Fn(&str) -> Option<Principal> + Send + Sync>;

pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get("authorization")?;
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }

    let token = token.trim();
    if token.is_empty() { None } else { Some(token) }
}

pub fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    for header in headers.get_all("cookie") {
        for pair in header.split(';') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            if key.trim() == name {
                return Some(value.trim());
            }
        }
    }

    None
}

fn lookup_from_tokens<I, S>(tokens: I) -> Lookup
where
    I: IntoIterator<Item = (S, Principal)>,
    S: Into<String>,
{
    let tokens: Vec<(String, Principal)> = tokens
        .into_iter()
        .map(|(token, principal)| (token.into(), principal))
        .collect();

    Arc::new(move |token: &str| {
        tokens
            .iter()
            .find(|(candidate, _)| candidate == token)
            .map(|(_, principal)| principal.clone())
    })
}

pub struct Bearer {
    lookup: Lookup,
}

impl Bearer {
    pub fn new(lookup: impl Fn(&str) -> Option<Principal> + Send + Sync + 'static) -> Self {
        Self {
            lookup: Arc::new(lookup),
        }
    }

    pub fn from_tokens<I, S>(tokens: I) -> Self
    where
        I: IntoIterator<Item = (S, Principal)>,
        S: Into<String>,
    {
        Self {
            lookup: lookup_from_tokens(tokens),
        }
    }
}

impl fmt::Debug for Bearer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Bearer(..)")
    }
}

impl Authenticator for Bearer {
    fn authenticate(&self, context: &Context) -> Option<Principal> {
        let token = bearer_token(context.headers())?;
        (self.lookup)(token)
    }
}

pub struct Cookie {
    name: String,
    lookup: Lookup,
}

impl Cookie {
    pub fn new(
        name: impl Into<String>,
        lookup: impl Fn(&str) -> Option<Principal> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            lookup: Arc::new(lookup),
        }
    }

    pub fn from_sessions<I, S>(name: impl Into<String>, sessions: I) -> Self
    where
        I: IntoIterator<Item = (S, Principal)>,
        S: Into<String>,
    {
        Self {
            name: name.into(),
            lookup: lookup_from_tokens(sessions),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Debug for Cookie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Cookie")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Authenticator for Cookie {
    fn authenticate(&self, context: &Context) -> Option<Principal> {
        let token = cookie_value(context.headers(), &self.name)?;
        (self.lookup)(token)
    }
}

pub struct Jwt {
    codec: JwtCodec,
    cookie: Option<String>,
}

impl Jwt {
    pub fn new(codec: JwtCodec) -> Self {
        Self {
            codec,
            cookie: None,
        }
    }

    pub fn from_cookie(name: impl Into<String>, codec: JwtCodec) -> Self {
        Self {
            codec,
            cookie: Some(name.into()),
        }
    }

    pub fn codec(&self) -> &JwtCodec {
        &self.codec
    }
}

impl fmt::Debug for Jwt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("Jwt");
        debug.field("codec", &self.codec);
        if let Some(cookie) = &self.cookie {
            debug.field("cookie", cookie);
        }
        debug.finish()
    }
}

impl Authenticator for Jwt {
    fn authenticate(&self, context: &Context) -> Option<Principal> {
        let token = match &self.cookie {
            Some(name) => cookie_value(context.headers(), name)?,
            None => bearer_token(context.headers())?,
        };

        let claims = self.codec.verify(token).ok()?;
        Some(Principal::new(claims.subject).with_roles(claims.roles))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ApiKeySource {
    Auto,

    Bearer,

    Header(String),
}

pub struct ApiKey {
    keys: ApiKeys,
    source: ApiKeySource,
}

impl ApiKey {
    pub fn new(keys: ApiKeys) -> Self {
        Self {
            keys,
            source: ApiKeySource::Auto,
        }
    }

    pub fn from_bearer(keys: ApiKeys) -> Self {
        Self {
            keys,
            source: ApiKeySource::Bearer,
        }
    }

    pub fn from_header(name: impl Into<String>, keys: ApiKeys) -> Self {
        Self {
            keys,
            source: ApiKeySource::Header(name.into()),
        }
    }

    pub fn keys(&self) -> &ApiKeys {
        &self.keys
    }

    fn token<'a>(&self, headers: &'a HeaderMap) -> Option<&'a str> {
        match &self.source {
            ApiKeySource::Auto => headers
                .get("x-api-key")
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .or_else(|| bearer_token(headers)),
            ApiKeySource::Bearer => bearer_token(headers),
            ApiKeySource::Header(name) => headers
                .get(name)
                .map(str::trim)
                .filter(|token| !token.is_empty()),
        }
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiKey")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl Authenticator for ApiKey {
    fn authenticate(&self, context: &Context) -> Option<Principal> {
        let token = self.token(context.headers())?;
        if !token.starts_with(crate::internal::api_key::PREFIX) {
            return None;
        }

        let record = self.keys.verify(token).ok()?;
        Some(Principal::new(record.subject).with_roles(record.roles))
    }
}

pub struct Authentication {
    authenticators: Vec<Arc<dyn Authenticator>>,
}

impl Authentication {
    pub fn new(authenticator: impl Authenticator + 'static) -> Self {
        Self {
            authenticators: vec![Arc::new(authenticator)],
        }
    }

    pub fn with_authenticator(mut self, authenticator: impl Authenticator + 'static) -> Self {
        self.authenticators.push(Arc::new(authenticator));
        self
    }

    pub fn len(&self) -> usize {
        self.authenticators.len()
    }

    pub fn is_empty(&self) -> bool {
        self.authenticators.is_empty()
    }
}

impl fmt::Debug for Authentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Authentication")
            .field("authenticators", &self.authenticators.len())
            .finish()
    }
}

impl Middleware for Authentication {
    fn call(&self, mut context: Context, next: Next) -> ServiceFuture {
        let principal = self
            .authenticators
            .iter()
            .find_map(|authenticator| authenticator.authenticate(&context));

        if let Some(principal) = principal {
            context.insert(principal);
        }

        next.run(context)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Requirement {
    Anonymous,

    Authenticated,

    Roles(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct Authorization {
    default: Requirement,
    rules: Vec<(String, Requirement)>,
    challenge: String,
    logger: Option<Logger>,
}

impl Authorization {
    pub fn authenticated() -> Self {
        Self {
            default: Requirement::Authenticated,
            rules: Vec::new(),
            challenge: "Bearer".to_owned(),
            logger: None,
        }
    }

    pub fn new() -> Self {
        Self::authenticated()
    }

    pub fn allow_anonymous(self, prefix: impl AsRef<str>) -> Self {
        self.rule(prefix.as_ref(), Requirement::Anonymous)
    }

    pub fn require_authentication(self, prefix: impl AsRef<str>) -> Self {
        self.rule(prefix.as_ref(), Requirement::Authenticated)
    }

    pub fn require_roles<I, S>(self, prefix: impl AsRef<str>, roles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.rule(
            prefix.as_ref(),
            Requirement::Roles(roles.into_iter().map(Into::into).collect()),
        )
    }

    pub fn with_challenge(mut self, challenge: impl Into<String>) -> Self {
        self.challenge = challenge.into();
        self
    }

    pub fn with_logger(mut self, logger: Logger) -> Self {
        self.logger = Some(logger);
        self
    }

    fn rule(mut self, prefix: &str, requirement: Requirement) -> Self {
        assert!(
            prefix.starts_with('/'),
            "authorization prefix must start with '/': {prefix}"
        );

        let prefix = normalise_prefix(prefix);
        match self
            .rules
            .iter_mut()
            .find(|(existing, _)| *existing == prefix)
        {
            Some(rule) => rule.1 = requirement,
            None => self.rules.push((prefix, requirement)),
        }
        self
    }

    fn requirement_for(&self, path: &str) -> &Requirement {
        let mut matched: Option<&(String, Requirement)> = None;

        for rule in &self.rules {
            if !path_matches(path, &rule.0) {
                continue;
            }

            let longer = matched
                .map(|(prefix, _)| rule.0.len() > prefix.len())
                .unwrap_or(true);
            if longer {
                matched = Some(rule);
            }
        }

        matched
            .map(|(_, requirement)| requirement)
            .unwrap_or(&self.default)
    }
}

impl Default for Authorization {
    fn default() -> Self {
        Self::authenticated()
    }
}

impl Middleware for Authorization {
    fn call(&self, context: Context, next: Next) -> ServiceFuture {
        let requirement = self.requirement_for(context.path()).clone();

        match requirement {
            Requirement::Anonymous => next.run(context),
            Requirement::Authenticated => match context.get::<Principal>() {
                Some(_) => next.run(context),
                None => {
                    let response = self.deny(
                        &context,
                        StatusCode::UNAUTHORIZED,
                        "authentication is required",
                        None,
                        None,
                    );
                    Box::pin(async move { Ok(response) })
                }
            },
            Requirement::Roles(roles) => {
                let Some(principal) = context.get::<Principal>() else {
                    let response = self.deny(
                        &context,
                        StatusCode::UNAUTHORIZED,
                        "authentication is required",
                        Some(&roles),
                        None,
                    );
                    return Box::pin(async move { Ok(response) });
                };

                if roles.iter().any(|role| principal.is_in_role(role)) {
                    next.run(context)
                } else {
                    let response = self.deny(
                        &context,
                        StatusCode::FORBIDDEN,
                        "the authenticated user lacks a required role",
                        Some(&roles),
                        Some(principal.name()),
                    );
                    Box::pin(async move { Ok(response) })
                }
            }
        }
    }
}

impl Authorization {
    fn deny(
        &self,
        context: &Context,
        status: StatusCode,
        detail: &str,
        required: Option<&[String]>,
        subject: Option<&str>,
    ) -> Response {
        self.audit(context, status, detail, required, subject);

        let response = problem_response(status, Some(detail));
        if status == StatusCode::UNAUTHORIZED && !self.challenge.is_empty() {
            response.with_header("WWW-Authenticate", self.challenge.clone())
        } else {
            response
        }
    }

    fn audit(
        &self,
        context: &Context,
        status: StatusCode,
        detail: &str,
        required: Option<&[String]>,
        subject: Option<&str>,
    ) {
        let logger = match &self.logger {
            Some(logger) => logger,
            None => match zhs_log::runtime::global() {
                Some(logger) => logger,
                None => return,
            },
        };
        if !logger.enabled(LogLevel::Warn) {
            return;
        }

        let mut record = LogRecord::new(
            LogLevel::Warn,
            "zhs_framework::http",
            "authorization denied",
        )
        .with_field("event", "auth_denied")
        .with_field("status", status.as_u16().to_string())
        .with_field("path", context.path().to_owned())
        .with_field(
            "peer",
            context
                .peer_addr()
                .map(|peer| peer.to_string())
                .unwrap_or_else(|| "unknown".to_owned()),
        )
        .with_field("detail", detail.to_owned());

        if let Some(subject) = subject {
            record = record.with_field("subject", subject.to_owned());
        }
        if let Some(required) = required {
            record = record.with_field("required_roles", required.join(", "));
        }

        let _ = logger.log(record);
    }
}

pub(crate) fn normalise_prefix(prefix: &str) -> String {
    if prefix.len() > 1 {
        prefix.trim_end_matches('/').to_owned()
    } else {
        prefix.to_owned()
    }
}

pub(crate) fn path_matches(path: &str, prefix: &str) -> bool {
    if prefix == "/" {
        return true;
    }

    match path.strip_prefix(prefix) {
        Some(rest) => rest.is_empty() || rest.starts_with('/'),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{normalise_prefix, path_matches};

    #[test]
    fn prefixes_match_whole_segments() {
        assert!(path_matches("/admin", "/admin"));
        assert!(path_matches("/admin/users", "/admin"));
        assert!(!path_matches("/administrator", "/admin"));
        assert!(!path_matches("/", "/admin"));
        assert!(path_matches("/anything", "/"));
    }

    #[test]
    fn trailing_slashes_are_normalised() {
        assert_eq!(normalise_prefix("/admin/"), "/admin");
        assert_eq!(normalise_prefix("/"), "/");
        assert_eq!(normalise_prefix("/admin//"), "/admin");
    }
}
