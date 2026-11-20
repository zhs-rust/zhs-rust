use std::io;
use std::time::SystemTime;

use super::body::ResponseBody;
use super::date::format_http_date;
use super::headers::{is_token_byte, is_valid_value_byte};
use super::response::Response;
use super::status::StatusCode;
use super::version::Version;

const MANAGED_HEADERS: [&str; 3] = ["content-length", "transfer-encoding", "connection"];

pub(crate) struct EncodedHead {
    pub(crate) bytes: Vec<u8>,

    pub(crate) chunked: bool,

    pub(crate) content_length: Option<u64>,
}

pub(crate) fn encode_response(
    response: &Response,
    version: Version,
    keep_alive: bool,
    head_only: bool,
) -> io::Result<EncodedHead> {
    let status = response.status();
    let body_allowed = !status.forbids_body();
    let streaming = response.body().is_stream();
    let declared_length = response.content_length();
    let chunked =
        streaming && body_allowed && version == Version::Http11 && declared_length.is_none();
    let reason = status.reason_phrase();
    let mut head = Vec::with_capacity(256);

    if reason.is_empty() {
        head.extend_from_slice(version.as_str().as_bytes());
        head.push(b' ');
        head.extend_from_slice(status.as_u16().to_string().as_bytes());
        head.extend_from_slice(b"\r\n");
    } else {
        head.extend_from_slice(version.as_str().as_bytes());
        head.push(b' ');
        head.extend_from_slice(status.as_u16().to_string().as_bytes());
        head.push(b' ');
        head.extend_from_slice(reason.as_bytes());
        head.extend_from_slice(b"\r\n");
    }

    let mut has_date = false;
    let mut has_server = false;

    for (name, value) in response.headers().iter() {
        if !name.bytes().all(is_token_byte) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid response header name",
            ));
        }
        if !value.bytes().all(is_valid_value_byte) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid response header value",
            ));
        }
        if MANAGED_HEADERS
            .iter()
            .any(|managed| name.eq_ignore_ascii_case(managed))
        {
            continue;
        }

        has_date |= name.eq_ignore_ascii_case("date");
        has_server |= name.eq_ignore_ascii_case("server");

        head.extend_from_slice(name.as_bytes());
        head.extend_from_slice(b": ");
        head.extend_from_slice(value.as_bytes());
        head.extend_from_slice(b"\r\n");
    }

    if !has_date {
        head.extend_from_slice(b"Date: ");
        head.extend_from_slice(format_http_date(SystemTime::now()).as_bytes());
        head.extend_from_slice(b"\r\n");
    }
    if !has_server {
        head.extend_from_slice(b"Server: zhs\r\n");
    }

    let mut body_length = None;
    if body_allowed {
        match response.body() {
            ResponseBody::Bytes(bytes) => {
                let length = if bytes.is_empty() && head_only {
                    declared_length.unwrap_or(0)
                } else {
                    bytes.len() as u64
                };
                head.extend_from_slice(b"Content-Length: ");
                head.extend_from_slice(length.to_string().as_bytes());
                head.extend_from_slice(b"\r\n");
            }
            ResponseBody::Stream(_) => {
                if let Some(length) = declared_length {
                    head.extend_from_slice(b"Content-Length: ");
                    head.extend_from_slice(length.to_string().as_bytes());
                    head.extend_from_slice(b"\r\n");
                    body_length = Some(length);
                } else if chunked {
                    head.extend_from_slice(b"Transfer-Encoding: chunked\r\n");
                }
            }
        }
    }

    if status == StatusCode::SWITCHING_PROTOCOLS {
        head.extend_from_slice(b"Connection: Upgrade\r\n");
    } else if !keep_alive {
        head.extend_from_slice(b"Connection: close\r\n");
    } else if version == Version::Http10 {
        head.extend_from_slice(b"Connection: keep-alive\r\n");
    }

    head.extend_from_slice(b"\r\n");
    Ok(EncodedHead {
        bytes: head,
        chunked,
        content_length: body_length,
    })
}

pub(crate) fn error_response(status: StatusCode) -> Response {
    Response::new(status)
        .with_header("Content-Type", "text/plain; charset=utf-8")
        .with_body(format!("{} {}\n", status.as_u16(), status.reason_phrase()))
        .with_close()
}
