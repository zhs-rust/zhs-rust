use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StatusCode(pub u16);

impl StatusCode {
    pub const OK: Self = Self(200);
    pub const CREATED: Self = Self(201);
    pub const ACCEPTED: Self = Self(202);
    pub const NO_CONTENT: Self = Self(204);
    pub const BAD_REQUEST: Self = Self(400);
    pub const UNAUTHORIZED: Self = Self(401);
    pub const FORBIDDEN: Self = Self(403);
    pub const NOT_FOUND: Self = Self(404);
    pub const METHOD_NOT_ALLOWED: Self = Self(405);
    pub const CONFLICT: Self = Self(409);
    pub const UNSUPPORTED_MEDIA_TYPE: Self = Self(415);
    pub const UNPROCESSABLE_ENTITY: Self = Self(422);
    pub const INTERNAL_SERVER_ERROR: Self = Self(500);

    pub const fn reason(self) -> &'static str {
        match self.0 {
            200 => "OK",
            201 => "Created",
            202 => "Accepted",
            204 => "No Content",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            409 => "Conflict",
            415 => "Unsupported Media Type",
            422 => "Unprocessable Entity",
            500 => "Internal Server Error",
            _ => "Unknown",
        }
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: StatusCode,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: StatusCode) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn text(body: impl Into<String>) -> Self {
        Self::new(StatusCode::OK)
            .with_header("content-type", "text/plain; charset=utf-8")
            .with_body(body.into().into_bytes())
    }

    pub fn html(body: impl Into<String>) -> Self {
        Self::new(StatusCode::OK)
            .with_header("content-type", "text/html; charset=utf-8")
            .with_body(body.into().into_bytes())
    }

    pub fn bytes(body: Vec<u8>) -> Self {
        Self::new(StatusCode::OK).with_body(body)
    }

    pub fn json_raw(body: impl Into<String>) -> Self {
        Self::new(StatusCode::OK)
            .with_header("content-type", "application/json; charset=utf-8")
            .with_body(body.into().into_bytes())
    }

    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        let value = value.into();

        if let Some(existing) = self
            .headers
            .iter_mut()
            .find(|(current, _)| current.eq_ignore_ascii_case(&name))
        {
            existing.1 = value;
        } else {
            self.headers.push((name, value));
        }

        self
    }

    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }
}

pub trait IntoResponse {
    fn into_response(self) -> Response;
}

impl IntoResponse for Response {
    fn into_response(self) -> Response {
        self
    }
}

impl IntoResponse for String {
    fn into_response(self) -> Response {
        Response::text(self)
    }
}

impl IntoResponse for &str {
    fn into_response(self) -> Response {
        Response::text(self)
    }
}

impl IntoResponse for () {
    fn into_response(self) -> Response {
        Response::new(StatusCode::OK)
    }
}
