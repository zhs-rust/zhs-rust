use std::any::Any;
use std::fmt;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use crate::internal::container::Container;
use crate::internal::error::Error;
use zhs_json::{Object, Value, to_string};
use zhs_log::runtime::{LogLevel, LogRecord, Logger};

use super::context::Context;
use super::method::Method;
use super::request::Request;
use super::request_id::{REQUEST_ID_HEADER, RequestId};
use super::response::Response;
use super::server::Handler;
use super::service::{DynService, Service, ServiceFuture};
use super::status::StatusCode;

pub trait Middleware: Send + Sync {
    fn call(&self, context: Context, next: Next) -> ServiceFuture;
}

impl<F> Middleware for F
where
    F: Fn(Context, Next) -> ServiceFuture + Send + Sync,
{
    fn call(&self, context: Context, next: Next) -> ServiceFuture {
        self(context, next)
    }
}

impl Middleware for Box<dyn Middleware> {
    fn call(&self, context: Context, next: Next) -> ServiceFuture {
        (**self).call(context, next)
    }
}

pub struct Next {
    run: Box<dyn FnOnce(Context) -> ServiceFuture + Send>,
}

impl Next {
    fn new(run: impl FnOnce(Context) -> ServiceFuture + Send + 'static) -> Self {
        Self { run: Box::new(run) }
    }

    pub fn run(self, context: Context) -> ServiceFuture {
        (self.run)(context)
    }
}

impl fmt::Debug for Next {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Next(..)")
    }
}

#[derive(Clone)]
pub struct Pipeline {
    layers: Arc<Vec<Arc<dyn Middleware>>>,
    terminal: Arc<dyn DynService>,
    container: Option<Arc<Container>>,
}

impl Pipeline {
    pub fn new(terminal: impl Service + 'static) -> Self {
        Self {
            layers: Arc::new(Vec::new()),
            terminal: Arc::new(terminal),
            container: None,
        }
    }

    pub fn with_container(mut self, container: Arc<Container>) -> Self {
        self.container = Some(container);
        self
    }

    pub fn layer(mut self, middleware: impl Middleware + 'static) -> Self {
        Arc::make_mut(&mut self.layers).push(Arc::new(middleware));
        self
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    fn invoke(&self, index: usize, context: Context) -> ServiceFuture {
        let Some(layer) = self.layers.get(index) else {
            return self.terminal.call_boxed(context);
        };

        let layer = Arc::clone(layer);
        let rest = self.clone();
        let next = Next::new(move |context| rest.invoke(index + 1, context));
        layer.call(context, next)
    }
}

impl fmt::Debug for Pipeline {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Pipeline")
            .field("layers", &self.layers.len())
            .finish()
    }
}

impl Service for Pipeline {
    type Future = ServiceFuture;

    fn call(&self, mut context: Context) -> Self::Future {
        context.inject_container(self.container.as_ref());
        self.invoke(0, context)
    }
}

impl Handler for Pipeline {
    type Future = ServiceFuture;

    fn handle(&self, request: Request) -> Self::Future {
        self.call(Context::new(request))
    }
}

pub struct ErrorHandler {
    mapper: Arc<dyn Fn(&Error) -> Response + Send + Sync>,
    logger: Option<Logger>,
}

impl ErrorHandler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_mapper<F>(mut self, mapper: F) -> Self
    where
        F: Fn(&Error) -> Response + Send + Sync + 'static,
    {
        self.mapper = Arc::new(mapper);
        self
    }

    pub fn with_logger(mut self, logger: Logger) -> Self {
        self.logger = Some(logger);
        self
    }
}

impl Default for ErrorHandler {
    fn default() -> Self {
        Self {
            mapper: Arc::new(problem_details),
            logger: None,
        }
    }
}

impl Middleware for ErrorHandler {
    fn call(&self, context: Context, next: Next) -> ServiceFuture {
        let mapper = Arc::clone(&self.mapper);
        let logger = self.logger.clone();
        let method = context.method().clone();
        let path = context.path().to_owned();
        let request_id = RequestId::of(&context)
            .or_else(|| context.headers().get(REQUEST_ID_HEADER).map(str::to_owned));

        Box::pin(async move {
            match CatchUnwind::new(next.run(context)).await {
                Ok(Ok(response)) => Ok(response),
                Ok(Err(error)) => {
                    let status = error.status();
                    if status.is_server_error() {
                        let detail = error.to_string();
                        log_server_error(
                            logger.as_ref(),
                            status,
                            &method,
                            &path,
                            request_id.as_deref(),
                            Some(&detail),
                        );
                    }
                    Ok(mapper(&error))
                }
                Err(payload) => {
                    log_server_error(
                        logger.as_ref(),
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &method,
                        &path,
                        request_id.as_deref(),
                        Some(&panic_detail(payload.as_ref())),
                    );

                    resume_unwind(payload)
                }
            }
        })
    }
}

struct CatchUnwind {
    future: ServiceFuture,
}

impl CatchUnwind {
    fn new(future: ServiceFuture) -> Self {
        Self { future }
    }
}

impl Future for CatchUnwind {
    type Output =
        std::result::Result<crate::internal::error::Result<Response>, Box<dyn Any + Send>>;

    fn poll(self: Pin<&mut Self>, task: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match catch_unwind(AssertUnwindSafe(|| this.future.as_mut().poll(task))) {
            Ok(Poll::Ready(result)) => Poll::Ready(Ok(result)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(payload) => Poll::Ready(Err(payload)),
        }
    }
}

fn panic_detail(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "panic with a non-string payload".to_owned()
    }
}

pub(crate) fn log_server_error(
    logger: Option<&Logger>,
    status: StatusCode,
    method: &Method,
    path: &str,
    request_id: Option<&str>,
    detail: Option<&str>,
) {
    let logger = match logger {
        Some(logger) => Some(logger),
        None => zhs_log::runtime::global(),
    };
    let Some(logger) = logger else {
        return;
    };

    let mut record = LogRecord::new(
        LogLevel::Error,
        "zhs_framework::http",
        format!("event=server_error {method} {path} -> {status}"),
    )
    .with_field("event", "server_error")
    .with_field("status", status.as_u16().to_string())
    .with_field("method", method.as_str().to_owned())
    .with_field("path", path.to_owned());

    if let Some(request_id) = request_id {
        record = record.with_field("request_id", request_id.to_owned());
    }
    if let Some(detail) = detail {
        record = record.with_field("detail", detail.to_owned());
    }

    let _ = logger.log(record);
}

pub fn problem_details(error: &Error) -> Response {
    let status = error.status();
    let detail = if status.is_client_error() {
        Some(error.to_string())
    } else {
        None
    };

    problem_response(status, detail.as_deref())
}

pub(crate) fn problem_response(status: StatusCode, detail: Option<&str>) -> Response {
    let mut problem = Object::new();
    problem.insert("type", Value::from("about:blank"));
    problem.insert("title", Value::from(title_for(status)));
    problem.insert("status", Value::from(u64::from(status.as_u16())));

    if let Some(detail) = detail {
        problem.insert("detail", Value::from(detail));
    }

    Response::new(status)
        .with_header("Content-Type", "application/problem+json; charset=utf-8")
        .with_body(to_string(&Value::from(problem)))
}

fn title_for(status: StatusCode) -> &'static str {
    match status.reason_phrase() {
        "" => "HTTP Error",
        reason => reason,
    }
}
