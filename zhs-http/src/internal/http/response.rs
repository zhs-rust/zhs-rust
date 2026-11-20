use std::fmt;

use zhs_json::ToJson;

use super::body::{BodyStream, ResponseBody};
use super::headers::HeaderMap;
use super::status::StatusCode;
use super::websocket::WebSocketHandler;

pub struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: ResponseBody,
    content_length: Option<u64>,
    close_connection: bool,
    websocket: Option<Box<dyn WebSocketHandler>>,
}

impl Response {
    pub fn new(status: StatusCode) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: ResponseBody::empty(),
            content_length: None,
            close_connection: false,
            websocket: None,
        }
    }

    pub fn ok() -> Self {
        Self::new(StatusCode::OK)
    }

    pub fn text(body: impl Into<String>) -> Self {
        Self::bytes(body.into().into_bytes())
            .with_header("Content-Type", "text/plain; charset=utf-8")
    }

    pub fn html(body: impl Into<String>) -> Self {
        Self::bytes(body.into().into_bytes())
            .with_header("Content-Type", "text/html; charset=utf-8")
    }

    pub fn json<T: ToJson + ?Sized>(value: &T) -> Self {
        Self::bytes(zhs_json::to_string(value).into_bytes())
            .with_header("Content-Type", "application/json; charset=utf-8")
    }

    pub fn bytes(body: impl Into<Vec<u8>>) -> Self {
        let mut response = Self::new(StatusCode::OK);
        response.body = ResponseBody::Bytes(body.into());
        response
    }

    pub fn stream(stream: impl BodyStream + 'static) -> Self {
        let mut response = Self::new(StatusCode::OK);
        response.body = ResponseBody::Stream(Box::new(stream));
        response
    }

    pub fn stream_with_length(stream: impl BodyStream + 'static, length: u64) -> Self {
        Self::stream(stream).with_content_length(length)
    }

    pub fn stream_chunks(chunks: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self::stream(super::body::ChunkList::new(chunks))
    }

    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.append(name, value);
        self
    }

    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = ResponseBody::Bytes(body.into());
        self.content_length = None;
        self
    }

    pub fn with_stream(mut self, stream: impl BodyStream + 'static) -> Self {
        self.body = ResponseBody::Stream(Box::new(stream));
        self.content_length = None;
        self
    }

    pub fn with_content_length(mut self, length: u64) -> Self {
        self.content_length = Some(length);
        self
    }

    pub fn with_close(mut self) -> Self {
        self.close_connection = true;
        self
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn status_mut(&mut self) -> &mut StatusCode {
        &mut self.status
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    pub fn body(&self) -> &ResponseBody {
        &self.body
    }

    pub fn body_mut(&mut self) -> &mut ResponseBody {
        &mut self.body
    }

    pub fn close_connection(&self) -> bool {
        self.close_connection
    }

    pub fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    pub fn is_upgrade(&self) -> bool {
        self.websocket.is_some()
    }

    pub(crate) fn set_websocket(&mut self, handler: Box<dyn WebSocketHandler>) {
        self.websocket = Some(handler);
    }

    pub(crate) fn take_websocket(&mut self) -> Option<Box<dyn WebSocketHandler>> {
        self.websocket.take()
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Response")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("body", &self.body)
            .field("content_length", &self.content_length)
            .field("close_connection", &self.close_connection)
            .field("upgrade", &self.websocket.is_some())
            .finish()
    }
}

impl Default for Response {
    fn default() -> Self {
        Self::ok()
    }
}
