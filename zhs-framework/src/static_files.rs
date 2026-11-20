use std::path::{Component, Path, PathBuf};

use zhs_http::{Method, Next, Response, StatusCode};

#[derive(Debug, Clone)]
pub struct StaticFiles {
    root: PathBuf,
    prefix: String,
    index: String,
    cache_control: Option<String>,
}

impl StaticFiles {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            prefix: String::new(),
            index: "index.html".to_owned(),
            cache_control: None,
        }
    }

    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    pub fn index(mut self, index: impl Into<String>) -> Self {
        self.index = index.into();
        self
    }

    pub fn cache_control(mut self, value: impl Into<String>) -> Self {
        self.cache_control = Some(value.into());
        self
    }
}

pub fn apply(
    options: StaticFiles,
    context: zhs_http::HttpContext,
    next: Next,
) -> zhs_http::BoxFuture<Result<Response, zhs_http::HttpError>> {
    if !matches!(context.request.method, Method::Get | Method::Head) {
        return next(context);
    }

    let path = context.request.path.clone();
    // Require a `/` boundary after the prefix so `/assetsX.json` does not
    // match a `/assets` prefix.
    let relative = match path.strip_prefix(&options.prefix) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            rest.trim_start_matches('/').to_owned()
        }
        _ => return next(context),
    };

    if relative.is_empty() {
        let index = options.index.clone();
        return serve_path(options, context, Path::new(&index), next);
    }

    let relative_path = Path::new(&relative);

    if relative_path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
        || relative.contains('\\')
        || relative.contains('\0')
    {
        return Box::pin(async move {
            Ok(Response::text("Bad Request").with_status(StatusCode::BAD_REQUEST))
        });
    }

    serve_path(options, context, relative_path, next)
}

fn serve_path(
    options: StaticFiles,
    context: zhs_http::HttpContext,
    relative: &Path,
    next: Next,
) -> zhs_http::BoxFuture<Result<Response, zhs_http::HttpError>> {
    let target = options.root.join(relative);

    if target.is_dir() {
        let index = relative.join(&options.index);
        return serve_path(options, context, &index, next);
    }

    if !target.is_file() {
        return next(context);
    }

    // `std::fs::read` follows symlinks, so the lexical `..` check in `apply` is
    // not enough: a symlink inside the root can point anywhere. Resolve both
    // sides and require the target to stay inside the root.
    let Ok(canonical_root) = options.root.canonicalize() else {
        return next(context);
    };

    let Ok(canonical_target) = target.canonicalize() else {
        return next(context);
    };

    if !canonical_target.starts_with(canonical_root) {
        return next(context);
    }

    let content = match std::fs::read(&target) {
        Ok(content) => content,
        Err(_) => return next(context),
    };

    let content_type = content_type(&target);
    let mut response = Response::new(StatusCode::OK)
        .with_header("content-type", content_type)
        .with_body(content);

    if let Some(cache_control) = &options.cache_control {
        response = response.with_header("cache-control", cache_control);
    }

    Box::pin(async move { Ok(response) })
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}
