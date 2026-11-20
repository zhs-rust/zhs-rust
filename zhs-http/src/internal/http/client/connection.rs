use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::internal::error::ClientError;
use crate::internal::http::{HeaderMap, StatusCode, Version};

use super::super::headers::{is_token_byte, is_valid_value_byte};

pub trait Transport: Read + Write + Send {}

impl Transport for TcpStream {}

pub trait Connector: Send + Sync {
    fn connect(
        &self,
        scheme: &str,
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<Box<dyn Transport>, ClientError>;
}

#[derive(Clone, Copy, Debug)]
pub struct TcpConnector {
    read_timeout: Duration,
    write_timeout: Duration,
}

impl TcpConnector {
    pub fn new(read_timeout: Duration, write_timeout: Duration) -> Self {
        Self {
            read_timeout,
            write_timeout,
        }
    }
}

impl Connector for TcpConnector {
    fn connect(
        &self,
        scheme: &str,
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<Box<dyn Transport>, ClientError> {
        if !scheme.eq_ignore_ascii_case("http") {
            return Err(ClientError::UnsupportedScheme {
                scheme: scheme.to_ascii_lowercase(),
            });
        }

        let addresses = (host, port)
            .to_socket_addrs()
            .map_err(|error| ClientError::connect(host, port, error))?;

        let mut last_error = None;
        for address in addresses {
            let attempt = if timeout.is_zero() {
                TcpStream::connect(address)
            } else {
                TcpStream::connect_timeout(&address, timeout)
            };

            match attempt {
                Ok(stream) => {
                    let _ = stream.set_nodelay(true);

                    stream
                        .set_read_timeout(optional_timeout(self.read_timeout))
                        .map_err(|error| ClientError::connect(host, port, error))?;
                    stream
                        .set_write_timeout(optional_timeout(self.write_timeout))
                        .map_err(|error| ClientError::connect(host, port, error))?;
                    return Ok(Box::new(stream));
                }
                Err(error) => last_error = Some(error),
            }
        }

        Err(ClientError::connect(
            host,
            port,
            last_error.unwrap_or_else(|| {
                io::Error::new(
                    io::ErrorKind::AddrNotAvailable,
                    "the host resolved to no address",
                )
            }),
        ))
    }
}

fn optional_timeout(timeout: Duration) -> Option<Duration> {
    (!timeout.is_zero()).then_some(timeout)
}

struct IdleConnection {
    transport: Box<dyn Transport>,
    since: Instant,
}

pub(crate) struct ConnectionPool {
    idle: Mutex<HashMap<String, Vec<IdleConnection>>>,
    max_idle_per_host: usize,
    idle_timeout: Duration,
}

impl ConnectionPool {
    pub(crate) fn new(max_idle_per_host: usize, idle_timeout: Duration) -> Self {
        Self {
            idle: Mutex::new(HashMap::new()),
            max_idle_per_host,
            idle_timeout,
        }
    }

    pub(crate) fn checkout(&self, origin: &str) -> Option<Box<dyn Transport>> {
        let mut idle = lock(&self.idle);
        let connections = idle.get_mut(origin)?;

        while let Some(connection) = connections.pop() {
            if connection.since.elapsed() <= self.idle_timeout {
                return Some(connection.transport);
            }
        }

        idle.remove(origin);
        None
    }

    pub(crate) fn checkin(&self, origin: String, transport: Box<dyn Transport>) {
        if self.max_idle_per_host == 0 {
            return;
        }

        let mut idle = lock(&self.idle);
        let connections = idle.entry(origin).or_default();
        if connections.len() >= self.max_idle_per_host {
            return;
        }
        connections.push(IdleConnection {
            transport,
            since: Instant::now(),
        });
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ResponseLimits {
    pub(crate) max_status_line: usize,
    pub(crate) max_header_line: usize,
    pub(crate) max_header_count: usize,
    pub(crate) max_head_bytes: usize,
    pub(crate) max_body_bytes: usize,
}

impl ResponseLimits {
    pub(crate) fn new(max_body_bytes: usize) -> Self {
        Self {
            max_status_line: 8 * 1024,
            max_header_line: 8 * 1024,
            max_header_count: 100,
            max_head_bytes: 64 * 1024,
            max_body_bytes,
        }
    }
}

pub(crate) struct ExchangeOutcome {
    pub(crate) status: StatusCode,
    pub(crate) version: Version,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,

    pub(crate) reusable: Option<Box<dyn Transport>>,
}

impl std::fmt::Debug for ExchangeOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExchangeOutcome")
            .field("status", &self.status)
            .field("version", &self.version)
            .field("headers", &self.headers)
            .field("body_len", &self.body.len())
            .field("reusable", &self.reusable.is_some())
            .finish()
    }
}

#[derive(Debug)]
pub(crate) struct ExchangeError {
    pub(crate) error: ClientError,
    pub(crate) response_started: bool,
}

impl ExchangeError {
    fn new(error: ClientError, response_started: bool) -> Self {
        Self {
            error,
            response_started,
        }
    }
}

pub(crate) fn exchange(
    transport: Box<dyn Transport>,
    request: &[u8],
    head_request: bool,
    request_close: bool,
    limits: &ResponseLimits,
) -> Result<ExchangeOutcome, ExchangeError> {
    let mut reader = BufReader::with_capacity(8 * 1024, transport);

    reader
        .get_mut()
        .write_all(request)
        .map_err(|error| ExchangeError::new(ClientError::io(error, "write"), false))?;
    reader
        .get_mut()
        .flush()
        .map_err(|error| ExchangeError::new(ClientError::io(error, "write"), false))?;

    let mut response_started = false;
    let mut informational_responses = 0;
    let (status, version, headers) = loop {
        let line = read_line(&mut reader, limits.max_status_line)
            .map_err(|error| ExchangeError::new(error, response_started))?;

        let Some(line) = line else {
            return Err(ExchangeError::new(
                ClientError::Incomplete,
                response_started,
            ));
        };
        response_started = true;

        let (status, version) = parse_status_line(&line)
            .map_err(|error| ExchangeError::new(error, response_started))?;
        let headers = read_headers(&mut reader, limits)
            .map_err(|error| ExchangeError::new(error, response_started))?;

        if status.is_informational() {
            if status == StatusCode::SWITCHING_PROTOCOLS {
                return Err(ExchangeError::new(
                    ClientError::InvalidResponseHeader {
                        reason: "the server switched protocols, which this client does not support"
                            .to_owned(),
                    },
                    response_started,
                ));
            }
            informational_responses += 1;
            if informational_responses >= 8 {
                return Err(ExchangeError::new(
                    ClientError::InvalidResponseHeader {
                        reason: "too many consecutive informational responses".to_owned(),
                    },
                    response_started,
                ));
            }
            continue;
        }

        break (status, version, headers);
    };

    let connection_close = header_has_token(&headers, "connection", "close");
    let connection_keep_alive = header_has_token(&headers, "connection", "keep-alive");
    let keep_alive = if version.is_http11() {
        !connection_close
    } else {
        connection_keep_alive && !connection_close
    } && !request_close;

    let (framing, keep_alive) =
        response_framing(head_request, status, &headers, limits, keep_alive)
            .map_err(|error| ExchangeError::new(error, response_started))?;

    let body = match framing {
        Framing::None => Vec::new(),
        Framing::Length(length) => {
            let mut body = vec![0_u8; length];
            reader
                .read_exact(&mut body)
                .map_err(|error| ExchangeError::new(read_error(error), response_started))?;
            body
        }
        Framing::Chunked => read_chunked_body(&mut reader, limits)
            .map_err(|error| ExchangeError::new(error, response_started))?,
        Framing::CloseDelimited => read_close_delimited(&mut reader, limits)
            .map_err(|error| ExchangeError::new(error, response_started))?,
    };

    let clean = reader.buffer().is_empty();
    let transport = reader.into_inner();
    let reusable = (keep_alive && clean).then_some(transport);

    Ok(ExchangeOutcome {
        status,
        version,
        headers,
        body,
        reusable,
    })
}

enum Framing {
    None,
    Length(usize),
    Chunked,
    CloseDelimited,
}

fn response_framing(
    head_request: bool,
    status: StatusCode,
    headers: &HeaderMap,
    limits: &ResponseLimits,
    keep_alive: bool,
) -> Result<(Framing, bool), ClientError> {
    if head_request || status.forbids_body() {
        return Ok((Framing::None, keep_alive));
    }

    let mut transfer_codings: Vec<String> = Vec::new();
    for value in headers.get_all("transfer-encoding") {
        for coding in value.split(',') {
            let coding = coding.trim();
            if !coding.is_empty() {
                transfer_codings.push(coding.to_ascii_lowercase());
            }
        }
    }

    let has_content_length = headers.contains("content-length");

    if !transfer_codings.is_empty() {
        if has_content_length {
            return Err(ClientError::InvalidResponseHeader {
                reason: "both Transfer-Encoding and Content-Length are present".to_owned(),
            });
        }
        if transfer_codings.len() != 1 || transfer_codings[0] != "chunked" {
            return Err(ClientError::UnsupportedTransferEncoding {
                coding: transfer_codings.join(", "),
            });
        }
        return Ok((Framing::Chunked, keep_alive));
    }

    if has_content_length {
        let length = parse_content_length(headers)?;
        if length > limits.max_body_bytes as u64 {
            return Err(ClientError::BodyTooLarge {
                limit: limits.max_body_bytes,
            });
        }
        return Ok((Framing::Length(length as usize), keep_alive));
    }

    Ok((Framing::CloseDelimited, false))
}

fn parse_content_length(headers: &HeaderMap) -> Result<u64, ClientError> {
    let mut length = None;

    for value in headers.get_all("content-length") {
        for item in value.split(',') {
            let item = item.trim();
            if item.is_empty() || !item.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(ClientError::InvalidResponseHeader {
                    reason: "invalid Content-Length".to_owned(),
                });
            }

            let parsed = item
                .parse::<u64>()
                .map_err(|_| ClientError::InvalidResponseHeader {
                    reason: "Content-Length is out of range".to_owned(),
                })?;

            match length {
                Some(previous) if previous != parsed => {
                    return Err(ClientError::InvalidResponseHeader {
                        reason: "conflicting Content-Length values".to_owned(),
                    });
                }
                _ => length = Some(parsed),
            }
        }
    }

    length.ok_or_else(|| ClientError::InvalidResponseHeader {
        reason: "missing Content-Length".to_owned(),
    })
}

fn read_chunked_body(
    reader: &mut impl BufRead,
    limits: &ResponseLimits,
) -> Result<Vec<u8>, ClientError> {
    const MAX_CHUNK_LINE: usize = 1024;

    let mut body = Vec::new();

    loop {
        let line = read_line(reader, MAX_CHUNK_LINE)?.ok_or(ClientError::Incomplete)?;
        let size = parse_chunk_size(&line)?;

        if size == 0 {
            read_trailers(reader, limits)?;
            return Ok(body);
        }

        let size = size as usize;
        if body.len() > limits.max_body_bytes || size > limits.max_body_bytes - body.len() {
            return Err(ClientError::BodyTooLarge {
                limit: limits.max_body_bytes,
            });
        }

        let mut chunk = vec![0_u8; size];
        reader.read_exact(&mut chunk).map_err(read_error)?;
        body.extend_from_slice(&chunk);

        let mut terminator = [0_u8; 2];
        reader.read_exact(&mut terminator).map_err(read_error)?;
        if terminator != *b"\r\n" {
            return Err(ClientError::InvalidChunk);
        }
    }
}

fn parse_chunk_size(line: &[u8]) -> Result<u64, ClientError> {
    let text = std::str::from_utf8(line).map_err(|_| ClientError::InvalidChunk)?;
    let size = text.split(';').next().unwrap_or_default().trim();

    if size.is_empty() || !size.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ClientError::InvalidChunk);
    }

    u64::from_str_radix(size, 16)
        .ok()
        .filter(|size| *size <= usize::MAX as u64)
        .ok_or(ClientError::InvalidChunk)
}

fn read_trailers(reader: &mut impl BufRead, limits: &ResponseLimits) -> Result<(), ClientError> {
    let mut bytes: usize = 0;
    let mut count = 0;

    loop {
        let line = read_line(reader, limits.max_header_line)?.ok_or(ClientError::Incomplete)?;
        bytes = bytes.saturating_add(line.len() + 2);
        if bytes > limits.max_head_bytes {
            return Err(ClientError::HeadTooLarge);
        }
        if line.is_empty() {
            return Ok(());
        }

        count += 1;
        if count > limits.max_header_count {
            return Err(ClientError::TooManyHeaders);
        }
        parse_header_line(&line)?;
    }
}

fn read_close_delimited(
    reader: &mut impl Read,
    limits: &ResponseLimits,
) -> Result<Vec<u8>, ClientError> {
    let mut body = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];

    loop {
        let read = reader.read(&mut buffer).map_err(read_error)?;
        if read == 0 {
            return Ok(body);
        }

        if body.len() > limits.max_body_bytes || read > limits.max_body_bytes - body.len() {
            return Err(ClientError::BodyTooLarge {
                limit: limits.max_body_bytes,
            });
        }
        body.extend_from_slice(&buffer[..read]);
    }
}

fn read_headers(
    reader: &mut impl BufRead,
    limits: &ResponseLimits,
) -> Result<HeaderMap, ClientError> {
    let mut headers = HeaderMap::new();
    let mut bytes: usize = 0;
    let mut count = 0;

    loop {
        let line = read_line(reader, limits.max_header_line)?.ok_or(ClientError::Incomplete)?;
        bytes = bytes.saturating_add(line.len() + 2);
        if bytes > limits.max_head_bytes {
            return Err(ClientError::HeadTooLarge);
        }
        if line.is_empty() {
            return Ok(headers);
        }

        count += 1;
        if count > limits.max_header_count {
            return Err(ClientError::TooManyHeaders);
        }
        if line.starts_with(b" ") || line.starts_with(b"\t") {
            return Err(ClientError::InvalidResponseHeader {
                reason: "obsolete header line folding is not accepted".to_owned(),
            });
        }

        let (name, value) = parse_header_line(&line)?;
        headers.append(name, value);
    }
}

fn parse_status_line(line: &[u8]) -> Result<(StatusCode, Version), ClientError> {
    let text = std::str::from_utf8(line).map_err(|_| ClientError::InvalidStatusLine)?;
    let mut parts = text.splitn(3, ' ');

    let version = parts
        .next()
        .and_then(Version::parse)
        .ok_or(ClientError::InvalidStatusLine)?;
    let code = parts
        .next()
        .and_then(|code| code.parse::<u16>().ok())
        .and_then(StatusCode::new)
        .ok_or(ClientError::InvalidStatusLine)?;

    if let Some(reason) = parts.next()
        && reason.bytes().any(|byte| !is_valid_value_byte(byte))
    {
        return Err(ClientError::InvalidStatusLine);
    }

    Ok((code, version))
}

fn parse_header_line(line: &[u8]) -> Result<(String, String), ClientError> {
    let Some(colon) = line.iter().position(|&byte| byte == b':') else {
        return Err(ClientError::InvalidResponseHeader {
            reason: "a header line has no colon".to_owned(),
        });
    };

    let name = &line[..colon];
    if name.is_empty() || !name.iter().all(|&byte| is_token_byte(byte)) {
        return Err(ClientError::InvalidResponseHeader {
            reason: "invalid header name".to_owned(),
        });
    }

    let value = &line[colon + 1..];
    let value = trim_ascii_whitespace(value);
    if value.iter().any(|&byte| !is_valid_value_byte(byte)) {
        return Err(ClientError::InvalidResponseHeader {
            reason: "invalid header value".to_owned(),
        });
    }

    let name =
        String::from_utf8(name.to_vec()).map_err(|_| ClientError::InvalidResponseHeader {
            reason: "the header name is not valid UTF-8".to_owned(),
        })?;
    let value =
        String::from_utf8(value.to_vec()).map_err(|_| ClientError::InvalidResponseHeader {
            reason: "the header value is not valid UTF-8".to_owned(),
        })?;

    Ok((name, value))
}

fn trim_ascii_whitespace(mut value: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = value
        && matches!(*first, b' ' | b'\t')
    {
        value = rest;
    }
    while let [rest @ .., last] = value
        && matches!(*last, b' ' | b'\t')
    {
        value = rest;
    }
    value
}

fn read_line(reader: &mut impl BufRead, max: usize) -> Result<Option<Vec<u8>>, ClientError> {
    let mut line = Vec::new();

    loop {
        let (chunk, found_newline) = {
            let buffer = reader.fill_buf().map_err(read_error)?;
            if buffer.is_empty() {
                return if line.is_empty() {
                    Ok(None)
                } else {
                    Err(ClientError::Incomplete)
                };
            }

            let found = buffer.iter().position(|&byte| byte == b'\n');
            let take = found.map_or(buffer.len(), |position| position + 1);
            (buffer[..take].to_vec(), found.is_some())
        };

        line.extend_from_slice(&chunk);
        reader.consume(chunk.len());

        if line.len() > max {
            return Err(ClientError::InvalidResponseHeader {
                reason: "a response line is too long".to_owned(),
            });
        }

        if found_newline {
            if !line.ends_with(b"\r\n") {
                return Err(ClientError::InvalidResponseHeader {
                    reason: "a response line does not end with CRLF".to_owned(),
                });
            }
            line.truncate(line.len() - 2);
            return Ok(Some(line));
        }
    }
}

fn read_error(error: io::Error) -> ClientError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        ClientError::Incomplete
    } else {
        ClientError::io(error, "read")
    }
}

pub(crate) fn header_has_token(headers: &HeaderMap, name: &str, token: &str) -> bool {
    headers.get_all(name).any(|value| {
        value
            .split(',')
            .any(|item| item.trim().eq_ignore_ascii_case(token))
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(crate::internal::recover_poison)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    struct MemoryTransport {
        response: Cursor<Vec<u8>>,
    }

    impl Read for MemoryTransport {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.response.read(buffer)
        }
    }

    impl Write for MemoryTransport {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Transport for MemoryTransport {}

    fn run(response: &[u8], head: bool) -> Result<ExchangeOutcome, ExchangeError> {
        exchange(
            Box::new(MemoryTransport {
                response: Cursor::new(response.to_vec()),
            }),
            b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n",
            head,
            false,
            &ResponseLimits::new(1024),
        )
    }

    #[test]
    fn reads_content_length_responses() {
        let outcome = run(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Type: text/plain\r\n\r\nhello",
            false,
        )
        .expect("the response should parse");

        assert_eq!(outcome.status, StatusCode::OK);
        assert_eq!(outcome.body, b"hello");
        assert_eq!(outcome.headers.get("content-type"), Some("text/plain"));
        assert!(outcome.reusable.is_some());
    }

    #[test]
    fn reads_chunked_responses() {
        let outcome = run(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\nX-Trailer: yes\r\n\r\n",
            false,
        )
        .expect("the response should parse");

        assert_eq!(outcome.body, b"hello world");
        assert!(outcome.reusable.is_some());
    }

    #[test]
    fn head_and_no_content_have_no_body() {
        let outcome =
            run(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n", true).expect("HEAD response");
        assert!(outcome.body.is_empty());

        let outcome = run(b"HTTP/1.1 204 No Content\r\n\r\n", false).expect("204");
        assert!(outcome.body.is_empty());
    }

    #[test]
    fn close_delimited_responses_are_not_reused() {
        let outcome = run(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nbody", false)
            .expect("close-delimited response");

        assert_eq!(outcome.body, b"body");
        assert!(outcome.reusable.is_none());
    }

    #[test]
    fn rejects_conflicting_framing() {
        let error = run(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n\r\nhello",
            false,
        )
        .expect_err("framing is ambiguous");

        assert!(matches!(
            error.error,
            ClientError::InvalidResponseHeader { .. }
        ));
    }

    #[test]
    fn enforces_the_body_limit() {
        let error = run(b"HTTP/1.1 200 OK\r\nContent-Length: 1025\r\n\r\n", false)
            .expect_err("body is too large");

        assert!(matches!(error.error, ClientError::BodyTooLarge { .. }));
    }

    #[test]
    fn rejects_malformed_status_lines() {
        let error = run(b"NOT HTTP\r\n\r\n", false).expect_err("malformed status line");
        assert!(matches!(error.error, ClientError::InvalidStatusLine));

        assert!(error.response_started);
    }
}
