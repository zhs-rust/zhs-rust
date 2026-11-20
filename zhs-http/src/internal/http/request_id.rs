use super::context::Context;
use super::pipeline::{Middleware, Next};
use super::random::random_hex;
use super::service::ServiceFuture;

pub const REQUEST_ID_HEADER: &str = "X-Request-Id";

pub const MAX_REQUEST_ID_LENGTH: usize = 64;

#[derive(Clone, Copy, Debug, Default)]
pub struct RequestId;

impl RequestId {
    pub fn new() -> Self {
        Self
    }

    pub fn is_valid(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= MAX_REQUEST_ID_LENGTH
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    }

    pub fn of(context: &Context) -> Option<String> {
        context.get::<RequestIdValue>().map(|value| value.0.clone())
    }
}

impl Middleware for RequestId {
    fn call(&self, mut context: Context, next: Next) -> ServiceFuture {
        let id = context
            .headers()
            .get(REQUEST_ID_HEADER)
            .filter(|value| RequestId::is_valid(value))
            .map(str::to_owned)
            .unwrap_or_else(|| random_hex(16));

        context.headers_mut().insert(REQUEST_ID_HEADER, id.as_str());
        context.insert(RequestIdValue(id.clone()));

        Box::pin(async move {
            let mut response = next.run(context).await?;
            response
                .headers_mut()
                .insert(REQUEST_ID_HEADER, id.as_str());
            Ok(response)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RequestIdValue(String);
