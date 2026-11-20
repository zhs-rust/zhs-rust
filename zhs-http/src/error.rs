use std::fmt;

use crate::method::Method;
use crate::response::{Response, StatusCode};

#[derive(Debug)]
pub enum HttpError {
    BadRequest(String),
    NotFound,
    MethodNotAllowed(Vec<Method>),
    Internal(String),
    Io(std::io::Error),
}

impl HttpError {
    pub fn status(&self) -> StatusCode {
        match self {
            HttpError::BadRequest(_) => StatusCode::BAD_REQUEST,
            HttpError::NotFound => StatusCode::NOT_FOUND,
            HttpError::MethodNotAllowed(_) => StatusCode::METHOD_NOT_ALLOWED,
            HttpError::Internal(_) | HttpError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn message(&self) -> String {
        match self {
            HttpError::BadRequest(message) | HttpError::Internal(message) => message.clone(),
            HttpError::NotFound => "not found".to_owned(),
            HttpError::MethodNotAllowed(_) => "method not allowed".to_owned(),
            HttpError::Io(error) => error.to_string(),
        }
    }

    pub fn into_response(self) -> Response {
        let status = self.status();
        let message = self.message();
        Response::text(message).with_status(status)
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for HttpError {}

impl From<std::io::Error> for HttpError {
    fn from(error: std::io::Error) -> Self {
        HttpError::Io(error)
    }
}
