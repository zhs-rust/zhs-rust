use std::fmt;

use super::headers::is_token_byte;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    Get,

    Head,

    Post,

    Put,

    Delete,

    Connect,

    Options,

    Trace,

    Patch,

    Extension(String),
}

impl Method {
    pub fn parse(value: &str) -> Option<Self> {
        let method = match value {
            "GET" => Method::Get,
            "HEAD" => Method::Head,
            "POST" => Method::Post,
            "PUT" => Method::Put,
            "DELETE" => Method::Delete,
            "CONNECT" => Method::Connect,
            "OPTIONS" => Method::Options,
            "TRACE" => Method::Trace,
            "PATCH" => Method::Patch,
            _ => {
                if value.is_empty() || !value.bytes().all(is_token_byte) {
                    return None;
                }
                Method::Extension(value.to_owned())
            }
        };

        Some(method)
    }

    pub fn as_str(&self) -> &str {
        match self {
            Method::Get => "GET",
            Method::Head => "HEAD",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Connect => "CONNECT",
            Method::Options => "OPTIONS",
            Method::Trace => "TRACE",
            Method::Patch => "PATCH",
            Method::Extension(name) => name,
        }
    }

    pub fn is_head(&self) -> bool {
        matches!(self, Method::Head)
    }

    pub fn is_idempotent(&self) -> bool {
        matches!(
            self,
            Method::Get
                | Method::Head
                | Method::Options
                | Method::Trace
                | Method::Put
                | Method::Delete
        )
    }
}

impl fmt::Display for Method {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
