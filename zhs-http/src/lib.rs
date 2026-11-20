#![deny(unsafe_op_in_unsafe_fn)]

pub mod auth;
pub mod context;
pub mod error;
pub mod executor;
#[doc(hidden)]
pub mod internal;
pub mod method;
pub mod middleware;
pub mod multipart;
pub mod request;
pub mod response;
pub mod router;
pub mod server;

pub use context::{HttpContext, ServiceResolver, resolve_required};
pub use error::HttpError;
pub use internal::http::{Client, ClientRequest, ClientResponse};
pub use method::Method;
pub use middleware::{FnMiddleware, Middleware, Next, compose};
pub use multipart::FormFile;
pub use request::Request;
pub use response::{IntoResponse, Response, StatusCode};
pub use router::{BoxFuture, Controller, HandlerFn, RouteDoc, Router};
pub use server::{Server, ServerHandle};
