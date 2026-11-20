use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use zhs_config::Configuration;
use zhs_http::auth::{JwtBearer, JwtCodec};
use zhs_http::{
    FnMiddleware, HandlerFn, HttpContext, Middleware, Next, Router, Server, ServerHandle,
    ServiceResolver, compose,
};
use zhs_log::{Log, LogLevel, LoggingBuilder};
use zhs_types::Guid;

use crate::di::{DiError, ServiceCollection, ServiceProvider};

#[derive(Debug, Clone)]
pub struct Environment {
    name: String,
}

impl Environment {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_development(&self) -> bool {
        self.name.eq_ignore_ascii_case("Development")
    }

    pub fn is_production(&self) -> bool {
        self.name.eq_ignore_ascii_case("Production")
    }
}

pub struct WebApplicationBuilder {
    pub services: ServiceCollection,
    pub configuration: Configuration,
    pub logging: LoggingBuilder,
    pub environment: Environment,
    address: String,
}

impl WebApplicationBuilder {
    pub fn address(mut self, address: impl Into<String>) -> Self {
        self.address = address.into();
        self
    }

    pub fn build(self) -> Result<WebApplication, DiError> {
        let mut services = self.services;
        services.add_singleton_instance(self.logging.build());
        services.add_singleton_instance(self.configuration.clone());

        let controllers = services.take_controllers();
        let provider = Arc::new(services.build()?);

        Ok(WebApplication {
            provider,
            router: Router::new(),
            controllers,
            middlewares: Vec::new(),
            mounted: false,
            address: self.address,
        })
    }
}

pub struct WebApplication {
    provider: Arc<ServiceProvider>,
    router: Router,
    controllers: Vec<fn(&mut Router)>,
    middlewares: Vec<Arc<dyn Middleware>>,
    mounted: bool,
    address: String,
}

impl WebApplication {
    pub fn builder(args: impl IntoIterator<Item = String>) -> WebApplicationBuilder {
        let arguments = args.into_iter().collect::<Vec<_>>();
        let environment = Environment::new(resolve_environment(&arguments));
        let configuration = load_configuration(&environment, &arguments);
        let level = configuration
            .get_str("Logging:LogLevel:Default")
            .and_then(|value| value.parse::<LogLevel>().ok())
            .unwrap_or(LogLevel::Information);
        let mut logging = LoggingBuilder::new();
        logging.set_minimum_level(level);

        WebApplicationBuilder {
            services: ServiceCollection::new(),
            configuration,
            logging,
            environment,
            address: resolve_address(&arguments),
        }
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn services(&self) -> &Arc<ServiceProvider> {
        &self.provider
    }

    pub fn map_controllers(&mut self) {
        if self.mounted {
            return;
        }

        for register in &self.controllers {
            register(&mut self.router);
        }

        self.mounted = true;
    }

    pub fn use_middleware<M>(&mut self, middleware: M) -> &mut Self
    where
        M: Middleware + 'static,
    {
        self.middlewares.push(Arc::new(middleware));
        self
    }

    pub fn use_fn<F>(&mut self, middleware: F) -> &mut Self
    where
        F: Fn(
                HttpContext,
                Next,
            ) -> zhs_http::BoxFuture<Result<zhs_http::Response, zhs_http::HttpError>>
            + Send
            + Sync
            + 'static,
    {
        self.middlewares.push(Arc::new(FnMiddleware(middleware)));
        self
    }

    pub fn use_jwt_bearer(&mut self, codec: JwtCodec) -> &mut Self {
        let bearer = Arc::new(JwtBearer::from_codec(codec));

        self.use_fn(move |mut context, next| {
            let bearer = Arc::clone(&bearer);

            Box::pin(async move {
                crate::auth::authenticate(&mut context, &bearer);
                next(context).await
            })
        })
    }

    pub fn use_cors(&mut self, options: crate::cors::CorsOptions) -> &mut Self {
        self.use_fn(move |context, next| crate::cors::apply(options.clone(), context, next))
    }

    pub fn use_rate_limiting(&mut self, limiter: crate::rate_limit::RateLimiter) -> &mut Self {
        self.use_fn(move |context, next| crate::rate_limit::apply(limiter.clone(), context, next))
    }

    pub fn use_static_files(&mut self, files: crate::static_files::StaticFiles) -> &mut Self {
        self.use_fn(move |context, next| crate::static_files::apply(files.clone(), context, next))
    }

    pub fn use_request_id(&mut self) -> &mut Self {
        self.use_fn(|context, next| {
            Box::pin(async move {
                let request_id = context
                    .request
                    .header("x-request-id")
                    .map(str::to_owned)
                    .unwrap_or_else(|| Guid::new_v4().to_string());

                let mut response = next(context).await?;
                response
                    .headers
                    .push(("x-request-id".to_owned(), request_id));

                Ok(response)
            })
        })
    }

    pub fn use_request_logging(&mut self) -> &mut Self {
        let logger = Log::new(LogLevel::Information, "zhs.request");

        self.use_fn(move |context, next| {
            let logger = logger.clone();
            let method = context.request.method.to_string();
            let path = context.request.path.clone();
            let started = Instant::now();

            Box::pin(async move {
                let response = next(context).await?;
                logger.info(format!(
                    "{} {} -> {} ({} ms)",
                    method,
                    path,
                    response.status.0,
                    started.elapsed().as_millis()
                ));
                Ok(response)
            })
        })
    }

    pub fn map_health_checks(&mut self, path: impl Into<String>) -> &mut Self {
        self.router
            .get(&path.into(), crate::health::health_handler());
        self
    }

    pub fn map_openapi(&mut self, path: &str, title: &str, version: &str) -> &mut Self {
        self.map_controllers();
        let document = zhs_json::to_string(&self.router.openapi_value(title, version));

        self.router.get(
            path,
            Arc::new(move |_context| {
                let document = document.clone();
                Box::pin(async move { Ok(zhs_http::Response::json_raw(document)) })
            }),
        );
        self
    }

    pub fn use_swagger_ui(&mut self, path: &str, openapi_path: &str) -> &mut Self {
        // `path` is only used as a route pattern; `openapi_path` is interpolated
        // into the generated HTML, so escape it for the HTML/JS context.
        let openapi_path = escape_html_js(openapi_path);
        let html = format!(
            r##"<!doctype html>
<html>
<head>
  <meta charset="utf-8" />
  <title>zhs API</title>
  <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css" />
</head>
<body>
  <div id="swagger-ui"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
  <script>
    SwaggerUIBundle({{ url: "{openapi_path}", dom_id: "#swagger-ui" }});
  </script>
</body>
</html>"##
        );

        self.router.get(
            path,
            Arc::new(move |_context| {
                let html = html.clone();
                Box::pin(async move { Ok(zhs_http::Response::html(html)) })
            }),
        );
        self
    }

    pub fn run(self) -> io::Result<()> {
        self.spawn()?.wait()
    }

    pub fn spawn(mut self) -> io::Result<HostHandle> {
        self.map_controllers();

        let server = Server::bind(&self.address)?;
        let address = server.local_addr();
        let handler = self.build_handler();
        let provider: Arc<dyn ServiceResolver> = self.provider;

        let server = server.spawn_with(handler, provider)?;

        Ok(HostHandle { address, server })
    }

    fn build_handler(&mut self) -> HandlerFn {
        let router = Arc::new(std::mem::replace(&mut self.router, Router::new()));
        let endpoint: HandlerFn = Arc::new(move |context| router.dispatch(context));
        let middlewares = std::mem::take(&mut self.middlewares);

        compose(middlewares, endpoint)
    }

    pub fn run_with(mut self, address: &str) -> io::Result<()> {
        self.address = address.to_owned();
        self.run()
    }
}

pub struct HostHandle {
    address: SocketAddr,
    server: ServerHandle,
}

impl HostHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }

    pub fn is_running(&self) -> bool {
        !self.server.is_shutdown()
    }

    pub fn shutdown(&self) -> io::Result<()> {
        self.server.shutdown()
    }

    pub fn wait(self) -> io::Result<()> {
        self.server.wait()
    }
}

// Escapes a value interpolated into the generated Swagger HTML page. The value
// ends up inside a JavaScript string literal within an HTML `<script>` block, so
// the backslash is escaped first and then the HTML/JS-significant characters.
fn escape_html_js(value: &str) -> String {
    let mut output = String::with_capacity(value.len());

    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            character => output.push(character),
        }
    }

    output
}

fn resolve_address(arguments: &[String]) -> String {
    let mut address = None;
    let mut index = 0;

    while index < arguments.len() {
        let argument = &arguments[index];

        if let Some(value) = argument.strip_prefix("--urls=") {
            address = Some(value.to_owned());
        } else if argument == "--urls" && index + 1 < arguments.len() {
            address = Some(arguments[index + 1].clone());
            index += 1;
        }

        index += 1;
    }

    address
        .or_else(|| std::env::var("ZHS_URLS").ok())
        .unwrap_or_else(|| "127.0.0.1:5000".to_owned())
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .to_owned()
}

fn resolve_environment(arguments: &[String]) -> String {
    let mut index = 0;

    while index < arguments.len() {
        let argument = &arguments[index];

        if let Some(value) = argument.strip_prefix("--environment=") {
            return value.to_owned();
        }
        if argument == "--environment" && index + 1 < arguments.len() {
            return arguments[index + 1].clone();
        }

        index += 1;
    }

    std::env::var("ZHS_ENVIRONMENT").unwrap_or_else(|_| {
        if cfg!(debug_assertions) {
            "Development".to_owned()
        } else {
            "Production".to_owned()
        }
    })
}

fn load_configuration(environment: &Environment, arguments: &[String]) -> Configuration {
    let mut configuration = Configuration::new();

    let paths = [
        PathBuf::from("appsettings.json"),
        PathBuf::from(format!("appsettings.{}.json", environment.name())),
    ];

    for path in paths {
        if path.exists()
            && let Ok(loaded) = Configuration::load_file(&path)
        {
            configuration.merge(&loaded);
        }
    }

    configuration.add_environment("ZHS_");
    configuration.add_command_line(arguments.iter().cloned());

    configuration
}
