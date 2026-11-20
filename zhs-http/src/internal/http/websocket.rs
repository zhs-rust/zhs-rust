use std::future::Future;
use std::pin::Pin;

use zhs_crypto::{base64, sha1};

use crate::internal::error::{Error, HttpError, Result};
use zhs_mn::scheduler::Scheduler;

use super::context::Context;
use super::headers::has_token;
use super::method::Method;
use super::reactor::AsyncTcpStream;
use super::reader::ConnectionReader;
use super::request::Request;
use super::response::Response;
use super::status::StatusCode;

const GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

const DEFAULT_MAX_FRAME_BYTES: usize = 1024 * 1024;

const DEFAULT_MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

const MAX_CONTROL_PAYLOAD: usize = 125;

const OPCODE_CONTINUATION: u8 = 0x0;
const OPCODE_TEXT: u8 = 0x1;
const OPCODE_BINARY: u8 = 0x2;
const OPCODE_CLOSE: u8 = 0x8;
const OPCODE_PING: u8 = 0x9;
const OPCODE_PONG: u8 = 0xA;

#[derive(Clone, Copy, Debug)]
pub struct WebSocketLimits {
    pub max_frame_bytes: usize,

    pub max_message_bytes: usize,
}

impl Default for WebSocketLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
        }
    }
}

impl WebSocketLimits {
    pub fn with_max_frame_bytes(mut self, bytes: usize) -> Self {
        self.max_frame_bytes = bytes;
        self
    }

    pub fn with_max_message_bytes(mut self, bytes: usize) -> Self {
        self.max_message_bytes = bytes;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Text(String),

    Binary(Vec<u8>),

    Ping(Vec<u8>),

    Pong(Vec<u8>),

    Close(Option<CloseFrame>),
}

impl Message {
    pub fn text(text: impl Into<String>) -> Self {
        Message::Text(text.into())
    }

    pub fn binary(bytes: impl Into<Vec<u8>>) -> Self {
        Message::Binary(bytes.into())
    }

    pub fn close(code: u16, reason: impl Into<String>) -> Self {
        Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseFrame {
    pub code: u16,

    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct WebSocketUpgrade {
    accept: String,
}

impl WebSocketUpgrade {
    pub fn from_request(request: &Request) -> Result<Self> {
        if !request.version().is_http11() {
            return Err(invalid("the WebSocket handshake requires HTTP/1.1"));
        }

        if request.method() != &Method::Get {
            return Err(invalid("the WebSocket handshake requires GET"));
        }

        let headers = request.headers();

        if !has_token(headers.get_all("upgrade"), "websocket") {
            return Err(invalid("the Upgrade header must contain websocket"));
        }

        if !has_token(headers.get_all("connection"), "upgrade") {
            return Err(invalid("the Connection header must contain Upgrade"));
        }

        let version = headers
            .get("sec-websocket-version")
            .unwrap_or_default()
            .trim();
        if version != "13" {
            return Err(invalid("Sec-WebSocket-Version must be 13"));
        }

        if headers.get("transfer-encoding").is_some() {
            return Err(invalid("the WebSocket handshake must not have a body"));
        }
        if let Some(length) = headers.get("content-length")
            && length.trim().parse::<u64>().unwrap_or(1) != 0
        {
            return Err(invalid("the WebSocket handshake must not have a body"));
        }

        let key = headers
            .get("sec-websocket-key")
            .ok_or_else(|| invalid("Sec-WebSocket-Key is required"))?
            .trim();

        let decoded = base64::decode(key)
            .map_err(|_| invalid("Sec-WebSocket-Key must be valid standard base64"))?;
        if decoded.len() != 16 {
            return Err(invalid("Sec-WebSocket-Key must decode to 16 bytes"));
        }

        Ok(Self {
            accept: accept_key(key),
        })
    }

    pub fn accept_key(&self) -> &str {
        &self.accept
    }

    pub fn accept<F, Fut>(self, handler: F) -> Response
    where
        F: FnOnce(WebSocket) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let mut response = Response::new(StatusCode::SWITCHING_PROTOCOLS);
        response.headers_mut().insert("Upgrade", "websocket");
        response.headers_mut().insert("Connection", "Upgrade");
        response
            .headers_mut()
            .insert("Sec-WebSocket-Accept", self.accept);
        response.set_websocket(Box::new(handler));
        response
    }
}

fn accept_key(key: &str) -> String {
    let mut hasher = sha1::Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(GUID);
    base64::encode(&hasher.finish())
}

pub trait WebSocketHandler: Send {
    fn run(self: Box<Self>, socket: WebSocket) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

impl<F, Fut> WebSocketHandler for F
where
    F: FnOnce(WebSocket) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn run(self: Box<Self>, socket: WebSocket) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(self(socket))
    }
}

pub struct WebSocket {
    reader: ConnectionReader,
    writer: AsyncTcpStream,
    scheduler: Scheduler,
    limits: WebSocketLimits,
    write_timeout: std::time::Duration,
    fragments: Option<Fragments>,
    closed: bool,
}

struct Fragments {
    opcode: u8,
    data: Vec<u8>,
}

impl WebSocket {
    pub(crate) fn new(
        reader: ConnectionReader,
        writer: AsyncTcpStream,
        scheduler: Scheduler,
        limits: WebSocketLimits,
        write_timeout: std::time::Duration,
    ) -> Self {
        Self {
            reader,
            writer,
            scheduler,
            limits,
            write_timeout,
            fragments: None,
            closed: false,
        }
    }

    pub fn limits(&self) -> &WebSocketLimits {
        &self.limits
    }

    pub fn with_limits(mut self, limits: WebSocketLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub async fn recv(&mut self) -> Result<Option<Message>> {
        if self.closed {
            return Ok(None);
        }

        loop {
            let header = self.read_header().await?;
            let payload = self.read_payload(&header).await?;

            match header.opcode {
                OPCODE_CLOSE => {
                    let close = parse_close(&payload)?;

                    self.write_frame(OPCODE_CLOSE, &payload).await?;
                    self.closed = true;
                    return Ok(Some(Message::Close(close)));
                }
                OPCODE_PING => {
                    self.write_frame(OPCODE_PONG, &payload).await?;
                    return Ok(Some(Message::Ping(payload)));
                }
                OPCODE_PONG => return Ok(Some(Message::Pong(payload))),
                OPCODE_TEXT | OPCODE_BINARY => {
                    if self.fragments.is_some() {
                        return self.fail(1002, "expected a continuation frame").await;
                    }

                    if payload.len() > self.limits.max_message_bytes {
                        return self.fail(1009, "message is too large").await;
                    }

                    if header.fin {
                        return self.complete(header.opcode, payload).await;
                    }

                    self.fragments = Some(Fragments {
                        opcode: header.opcode,
                        data: payload,
                    });
                }
                OPCODE_CONTINUATION => {
                    let Some(fragments) = self.fragments.as_mut() else {
                        return self.fail(1002, "unexpected continuation frame").await;
                    };

                    let total = fragments.data.len() + payload.len();
                    if total > self.limits.max_message_bytes {
                        return self.fail(1009, "message is too large").await;
                    }
                    fragments.data.extend_from_slice(&payload);

                    if header.fin {
                        let fragments = self.fragments.take().expect("fragments exist");
                        return self.complete(fragments.opcode, fragments.data).await;
                    }
                }
                _ => return self.fail(1002, "unknown opcode").await,
            }
        }
    }

    pub async fn send(&mut self, message: Message) -> Result<()> {
        if self.closed {
            return Err(invalid("the WebSocket connection is closed"));
        }

        match message {
            Message::Text(text) => self.write_frame(OPCODE_TEXT, text.as_bytes()).await,
            Message::Binary(bytes) => self.write_frame(OPCODE_BINARY, &bytes).await,
            Message::Ping(bytes) => {
                check_control_payload(&bytes)?;
                self.write_frame(OPCODE_PING, &bytes).await
            }
            Message::Pong(bytes) => {
                check_control_payload(&bytes)?;
                self.write_frame(OPCODE_PONG, &bytes).await
            }
            Message::Close(close) => {
                let frame = encode_close(close.as_ref());
                self.write_frame(OPCODE_CLOSE, &frame).await?;
                self.closed = true;
                Ok(())
            }
        }
    }

    pub async fn send_text(&mut self, text: impl Into<String>) -> Result<()> {
        self.send(Message::Text(text.into())).await
    }

    pub async fn send_binary(&mut self, bytes: impl Into<Vec<u8>>) -> Result<()> {
        self.send(Message::Binary(bytes.into())).await
    }

    pub async fn close(&mut self, code: u16, reason: impl Into<String>) -> Result<()> {
        self.send(Message::close(code, reason)).await
    }

    async fn complete(&mut self, opcode: u8, data: Vec<u8>) -> Result<Option<Message>> {
        if opcode != OPCODE_TEXT {
            return Ok(Some(Message::Binary(data)));
        }

        match String::from_utf8(data) {
            Ok(text) => Ok(Some(Message::Text(text))),
            Err(_) => self.fail(1007, "text messages must be valid UTF-8").await,
        }
    }

    async fn read_header(&mut self) -> Result<FrameHeader> {
        let mut head = [0u8; 2];
        self.read_exact(&mut head).await?;

        let fin = head[0] & 0x80 != 0;
        let opcode = head[0] & 0x0f;

        if head[0] & 0x70 != 0 {
            return self.fail(1002, "reserved bits must be zero").await;
        }
        if head[1] & 0x80 == 0 {
            return self.fail(1002, "client frames must be masked").await;
        }
        if opcode >= OPCODE_CLOSE && !fin {
            return self
                .fail(1002, "control frames must not be fragmented")
                .await;
        }

        let length = match head[1] & 0x7f {
            126 => {
                let mut extended = [0u8; 2];
                self.read_exact(&mut extended).await?;
                u16::from_be_bytes(extended) as u64
            }
            127 => {
                let mut extended = [0u8; 8];
                self.read_exact(&mut extended).await?;
                u64::from_be_bytes(extended)
            }
            short => short as u64,
        };

        if length > self.limits.max_frame_bytes as u64 {
            return self.fail(1009, "frame is too large").await;
        }

        if opcode >= OPCODE_CLOSE && length > MAX_CONTROL_PAYLOAD as u64 {
            return self
                .fail(1002, "control frames are limited to 125 bytes")
                .await;
        }

        let mut mask = [0u8; 4];
        self.read_exact(&mut mask).await?;

        Ok(FrameHeader {
            fin,
            opcode,
            length: length as usize,
            mask,
        })
    }

    async fn read_payload(&mut self, header: &FrameHeader) -> Result<Vec<u8>> {
        if header.length == 0 {
            return Ok(Vec::new());
        }

        let mut payload = vec![0u8; header.length];
        self.read_exact(&mut payload).await?;
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= header.mask[index % 4];
        }
        Ok(payload)
    }

    async fn read_exact(&mut self, buffer: &mut [u8]) -> Result<()> {
        self.reader
            .read_exact_buffer(buffer)
            .await
            .map_err(Error::Http)
    }

    async fn write_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<()> {
        let mut frame = Vec::with_capacity(payload.len() + 10);
        frame.push(0x80 | opcode);

        let length = payload.len();
        if length < 126 {
            frame.push(length as u8);
        } else if length <= u16::MAX as usize {
            frame.push(126);
            frame.extend_from_slice(&(length as u16).to_be_bytes());
        } else {
            frame.push(127);
            frame.extend_from_slice(&(length as u64).to_be_bytes());
        }

        frame.extend_from_slice(payload);
        self.write_all(&frame).await
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        match self
            .scheduler
            .timeout(self.write_timeout, self.writer.write_all(bytes))
            .await
        {
            Ok(result) => result.map_err(|error| Error::Http(HttpError::from_io(error))),
            Err(_) => Err(Error::Http(HttpError::Timeout)),
        }
    }

    async fn fail<T>(&mut self, code: u16, reason: &str) -> Result<T> {
        if !self.closed {
            let frame = encode_close(Some(&CloseFrame {
                code,
                reason: reason.to_owned(),
            }));
            let _ = self.write_frame(OPCODE_CLOSE, &frame).await;
            self.closed = true;
            self.drain_input().await;
        }

        Err(invalid(reason))
    }

    async fn drain_input(&mut self) {
        let scheduler = self.scheduler.clone();
        let reader = &mut self.reader;

        let _ = scheduler
            .timeout(std::time::Duration::from_millis(250), async {
                loop {
                    let available = reader.available().len();
                    if available > 0 {
                        reader.consume(available);
                    }
                    match reader.fill_more().await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            })
            .await;
    }

    pub(crate) fn shutdown(&mut self) {
        self.reader.shutdown();
    }
}

impl Drop for WebSocket {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl std::fmt::Debug for WebSocket {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebSocket")
            .field("closed", &self.closed)
            .finish()
    }
}

struct FrameHeader {
    fin: bool,
    opcode: u8,
    length: usize,
    mask: [u8; 4],
}

fn parse_close(payload: &[u8]) -> Result<Option<CloseFrame>> {
    if payload.is_empty() {
        return Ok(None);
    }

    if payload.len() == 1 {
        return Err(invalid("a close frame with a body needs a two-byte code"));
    }

    let code = u16::from_be_bytes([payload[0], payload[1]]);
    if !valid_close_code(code) {
        return Err(invalid("invalid close code"));
    }

    let reason = std::str::from_utf8(&payload[2..])
        .map_err(|_| invalid("the close reason must be valid UTF-8"))?
        .to_owned();

    Ok(Some(CloseFrame { code, reason }))
}

fn encode_close(close: Option<&CloseFrame>) -> Vec<u8> {
    let Some(close) = close else {
        return Vec::new();
    };

    let mut payload = Vec::with_capacity(2 + close.reason.len());
    payload.extend_from_slice(&close.code.to_be_bytes());

    let mut reason = close.reason.as_str();
    if reason.len() > MAX_CONTROL_PAYLOAD - 2 {
        let mut end = MAX_CONTROL_PAYLOAD - 2;
        while end > 0 && !reason.is_char_boundary(end) {
            end -= 1;
        }
        reason = &reason[..end];
    }
    payload.extend_from_slice(reason.as_bytes());
    payload
}

fn check_control_payload(payload: &[u8]) -> Result<()> {
    if payload.len() > MAX_CONTROL_PAYLOAD {
        return Err(invalid("control frames are limited to 125 bytes"));
    }
    Ok(())
}

fn valid_close_code(code: u16) -> bool {
    matches!(code, 1000..=1003 | 1007..=1014 | 3000..=4999)
}

fn invalid(reason: &str) -> Error {
    Error::Http(HttpError::InvalidWebSocket {
        reason: reason.to_owned(),
    })
}

impl Context {
    pub fn websocket(&self) -> Result<WebSocketUpgrade> {
        WebSocketUpgrade::from_request(self.request())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_the_rfc_accept_key() {
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn validates_close_codes() {
        for code in [
            1000, 1001, 1002, 1003, 1007, 1008, 1009, 1010, 1011, 3000, 4999,
        ] {
            assert!(valid_close_code(code), "{code}");
        }
        for code in [0, 999, 1004, 1005, 1006, 1015, 1016, 2999, 5000] {
            assert!(!valid_close_code(code), "{code}");
        }
    }

    #[test]
    fn parses_and_encodes_close_frames() {
        assert_eq!(parse_close(&[]).expect("empty"), None);

        let close = parse_close(&[0x03, 0xe8, b'b', b'y', b'e']).expect("parsed");
        assert_eq!(
            close,
            Some(CloseFrame {
                code: 1000,
                reason: "bye".to_owned(),
            })
        );

        assert!(parse_close(&[0x03]).is_err());
        assert!(parse_close(&[0x03, 0xe7]).is_err());
        assert!(parse_close(&[0x03, 0xe8, 0xff]).is_err());

        let encoded = encode_close(Some(&CloseFrame {
            code: 1000,
            reason: "bye".to_owned(),
        }));
        assert_eq!(encoded, vec![0x03, 0xe8, b'b', b'y', b'e']);
        assert!(encode_close(None).is_empty());
    }

    #[test]
    fn truncates_long_close_reasons() {
        let encoded = encode_close(Some(&CloseFrame {
            code: 1000,
            reason: "汉".repeat(100),
        }));
        assert!(encoded.len() <= MAX_CONTROL_PAYLOAD);

        assert!(std::str::from_utf8(&encoded[2..]).is_ok());
    }
}
