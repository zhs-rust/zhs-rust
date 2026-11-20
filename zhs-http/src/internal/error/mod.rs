mod client;
mod config;
mod container;
mod data;
mod host;
mod http;
mod json;
mod log;
mod scheduler;
mod thread_pool;
mod watchdog;

pub use client::ClientError;
pub use config::ConfigError;
pub use container::ContainerError;
pub use data::DataError;
pub use host::HostError;
pub use http::HttpError;
pub use json::{JsonDecodeError, JsonError, JsonErrorKind};
pub use log::LogError;
pub use scheduler::{JoinError, SchedulerError, TimeoutError};
pub use thread_pool::ThreadPoolError;
pub use watchdog::WatchdogError;

pub use crate::internal::api_key::ApiKeyError;
pub use crate::internal::refresh::RefreshError;

use std::error::Error as StdError;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    Json(JsonError),

    JsonDecode(JsonDecodeError),

    Scheduler(SchedulerError),

    ThreadPool(ThreadPoolError),

    Watchdog(WatchdogError),

    Http(HttpError),

    Container(ContainerError),

    Log(LogError),

    Config(ConfigError),

    Host(HostError),

    Join(JoinError),

    Timeout(TimeoutError),

    Refresh(RefreshError),

    Jwt(crate::internal::jwt::JwtError),

    ApiKey(crate::internal::api_key::ApiKeyError),

    Client(ClientError),

    Data(DataError),
}

impl Error {
    pub fn as_json(&self) -> Option<&JsonError> {
        match self {
            Error::Json(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_json(self) -> Option<JsonError> {
        match self {
            Error::Json(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_json_decode(&self) -> Option<&JsonDecodeError> {
        match self {
            Error::JsonDecode(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_json_decode(self) -> Option<JsonDecodeError> {
        match self {
            Error::JsonDecode(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_scheduler(&self) -> Option<&SchedulerError> {
        match self {
            Error::Scheduler(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_scheduler(self) -> Option<SchedulerError> {
        match self {
            Error::Scheduler(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_thread_pool(&self) -> Option<&ThreadPoolError> {
        match self {
            Error::ThreadPool(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_thread_pool(self) -> Option<ThreadPoolError> {
        match self {
            Error::ThreadPool(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_watchdog(&self) -> Option<&WatchdogError> {
        match self {
            Error::Watchdog(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_watchdog(self) -> Option<WatchdogError> {
        match self {
            Error::Watchdog(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_http(&self) -> Option<&HttpError> {
        match self {
            Error::Http(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_http(self) -> Option<HttpError> {
        match self {
            Error::Http(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_container(&self) -> Option<&ContainerError> {
        match self {
            Error::Container(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_container(self) -> Option<ContainerError> {
        match self {
            Error::Container(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_log(&self) -> Option<&LogError> {
        match self {
            Error::Log(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_log(self) -> Option<LogError> {
        match self {
            Error::Log(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_config(&self) -> Option<&ConfigError> {
        match self {
            Error::Config(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_config(self) -> Option<ConfigError> {
        match self {
            Error::Config(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_host(&self) -> Option<&HostError> {
        match self {
            Error::Host(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_host(self) -> Option<HostError> {
        match self {
            Error::Host(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_join(&self) -> Option<&JoinError> {
        match self {
            Error::Join(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_join(self) -> Option<JoinError> {
        match self {
            Error::Join(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_timeout(&self) -> Option<&TimeoutError> {
        match self {
            Error::Timeout(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_timeout(self) -> Option<TimeoutError> {
        match self {
            Error::Timeout(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_refresh(&self) -> Option<&RefreshError> {
        match self {
            Error::Refresh(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_refresh(self) -> Option<RefreshError> {
        match self {
            Error::Refresh(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_jwt(&self) -> Option<&crate::internal::jwt::JwtError> {
        match self {
            Error::Jwt(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_jwt(self) -> Option<crate::internal::jwt::JwtError> {
        match self {
            Error::Jwt(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_client(&self) -> Option<&ClientError> {
        match self {
            Error::Client(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_client(self) -> Option<ClientError> {
        match self {
            Error::Client(error) => Some(error),
            _ => None,
        }
    }

    pub fn as_api_key(&self) -> Option<&crate::internal::api_key::ApiKeyError> {
        match self {
            Error::ApiKey(error) => Some(error),
            _ => None,
        }
    }

    pub fn into_api_key(self) -> Option<crate::internal::api_key::ApiKeyError> {
        match self {
            Error::ApiKey(error) => Some(error),
            _ => None,
        }
    }

    pub fn status(&self) -> crate::internal::http::StatusCode {
        match self {
            Error::Http(error) => error.status(),
            Error::Json(_) | Error::JsonDecode(_) => crate::internal::http::StatusCode::BAD_REQUEST,
            Error::Refresh(error) => refresh_status(error),
            Error::Jwt(error) => jwt_status(error),
            Error::ApiKey(error) => api_key_status(error),
            Error::Client(error) => error.status(),
            Error::Data(DataError::Timeout(_)) => {
                crate::internal::http::StatusCode::SERVICE_UNAVAILABLE
            }
            Error::Data(error) if error.is_conflict() => {
                crate::internal::http::StatusCode::CONFLICT
            }
            _ => crate::internal::http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

fn refresh_status(error: &RefreshError) -> crate::internal::http::StatusCode {
    use crate::internal::http::StatusCode;

    match error {
        RefreshError::Malformed => StatusCode::BAD_REQUEST,
        RefreshError::NotFound
        | RefreshError::Expired
        | RefreshError::Revoked
        | RefreshError::ReuseDetected => StatusCode::UNAUTHORIZED,
        RefreshError::EmptySubject | RefreshError::Entropy | RefreshError::Store(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

fn jwt_status(error: &crate::internal::jwt::JwtError) -> crate::internal::http::StatusCode {
    use crate::internal::http::StatusCode;
    use crate::internal::jwt::JwtError;

    match error {
        JwtError::TooLarge | JwtError::Malformed => StatusCode::BAD_REQUEST,
        JwtError::UnsupportedAlgorithm
        | JwtError::InvalidSignature
        | JwtError::MissingClaim(_)
        | JwtError::InvalidClaim(_)
        | JwtError::Expired
        | JwtError::NotYetValid
        | JwtError::WrongIssuer
        | JwtError::WrongAudience => StatusCode::UNAUTHORIZED,
        JwtError::SecretTooShort => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn api_key_status(
    error: &crate::internal::api_key::ApiKeyError,
) -> crate::internal::http::StatusCode {
    use crate::internal::api_key::ApiKeyError;
    use crate::internal::http::StatusCode;

    match error {
        ApiKeyError::Malformed => StatusCode::BAD_REQUEST,
        ApiKeyError::NotFound | ApiKeyError::Expired | ApiKeyError::Revoked => {
            StatusCode::UNAUTHORIZED
        }
        ApiKeyError::EmptyName
        | ApiKeyError::EmptySubject
        | ApiKeyError::Entropy
        | ApiKeyError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

impl From<zhs_json::Error> for Error {
    fn from(error: zhs_json::Error) -> Self {
        match error {
            zhs_json::Error::Json(error) => Error::Json(error),
            zhs_json::Error::JsonDecode(error) => Error::JsonDecode(error),
        }
    }
}

impl From<JsonError> for Error {
    fn from(error: JsonError) -> Self {
        Error::Json(error)
    }
}

impl From<JsonDecodeError> for Error {
    fn from(error: JsonDecodeError) -> Self {
        Error::JsonDecode(error)
    }
}

impl From<SchedulerError> for Error {
    fn from(error: SchedulerError) -> Self {
        Error::Scheduler(error)
    }
}

impl From<ThreadPoolError> for Error {
    fn from(error: ThreadPoolError) -> Self {
        Error::ThreadPool(error)
    }
}

impl From<WatchdogError> for Error {
    fn from(error: WatchdogError) -> Self {
        Error::Watchdog(error)
    }
}

impl From<HttpError> for Error {
    fn from(error: HttpError) -> Self {
        Error::Http(error)
    }
}

impl From<ContainerError> for Error {
    fn from(error: ContainerError) -> Self {
        Error::Container(error)
    }
}

impl From<LogError> for Error {
    fn from(error: LogError) -> Self {
        Error::Log(error)
    }
}

impl From<ConfigError> for Error {
    fn from(error: ConfigError) -> Self {
        Error::Config(error)
    }
}

impl From<HostError> for Error {
    fn from(error: HostError) -> Self {
        Error::Host(error)
    }
}

impl From<JoinError> for Error {
    fn from(error: JoinError) -> Self {
        Error::Join(error)
    }
}

impl From<TimeoutError> for Error {
    fn from(error: TimeoutError) -> Self {
        Error::Timeout(error)
    }
}

impl From<RefreshError> for Error {
    fn from(error: RefreshError) -> Self {
        Error::Refresh(error)
    }
}

impl From<crate::internal::jwt::JwtError> for Error {
    fn from(error: crate::internal::jwt::JwtError) -> Self {
        Error::Jwt(error)
    }
}

impl From<crate::internal::api_key::ApiKeyError> for Error {
    fn from(error: crate::internal::api_key::ApiKeyError) -> Self {
        Error::ApiKey(error)
    }
}

impl From<ClientError> for Error {
    fn from(error: ClientError) -> Self {
        Error::Client(error)
    }
}

impl From<DataError> for Error {
    fn from(error: DataError) -> Self {
        Error::Data(error)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Json(error) => error.fmt(formatter),
            Error::JsonDecode(error) => error.fmt(formatter),
            Error::Scheduler(error) => error.fmt(formatter),
            Error::ThreadPool(error) => error.fmt(formatter),
            Error::Watchdog(error) => error.fmt(formatter),
            Error::Http(error) => error.fmt(formatter),
            Error::Container(error) => error.fmt(formatter),
            Error::Log(error) => error.fmt(formatter),
            Error::Config(error) => error.fmt(formatter),
            Error::Host(error) => error.fmt(formatter),
            Error::Join(error) => error.fmt(formatter),
            Error::Timeout(error) => error.fmt(formatter),
            Error::Refresh(error) => error.fmt(formatter),
            Error::Jwt(error) => error.fmt(formatter),
            Error::ApiKey(error) => error.fmt(formatter),
            Error::Client(error) => error.fmt(formatter),
            Error::Data(error) => error.fmt(formatter),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Error::Json(error) => Some(error),
            Error::JsonDecode(error) => Some(error),
            Error::Scheduler(error) => Some(error),
            Error::ThreadPool(error) => Some(error),
            Error::Watchdog(error) => Some(error),
            Error::Http(error) => Some(error),
            Error::Container(error) => Some(error),
            Error::Log(error) => Some(error),
            Error::Config(error) => Some(error),
            Error::Host(error) => Some(error),
            Error::Join(error) => Some(error),
            Error::Timeout(error) => Some(error),
            Error::Refresh(error) => Some(error),
            Error::Jwt(error) => Some(error),
            Error::ApiKey(error) => Some(error),
            Error::Client(error) => Some(error),
            Error::Data(error) => Some(error),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
