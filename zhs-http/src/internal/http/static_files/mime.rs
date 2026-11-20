use std::path::Path;

pub(super) fn content_type(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();

    match extension.to_ascii_lowercase().as_str() {
        "htm" | "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "xml" => "application/xml",
        "txt" | "log" => "text/plain; charset=utf-8",
        "md" | "markdown" => "text/markdown; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        "ics" => "text/calendar; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "tar" => "application/x-tar",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "ogg" => "audio/ogg",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::content_type;
    use std::path::Path;

    #[test]
    fn maps_common_extensions_case_insensitively() {
        assert_eq!(
            content_type(Path::new("index.HTML")),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            content_type(Path::new("app.css")),
            "text/css; charset=utf-8"
        );
        assert_eq!(content_type(Path::new("icon.svg")), "image/svg+xml");
        assert_eq!(content_type(Path::new("font.woff2")), "font/woff2");
        assert_eq!(
            content_type(Path::new("unknown.bin")),
            "application/octet-stream"
        );
        assert_eq!(
            content_type(Path::new("no-extension")),
            "application/octet-stream"
        );
    }
}
