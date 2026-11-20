use std::io::{self, BufRead};

use crate::internal::error::HttpError;

use super::body::RequestBody;
use super::headers::{HeaderMap, is_token_byte, is_valid_value_byte};
use super::method::Method;
use super::request::Request;
use super::version::Version;

pub(crate) const MAX_CHUNK_LINE: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HttpLimits {
    pub max_request_line: usize,

    pub max_header_line: usize,

    pub max_header_count: usize,

    pub max_head_bytes: usize,

    pub max_body_bytes: usize,

    pub max_chunk_bytes: usize,
}

impl HttpLimits {
    pub fn new() -> Self {
        Self {
            max_request_line: 8 * 1024,
            max_header_line: 8 * 1024,
            max_header_count: 100,
            max_head_bytes: 64 * 1024,
            max_body_bytes: 8 * 1024 * 1024,
            max_chunk_bytes: 64 * 1024,
        }
    }

    pub fn with_max_request_line(mut self, bytes: usize) -> Self {
        self.max_request_line = bytes;
        self
    }

    pub fn with_max_header_line(mut self, bytes: usize) -> Self {
        self.max_header_line = bytes;
        self
    }

    pub fn with_max_header_count(mut self, count: usize) -> Self {
        self.max_header_count = count;
        self
    }

    pub fn with_max_head_bytes(mut self, bytes: usize) -> Self {
        self.max_head_bytes = bytes;
        self
    }

    pub fn with_max_body_bytes(mut self, bytes: usize) -> Self {
        self.max_body_bytes = bytes;
        self
    }

    pub fn with_max_chunk_bytes(mut self, bytes: usize) -> Self {
        self.max_chunk_bytes = bytes;
        self
    }
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyFraming {
    None,

    ContentLength(u64),

    Chunked,
}

impl BodyFraming {
    pub fn is_chunked(self) -> bool {
        matches!(self, BodyFraming::Chunked)
    }

    pub fn content_length(self) -> Option<u64> {
        match self {
            BodyFraming::ContentLength(length) => Some(length),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RequestHead {
    method: Method,
    target: String,
    path: String,
    query: String,
    version: Version,
    headers: HeaderMap,
    framing: BodyFraming,
}

impl RequestHead {
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

    pub fn version(&self) -> Version {
        self.version
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn framing(&self) -> BodyFraming {
        self.framing
    }

    pub fn has_body(&self) -> bool {
        !matches!(
            self.framing,
            BodyFraming::None | BodyFraming::ContentLength(0)
        )
    }

    pub fn into_request(self, body: RequestBody) -> Request {
        let RequestHead {
            method,
            target,
            path,
            query,
            version,
            headers,
            framing: _,
        } = self;
        Request::new(method, target, path, query, version, headers, body)
    }
}

pub fn read_request_head(
    reader: &mut impl BufRead,
    limits: &HttpLimits,
) -> Result<Option<RequestHead>, HttpError> {
    let mut head_bytes;

    let mut line = match read_line(reader, limits.max_request_line, LineKind::RequestLine)? {
        Some(line) => line,
        None => return Ok(None),
    };
    head_bytes = line.len() + 2;

    let mut empty_lines = 0;
    while line.is_empty() {
        empty_lines += 1;
        if empty_lines > 4 {
            return Err(HttpError::InvalidRequestLine);
        }
        line = read_line(reader, limits.max_request_line, LineKind::RequestLine)?
            .ok_or(HttpError::Incomplete)?;
        head_bytes = head_bytes.saturating_add(line.len() + 2);
        if head_bytes > limits.max_head_bytes {
            return Err(HttpError::HeadTooLarge);
        }
    }
    if head_bytes > limits.max_head_bytes {
        return Err(HttpError::HeadTooLarge);
    }

    let request_line = std::str::from_utf8(&line).map_err(|_| HttpError::InvalidRequestLine)?;
    let (method, target, version) = parse_request_line(request_line)?;
    let (path, query, absolute_authority) = split_target(target, &method)?;

    let mut headers = HeaderMap::new();
    let mut header_count = 0;

    loop {
        let line = read_line(reader, limits.max_header_line, LineKind::Header)?
            .ok_or(HttpError::Incomplete)?;
        head_bytes = head_bytes.saturating_add(line.len() + 2);
        if head_bytes > limits.max_head_bytes {
            return Err(HttpError::HeadTooLarge);
        }
        if line.is_empty() {
            break;
        }

        header_count += 1;
        if header_count > limits.max_header_count {
            return Err(HttpError::TooManyHeaders);
        }

        let (name, value) = parse_header_line(&line)?;
        headers.append(name, value);
    }

    if version == Version::Http11 {
        let mut hosts = headers.get_all("host");
        let host = hosts.next();
        let duplicate = hosts.next().is_some();
        match host {
            Some(host) if !duplicate && !host.is_empty() => {}
            _ => return Err(HttpError::InvalidHost),
        }
    }

    // RFC 9110 sec. 3.2.2/7.2: when the request target is absolute-form, its
    // authority must match the Host header; otherwise intermediaries and
    // origins can disagree about where the request is directed.
    if let Some((scheme, authority)) = absolute_authority
        && let Some(host) = headers.get("host")
        && !authority_matches_host(scheme, authority, host)
    {
        return Err(HttpError::InvalidTarget);
    }

    let framing = parse_framing(version, &headers)?;

    Ok(Some(RequestHead {
        method,
        target: target.to_owned(),
        path,
        query,
        version,
        headers,
        framing,
    }))
}

pub fn read_body(
    reader: &mut impl BufRead,
    head: &RequestHead,
    limits: &HttpLimits,
) -> Result<Vec<u8>, HttpError> {
    match head.framing {
        BodyFraming::None | BodyFraming::ContentLength(0) => Ok(Vec::new()),
        BodyFraming::ContentLength(length) => {
            if length > limits.max_body_bytes as u64 {
                return Err(HttpError::BodyTooLarge);
            }
            let mut body = vec![0; length as usize];
            read_exact(reader, &mut body)?;
            Ok(body)
        }
        BodyFraming::Chunked => read_chunked_body(reader, limits),
    }
}

pub fn read_request(
    reader: &mut impl BufRead,
    limits: &HttpLimits,
) -> Result<Option<Request>, HttpError> {
    match read_request_head(reader, limits)? {
        None => Ok(None),
        Some(head) => {
            let body = read_body(reader, &head, limits)?;
            Ok(Some(head.into_request(RequestBody::from_bytes(body))))
        }
    }
}

#[derive(Clone, Copy)]
enum LineKind {
    RequestLine,
    Header,
    ChunkSize,
    Trailer,
}

impl LineKind {
    fn too_long(self) -> HttpError {
        match self {
            LineKind::RequestLine => HttpError::RequestLineTooLong,
            LineKind::Header | LineKind::Trailer => HttpError::HeadTooLarge,
            LineKind::ChunkSize => HttpError::InvalidChunk,
        }
    }

    fn malformed(self) -> HttpError {
        match self {
            LineKind::RequestLine => HttpError::InvalidRequestLine,
            LineKind::Header | LineKind::Trailer => HttpError::InvalidHeader,
            LineKind::ChunkSize => HttpError::InvalidChunk,
        }
    }
}

fn read_line(
    reader: &mut impl BufRead,
    max: usize,
    kind: LineKind,
) -> Result<Option<Vec<u8>>, HttpError> {
    let mut line = Vec::new();

    loop {
        let buffer = match reader.fill_buf() {
            Ok(buffer) => buffer,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let timed_out = matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                );
                if timed_out {
                    return Err(if line.is_empty() {
                        HttpError::IdleTimeout
                    } else {
                        HttpError::Timeout
                    });
                }
                return Err(HttpError::from_io(error));
            }
        };

        if buffer.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            return Err(HttpError::Incomplete);
        }

        if let Some(position) = buffer.iter().position(|&byte| byte == b'\n') {
            line.extend_from_slice(&buffer[..position]);
            reader.consume(position + 1);
            if line.len() > max {
                return Err(kind.too_long());
            }
            if line.last() != Some(&b'\r') {
                return Err(kind.malformed());
            }
            line.pop();
            return Ok(Some(line));
        }

        line.extend_from_slice(buffer);
        let consumed = buffer.len();
        reader.consume(consumed);
        if line.len() > max {
            return Err(kind.too_long());
        }
    }
}

fn parse_request_line(line: &str) -> Result<(Method, &str, Version), HttpError> {
    let mut pieces = line.split(' ');

    let method = pieces.next().ok_or(HttpError::InvalidRequestLine)?;
    let target = pieces.next().ok_or(HttpError::InvalidRequestLine)?;
    let version = pieces.next().ok_or(HttpError::InvalidRequestLine)?;

    if pieces.next().is_some() || method.is_empty() || target.is_empty() {
        return Err(HttpError::InvalidRequestLine);
    }

    let method = Method::parse(method).ok_or(HttpError::InvalidRequestLine)?;
    let version = match Version::parse(version) {
        Some(version) => version,
        None if version.starts_with("HTTP/") => return Err(HttpError::UnsupportedVersion),
        None => return Err(HttpError::InvalidRequestLine),
    };

    Ok((method, target, version))
}

type SplitTarget<'a> = (String, String, Option<(&'a str, &'a str)>);

fn split_target<'a>(
    target: &'a str,
    method: &Method,
) -> Result<SplitTarget<'a>, HttpError> {
    if target.is_empty()
        || !target.bytes().all(|byte| (0x21..=0x7E).contains(&byte))
        || target.contains('#')
    {
        return Err(HttpError::InvalidTarget);
    }

    if target == "*" {
        return Ok(("*".to_owned(), String::new(), None));
    }

    if matches!(method, Method::Connect) && !target.contains('/') {
        return Ok((target.to_owned(), String::new(), None));
    }

    let origin = if target.starts_with('/') {
        target
    } else {
        let scheme_end = target.find("://").ok_or(HttpError::InvalidTarget)?;
        let scheme = &target[..scheme_end];
        if scheme.is_empty()
            || !scheme
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
        {
            return Err(HttpError::InvalidTarget);
        }

        let rest = &target[scheme_end + 3..];
        if rest.is_empty() {
            return Err(HttpError::InvalidTarget);
        }
        let authority_end = match rest.find(['/', '?']) {
            Some(0) => return Err(HttpError::InvalidTarget),
            Some(index) => index,

            None => return Ok(("/".to_owned(), String::new(), Some((scheme, rest)))),
        };

        let authority = &rest[..authority_end];
        let remainder = &rest[authority_end..];
        if let Some(query) = remainder.strip_prefix('?') {
            return Ok(("/".to_owned(), query.to_owned(), Some((scheme, authority))));
        }

        let (path, query) = split_path_query(remainder)?;
        return Ok((path, query, Some((scheme, authority))));
    };

    let (path, query) = split_path_query(origin)?;
    Ok((path, query, None))
}

fn split_path_query(origin: &str) -> Result<(String, String), HttpError> {
    match origin.split_once('?') {
        Some((path, query)) => Ok((path.to_owned(), query.to_owned())),
        None => Ok((origin.to_owned(), String::new())),
    }
}

fn authority_matches_host(scheme: &str, authority: &str, host: &str) -> bool {
    let default_port = match scheme.to_ascii_lowercase().as_str() {
        "http" => Some("80"),
        "https" => Some("443"),
        _ => None,
    };

    normalize_authority(authority, default_port) == normalize_authority(host, default_port)
}

fn normalize_authority(value: &str, default_port: Option<&str>) -> (String, Option<String>) {
    // An IPv6 literal is bracketed and itself contains ':', so it must be
    // split at the closing bracket rather than at the last colon.
    let (host, port) = if let Some(rest) = value.strip_prefix('[') {
        match rest.find(']') {
            Some(end) => {
                let host = &value[..end + 2];
                match rest[end + 1..].strip_prefix(':') {
                    Some(port)
                        if !port.is_empty()
                            && port.bytes().all(|byte| byte.is_ascii_digit()) =>
                    {
                        (host, Some(port))
                    }
                    _ => (host, None),
                }
            }
            None => (value, None),
        }
    } else {
        match value.rsplit_once(':') {
            Some((host, port))
                if !host.is_empty()
                    && !port.is_empty()
                    && port.bytes().all(|byte| byte.is_ascii_digit()) =>
            {
                (host, Some(port))
            }
            _ => (value, None),
        }
    };

    let port = port
        .filter(|port| Some(*port) != default_port)
        .map(str::to_owned);
    (host.to_ascii_lowercase(), port)
}

pub(crate) fn parse_header_line(line: &[u8]) -> Result<(String, String), HttpError> {
    if matches!(line.first(), Some(b' ') | Some(b'\t')) {
        return Err(HttpError::ObsoleteLineFolding);
    }

    let colon = line
        .iter()
        .position(|&byte| byte == b':')
        .ok_or(HttpError::InvalidHeader)?;
    let name = &line[..colon];
    if name.is_empty() || !name.iter().all(|&byte| is_token_byte(byte)) {
        return Err(HttpError::InvalidHeader);
    }

    let value = &line[colon + 1..];
    let start = value
        .iter()
        .position(|&byte| byte != b' ' && byte != b'\t')
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|&byte| byte != b' ' && byte != b'\t')
        .map_or(start, |position| position + 1);
    let value = &value[start..end];

    if !value.iter().all(|&byte| is_valid_value_byte(byte)) {
        return Err(HttpError::InvalidHeaderValue);
    }

    let name = std::str::from_utf8(name)
        .map_err(|_| HttpError::InvalidHeader)?
        .to_owned();
    let value = std::str::from_utf8(value)
        .map_err(|_| HttpError::InvalidHeaderValue)?
        .to_owned();

    Ok((name, value))
}

fn parse_framing(version: Version, headers: &HeaderMap) -> Result<BodyFraming, HttpError> {
    let codings: Vec<&str> = headers
        .get_all("transfer-encoding")
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|coding| !coding.is_empty())
        .collect();

    if headers.contains("transfer-encoding") {
        if version == Version::Http10 {
            return Err(HttpError::InvalidTransferEncoding);
        }
        if headers.contains("content-length") {
            return Err(HttpError::InvalidTransferEncoding);
        }
        if !codings
            .last()
            .map(|coding| coding.eq_ignore_ascii_case("chunked"))
            .unwrap_or(false)
        {
            return Err(HttpError::InvalidTransferEncoding);
        }
        if codings.len() > 1 {
            return Err(HttpError::UnsupportedTransferEncoding);
        }
        return Ok(BodyFraming::Chunked);
    }

    if headers.contains("content-length") {
        let mut declared = None;
        for value in headers.get_all("content-length") {
            for piece in value.split(',') {
                let piece = piece.trim();
                if piece.is_empty() || !piece.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(HttpError::InvalidContentLength);
                }

                // RFC 9110 sec. 8.6: a Content-Length member with leading zeros
                // is ambiguous across implementations, so reject it outright.
                if piece.len() > 1 && piece.starts_with('0') {
                    return Err(HttpError::InvalidContentLength);
                }
                let length = piece
                    .parse::<u64>()
                    .map_err(|_| HttpError::InvalidContentLength)?;
                match declared {
                    None => declared = Some(length),
                    Some(existing) if existing != length => {
                        return Err(HttpError::InvalidContentLength);
                    }
                    Some(_) => {}
                }
            }
        }
        return Ok(BodyFraming::ContentLength(declared.unwrap_or(0)));
    }

    Ok(BodyFraming::None)
}

fn read_chunked_body(reader: &mut impl BufRead, limits: &HttpLimits) -> Result<Vec<u8>, HttpError> {
    let mut body = Vec::new();

    loop {
        let line =
            read_line(reader, MAX_CHUNK_LINE, LineKind::ChunkSize)?.ok_or(HttpError::Incomplete)?;
        let size = parse_chunk_size(&line)?;

        if size == 0 {
            read_trailers(reader, limits)?;
            return Ok(body);
        }

        let size = usize::try_from(size).map_err(|_| HttpError::InvalidChunk)?;
        if body.len() > limits.max_body_bytes || size > limits.max_body_bytes - body.len() {
            return Err(HttpError::BodyTooLarge);
        }

        let start = body.len();
        body.resize(start + size, 0);
        read_exact(reader, &mut body[start..])?;

        let mut terminator = [0u8; 2];
        read_exact(reader, &mut terminator)?;
        if terminator != *b"\r\n" {
            return Err(HttpError::InvalidChunk);
        }
    }
}

pub(crate) fn parse_chunk_size(line: &[u8]) -> Result<u64, HttpError> {
    let text = std::str::from_utf8(line).map_err(|_| HttpError::InvalidChunk)?;
    let size = text.split(';').next().unwrap_or("").trim();

    if size.is_empty() || !size.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(HttpError::InvalidChunk);
    }

    u64::from_str_radix(size, 16).map_err(|_| HttpError::InvalidChunk)
}

fn read_trailers(reader: &mut impl BufRead, limits: &HttpLimits) -> Result<(), HttpError> {
    let mut trailer_bytes: usize = 0;
    let mut trailer_count = 0;

    loop {
        let line = read_line(reader, limits.max_header_line, LineKind::Trailer)?
            .ok_or(HttpError::Incomplete)?;
        trailer_bytes = trailer_bytes.saturating_add(line.len() + 2);
        if trailer_bytes > limits.max_head_bytes {
            return Err(HttpError::HeadTooLarge);
        }
        if line.is_empty() {
            return Ok(());
        }

        trailer_count += 1;
        if trailer_count > limits.max_header_count {
            return Err(HttpError::TooManyHeaders);
        }
        parse_header_line(&line)?;
    }
}

fn read_exact(reader: &mut impl BufRead, buffer: &mut [u8]) -> Result<(), HttpError> {
    reader
        .read_exact(buffer)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => HttpError::Incomplete,
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => HttpError::Timeout,
            _ => HttpError::from_io(error),
        })
}

#[cfg(test)]
mod tests {
    use super::{BodyFraming, HttpError, HttpLimits, read_request_head};

    fn parse_head(raw: &[u8]) -> Result<super::RequestHead, HttpError> {
        let mut input = raw;
        read_request_head(&mut input, &HttpLimits::new()).map(|head| head.unwrap())
    }

    #[test]
    fn rejects_content_length_with_leading_zeros() {
        let error = parse_head(
            b"POST /a HTTP/1.1\r\n\
              Host: example.com\r\n\
              Content-Length: 05\r\n\
              \r\n",
        )
        .unwrap_err();
        assert_eq!(error, HttpError::InvalidContentLength);

        let error = parse_head(
            b"POST /a HTTP/1.1\r\n\
              Host: example.com\r\n\
              Content-Length: 3, 03\r\n\
              \r\n",
        )
        .unwrap_err();
        assert_eq!(error, HttpError::InvalidContentLength);
    }

    #[test]
    fn accepts_content_length_zero() {
        let head = parse_head(
            b"POST /a HTTP/1.1\r\n\
              Host: example.com\r\n\
              Content-Length: 0\r\n\
              \r\n",
        )
        .unwrap();
        assert_eq!(head.framing(), BodyFraming::ContentLength(0));
        assert!(!head.has_body());
    }

    #[test]
    fn absolute_form_authority_must_match_host() {
        let head = parse_head(
            b"GET http://example.com/path?q=1 HTTP/1.1\r\n\
              Host: example.com\r\n\
              \r\n",
        )
        .unwrap();
        assert_eq!(head.path(), "/path");
        assert_eq!(head.query(), "q=1");

        // Host comparison is case-insensitive and ignores the default port.
        let head = parse_head(
            b"GET http://EXAMPLE.com:80/p HTTP/1.1\r\n\
              Host: example.com\r\n\
              \r\n",
        )
        .unwrap();
        assert_eq!(head.path(), "/p");

        let head = parse_head(
            b"GET http://example.com/p HTTP/1.1\r\n\
              Host: example.com:80\r\n\
              \r\n",
        )
        .unwrap();
        assert_eq!(head.path(), "/p");

        // A non-default port must match exactly.
        let head = parse_head(
            b"GET http://example.com:8080/p HTTP/1.1\r\n\
              Host: example.com:8080\r\n\
              \r\n",
        )
        .unwrap();
        assert_eq!(head.path(), "/p");

        // IPv6 literals: the bracketed host must not be split at its own
        // colons, and a default port may be omitted in Host.
        let head = parse_head(
            b"GET http://[::1]:80/p HTTP/1.1\r\n\
              Host: [::1]\r\n\
              \r\n",
        )
        .unwrap();
        assert_eq!(head.path(), "/p");

        let head = parse_head(
            b"GET http://[2001:db8::1]:8080/p HTTP/1.1\r\n\
              Host: [2001:db8::1]:8080\r\n\
              \r\n",
        )
        .unwrap();
        assert_eq!(head.path(), "/p");

        let error = parse_head(
            b"GET http://[::1]/p HTTP/1.1\r\n\
              Host: [::2]\r\n\
              \r\n",
        )
        .unwrap_err();
        assert_eq!(error, HttpError::InvalidTarget);
    }

    #[test]
    fn absolute_form_authority_mismatch_is_rejected() {
        let error = parse_head(
            b"GET http://evil.com/path HTTP/1.1\r\n\
              Host: example.com\r\n\
              \r\n",
        )
        .unwrap_err();
        assert_eq!(error, HttpError::InvalidTarget);

        let error = parse_head(
            b"GET http://example.com:8080/path HTTP/1.1\r\n\
              Host: example.com\r\n\
              \r\n",
        )
        .unwrap_err();
        assert_eq!(error, HttpError::InvalidTarget);
    }

    #[test]
    fn asterisk_and_connect_authority_form_are_unchanged() {
        let head = parse_head(
            b"OPTIONS * HTTP/1.1\r\n\
              Host: example.com\r\n\
              \r\n",
        )
        .unwrap();
        assert_eq!(head.path(), "*");

        let head = parse_head(
            b"CONNECT example.com:443 HTTP/1.1\r\n\
              Host: example.com:443\r\n\
              \r\n",
        )
        .unwrap();
        assert_eq!(head.path(), "example.com:443");

        // Origin-form targets are not authority-checked against Host.
        let head = parse_head(
            b"GET /path HTTP/1.1\r\n\
              Host: example.com\r\n\
              \r\n",
        )
        .unwrap();
        assert_eq!(head.path(), "/path");
    }
}
