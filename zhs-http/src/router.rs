use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::context::HttpContext;
use crate::error::HttpError;
use crate::method::Method;
use crate::response::{Response, StatusCode};

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

pub type HandlerFn =
    Arc<dyn Fn(HttpContext) -> BoxFuture<Result<Response, HttpError>> + Send + Sync>;

pub trait Controller: Send + Sync + 'static {
    fn register(router: &mut Router);
}

struct Route {
    method: Method,
    segments: Vec<Segment>,
    handler: HandlerFn,
}

enum Segment {
    Static(String),
    Parameter {
        name: String,
        constraint: Constraint,
    },
}

#[derive(Clone, Copy)]
enum Constraint {
    Any,
    Int,
    Long,
    Bool,
    Alpha,
}

impl Constraint {
    fn matches(self, value: &str) -> bool {
        match self {
            Constraint::Any => true,
            Constraint::Int => value.parse::<i32>().is_ok(),
            Constraint::Long => value.parse::<i64>().is_ok(),
            Constraint::Bool => value.parse::<bool>().is_ok(),
            Constraint::Alpha => !value.is_empty() && value.chars().all(char::is_alphabetic),
        }
    }
}

enum MatchOutcome {
    Found {
        params: HashMap<String, String>,
        handler: HandlerFn,
    },
    MethodNotAllowed(Vec<Method>),
    NotFound,
}

#[derive(Clone, Debug)]
pub struct RouteDoc {
    pub method: Method,
    pub path: String,
    pub operation_id: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub secured: bool,
}

impl RouteDoc {
    pub fn new(
        method: Method,
        path: impl Into<String>,
        operation_id: impl Into<String>,
        summary: impl Into<String>,
        tags: Vec<String>,
        secured: bool,
    ) -> Self {
        Self {
            method,
            path: path.into(),
            operation_id: operation_id.into(),
            summary: summary.into(),
            tags,
            secured,
        }
    }
}

#[derive(Default)]
pub struct Router {
    routes: Vec<Route>,
    docs: Vec<RouteDoc>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            routes: Vec::new(),
            docs: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn document(&mut self, doc: RouteDoc) {
        self.docs.push(doc);
    }

    pub fn openapi_value(&self, title: &str, version: &str) -> zhs_json::Value {
        use zhs_json::{Object, Value};

        let mut paths = Object::new();

        for doc in &self.docs {
            let method = doc.method.as_str().to_ascii_lowercase();
            let mut operation = Object::new();
            operation.insert("operationId", Value::from(doc.operation_id.as_str()));
            operation.insert("summary", Value::from(doc.summary.as_str()));

            if !doc.tags.is_empty() {
                operation.insert(
                    "tags",
                    Value::Array(
                        doc.tags
                            .iter()
                            .map(|tag| Value::from(tag.as_str()))
                            .collect(),
                    ),
                );
            }

            let parameters = path_parameters(&doc.path);
            if !parameters.is_empty() {
                operation.insert("parameters", Value::Array(parameters));
            }

            let mut responses = Object::new();
            let mut ok = Object::new();
            ok.insert("description", Value::from("OK"));
            responses.insert("200", Value::Object(ok));
            operation.insert("responses", Value::Object(responses));

            if doc.secured {
                operation.insert(
                    "security",
                    Value::Array(vec![Value::Object({
                        let mut scheme = Object::new();
                        scheme.insert("bearerAuth", Value::Array(Vec::new()));
                        scheme
                    })]),
                );
            }

            let method_value = Value::Object(operation);
            let entry = paths.get_mut(&doc.path);

            if let Some(Value::Object(object)) = entry {
                object.insert(method, method_value);
            } else {
                let mut object = Object::new();
                object.insert(method, method_value);
                paths.insert(doc.path.as_str(), Value::Object(object));
            }
        }

        let mut info = Object::new();
        info.insert("title", Value::from(title));
        info.insert("version", Value::from(version));

        let mut root = Object::new();
        root.insert("openapi", Value::from("3.0.0"));
        root.insert("info", Value::Object(info));
        root.insert("paths", Value::Object(paths));

        let mut schemes = Object::new();
        let mut bearer = Object::new();
        bearer.insert("type", Value::from("http"));
        bearer.insert("scheme", Value::from("bearer"));
        bearer.insert("bearerFormat", Value::from("JWT"));
        schemes.insert("bearerAuth", Value::Object(bearer));

        let mut components = Object::new();
        components.insert("securitySchemes", Value::Object(schemes));
        root.insert("components", Value::Object(components));

        Value::Object(root)
    }

    pub fn route(&mut self, method: Method, pattern: &str, handler: HandlerFn) {
        self.routes.push(Route {
            method,
            segments: parse_pattern(pattern),
            handler,
        });
    }

    pub fn get(&mut self, pattern: &str, handler: HandlerFn) {
        self.route(Method::Get, pattern, handler);
    }

    pub fn post(&mut self, pattern: &str, handler: HandlerFn) {
        self.route(Method::Post, pattern, handler);
    }

    pub fn put(&mut self, pattern: &str, handler: HandlerFn) {
        self.route(Method::Put, pattern, handler);
    }

    pub fn patch(&mut self, pattern: &str, handler: HandlerFn) {
        self.route(Method::Patch, pattern, handler);
    }

    pub fn delete(&mut self, pattern: &str, handler: HandlerFn) {
        self.route(Method::Delete, pattern, handler);
    }

    pub fn dispatch(&self, mut context: HttpContext) -> BoxFuture<Result<Response, HttpError>> {
        match self.match_route(context.request.method, &context.request.path) {
            MatchOutcome::Found { params, handler } => {
                context.params = params;
                handler(context)
            }
            MatchOutcome::MethodNotAllowed(allowed) => {
                let allow = allowed
                    .iter()
                    .map(|method| method.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");

                Box::pin(async move {
                    Ok(Response::text("Method Not Allowed")
                        .with_status(StatusCode::METHOD_NOT_ALLOWED)
                        .with_header("allow", allow))
                })
            }
            MatchOutcome::NotFound => Box::pin(async {
                Ok(Response::text("Not Found").with_status(StatusCode::NOT_FOUND))
            }),
        }
    }

    fn match_route(&self, method: Method, path: &str) -> MatchOutcome {
        let path_segments = split_path(path);
        let mut allowed = Vec::new();
        let mut best: Option<(usize, HashMap<String, String>, HandlerFn)> = None;

        for route in &self.routes {
            let Some(params) = route.match_path(&path_segments) else {
                continue;
            };

            if route.method == method || (method == Method::Head && route.method == Method::Get) {
                let score = route.specificity();
                let better = match &best {
                    Some((best_score, _, _)) => score > *best_score,
                    None => true,
                };

                if better {
                    best = Some((score, params, route.handler.clone()));
                }
            } else if !allowed.contains(&route.method) {
                allowed.push(route.method);
            }
        }

        if let Some((_, params, handler)) = best {
            MatchOutcome::Found { params, handler }
        } else if allowed.is_empty() {
            MatchOutcome::NotFound
        } else {
            MatchOutcome::MethodNotAllowed(allowed)
        }
    }
}

impl Route {
    fn specificity(&self) -> usize {
        self.segments
            .iter()
            .map(|segment| match segment {
                Segment::Static(_) => 2,
                Segment::Parameter {
                    constraint: Constraint::Any,
                    ..
                } => 0,
                Segment::Parameter { .. } => 1,
            })
            .sum()
    }

    fn match_path(&self, path: &[&str]) -> Option<HashMap<String, String>> {
        if self.segments.len() != path.len() {
            return None;
        }

        let mut params = HashMap::new();

        for (segment, value) in self.segments.iter().zip(path) {
            match segment {
                Segment::Static(expected) => {
                    if expected != value {
                        return None;
                    }
                }
                Segment::Parameter { name, constraint } => {
                    if value.is_empty() || !constraint.matches(value) {
                        return None;
                    }
                    params.insert(name.clone(), (*value).to_owned());
                }
            }
        }

        Some(params)
    }
}

fn parse_pattern(pattern: &str) -> Vec<Segment> {
    split_path(pattern)
        .into_iter()
        .map(|segment| {
            if let Some(inner) = segment
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
            {
                let (name, constraint) = inner.split_once(':').unwrap_or((inner, ""));
                let constraint = match constraint {
                    "int" => Constraint::Int,
                    "long" => Constraint::Long,
                    "bool" => Constraint::Bool,
                    "alpha" => Constraint::Alpha,
                    _ => Constraint::Any,
                };

                Segment::Parameter {
                    name: name.to_owned(),
                    constraint,
                }
            } else {
                Segment::Static(segment.to_owned())
            }
        })
        .collect()
}

fn path_parameters(path: &str) -> Vec<zhs_json::Value> {
    use zhs_json::{Object, Value};

    split_path(path)
        .into_iter()
        .filter_map(|segment| {
            let name = segment
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))?;
            let name = name.split_once(':').map(|(name, _)| name).unwrap_or(name);

            let mut schema = Object::new();
            schema.insert("type", Value::from("string"));

            let mut parameter = Object::new();
            parameter.insert("name", Value::from(name));
            parameter.insert("in", Value::from("path"));
            parameter.insert("required", Value::Bool(true));
            parameter.insert("schema", Value::Object(schema));

            Some(Value::Object(parameter))
        })
        .collect()
}

fn split_path(path: &str) -> Vec<&str> {
    let path = path.trim_matches('/');

    if path.is_empty() {
        Vec::new()
    } else {
        path.split('/').collect()
    }
}
