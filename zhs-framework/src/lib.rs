extern crate self as zhs;

pub mod app;
pub mod auth;
pub mod cors;
pub mod di;
pub mod error;
pub mod health;
pub mod logging;

#[cfg(feature = "ldap")]
pub mod ldap {
    pub use zhs_ldap::*;
}
pub mod rate_limit;
pub mod results;
pub mod scheduler {
    pub use zhs_scheduler::*;
}
pub mod static_files;
pub mod testing;

pub mod http {
    pub use zhs_http::*;
}

pub mod config {
    pub use zhs_config::*;
}

pub mod entity {
    pub use zhs_entity::*;
}

pub mod json {
    pub use zhs_json::*;
}

pub mod types {
    pub use zhs_types::*;
}

pub use app::{HostHandle, WebApplication, WebApplicationBuilder};
pub use di::{DiError, Injectable, ServiceCollection, ServiceProvider};
pub use error::ApiError;
pub use results::{ApiResult, Json, Results, into_response};
pub use zhs_entity::params;
pub use zhs_macros::{FromJson, FromRow, ToJson, controller, injectable, test};

pub mod prelude {
    pub use crate::WebApplication;
    pub use crate::config::{Configuration, Options};
    pub use crate::cors::CorsOptions;
    pub use crate::di::{Injectable, ServiceCollection, ServiceProvider};
    pub use crate::entity::{Database, FromRow, Params, Row, Value};
    pub use crate::error::ApiError;
    pub use crate::health::{HealthCheckRegistry, HealthReport};
    pub use crate::http::auth::{Claims, ClaimsPrincipal, JwtBearer, JwtCodec};
    pub use crate::http::multipart::FormFile;
    pub use crate::http::{Controller, HttpContext, IntoResponse, Method, Response, StatusCode};
    pub use crate::json::{FromJson, ToJson};
    pub use crate::logging::{LogLevel, Logger, LoggerFactory};
    pub use crate::rate_limit::RateLimiter;
    pub use crate::results::{ApiResult, Json, Results};
    pub use crate::static_files::StaticFiles;
    pub use crate::testing::{TestApplication, TestClient, TestResponse};
    pub use crate::types::{DateTimeOffset, Decimal, Guid};
    pub use crate::{controller, injectable};
    pub use zhs_entity::params;
    #[cfg(feature = "ldap")]
    pub use zhs_ldap::{LdapAuthenticator, LdapError, LdapOptions, LdapUser};
    pub use zhs_macros::{FromJson, FromRow, ToJson};
}
