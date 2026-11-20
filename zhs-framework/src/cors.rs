use zhs_http::{Method, Response, StatusCode};

#[derive(Debug, Clone, Default)]
pub struct CorsOptions {
    allowed_origins: Vec<String>,
    allowed_methods: Vec<String>,
    allowed_headers: Vec<String>,
    allow_credentials: bool,
    max_age: Option<u64>,
}

impl CorsOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow_origin(mut self, origin: impl Into<String>) -> Self {
        self.allowed_origins.push(origin.into());
        self
    }

    pub fn allow_any_origin(mut self) -> Self {
        self.allowed_origins.push("*".to_owned());
        self
    }

    pub fn allow_method(mut self, method: impl Into<String>) -> Self {
        self.allowed_methods.push(method.into());
        self
    }

    pub fn allow_methods<I, S>(mut self, methods: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_methods
            .extend(methods.into_iter().map(Into::into));
        self
    }

    pub fn allow_header(mut self, header: impl Into<String>) -> Self {
        self.allowed_headers.push(header.into());
        self
    }

    pub fn allow_headers<I, S>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_headers
            .extend(headers.into_iter().map(Into::into));
        self
    }

    pub fn allow_credentials(mut self, value: bool) -> Self {
        self.allow_credentials = value;
        self
    }

    pub fn max_age(mut self, seconds: u64) -> Self {
        self.max_age = Some(seconds);
        self
    }

    fn allows_origin(&self, origin: &str) -> bool {
        self.allowed_origins
            .iter()
            .any(|allowed| allowed == "*" || allowed.eq_ignore_ascii_case(origin))
    }

    fn any_origin(&self) -> bool {
        self.allowed_origins.iter().any(|allowed| allowed == "*")
    }

    fn allow_origin_value(&self, origin: &str) -> String {
        if self.any_origin() {
            "*".to_owned()
        } else {
            origin.to_owned()
        }
    }

    fn allow_methods_value(&self) -> String {
        if self.allowed_methods.is_empty() {
            "GET, POST, PUT, PATCH, DELETE, OPTIONS".to_owned()
        } else {
            self.allowed_methods.join(", ")
        }
    }

    fn allow_headers_value(&self) -> String {
        if self.allowed_headers.is_empty() {
            "content-type, authorization".to_owned()
        } else {
            self.allowed_headers.join(", ")
        }
    }
}

pub fn apply(
    options: CorsOptions,
    context: zhs_http::HttpContext,
    next: zhs_http::Next,
) -> zhs_http::BoxFuture<Result<Response, zhs_http::HttpError>> {
    let origin = context.request.header("origin").map(str::to_owned);

    let Some(origin) = origin else {
        return next(context);
    };

    let allowed = options.allows_origin(&origin);

    if context.request.method == Method::Options
        && context
            .request
            .header("access-control-request-method")
            .is_some()
    {
        if !allowed {
            return Box::pin(async move { Ok(Response::new(StatusCode::NO_CONTENT)) });
        }

        let mut response = Response::new(StatusCode::NO_CONTENT)
            .with_header(
                "access-control-allow-origin",
                options.allow_origin_value(&origin),
            )
            .with_header(
                "access-control-allow-methods",
                options.allow_methods_value(),
            )
            .with_header(
                "access-control-allow-headers",
                options.allow_headers_value(),
            )
            .with_header("vary", "Origin");

        if options.allow_credentials && !options.any_origin() {
            response = response.with_header("access-control-allow-credentials", "true");
        }
        if let Some(max_age) = options.max_age {
            response = response.with_header("access-control-max-age", max_age.to_string());
        }

        return Box::pin(async move { Ok(response) });
    }

    Box::pin(async move {
        let mut response = next(context).await?;

        if allowed {
            response = response
                .with_header(
                    "access-control-allow-origin",
                    options.allow_origin_value(&origin),
                )
                .with_header("vary", "Origin");

            if options.allow_credentials && !options.any_origin() {
                response = response.with_header("access-control-allow-credentials", "true");
            }
        }

        Ok(response)
    })
}
