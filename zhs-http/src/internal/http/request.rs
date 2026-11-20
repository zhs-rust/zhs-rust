use std::net::SocketAddr;

use crate::internal::error::{Error, HttpError, Result};
use zhs_json::FromJson;

use super::body::RequestBody;
use super::fields::{FORM_CONTENT_TYPE, Fields, is_form_content_type};
use super::headers::{HeaderMap, has_token};
use super::method::Method;
use super::multipart::Multipart;
use super::version::Version;

#[derive(Debug)]
pub struct Request {
    method: Method,
    target: String,
    path: String,
    query: String,
    version: Version,
    headers: HeaderMap,
    body: RequestBody,
    peer_addr: Option<SocketAddr>,
    local_addr: Option<SocketAddr>,
}

impl Request {
    pub(crate) fn new(
        method: Method,
        target: String,
        path: String,
        query: String,
        version: Version,
        headers: HeaderMap,
        body: RequestBody,
    ) -> Self {
        Self {
            method,
            target,
            path,
            query,
            version,
            headers,
            body,
            peer_addr: None,
            local_addr: None,
        }
    }

    pub fn method(&self) -> &Method {
        &self.method
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn query_params(&self) -> Result<Fields> {
        Fields::parse_query(&self.query)
    }

    pub fn version(&self) -> Version {
        self.version
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    pub fn body_mut(&mut self) -> &mut RequestBody {
        &mut self.body
    }

    pub fn into_body(self) -> RequestBody {
        self.body
    }

    pub async fn body(&mut self) -> Result<Vec<u8>> {
        self.body.bytes().await
    }

    pub async fn text(&mut self) -> Result<String> {
        let bytes = self.body().await?;
        String::from_utf8(bytes).map_err(|_| Error::Http(HttpError::InvalidBodyEncoding))
    }

    pub async fn json<T: FromJson>(&mut self) -> Result<T> {
        let bytes = self.body().await?;
        zhs_json::from_slice(&bytes).map_err(crate::internal::error::Error::from)
    }

    pub async fn form(&mut self) -> Result<Fields> {
        let content_type = self
            .headers
            .get("content-type")
            .unwrap_or_default()
            .to_owned();

        if !is_form_content_type(&content_type) {
            return Err(Error::Http(HttpError::UnsupportedContentType {
                actual: content_type,
                expected: FORM_CONTENT_TYPE,
            }));
        }

        let text = self.text().await?;
        Fields::parse_form(&text)
    }

    pub fn multipart(&mut self) -> Result<Multipart<'_>> {
        let content_type = self
            .headers
            .get("content-type")
            .unwrap_or_default()
            .to_owned();

        Multipart::new(self.body_mut(), &content_type)
    }

    pub fn peer_addr(&self) -> Option<SocketAddr> {
        self.peer_addr
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }

    pub fn set_peer_addr(&mut self, peer: SocketAddr) {
        self.peer_addr = Some(peer);
    }

    pub fn set_connection_info(&mut self, peer: SocketAddr, local: Option<SocketAddr>) {
        self.peer_addr = Some(peer);
        self.local_addr = local;
    }

    pub fn connection_close(&self) -> bool {
        has_token(self.headers.get_all("connection"), "close")
    }

    pub fn connection_keep_alive(&self) -> bool {
        has_token(self.headers.get_all("connection"), "keep-alive")
    }

    pub fn expects_continue(&self) -> bool {
        self.headers
            .get("expect")
            .map(|value| value.trim().eq_ignore_ascii_case("100-continue"))
            .unwrap_or(false)
    }
}
