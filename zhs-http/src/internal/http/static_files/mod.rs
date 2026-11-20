mod file_stream;
mod mime;

use std::fmt;
use std::fs::{self, File, Metadata};
use std::io::{self, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use zhs_mn::thread_pool::ThreadPool;

use super::context::Context;
use super::date::{format_http_date, parse_http_date};
use super::method::Method;
use super::response::Response;
use super::service::{Service, ServiceFuture};
use super::status::StatusCode;

use file_stream::FileStream;

const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;

#[derive(Clone)]
pub struct StaticFiles {
    inner: Arc<Inner>,
}

#[derive(Clone)]
struct Inner {
    root: PathBuf,
    prefix: String,
    index: Option<String>,
    cache_control: Option<String>,
    directory_listing: bool,
    pool: ThreadPool,
    chunk_size: usize,
}

impl StaticFiles {
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = fs::canonicalize(root.as_ref())?;
        let metadata = fs::metadata(&root)?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the static files root must be a directory",
            ));
        }

        let pool = ThreadPool::with_available_parallelism().map_err(io::Error::other)?;

        Ok(Self {
            inner: Arc::new(Inner {
                root,
                prefix: String::new(),
                index: Some("index.html".to_owned()),
                cache_control: None,
                directory_listing: false,
                pool,
                chunk_size: DEFAULT_CHUNK_SIZE,
            }),
        })
    }

    pub fn with_prefix(mut self, prefix: impl AsRef<str>) -> Self {
        let prefix = normalize_prefix(prefix.as_ref());
        Arc::make_mut(&mut self.inner).prefix = prefix;
        self
    }

    pub fn with_index(mut self, index: impl Into<String>) -> Self {
        let index = index.into();
        assert!(
            !index.is_empty()
                && !index.contains('/')
                && !index.contains('\\')
                && index != ".."
                && index != ".",
            "the static files index must be a plain file name"
        );
        Arc::make_mut(&mut self.inner).index = Some(index);
        self
    }

    pub fn without_index(mut self) -> Self {
        Arc::make_mut(&mut self.inner).index = None;
        self
    }

    pub fn with_directory_listing(mut self, enabled: bool) -> Self {
        Arc::make_mut(&mut self.inner).directory_listing = enabled;
        self
    }

    pub fn with_cache_control(mut self, value: impl Into<String>) -> Self {
        let value = value.into();
        assert!(
            !value.chars().any(char::is_control),
            "the Cache-Control value must not contain control characters"
        );
        Arc::make_mut(&mut self.inner).cache_control = Some(value);
        self
    }

    pub fn with_thread_pool(mut self, pool: ThreadPool) -> Self {
        Arc::make_mut(&mut self.inner).pool = pool;
        self
    }

    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        assert!(
            chunk_size > 0,
            "the static files chunk size must be positive"
        );
        Arc::make_mut(&mut self.inner).chunk_size = chunk_size;
        self
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    pub fn prefix(&self) -> &str {
        &self.inner.prefix
    }

    pub fn index(&self) -> Option<&str> {
        self.inner.index.as_deref()
    }

    pub fn cache_control(&self) -> Option<&str> {
        self.inner.cache_control.as_deref()
    }

    pub fn directory_listing(&self) -> bool {
        self.inner.directory_listing
    }
}

impl Service for StaticFiles {
    type Future = ServiceFuture;

    fn call(&self, context: Context) -> ServiceFuture {
        let method = context.method().clone();
        let path = context.path().to_owned();
        let target = context.request().target().to_owned();
        let if_none_match = context.headers().get("if-none-match").map(str::to_owned);
        let if_modified_since = context
            .headers()
            .get("if-modified-since")
            .and_then(parse_http_date);
        let range = if method == Method::Get {
            context.headers().get("range").map(str::to_owned)
        } else {
            None
        };
        let if_range = if method == Method::Get {
            context.headers().get("if-range").map(str::to_owned)
        } else {
            None
        };
        let inner = Arc::clone(&self.inner);

        Box::pin(async move {
            if method != Method::Get && method != Method::Head {
                return Ok(method_not_allowed());
            }

            let Some(relative) = inner.strip_prefix(&path) else {
                return Ok(not_found());
            };
            let parsed = match ParsedPath::parse(relative) {
                Ok(parsed) => parsed,
                Err(status) => return Ok(status_response(status)),
            };

            let conditions = Conditions {
                if_none_match,
                if_modified_since,
                range,
                if_range,
            };
            let location = redirect_location(&target);
            let task_inner = Arc::clone(&inner);
            let handle = match inner
                .pool
                .try_spawn_blocking(move || task_inner.resolve(&parsed, &conditions, location))
            {
                Ok(handle) => handle,
                Err(_) => return Ok(internal_error()),
            };
            let resolved = match handle.await {
                Ok(resolved) => resolved,
                Err(_) => return Ok(internal_error()),
            };

            match resolved {
                Resolved::File {
                    file,
                    body_length,
                    total_length,
                    range,
                    modified,
                    etag,
                    content_type,
                } => {
                    let status = if range.is_some() {
                        StatusCode::PARTIAL_CONTENT
                    } else {
                        StatusCode::OK
                    };
                    let response = if method == Method::Head {
                        Response::new(status).with_content_length(body_length)
                    } else {
                        Response::stream_with_length(
                            FileStream::new(
                                file,
                                inner.pool.clone(),
                                body_length,
                                inner.chunk_size,
                            ),
                            body_length,
                        )
                        .with_status(status)
                    };
                    let mut response = response
                        .with_header("Content-Type", content_type)
                        .with_header("Accept-Ranges", "bytes");
                    if let Some((start, end)) = range {
                        response = response.with_header(
                            "Content-Range",
                            format!("bytes {start}-{end}/{total_length}"),
                        );
                    }
                    Ok(inner.decorate(response, &etag, modified))
                }
                Resolved::NotModified { etag, modified } => {
                    Ok(inner.decorate(Response::new(StatusCode::NOT_MODIFIED), &etag, modified))
                }
                Resolved::RangeNotSatisfiable { total_length } => {
                    Ok(status_response(StatusCode::RANGE_NOT_SATISFIABLE)
                        .with_header("Content-Range", format!("bytes */{total_length}"))
                        .with_header("Accept-Ranges", "bytes"))
                }
                Resolved::Listing { html } => {
                    Ok(Response::html(html).with_header("Cache-Control", "no-store"))
                }
                Resolved::Redirect(location) => {
                    Ok(Response::new(StatusCode::MOVED_PERMANENTLY)
                        .with_header("Location", location))
                }
                Resolved::NotFound => Ok(not_found()),
                Resolved::Error => Ok(internal_error()),
            }
        })
    }
}

impl fmt::Debug for StaticFiles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StaticFiles")
            .field("root", &self.inner.root)
            .field("prefix", &self.inner.prefix)
            .field("index", &self.inner.index)
            .field("cache_control", &self.inner.cache_control)
            .field("directory_listing", &self.inner.directory_listing)
            .field("chunk_size", &self.inner.chunk_size)
            .finish_non_exhaustive()
    }
}

impl Inner {
    fn strip_prefix<'a>(&self, path: &'a str) -> Option<&'a str> {
        if self.prefix.is_empty() {
            return Some(path);
        }
        if path == self.prefix {
            return Some("");
        }

        path.strip_prefix(&self.prefix)
            .filter(|rest| rest.starts_with('/'))
    }

    fn decorate(
        &self,
        mut response: Response,
        etag: &str,
        modified: Option<SystemTime>,
    ) -> Response {
        response = response.with_header("ETag", etag);
        if let Some(modified) = modified {
            response = response.with_header("Last-Modified", format_http_date(modified));
        }
        if let Some(cache_control) = &self.cache_control {
            response = response.with_header("Cache-Control", cache_control.clone());
        }
        response
    }

    fn resolve(&self, path: &ParsedPath, conditions: &Conditions, redirect: String) -> Resolved {
        let mut candidate = self.root.clone();
        for segment in &path.segments {
            candidate.push(segment);
        }

        let canonical = match fs::canonicalize(&candidate) {
            Ok(canonical) => canonical,
            Err(_) => return Resolved::NotFound,
        };
        if !canonical.starts_with(&self.root) {
            return Resolved::NotFound;
        }

        let metadata = match fs::metadata(&canonical) {
            Ok(metadata) => metadata,
            Err(_) => return Resolved::NotFound,
        };

        if metadata.is_dir() {
            if !path.trailing_slash {
                return Resolved::Redirect(redirect);
            }

            if let Some(index) = &self.index
                && let Ok(index_path) = fs::canonicalize(canonical.join(index))
                && index_path.starts_with(&self.root)
                && let Ok(index_metadata) = fs::metadata(&index_path)
                && index_metadata.is_file()
            {
                return self.serve(index_path, index_metadata, conditions);
            }

            if self.directory_listing {
                return self.listing(&canonical, path);
            }

            Resolved::NotFound
        } else if metadata.is_file() {
            self.serve(canonical, metadata, conditions)
        } else {
            Resolved::NotFound
        }
    }

    fn listing(&self, directory: &Path, path: &ParsedPath) -> Resolved {
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(_) => return Resolved::NotFound,
        };

        let mut rows = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.is_empty() {
                continue;
            }
            let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            let mut href = encode_path_segment(&name);
            if is_dir {
                href.push('/');
            }
            rows.push((name, href, is_dir));
        }
        rows.sort_by(|left, right| {
            right
                .2
                .cmp(&left.2)
                .then_with(|| left.0.to_lowercase().cmp(&right.0.to_lowercase()))
        });

        let mut display = self.prefix.clone();
        for segment in &path.segments {
            display.push('/');
            display.push_str(segment);
        }
        display.push('/');
        if display.is_empty() {
            display.push('/');
        }
        let display = escape_html(&display);

        let mut html =
            String::from("<!doctype html><html><head><meta charset=\"utf-8\"><title>Index of ");
        html.push_str(&display);
        html.push_str("</title></head><body><h1>Index of ");
        html.push_str(&display);
        html.push_str("</h1><ul>");
        if directory != self.root {
            html.push_str("<li><a href=\"../\">../</a></li>");
        }
        for (name, href, is_dir) in rows {
            html.push_str("<li><a href=\"");
            html.push_str(&href);
            html.push_str("\">");
            html.push_str(&escape_html(&name));
            if is_dir {
                html.push('/');
            }
            html.push_str("</a></li>");
        }
        html.push_str("</ul></body></html>");

        Resolved::Listing { html }
    }

    fn serve(&self, path: PathBuf, metadata: Metadata, conditions: &Conditions) -> Resolved {
        let length = metadata.len();
        let modified = metadata.modified().ok();
        let etag = make_etag(length, modified);

        if conditions.is_not_modified(&etag, modified) {
            return Resolved::NotModified { etag, modified };
        }

        let range = match conditions.requested_range(length, &etag, modified) {
            Ok(range) => range,
            Err(()) => {
                return Resolved::RangeNotSatisfiable {
                    total_length: length,
                };
            }
        };

        let content_type = mime::content_type(&path);
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                ) =>
            {
                return Resolved::NotFound;
            }
            Err(_) => return Resolved::Error,
        };

        let body_length = match range {
            Some((start, end)) => {
                if file.seek(SeekFrom::Start(start)).is_err() {
                    return Resolved::Error;
                }
                end - start + 1
            }
            None => length,
        };

        Resolved::File {
            file,
            body_length,
            total_length: length,
            range,
            modified,
            etag,
            content_type,
        }
    }
}

#[derive(Debug)]
struct ParsedPath {
    segments: Vec<String>,
    trailing_slash: bool,
}

impl ParsedPath {
    fn parse(relative: &str) -> std::result::Result<Self, StatusCode> {
        if relative.is_empty() {
            return Ok(Self {
                segments: Vec::new(),
                trailing_slash: false,
            });
        }
        if !relative.starts_with('/') {
            return Err(StatusCode::NOT_FOUND);
        }

        let decoded = decode_path(relative).map_err(|()| StatusCode::BAD_REQUEST)?;
        let trailing_slash = decoded.ends_with('/');
        let mut segments = Vec::new();

        for segment in decoded.split('/') {
            if segment.is_empty() || segment == "." {
                continue;
            }
            if segment == ".." {
                return Err(StatusCode::NOT_FOUND);
            }
            if segment
                .chars()
                .any(|character| character == '\\' || character == ':' || character.is_control())
            {
                return Err(StatusCode::NOT_FOUND);
            }
            if cfg!(windows) && is_reserved_windows_name(segment) {
                return Err(StatusCode::NOT_FOUND);
            }
            segments.push(segment.to_owned());
        }

        Ok(Self {
            segments,
            trailing_slash,
        })
    }
}

fn decode_path(value: &str) -> std::result::Result<String, ()> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(());
            }
            let high = hex_value(bytes[index + 1]).ok_or(())?;
            let low = hex_value(bytes[index + 2]).ok_or(())?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8(decoded).map_err(|_| ())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_reserved_windows_name(segment: &str) -> bool {
    let base = segment
        .split('.')
        .next()
        .unwrap_or(segment)
        .trim_end_matches(' ')
        .to_ascii_uppercase();

    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && base.as_bytes()[3].is_ascii_digit()
            && base.as_bytes()[3] != b'0')
}

fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(*byte));
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0f));
        }
    }
    encoded
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        _ => char::from(b'A' + value - 10),
    }
}

fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn redirect_location(target: &str) -> String {
    match target.split_once('?') {
        Some((path, query)) => format!("{path}/?{query}"),
        None => format!("{target}/"),
    }
}

fn normalize_prefix(prefix: &str) -> String {
    assert!(
        prefix.starts_with('/'),
        "the static files prefix must start with '/'"
    );
    assert!(
        !prefix.contains('?') && !prefix.contains('#'),
        "the static files prefix must not contain a query or fragment"
    );
    assert!(
        !prefix.chars().any(char::is_control),
        "the static files prefix must not contain control characters"
    );
    assert!(
        !prefix.split('/').any(|segment| segment == ".."),
        "the static files prefix must not contain '..'"
    );

    prefix.trim_end_matches('/').to_owned()
}

struct Conditions {
    if_none_match: Option<String>,
    if_modified_since: Option<SystemTime>,
    range: Option<String>,
    if_range: Option<String>,
}

impl Conditions {
    fn is_not_modified(&self, etag: &str, modified: Option<SystemTime>) -> bool {
        if let Some(value) = &self.if_none_match {
            return etag_matches(value, etag);
        }
        match (self.if_modified_since, modified) {
            (Some(since), Some(modified)) => unix_seconds(modified) <= unix_seconds(since),
            _ => false,
        }
    }

    fn requested_range(
        &self,
        total_length: u64,
        etag: &str,
        modified: Option<SystemTime>,
    ) -> std::result::Result<Option<(u64, u64)>, ()> {
        let Some(value) = self.range.as_deref() else {
            return Ok(None);
        };
        if let Some(if_range) = self.if_range.as_deref()
            && !if_range_matches(if_range, etag, modified)
        {
            return Ok(None);
        }

        parse_range(value, total_length)
    }
}

fn if_range_matches(value: &str, etag: &str, modified: Option<SystemTime>) -> bool {
    let value = value.trim();

    if value.starts_with('"') {
        return value == etag;
    }
    if value.starts_with("W/") {
        return false;
    }

    match (parse_http_date(value), modified) {
        (Some(date), Some(modified)) => unix_seconds(modified) <= unix_seconds(date),
        _ => false,
    }
}

fn parse_range(value: &str, total_length: u64) -> std::result::Result<Option<(u64, u64)>, ()> {
    let Some((unit, spec)) = value.trim().split_once('=') else {
        return Ok(None);
    };
    if !unit.trim().eq_ignore_ascii_case("bytes") {
        return Ok(None);
    }

    let spec = spec.trim();
    if spec.contains(',') {
        return Ok(None);
    }
    let Some((start, end)) = spec.split_once('-') else {
        return Ok(None);
    };
    let start = start.trim();
    let end = end.trim();

    if start.is_empty() {
        let suffix = match end.parse::<u64>() {
            Ok(suffix) => suffix,
            Err(_) => return Ok(None),
        };
        if suffix == 0 || total_length == 0 {
            return Err(());
        }
        let length = suffix.min(total_length);
        return Ok(Some((total_length - length, total_length - 1)));
    }

    let start = match start.parse::<u64>() {
        Ok(start) => start,
        Err(_) => return Ok(None),
    };
    if total_length == 0 || start >= total_length {
        return Err(());
    }

    let end = if end.is_empty() {
        total_length - 1
    } else {
        let end = match end.parse::<u64>() {
            Ok(end) => end,
            Err(_) => return Ok(None),
        };
        // RFC 9110 §14.1.1: an inverted byte-range-spec is invalid and MUST be
        // ignored, so the request is served as a full 200 response.
        if end < start {
            return Ok(None);
        }
        end.min(total_length - 1)
    };

    Ok(Some((start, end)))
}

fn etag_matches(value: &str, etag: &str) -> bool {
    value.split(',').any(|candidate| {
        let candidate = candidate.trim();
        candidate == "*" || candidate.trim_start_matches("W/") == etag
    })
}

fn make_etag(length: u64, modified: Option<SystemTime>) -> String {
    match modified.and_then(|time| time.duration_since(UNIX_EPOCH).ok()) {
        Some(modified) => format!(
            "\"{length:x}-{:x}-{:x}\"",
            modified.as_secs(),
            modified.subsec_nanos()
        ),
        None => format!("\"{length:x}\""),
    }
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

enum Resolved {
    File {
        file: File,
        body_length: u64,
        total_length: u64,
        range: Option<(u64, u64)>,
        modified: Option<SystemTime>,
        etag: String,
        content_type: &'static str,
    },
    NotModified {
        etag: String,
        modified: Option<SystemTime>,
    },
    RangeNotSatisfiable {
        total_length: u64,
    },
    Listing {
        html: String,
    },
    Redirect(String),
    NotFound,
    Error,
}

fn not_found() -> Response {
    status_response(StatusCode::NOT_FOUND)
}

fn internal_error() -> Response {
    status_response(StatusCode::INTERNAL_SERVER_ERROR)
}

fn method_not_allowed() -> Response {
    status_response(StatusCode::METHOD_NOT_ALLOWED).with_header("Allow", "GET, HEAD")
}

fn status_response(status: StatusCode) -> Response {
    Response::new(status)
        .with_header("Content-Type", "text/plain; charset=utf-8")
        .with_body(format!("{} {}\n", status.as_u16(), status.reason_phrase()))
}

#[cfg(test)]
mod tests {
    use super::{ParsedPath, decode_path, normalize_prefix, parse_range, redirect_location};
    use crate::internal::http::StatusCode;

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(
            decode_path("/hello%20world.txt").as_deref(),
            Ok("/hello world.txt")
        );
        assert_eq!(decode_path("/%E4%B8%AD%E6%96%87"), Ok("/中文".to_owned()));
        assert!(decode_path("/bad%2").is_err());
        assert!(decode_path("/bad%zz").is_err());
    }

    #[test]
    fn rejects_traversal_and_platform_separators() {
        assert_eq!(
            ParsedPath::parse("/../secret").unwrap_err(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            ParsedPath::parse("/%2e%2e/secret").unwrap_err(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            ParsedPath::parse("/..\\secret").unwrap_err(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            ParsedPath::parse("/C:/secret").unwrap_err(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            ParsedPath::parse("/foo%00bar").unwrap_err(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn normalizes_segments_and_trailing_slash() {
        let parsed = ParsedPath::parse("/a/./b//c/").unwrap();
        assert_eq!(parsed.segments, ["a", "b", "c"]);
        assert!(parsed.trailing_slash);

        let empty = ParsedPath::parse("").unwrap();
        assert!(empty.segments.is_empty());
        assert!(!empty.trailing_slash);
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_device_names() {
        for name in ["NUL", "con.txt", "COM1", "lpt9"] {
            assert_eq!(
                ParsedPath::parse(&format!("/{name}")).unwrap_err(),
                StatusCode::NOT_FOUND
            );
        }
        assert!(ParsedPath::parse("/COM0").is_ok());
    }

    #[test]
    fn normalizes_prefixes() {
        assert_eq!(normalize_prefix("/"), "");
        assert_eq!(normalize_prefix("/assets/"), "/assets");
        assert_eq!(normalize_prefix("/a/b"), "/a/b");
    }

    #[test]
    fn appends_slash_preserving_the_query() {
        assert_eq!(redirect_location("/assets/docs"), "/assets/docs/");
        assert_eq!(redirect_location("/assets/docs?x=1"), "/assets/docs/?x=1");
    }

    #[test]
    fn inverted_ranges_are_ignored_per_rfc() {
        assert_eq!(parse_range("bytes=5-3", 100), Ok(None));
        assert_eq!(parse_range("bytes=10-9", 100), Ok(None));
        // Valid but unsatisfiable ranges still produce 416.
        assert_eq!(parse_range("bytes=100-", 100), Err(()));
        assert_eq!(parse_range("bytes=-0", 100), Err(()));
    }
}
