use super::context::Context;
use super::headers::HeaderMap;
use super::method::Method;
use super::pipeline::{Middleware, Next, problem_response};
use super::response::Response;
use super::service::ServiceFuture;
use super::status::StatusCode;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Origins {
    Any,

    List(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct Cors {
    origins: Origins,
    methods: Vec<Method>,
    allowed_headers: Option<Vec<String>>,
    exposed_headers: Vec<String>,
    credentials: bool,
    max_age: Option<u64>,
}

impl Cors {
    pub fn new() -> Self {
        Self {
            origins: Origins::List(Vec::new()),
            methods: vec![Method::Get, Method::Head, Method::Post],
            allowed_headers: None,
            exposed_headers: Vec::new(),
            credentials: false,
            max_age: None,
        }
    }

    pub fn allow_origin(mut self, origin: impl Into<String>) -> Self {
        let origin = origin.into();
        match &mut self.origins {
            Origins::Any => {}
            Origins::List(list) => {
                if !list.contains(&origin) {
                    list.push(origin);
                }
            }
        }
        self
    }

    pub fn allow_origins<I, S>(mut self, origins: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for origin in origins {
            self = self.allow_origin(origin);
        }
        self
    }

    pub fn allow_any_origin(mut self) -> Self {
        self.origins = Origins::Any;
        self
    }

    pub fn allow_methods<I>(mut self, methods: I) -> Self
    where
        I: IntoIterator<Item = Method>,
    {
        self.methods = methods.into_iter().collect();
        self
    }

    pub fn allow_all_methods(mut self) -> Self {
        self.methods = vec![
            Method::Get,
            Method::Head,
            Method::Post,
            Method::Put,
            Method::Patch,
            Method::Delete,
            Method::Options,
        ];
        self
    }

    pub fn allow_headers<I, S>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_headers = Some(headers.into_iter().map(Into::into).collect());
        self
    }

    pub fn expose_headers<I, S>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.exposed_headers = headers.into_iter().map(Into::into).collect();
        self
    }

    pub fn allow_credentials(mut self, allowed: bool) -> Self {
        self.credentials = allowed;
        self
    }

    pub fn max_age(mut self, seconds: u64) -> Self {
        self.max_age = Some(seconds);
        self
    }

    fn origin_allowed(&self, origin: &str) -> bool {
        match &self.origins {
            Origins::Any => true,
            Origins::List(list) => list.iter().any(|allowed| allowed == origin),
        }
    }

    fn any_origin(&self) -> bool {
        matches!(self.origins, Origins::Any)
    }

    fn echo_origin(&self) -> bool {
        !self.any_origin()
    }

    fn methods_header(&self) -> String {
        self.methods
            .iter()
            .map(Method::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn preflight_response(&self, origin: &str, requested_headers: Option<&str>) -> Response {
        let mut response = Response::new(StatusCode::NO_CONTENT);

        if self.echo_origin() {
            response = response
                .with_header("Access-Control-Allow-Origin", origin)
                .with_header("Vary", "Origin");
        } else {
            response = response.with_header("Access-Control-Allow-Origin", "*");
        }

        response = response.with_header("Access-Control-Allow-Methods", self.methods_header());

        match (&self.allowed_headers, requested_headers) {
            (Some(allowed), _) => {
                response = response.with_header("Access-Control-Allow-Headers", allowed.join(", "));
            }
            (None, Some(requested)) if !requested.is_empty() => {
                response =
                    response.with_header("Access-Control-Allow-Headers", requested.to_owned());
            }
            _ => {}
        }

        if self.credentials && !self.any_origin() {
            response = response.with_header("Access-Control-Allow-Credentials", "true");
        }

        if let Some(max_age) = self.max_age {
            response = response.with_header("Access-Control-Max-Age", max_age.to_string());
        }

        response
    }
}

impl Default for Cors {
    fn default() -> Self {
        Self::new()
    }
}

impl Middleware for Cors {
    fn call(&self, context: Context, next: Next) -> ServiceFuture {
        let Some(origin) = context.headers().get("origin").map(str::to_owned) else {
            return next.run(context);
        };

        let preflight = context.method() == &Method::Options
            && context.headers().contains("access-control-request-method");

        if preflight {
            if !self.origin_allowed(&origin) {
                return forbidden("origin is not allowed");
            }

            let requested_method = context
                .headers()
                .get("access-control-request-method")
                .unwrap_or_default()
                .trim();

            let method_allowed = Method::parse(requested_method)
                .is_some_and(|method| self.methods.contains(&method));
            if !method_allowed {
                return forbidden("method is not allowed");
            }

            let requested_headers = context
                .headers()
                .get("access-control-request-headers")
                .map(str::to_owned);

            if let (Some(allowed), Some(requested)) = (&self.allowed_headers, &requested_headers) {
                let unsupported = requested
                    .split(',')
                    .map(str::trim)
                    .filter(|header| !header.is_empty())
                    .any(|header| {
                        !allowed
                            .iter()
                            .any(|allowed| allowed.eq_ignore_ascii_case(header))
                    });

                if unsupported {
                    return forbidden("request header is not allowed");
                }
            }

            let response = self.preflight_response(&origin, requested_headers.as_deref());
            return Box::pin(async move { Ok(response) });
        }

        if !self.origin_allowed(&origin) {
            return next.run(context);
        }

        let echo = self.echo_origin();
        let credentials = self.credentials;
        let any_origin = self.any_origin();
        let exposed = if self.exposed_headers.is_empty() {
            None
        } else {
            Some(self.exposed_headers.join(", "))
        };

        Box::pin(async move {
            let mut response = next.run(context).await?;
            let headers = response.headers_mut();

            if echo {
                headers.insert("Access-Control-Allow-Origin", origin);
                append_vary(headers, "Origin");
            } else {
                headers.insert("Access-Control-Allow-Origin", "*");
            }

            if credentials && !any_origin {
                headers.insert("Access-Control-Allow-Credentials", "true");
            }
            if let Some(exposed) = exposed {
                headers.insert("Access-Control-Expose-Headers", exposed);
            }

            Ok(response)
        })
    }
}

fn forbidden(detail: &str) -> ServiceFuture {
    let response = problem_response(StatusCode::FORBIDDEN, Some(detail));
    Box::pin(async move { Ok(response) })
}

fn append_vary(headers: &mut HeaderMap, value: &str) {
    let present = headers
        .get_all("vary")
        .flat_map(|vary| vary.split(','))
        .any(|existing| existing.trim().eq_ignore_ascii_case(value));

    if !present {
        headers.append("Vary", value);
    }
}

#[cfg(test)]
mod tests {
    use super::{Cors, append_vary};
    use crate::internal::http::HeaderMap;

    #[test]
    fn new_policy_allows_no_origin() {
        let cors = Cors::new();
        assert!(!cors.origin_allowed("https://example.com"));
    }

    #[test]
    fn any_origin_never_echoes_or_emits_credentials() {
        let cors = Cors::new().allow_any_origin().allow_credentials(true);
        assert!(cors.origin_allowed("https://example.com"));
        assert!(!cors.echo_origin());
        assert!(cors.any_origin());

        let cors = Cors::new().allow_any_origin();
        assert!(!cors.echo_origin());

        let cors = Cors::new()
            .allow_origin("https://example.com")
            .allow_credentials(true);
        assert!(cors.echo_origin());
        assert!(!cors.any_origin());
    }

    #[test]
    fn vary_is_not_duplicated() {
        let mut headers = HeaderMap::new();
        append_vary(&mut headers, "Origin");
        append_vary(&mut headers, "origin");
        append_vary(&mut headers, "Accept-Encoding");

        let values: Vec<&str> = headers.get_all("vary").collect();
        assert_eq!(values, vec!["Origin", "Accept-Encoding"]);
    }
}
