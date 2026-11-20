use std::sync::Arc;

use crate::internal::container::Container;
use crate::internal::error::Result;

use super::context::Context;
use super::method::Method;
use super::params::Params;
use super::request::Request;
use super::response::Response;
use super::server::Handler;
use super::service::{DynService, Service, ServiceFuture};
use super::status::StatusCode;

pub struct RouteMatch {
    params: Params,
    method: Method,
    service: Arc<dyn DynService>,
}

impl RouteMatch {
    pub fn params(&self) -> &Params {
        &self.params
    }

    pub fn method(&self) -> &Method {
        &self.method
    }

    pub(crate) fn into_parts(self) -> (Params, Arc<dyn DynService>) {
        (self.params, self.service)
    }
}

pub enum RouteOutcome {
    Found(RouteMatch),

    NotFound,

    MethodNotAllowed(Vec<Method>),
}

#[derive(Clone)]
pub struct Router {
    inner: Arc<RouterInner>,
}

#[derive(Clone)]
struct RouterInner {
    routes: Vec<Route>,
    fallback: Option<Arc<dyn DynService>>,
    container: Option<Arc<Container>>,
}

#[derive(Clone)]
struct Route {
    method: Method,
    pattern: Pattern,
    service: Arc<dyn DynService>,
}

#[derive(Clone)]
struct Pattern {
    segments: Vec<Segment>,
}

#[derive(Clone)]
enum Segment {
    Static(String),
    Param(String),

    Wildcard(String),
}

impl Pattern {
    fn parse(path: &str) -> Self {
        assert!(
            path.starts_with('/'),
            "route path must start with '/': {path}"
        );

        if path == "/" {
            return Self {
                segments: Vec::new(),
            };
        }

        let mut segments = Vec::new();
        let mut names: Vec<&str> = Vec::new();

        for piece in path.strip_prefix('/').unwrap().split('/') {
            assert!(
                !piece.is_empty(),
                "route path contains an empty segment: {path}"
            );

            let segment = match piece
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
            {
                Some(name) if name.starts_with('*') => {
                    let name = &name[1..];
                    assert!(
                        !name.is_empty() && !name.contains('{') && !name.contains('}'),
                        "invalid wildcard parameter in `{path}`"
                    );
                    assert!(
                        !names.contains(&name),
                        "duplicate route parameter `{name}` in `{path}`"
                    );
                    names.push(name);
                    Segment::Wildcard(name.to_owned())
                }
                Some(name) => {
                    assert!(
                        !name.is_empty() && !name.contains('{') && !name.contains('}'),
                        "invalid route parameter in `{path}`"
                    );
                    assert!(
                        !names.contains(&name),
                        "duplicate route parameter `{name}` in `{path}`"
                    );
                    names.push(name);
                    Segment::Param(name.to_owned())
                }
                None => {
                    assert!(
                        !piece.contains('{') && !piece.contains('}'),
                        "invalid route segment `{piece}` in `{path}`"
                    );
                    Segment::Static(piece.to_owned())
                }
            };

            segments.push(segment);
        }

        assert!(
            segments
                .iter()
                .position(|segment| matches!(segment, Segment::Wildcard(_)))
                .is_none_or(|index| index + 1 == segments.len()),
            "route wildcard must be the last segment: {path}"
        );

        Self { segments }
    }

    fn matches(&self, segments: &[&str]) -> Option<(Params, usize)> {
        let wildcard = self
            .segments
            .iter()
            .position(|segment| matches!(segment, Segment::Wildcard(_)));

        if let Some(index) = wildcard {
            if segments.len() < index {
                return None;
            }
        } else if self.segments.len() != segments.len() {
            return None;
        }

        let mut params = Params::new();
        let mut score = 0;

        for (index, pattern) in self.segments.iter().enumerate() {
            match pattern {
                Segment::Static(expected) => {
                    let actual = segments.get(index)?;
                    if expected != actual {
                        return None;
                    }
                    score += 1;
                }
                Segment::Param(name) => {
                    let actual = segments.get(index)?;
                    params.push(name.clone(), (*actual).to_owned());
                }
                Segment::Wildcard(name) => {
                    params.push(name.clone(), segments[index..].join("/"));
                }
            }
        }

        Some((params, score))
    }
}

impl Router {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RouterInner {
                routes: Vec::new(),
                fallback: None,
                container: None,
            }),
        }
    }

    pub fn route(mut self, method: Method, path: &str, handler: impl Service + 'static) -> Self {
        let pattern = Pattern::parse(path);
        let service: Arc<dyn DynService> = Arc::new(handler);
        Arc::make_mut(&mut self.inner).routes.push(Route {
            method,
            pattern,
            service,
        });
        self
    }

    pub fn get(self, path: &str, handler: impl Service + 'static) -> Self {
        self.route(Method::Get, path, handler)
    }

    pub fn post(self, path: &str, handler: impl Service + 'static) -> Self {
        self.route(Method::Post, path, handler)
    }

    pub fn put(self, path: &str, handler: impl Service + 'static) -> Self {
        self.route(Method::Put, path, handler)
    }

    pub fn patch(self, path: &str, handler: impl Service + 'static) -> Self {
        self.route(Method::Patch, path, handler)
    }

    pub fn delete(self, path: &str, handler: impl Service + 'static) -> Self {
        self.route(Method::Delete, path, handler)
    }

    pub fn head(self, path: &str, handler: impl Service + 'static) -> Self {
        self.route(Method::Head, path, handler)
    }

    pub fn options(self, path: &str, handler: impl Service + 'static) -> Self {
        self.route(Method::Options, path, handler)
    }

    pub fn fallback(mut self, handler: impl Service + 'static) -> Self {
        let service: Arc<dyn DynService> = Arc::new(handler);
        Arc::make_mut(&mut self.inner).fallback = Some(service);
        self
    }

    pub fn merge(mut self, other: Router) -> Self {
        let other = Arc::try_unwrap(other.inner).unwrap_or_else(|inner| (*inner).clone());
        let inner = Arc::make_mut(&mut self.inner);
        inner.routes.extend(other.routes);
        if inner.fallback.is_none() {
            inner.fallback = other.fallback;
        }
        if inner.container.is_none() {
            inner.container = other.container;
        }
        self
    }

    pub fn with_container(mut self, container: Arc<Container>) -> Self {
        Arc::make_mut(&mut self.inner).container = Some(container);
        self
    }

    pub fn match_route(&self, method: &Method, path: &str) -> RouteOutcome {
        let segments = split_path(path);
        let mut best: Option<(usize, &Route, Params)> = None;
        let mut allowed = Vec::new();
        let mut path_matched = false;

        for route in &self.inner.routes {
            let Some((params, score)) = route.pattern.matches(&segments) else {
                continue;
            };

            path_matched = true;

            if &route.method == method {
                let better = match &best {
                    Some((best_score, _, _)) => score > *best_score,
                    None => true,
                };
                if better {
                    best = Some((score, route, params));
                }
            } else {
                allowed.push(route.method.clone());
            }
        }

        if let Some((_, route, params)) = best {
            return RouteOutcome::Found(RouteMatch {
                params,
                method: route.method.clone(),
                service: Arc::clone(&route.service),
            });
        }

        if path_matched {
            RouteOutcome::MethodNotAllowed(normalise_allowed(allowed))
        } else {
            RouteOutcome::NotFound
        }
    }

    pub fn execute(&self, matched: RouteMatch, mut context: Context) -> ServiceFuture {
        context.inject_container(self.inner.container.as_ref());
        let (params, service) = matched.into_parts();
        context.set_params(params);
        service.call_boxed(context)
    }

    pub async fn dispatch(&self, context: Context) -> Result<Response> {
        let method = context.method().clone();
        let mut outcome = self.match_route(&method, context.path());

        if method == Method::Head
            && matches!(
                &outcome,
                RouteOutcome::NotFound | RouteOutcome::MethodNotAllowed(_)
            )
        {
            outcome = self.match_route(&Method::Get, context.path());
        }

        match outcome {
            RouteOutcome::Found(matched) => self.execute(matched, context).await,
            RouteOutcome::NotFound => match &self.inner.fallback {
                Some(fallback) => fallback.call_boxed(context).await,
                None => Ok(not_found()),
            },
            RouteOutcome::MethodNotAllowed(allowed) => Ok(method_not_allowed(&allowed)),
        }
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

impl Service for Router {
    type Future = ServiceFuture;

    fn call(&self, mut context: Context) -> Self::Future {
        context.inject_container(self.inner.container.as_ref());
        let router = self.clone();
        Box::pin(async move { router.dispatch(context).await })
    }
}

impl Handler for Router {
    type Future = ServiceFuture;

    fn handle(&self, request: Request) -> Self::Future {
        self.call(Context::new(request))
    }
}

fn split_path(path: &str) -> Vec<&str> {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    if trimmed.is_empty() {
        Vec::new()
    } else {
        trimmed.split('/').collect()
    }
}

fn normalise_allowed(mut allowed: Vec<Method>) -> Vec<Method> {
    let mut result: Vec<Method> = Vec::new();
    for method in allowed.drain(..) {
        if !result
            .iter()
            .any(|existing| existing.as_str() == method.as_str())
        {
            result.push(method);
        }
    }

    if result.iter().any(|method| method.as_str() == "GET")
        && !result.iter().any(|method| method.as_str() == "HEAD")
        && let Some(index) = result.iter().position(|method| method.as_str() == "GET")
    {
        result.insert(index + 1, Method::Head);
    }

    result
}

fn not_found() -> Response {
    Response::new(StatusCode::NOT_FOUND)
        .with_header("Content-Type", "text/plain; charset=utf-8")
        .with_body("404 Not Found\n")
}

fn method_not_allowed(allowed: &[Method]) -> Response {
    let allow = allowed
        .iter()
        .map(Method::as_str)
        .collect::<Vec<_>>()
        .join(", ");

    Response::new(StatusCode::METHOD_NOT_ALLOWED)
        .with_header("Content-Type", "text/plain; charset=utf-8")
        .with_header("Allow", allow)
        .with_body("405 Method Not Allowed\n")
}
