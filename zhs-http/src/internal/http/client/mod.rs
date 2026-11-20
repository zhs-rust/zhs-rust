mod connection;
mod url;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use zhs_crypto::base64;

use crate::internal::error::{ClientError, Error, Result};
use crate::internal::http::{Fields, HeaderMap, Method, StatusCode, Version};
use zhs_json::{FromJson, ToJson};
use zhs_mn::thread_pool::ThreadPool;

pub use connection::{Connector, TcpConnector, Transport};

use super::fields::encode_component_into;
use super::headers::{is_token_byte, is_valid_value_byte};
use connection::{ConnectionPool, ExchangeOutcome, ResponseLimits, exchange, header_has_token};
use url::Url;

pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);

pub const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(30);

pub const DEFAULT_MAX_REDIRECTS: usize = 5;

pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

pub const DEFAULT_MAX_IDLE_PER_HOST: usize = 8;

pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

pub const DEFAULT_MAX_WORKERS: usize = 32;

pub const DEFAULT_USER_AGENT: &str = concat!("zhs/", env!("CARGO_PKG_VERSION"));

#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    base_url: Option<Url>,
    default_headers: HeaderMap,
    user_agent: String,
    connect_timeout: Duration,
    read_timeout: Duration,
    follow_redirects: bool,
    max_redirects: usize,
    limits: ResponseLimits,
    connection_pool: ConnectionPool,
    connector: Arc<dyn Connector>,
    thread_pool: ThreadPool,
}

impl Client {
    pub fn new() -> Result<Self> {
        Self::builder().build()
    }

    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    pub fn get(&self, url: impl Into<String>) -> ClientRequest {
        self.request(Method::Get, url)
    }

    pub fn post(&self, url: impl Into<String>) -> ClientRequest {
        self.request(Method::Post, url)
    }

    pub fn put(&self, url: impl Into<String>) -> ClientRequest {
        self.request(Method::Put, url)
    }

    pub fn patch(&self, url: impl Into<String>) -> ClientRequest {
        self.request(Method::Patch, url)
    }

    pub fn delete(&self, url: impl Into<String>) -> ClientRequest {
        self.request(Method::Delete, url)
    }

    pub fn head(&self, url: impl Into<String>) -> ClientRequest {
        self.request(Method::Head, url)
    }

    pub fn request(&self, method: Method, url: impl Into<String>) -> ClientRequest {
        ClientRequest {
            client: self.clone(),
            method,
            url: url.into(),
            headers: HeaderMap::new(),
            query: Vec::new(),
            body: Vec::new(),
            body_set: false,
            suppress_sensitive_headers: false,
        }
    }

    pub fn execute_blocking(&self, request: ClientRequest) -> Result<ClientResponse> {
        self.inner.execute(request)
    }
}

impl fmt::Debug for Client {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Client")
            .field(
                "base_url",
                &self.inner.base_url.as_ref().map(Url::as_string),
            )
            .field("user_agent", &self.inner.user_agent)
            .field("follow_redirects", &self.inner.follow_redirects)
            .field("connect_timeout", &self.inner.connect_timeout)
            .field("read_timeout", &self.inner.read_timeout)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct ClientBuilder {
    base_url: Option<String>,
    default_headers: HeaderMap,
    user_agent: String,
    connect_timeout: Duration,
    read_timeout: Duration,
    write_timeout: Duration,
    follow_redirects: bool,
    max_redirects: usize,
    max_response_bytes: usize,
    max_idle_per_host: usize,
    idle_timeout: Duration,
    max_workers: usize,
    connector: Option<Arc<dyn Connector>>,
    thread_pool: Option<ThreadPool>,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            base_url: None,
            default_headers: HeaderMap::new(),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            read_timeout: DEFAULT_READ_TIMEOUT,
            write_timeout: DEFAULT_WRITE_TIMEOUT,
            follow_redirects: true,
            max_redirects: DEFAULT_MAX_REDIRECTS,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            max_idle_per_host: DEFAULT_MAX_IDLE_PER_HOST,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            max_workers: DEFAULT_MAX_WORKERS,
            connector: None,
            thread_pool: None,
        }
    }
}

impl ClientBuilder {
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    pub fn default_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.default_headers.append(name, value);
        self
    }

    pub fn default_headers(mut self, headers: HeaderMap) -> Self {
        for (name, value) in headers.iter() {
            self.default_headers.append(name, value);
        }
        self
    }

    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    pub fn read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = timeout;
        self
    }

    pub fn write_timeout(mut self, timeout: Duration) -> Self {
        self.write_timeout = timeout;
        self
    }

    pub fn follow_redirects(mut self, follow: bool) -> Self {
        self.follow_redirects = follow;
        self
    }

    pub fn max_redirects(mut self, limit: usize) -> Self {
        self.max_redirects = limit;
        self
    }

    pub fn max_response_bytes(mut self, limit: usize) -> Self {
        self.max_response_bytes = limit;
        self
    }

    pub fn max_idle_connections_per_host(mut self, limit: usize) -> Self {
        self.max_idle_per_host = limit;
        self
    }

    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    pub fn max_workers(mut self, workers: usize) -> Self {
        self.max_workers = workers;
        self
    }

    pub fn thread_pool(mut self, pool: ThreadPool) -> Self {
        self.thread_pool = Some(pool);
        self
    }

    pub fn connector(mut self, connector: Arc<dyn Connector>) -> Self {
        self.connector = Some(connector);
        self
    }

    pub fn build(self) -> Result<Client> {
        let base_url = match &self.base_url {
            Some(raw) => {
                let url = Url::parse(raw)?;
                if url.has_query() {
                    return Err(ClientError::InvalidUrl {
                        url: raw.clone(),
                        reason: "base_url must not contain a query string".to_owned(),
                    }
                    .into());
                }
                Some(url)
            }
            None => None,
        };

        for (name, value) in self.default_headers.iter() {
            validate_header(name, value)?;
        }

        let connector = self
            .connector
            .unwrap_or_else(|| Arc::new(TcpConnector::new(self.read_timeout, self.write_timeout)));
        let thread_pool = match self.thread_pool {
            Some(pool) => pool,
            None => ThreadPool::new(self.max_workers)?,
        };

        Ok(Client {
            inner: Arc::new(ClientInner {
                base_url,
                default_headers: self.default_headers,
                user_agent: self.user_agent,
                connect_timeout: self.connect_timeout,
                read_timeout: self.read_timeout,
                follow_redirects: self.follow_redirects && self.max_redirects > 0,
                max_redirects: self.max_redirects,
                limits: ResponseLimits::new(self.max_response_bytes),
                connection_pool: ConnectionPool::new(self.max_idle_per_host, self.idle_timeout),
                connector,
                thread_pool,
            }),
        })
    }
}

impl fmt::Debug for ClientBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientBuilder")
            .field("base_url", &self.base_url)
            .field("user_agent", &self.user_agent)
            .field("connect_timeout", &self.connect_timeout)
            .field("read_timeout", &self.read_timeout)
            .field("follow_redirects", &self.follow_redirects)
            .field("max_redirects", &self.max_redirects)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish_non_exhaustive()
    }
}

pub struct ClientRequest {
    client: Client,
    method: Method,
    url: String,
    headers: HeaderMap,
    query: Vec<(String, String)>,
    body: Vec<u8>,
    body_set: bool,

    suppress_sensitive_headers: bool,
}

impl ClientRequest {
    pub fn method(&self) -> &Method {
        &self.method
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.append(name, value);
        self
    }

    pub fn headers(mut self, headers: HeaderMap) -> Self {
        for (name, value) in headers.iter() {
            self.headers.append(name, value);
        }
        self
    }

    pub fn query(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.query.push((name.into(), value.into()));
        self
    }

    pub fn query_pairs<I, K, V>(mut self, pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (name, value) in pairs {
            self.query.push((name.into(), value.into()));
        }
        self
    }

    pub fn json<T: ToJson + ?Sized>(mut self, value: &T) -> Self {
        self.body = zhs_json::to_string(value).into_bytes();
        self.body_set = true;
        if !self.headers.contains("content-type") {
            self.headers
                .insert("Content-Type", "application/json; charset=utf-8");
        }
        self
    }

    pub fn text(mut self, body: impl Into<String>) -> Self {
        self.body = body.into().into_bytes();
        self.body_set = true;
        if !self.headers.contains("content-type") {
            self.headers
                .insert("Content-Type", "text/plain; charset=utf-8");
        }
        self
    }

    pub fn form(mut self, fields: &Fields) -> Self {
        self.body = fields.to_query_string().into_bytes();
        self.body_set = true;
        if !self.headers.contains("content-type") {
            self.headers.insert(
                "Content-Type",
                "application/x-www-form-urlencoded; charset=utf-8",
            );
        }
        self
    }

    pub fn bytes(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self.body_set = true;
        self
    }

    pub fn basic_auth(self, username: impl AsRef<str>, password: Option<&str>) -> Self {
        let credentials = format!("{}:{}", username.as_ref(), password.unwrap_or_default());
        let encoded = base64::encode(credentials.as_bytes());
        self.header("Authorization", format!("Basic {encoded}"))
    }

    pub fn bearer_auth(self, token: impl AsRef<str>) -> Self {
        self.header("Authorization", format!("Bearer {}", token.as_ref()))
    }

    pub async fn send(self) -> Result<ClientResponse> {
        let client = self.client.clone();
        let pool = client.inner.thread_pool.clone();
        let handle = pool.try_spawn_blocking(move || client.inner.execute(self))?;
        match handle.await {
            Ok(result) => result,
            Err(error) => Err(Error::Join(error)),
        }
    }

    pub fn send_blocking(self) -> Result<ClientResponse> {
        let client = self.client.clone();
        client.inner.execute(self)
    }
}

impl fmt::Debug for ClientRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &self.headers)
            .field("query", &self.query)
            .field("body_len", &self.body.len())
            .finish()
    }
}

pub struct ClientResponse {
    status: StatusCode,
    version: Version,
    headers: HeaderMap,
    body: Vec<u8>,
    url: String,
    redirected: bool,
}

impl ClientResponse {
    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn version(&self) -> Version {
        self.version
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn redirected(&self) -> bool {
        self.redirected
    }

    pub fn bytes(&self) -> &[u8] {
        &self.body
    }

    pub fn len(&self) -> usize {
        self.body.len()
    }

    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }

    pub fn text(&self) -> Result<&str> {
        std::str::from_utf8(&self.body).map_err(|_| ClientError::InvalidBodyEncoding.into())
    }

    pub fn json<T: FromJson>(&self) -> Result<T> {
        zhs_json::from_slice(&self.body).map_err(crate::internal::error::Error::from)
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.body
    }

    pub fn into_text(self) -> Result<String> {
        String::from_utf8(self.body).map_err(|_| ClientError::InvalidBodyEncoding.into())
    }

    pub fn into_json<T: FromJson>(self) -> Result<T> {
        zhs_json::from_slice(&self.body).map_err(crate::internal::error::Error::from)
    }

    pub fn error_for_status(self) -> Result<Self> {
        if self.status.is_client_error() || self.status.is_server_error() {
            return Err(ClientError::Status {
                status: self.status,
                body: self.body,
            }
            .into());
        }
        Ok(self)
    }
}

impl fmt::Debug for ClientResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientResponse")
            .field("status", &self.status)
            .field("version", &self.version)
            .field("headers", &self.headers)
            .field("body_len", &self.body.len())
            .field("url", &self.url)
            .field("redirected", &self.redirected)
            .finish()
    }
}

struct WireRequest {
    bytes: Vec<u8>,
    close: bool,
}

impl ClientInner {
    fn execute(&self, request: ClientRequest) -> Result<ClientResponse> {
        let mut url = self.resolve_url(&request.url)?;

        for (name, value) in &request.query {
            let mut encoded_name = String::new();
            let mut encoded_value = String::new();
            encode_component_into(name, &mut encoded_name);
            encode_component_into(value, &mut encoded_value);
            url.append_query(&encoded_name, &encoded_value);
        }

        let mut request = request;
        let mut redirects = 0;
        let mut redirected = false;

        loop {
            let wire = request.build_wire(self, &url)?;
            let mut response = self.exchange_once(&request, &url, &wire)?;

            if !self.follow_redirects || !is_followable(response.status) {
                response.redirected = redirected;
                return Ok(response);
            }

            let Some(location) = response.header("location").map(str::to_owned) else {
                response.redirected = redirected;
                return Ok(response);
            };

            if redirects >= self.max_redirects {
                return Err(ClientError::TooManyRedirects {
                    limit: self.max_redirects,
                }
                .into());
            }

            let next = url.resolve_redirect(&location)?;
            if next.origin_key() != url.origin_key() {
                request.suppress_sensitive_headers = true;
            }
            request.headers.remove("host");

            apply_redirect_method(&mut request, response.status);

            url = next;
            redirects += 1;
            redirected = true;
        }
    }

    fn resolve_url(&self, raw: &str) -> Result<Url> {
        match &self.base_url {
            Some(base) => Ok(base.join_request(raw)?),
            None => Ok(Url::parse(raw)?),
        }
    }

    fn exchange_once(
        &self,
        request: &ClientRequest,
        url: &Url,
        wire: &WireRequest,
    ) -> Result<ClientResponse> {
        let origin = url.origin_key();

        if let Some(transport) = self.connection_pool.checkout(&origin) {
            match exchange(
                transport,
                &wire.bytes,
                request.method.is_head(),
                wire.close,
                &self.limits,
            ) {
                Ok(outcome) => return Ok(self.finish(outcome, &origin, url)),
                Err(failure) => {
                    if failure.response_started || !request.method.is_idempotent() {
                        return Err(failure.error.into());
                    }
                }
            }
        }

        let transport = self.connector.connect(
            url.scheme().as_str(),
            url.host(),
            url.port(),
            self.connect_timeout,
        )?;
        let outcome = exchange(
            transport,
            &wire.bytes,
            request.method.is_head(),
            wire.close,
            &self.limits,
        )
        .map_err(|failure| failure.error)?;

        Ok(self.finish(outcome, &origin, url))
    }

    fn finish(&self, outcome: ExchangeOutcome, origin: &str, url: &Url) -> ClientResponse {
        if let Some(transport) = outcome.reusable {
            self.connection_pool.checkin(origin.to_owned(), transport);
        }

        ClientResponse {
            status: outcome.status,
            version: outcome.version,
            headers: outcome.headers,
            body: outcome.body,
            url: url.as_string(),
            redirected: false,
        }
    }
}

impl ClientRequest {
    fn build_wire(&self, client: &ClientInner, url: &Url) -> Result<WireRequest> {
        let mut headers = client.default_headers.clone();

        let mut overridden: Vec<String> = Vec::new();
        for (name, value) in self.headers.iter() {
            let lower = name.to_ascii_lowercase();
            if !overridden.contains(&lower) {
                headers.remove(name);
                overridden.push(lower);
            }
            headers.append(name, value);
        }

        if self.suppress_sensitive_headers {
            headers.remove("authorization");
            headers.remove("cookie");
            headers.remove("proxy-authorization");
        }

        headers.remove("host");
        headers.insert("Host", url.host_header());
        headers.remove("content-length");

        if !headers.contains("user-agent") {
            headers.insert("User-Agent", client.user_agent.clone());
        }
        if !headers.contains("accept") {
            headers.insert("Accept", "*/*");
        }
        if headers.contains("transfer-encoding") {
            return Err(ClientError::InvalidHeader {
                name: "Transfer-Encoding".to_owned(),
                reason: "chunked request bodies are not supported".to_owned(),
            }
            .into());
        }

        let expects_body =
            self.body_set || matches!(self.method, Method::Post | Method::Put | Method::Patch);
        if expects_body {
            headers.insert("Content-Length", self.body.len().to_string());
        }

        for (name, value) in headers.iter() {
            validate_header(name, value)?;
        }

        let mut bytes = Vec::with_capacity(256 + self.body.len());
        bytes.extend_from_slice(self.method.as_str().as_bytes());
        bytes.push(b' ');
        bytes.extend_from_slice(url.request_target().as_bytes());
        bytes.extend_from_slice(b" HTTP/1.1\r\n");
        for (name, value) in headers.iter() {
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(b": ");
            bytes.extend_from_slice(value.as_bytes());
            bytes.extend_from_slice(b"\r\n");
        }
        bytes.extend_from_slice(b"\r\n");
        bytes.extend_from_slice(&self.body);

        let close = header_has_token(&headers, "connection", "close");

        Ok(WireRequest { bytes, close })
    }
}

fn is_followable(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

fn apply_redirect_method(request: &mut ClientRequest, status: StatusCode) {
    let switch_to_get = match status.as_u16() {
        303 => !request.method.is_head(),
        301 | 302 => request.method == Method::Post,
        _ => false,
    };

    if switch_to_get {
        request.method = Method::Get;
        request.body.clear();
        request.body_set = false;
        request.headers.remove("content-type");
        request.headers.remove("content-length");
        request.headers.remove("transfer-encoding");
    }
}

fn validate_header(name: &str, value: &str) -> std::result::Result<(), ClientError> {
    if name.is_empty() || !name.bytes().all(is_token_byte) {
        return Err(ClientError::InvalidHeader {
            name: name.to_owned(),
            reason: "the header name must be an HTTP token".to_owned(),
        });
    }

    if value
        .bytes()
        .any(|byte| !is_valid_value_byte(byte) || byte == b'\r' || byte == b'\n')
    {
        return Err(ClientError::InvalidHeader {
            name: name.to_owned(),
            reason: "the header value contains a control character".to_owned(),
        });
    }

    Ok(())
}
