mod auth;
mod body;
mod client;
mod connection;
mod context;
mod cors;
mod csrf;
mod date;
mod fields;
mod headers;
mod health;
mod method;
mod metrics;
mod multipart;
mod params;
mod parse;
mod pipeline;
mod proxy;
mod random;
mod rate_limit;
mod reactor;
mod reader;
mod request;
mod request_id;
mod request_log;
mod response;
mod router;
mod server;
mod service;
mod static_files;
mod status;
mod throttle;
mod timeout;
mod version;
mod websocket;
mod write;

pub use auth::{
    ApiKey, Authentication, Authenticator, Authorization, Bearer, Cookie, Jwt, Principal,
    Requirement, bearer_token, cookie_value,
};
pub use body::{BodyChunk, BodyChunkFuture, BodyStream, ChunkList, RequestBody, ResponseBody};
pub use client::{
    Client, ClientBuilder, ClientRequest, ClientResponse, Connector, DEFAULT_CONNECT_TIMEOUT,
    DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_IDLE_PER_HOST, DEFAULT_MAX_REDIRECTS,
    DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_MAX_WORKERS, DEFAULT_READ_TIMEOUT, DEFAULT_USER_AGENT,
    DEFAULT_WRITE_TIMEOUT, TcpConnector, Transport,
};
pub use context::Context;
pub use cors::Cors;
pub use csrf::Csrf;
pub use date::format_http_date;
pub use fields::{Fields, FromQuery};
pub use headers::HeaderMap;
pub use health::{Health, HealthProbe};
pub use method::Method;
pub use metrics::{DURATION_BUCKETS, Metrics};
pub use multipart::{Multipart, MultipartLimits, Part, sanitize_file_name};
pub use params::{FromPath, Params};
pub use parse::{BodyFraming, HttpLimits, RequestHead, read_body, read_request, read_request_head};
pub use pipeline::{ErrorHandler, Middleware, Next, Pipeline, problem_details};
pub use proxy::{FORWARDED_FOR, TrustedProxies};
pub use rate_limit::{DEFAULT_MAX_ENTRIES, Quota, RateLimit};
pub use request::Request;
pub use request_id::{MAX_REQUEST_ID_LENGTH, REQUEST_ID_HEADER, RequestId};
pub use request_log::RequestLogger;
pub use response::Response;
pub use router::{RouteMatch, RouteOutcome, Router};
pub use server::{Handler, Server, ServerConfig, ServerHandle, ServerStats, ServerStatsSource};
pub use service::{Service, ServiceFuture, ServiceHandler};
pub use static_files::StaticFiles;
pub use status::StatusCode;
pub use throttle::Throttle;
pub use timeout::Timeout;
pub use version::Version;
pub use websocket::{
    CloseFrame, Message, WebSocket, WebSocketHandler, WebSocketLimits, WebSocketUpgrade,
};

pub fn reactor_backend_name() -> &'static str {
    reactor::backend_name()
}
