use std::net::Ipv6Addr;

use crate::internal::error::ClientError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Url {
    scheme: Scheme,
    host: String,
    port: u16,
    path: String,
    query: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scheme {
    Http,
    Https,
}

impl Scheme {
    fn parse(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("http") {
            Some(Scheme::Http)
        } else if value.eq_ignore_ascii_case("https") {
            Some(Scheme::Https)
        } else {
            None
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
        }
    }

    fn default_port(self) -> u16 {
        match self {
            Scheme::Http => 80,
            Scheme::Https => 443,
        }
    }
}

impl Url {
    pub(crate) fn parse(input: &str) -> Result<Self, ClientError> {
        let input = input.trim();
        let Some((scheme, rest)) = input.split_once("://") else {
            return Err(invalid_url(
                input,
                "the URL must start with http:// or https://",
            ));
        };

        let Some(scheme) = Scheme::parse(scheme) else {
            return Err(ClientError::UnsupportedScheme {
                scheme: scheme.to_ascii_lowercase(),
            });
        };

        let rest = rest.split_once('#').map_or(rest, |(before, _)| before);
        let (authority, target) = split_authority_and_target(rest);
        let (host, port) = parse_authority(authority, scheme, input)?;
        let (path, query) = parse_target(target, input)?;

        Ok(Self {
            scheme,
            host,
            port,
            path,
            query,
        })
    }

    pub(crate) fn scheme(&self) -> Scheme {
        self.scheme
    }

    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    pub(crate) fn origin_key(&self) -> String {
        format!("{}://{}:{}", self.scheme.as_str(), self.host, self.port)
    }

    pub(crate) fn host_header(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };

        if self.port == self.scheme.default_port() {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }

    pub(crate) fn request_target(&self) -> String {
        if self.query.is_empty() {
            self.path.clone()
        } else {
            format!("{}?{}", self.path, self.query)
        }
    }

    pub(crate) fn as_string(&self) -> String {
        format!(
            "{}://{}{}",
            self.scheme.as_str(),
            self.host_header(),
            self.request_target()
        )
    }

    pub(crate) fn has_query(&self) -> bool {
        !self.query.is_empty()
    }

    pub(crate) fn append_query(&mut self, encoded_name: &str, encoded_value: &str) {
        if !self.query.is_empty() {
            self.query.push('&');
        }
        self.query.push_str(encoded_name);
        self.query.push('=');
        self.query.push_str(encoded_value);
    }

    pub(crate) fn join_request(&self, input: &str) -> Result<Self, ClientError> {
        let input = input.trim();
        let input = input.split_once('#').map_or(input, |(before, _)| before);
        if input.is_empty() {
            return Ok(self.clone());
        }

        if has_scheme(input) {
            return Self::parse(input);
        }
        if let Some(rest) = input.strip_prefix("//") {
            return Self::parse(&format!("{}://{rest}", self.scheme.as_str()));
        }
        if let Some(query) = input.strip_prefix('?') {
            return self.with_query(query);
        }
        if input.starts_with('/') {
            return self.with_target(input);
        }

        let prefix = if self.path.ends_with('/') {
            self.path.clone()
        } else {
            format!("{}/", self.path)
        };
        self.with_target(&format!("{prefix}{input}"))
    }

    pub(crate) fn resolve_redirect(&self, location: &str) -> Result<Self, ClientError> {
        let location = location.trim();
        let location = location
            .split_once('#')
            .map_or(location, |(before, _)| before);

        if location.is_empty() {
            return Ok(self.clone());
        }
        if has_scheme(location) {
            return Self::parse(location);
        }
        if let Some(rest) = location.strip_prefix("//") {
            return Self::parse(&format!("{}://{rest}", self.scheme.as_str()));
        }
        if location.starts_with('/') {
            return self.with_target(location);
        }
        if let Some(query) = location.strip_prefix('?') {
            return self.with_query(query);
        }

        let directory = self
            .path
            .rfind('/')
            .map_or("/", |index| &self.path[..=index]);
        self.with_target(&format!("{directory}{location}"))
    }

    fn with_query(&self, query: &str) -> Result<Self, ClientError> {
        validate_query(query, query)?;
        Ok(Self {
            scheme: self.scheme,
            host: self.host.clone(),
            port: self.port,
            path: self.path.clone(),
            query: query.to_owned(),
        })
    }

    fn with_target(&self, target: &str) -> Result<Self, ClientError> {
        let (path, query) = parse_target(target, target)?;
        let path = normalize_path(&path);
        Ok(Self {
            scheme: self.scheme,
            host: self.host.clone(),
            port: self.port,
            path,
            query,
        })
    }
}

fn has_scheme(input: &str) -> bool {
    let Some((scheme, _)) = input.split_once("://") else {
        return false;
    };

    let mut bytes = scheme.bytes();
    matches!(bytes.next(), Some(byte) if byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

fn split_authority_and_target(rest: &str) -> (&str, &str) {
    match rest.find(['/', '?']) {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    }
}

fn parse_authority(
    authority: &str,
    scheme: Scheme,
    original: &str,
) -> Result<(String, u16), ClientError> {
    if authority.is_empty() {
        return Err(invalid_url(original, "the URL has no host"));
    }
    if authority.contains('@') {
        return Err(invalid_url(
            original,
            "userinfo in URLs is not supported; use basic_auth()",
        ));
    }

    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let Some(end) = rest.find(']') else {
            return Err(invalid_url(original, "unterminated IPv6 address"));
        };
        let host = &rest[..end];
        let suffix = &rest[end + 1..];
        host.parse::<Ipv6Addr>()
            .map_err(|_| invalid_url(original, "invalid IPv6 address"))?;

        let port = if suffix.is_empty() {
            None
        } else if let Some(port) = suffix.strip_prefix(':') {
            Some(port)
        } else {
            return Err(invalid_url(original, "unexpected text after IPv6 address"));
        };
        (host.to_owned(), port)
    } else {
        if authority.contains(['[', ']']) {
            return Err(invalid_url(original, "invalid host"));
        }
        match authority.split_once(':') {
            Some((host, port)) => (host.to_owned(), Some(port)),
            None => (authority.to_owned(), None),
        }
    };

    if host.is_empty() {
        return Err(invalid_url(original, "the URL has no host"));
    }
    if !host
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b':'))
    {
        return Err(invalid_url(
            original,
            "the host must be an ASCII hostname or IP address",
        ));
    }

    let host = host.to_ascii_lowercase();
    let port = match port {
        Some(port) => parse_port(port, original)?,
        None => scheme.default_port(),
    };

    Ok((host, port))
}

fn parse_port(value: &str, original: &str) -> Result<u16, ClientError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_url(original, "the port must be a decimal number"));
    }

    let port = value
        .parse::<u16>()
        .map_err(|_| invalid_url(original, "the port is out of range"))?;
    if port == 0 {
        return Err(invalid_url(original, "the port must be non-zero"));
    }
    Ok(port)
}

fn parse_target(target: &str, original: &str) -> Result<(String, String), ClientError> {
    if target.is_empty() {
        return Ok(("/".to_owned(), String::new()));
    }

    let (path, query) = if let Some(query) = target.strip_prefix('?') {
        ("/".to_owned(), query.to_owned())
    } else {
        match target.split_once('?') {
            Some((path, query)) => (path.to_owned(), query.to_owned()),
            None => (target.to_owned(), String::new()),
        }
    };

    if !path.starts_with('/') {
        return Err(invalid_url(original, "the path must start with '/'"));
    }
    if path.len() + query.len() > MAX_TARGET_BYTES {
        return Err(invalid_url(original, "the request target is too long"));
    }
    if path
        .bytes()
        .chain(query.bytes())
        .any(|byte| !(0x21..=0x7e).contains(&byte))
    {
        return Err(invalid_url(
            original,
            "the path and query must be printable ASCII and must already be percent-encoded",
        ));
    }

    Ok((path, query))
}

const MAX_TARGET_BYTES: usize = 8 * 1024;

fn validate_query(query: &str, original: &str) -> Result<(), ClientError> {
    if query.len() > MAX_TARGET_BYTES {
        return Err(invalid_url(original, "the query string is too long"));
    }
    if query.bytes().any(|byte| !(0x21..=0x7e).contains(&byte)) {
        return Err(invalid_url(
            original,
            "the query string must be printable ASCII and must already be percent-encoded",
        ));
    }
    Ok(())
}

fn normalize_path(path: &str) -> String {
    let trailing_slash = path.ends_with('/');
    let mut segments: Vec<&str> = Vec::new();

    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            segment => segments.push(segment),
        }
    }

    let mut normalized = String::from("/");
    normalized.push_str(&segments.join("/"));
    if trailing_slash && !normalized.ends_with('/') {
        normalized.push('/');
    }
    normalized
}

fn invalid_url(url: &str, reason: impl Into<String>) -> ClientError {
    let mut trimmed = url.to_owned();
    if trimmed.len() > 256 {
        let end = (0..=256)
            .rev()
            .find(|&index| trimmed.is_char_boundary(index))
            .unwrap_or(0);
        trimmed.truncate(end);
        trimmed.push('…');
    }

    ClientError::InvalidUrl {
        url: trimmed,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(url: &str) -> Url {
        Url::parse(url).expect("the URL should parse")
    }

    #[test]
    fn parses_absolute_urls_and_defaults() {
        let url = parse("HTTP://Example.COM");
        assert_eq!(url.scheme(), Scheme::Http);
        assert_eq!(url.host(), "example.com");
        assert_eq!(url.port(), 80);
        assert_eq!(url.request_target(), "/");
        assert_eq!(url.host_header(), "example.com");

        let url = parse("https://api.example.com:8443/v1/users?page=2#fragment");
        assert_eq!(url.scheme(), Scheme::Https);
        assert_eq!(url.port(), 8443);
        assert_eq!(url.request_target(), "/v1/users?page=2");
        assert_eq!(url.host_header(), "api.example.com:8443");
        assert_eq!(
            url.as_string(),
            "https://api.example.com:8443/v1/users?page=2"
        );
    }

    #[test]
    fn parses_ipv6_literals() {
        let url = parse("http://[::1]:8080/ping");
        assert_eq!(url.host(), "::1");
        assert_eq!(url.host_header(), "[::1]:8080");

        let url = parse("http://[::1]/");
        assert_eq!(url.host_header(), "[::1]");
    }

    #[test]
    fn rejects_bad_urls() {
        assert!(matches!(
            Url::parse("example.com"),
            Err(ClientError::InvalidUrl { .. })
        ));
        assert!(matches!(
            Url::parse("ftp://example.com"),
            Err(ClientError::UnsupportedScheme { .. })
        ));
        assert!(matches!(
            Url::parse("http://user:pass@example.com"),
            Err(ClientError::InvalidUrl { .. })
        ));
        assert!(matches!(
            Url::parse("http://example.com:0/"),
            Err(ClientError::InvalidUrl { .. })
        ));
        assert!(matches!(
            Url::parse("http://example.com/a b"),
            Err(ClientError::InvalidUrl { .. })
        ));
        assert!(matches!(
            Url::parse("http://[::1/a"),
            Err(ClientError::InvalidUrl { .. })
        ));
    }

    #[test]
    fn truncates_long_urls_on_character_boundaries() {
        let url = format!("http://例子.test/{}", "中".repeat(300));
        let error = Url::parse(&url).expect_err("non-ASCII hosts are rejected");
        let ClientError::InvalidUrl { url, .. } = error else {
            panic!("unexpected error");
        };
        assert!(url.ends_with('…'));
        assert!(url.len() <= 256 + '…'.len_utf8());
    }

    #[test]
    fn joins_request_paths_to_a_base() {
        let base = parse("http://api.example.com/v1");

        assert_eq!(
            base.join_request("weather").expect("join").as_string(),
            "http://api.example.com/v1/weather"
        );
        assert_eq!(
            base.join_request("/health").expect("join").as_string(),
            "http://api.example.com/health"
        );
        assert_eq!(
            base.join_request("?page=2").expect("join").as_string(),
            "http://api.example.com/v1?page=2"
        );
        assert_eq!(
            base.join_request("http://other.example.com/x")
                .expect("absolute URL")
                .as_string(),
            "http://other.example.com/x"
        );
    }

    #[test]
    fn resolves_redirects_relative_to_the_current_path() {
        let base = parse("http://api.example.com/v1/users/42?expand=1");

        assert_eq!(
            base.resolve_redirect("../orders")
                .expect("resolve")
                .as_string(),
            "http://api.example.com/v1/orders"
        );
        assert_eq!(
            base.resolve_redirect("/login")
                .expect("resolve")
                .as_string(),
            "http://api.example.com/login"
        );
        assert_eq!(
            base.resolve_redirect("?page=2")
                .expect("resolve")
                .as_string(),
            "http://api.example.com/v1/users/42?page=2"
        );
        assert_eq!(
            base.resolve_redirect("//cdn.example.com/asset")
                .expect("resolve")
                .as_string(),
            "http://cdn.example.com/asset"
        );
    }

    #[test]
    fn appends_encoded_query_pairs() {
        let mut url = parse("http://example.com/search?page=1");
        url.append_query("q", "hello%20world");
        assert_eq!(url.request_target(), "/search?page=1&q=hello%20world");
    }
}
