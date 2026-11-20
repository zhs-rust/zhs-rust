use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::internal::container::Container;
use crate::internal::error::Result;

use super::context::Context;
use super::request::Request;
use super::response::Response;
use super::server::Handler;

pub type ServiceFuture = Pin<Box<dyn Future<Output = Result<Response>> + Send>>;

pub trait Service: Send + Sync {
    type Future: Future<Output = Result<Response>> + Send + 'static;

    fn call(&self, context: Context) -> Self::Future;
}

impl<F, Fut> Service for F
where
    F: Fn(Context) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Response>> + Send + 'static,
{
    type Future = Fut;

    fn call(&self, context: Context) -> Self::Future {
        self(context)
    }
}

pub(crate) trait DynService: Send + Sync {
    fn call_boxed(&self, context: Context) -> ServiceFuture;
}

impl<S: Service> DynService for S {
    fn call_boxed(&self, context: Context) -> ServiceFuture {
        Box::pin(self.call(context))
    }
}

pub struct ServiceHandler<S> {
    service: S,
    container: Option<Arc<Container>>,
}

impl<S> ServiceHandler<S> {
    pub fn new(service: S) -> Self {
        Self {
            service,
            container: None,
        }
    }

    pub fn with_container(mut self, container: Arc<Container>) -> Self {
        self.container = Some(container);
        self
    }
}

impl<S> Handler for ServiceHandler<S>
where
    S: Service,
{
    type Future = S::Future;

    fn handle(&self, request: Request) -> Self::Future {
        let mut context = Context::new(request);
        context.inject_container(self.container.as_ref());
        self.service.call(context)
    }
}
