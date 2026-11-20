use std::sync::Arc;

use crate::context::HttpContext;
use crate::error::HttpError;
use crate::response::Response;
use crate::router::HandlerFn;

pub type Next = HandlerFn;

pub trait Middleware: Send + Sync {
    fn call(
        &self,
        context: HttpContext,
        next: Next,
    ) -> crate::router::BoxFuture<Result<Response, HttpError>>;
}

pub fn compose(middlewares: Vec<Arc<dyn Middleware>>, endpoint: HandlerFn) -> HandlerFn {
    let mut handler = endpoint;

    for middleware in middlewares.into_iter().rev() {
        let next = handler;
        handler = Arc::new(move |context| middleware.call(context, Arc::clone(&next)));
    }

    handler
}

pub struct FnMiddleware<F>(pub F);

impl<F> Middleware for FnMiddleware<F>
where
    F: Fn(HttpContext, Next) -> crate::router::BoxFuture<Result<Response, HttpError>>
        + Send
        + Sync
        + 'static,
{
    fn call(
        &self,
        context: HttpContext,
        next: Next,
    ) -> crate::router::BoxFuture<Result<Response, HttpError>> {
        (self.0)(context, next)
    }
}
