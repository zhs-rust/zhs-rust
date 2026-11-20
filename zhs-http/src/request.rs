use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;

use crate::error::HttpError;
use crate::method::Method;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub path: String,
    pub http_version: String,
    pub query: HashMap<String, Vec<String>>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub peer_addr: Option<SocketAddr>,
}

impl Request {
    pub fn query_value<T>(&self, key: &str) -> Result<Option<T>, HttpError>
    where
        T: FromStr,
    {
        let Some(raw) = self.query.get(key).and_then(|values| values.last()) else {
            return Ok(None);
        };

        raw.parse::<T>().map(Some).map_err(|_| {
            HttpError::BadRequest(format!("query parameter `{key}` has an invalid value"))
        })
    }

    pub fn query_values(&self, key: &str) -> &[String] {
        self.query.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn body_text(&self) -> Result<String, HttpError> {
        String::from_utf8(self.body.clone())
            .map_err(|_| HttpError::BadRequest("request body is not valid UTF-8".to_owned()))
    }

    pub fn peer_addr(&self) -> Option<SocketAddr> {
        self.peer_addr
    }

    pub fn is_keep_alive(&self) -> bool {
        if let Some(connection) = self.header("connection") {
            if connection.eq_ignore_ascii_case("close") {
                return false;
            }
            if connection.eq_ignore_ascii_case("keep-alive") {
                return true;
            }
        }

        self.http_version == "HTTP/1.1"
    }
}
