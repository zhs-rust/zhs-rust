use std::fmt;

use crate::internal::error::{Error, HttpError, Result};

use super::body::{BodyChunk, RequestBody};
use super::headers::HeaderMap;
use super::parse::parse_header_line;

pub(crate) const MULTIPART_CONTENT_TYPE: &str = "multipart/form-data";

const MAX_BOUNDARY_LENGTH: usize = 70;

const DEFAULT_MAX_PART_HEAD_BYTES: usize = 8 * 1024;

const DEFAULT_MAX_PART_HEADERS: usize = 32;

const DEFAULT_MAX_PARTS: usize = 128;

#[derive(Clone, Copy, Debug)]
pub struct MultipartLimits {
    pub max_part_head_bytes: usize,

    pub max_part_headers: usize,

    pub max_parts: usize,
}

impl Default for MultipartLimits {
    fn default() -> Self {
        Self {
            max_part_head_bytes: DEFAULT_MAX_PART_HEAD_BYTES,
            max_part_headers: DEFAULT_MAX_PART_HEADERS,
            max_parts: DEFAULT_MAX_PARTS,
        }
    }
}

impl MultipartLimits {
    pub fn with_max_part_head_bytes(mut self, bytes: usize) -> Self {
        self.max_part_head_bytes = bytes.max(64);
        self
    }

    pub fn with_max_part_headers(mut self, count: usize) -> Self {
        self.max_part_headers = count.max(1);
        self
    }

    pub fn with_max_parts(mut self, count: usize) -> Self {
        self.max_parts = count.max(1);
        self
    }
}

pub struct Multipart<'a> {
    body: &'a mut RequestBody,
    boundary: Vec<u8>,
    buffer: Buffer,
    limits: MultipartLimits,
    started: bool,
    open_part: bool,
    parts: usize,
    done: bool,
}

impl<'a> Multipart<'a> {
    pub fn new(body: &'a mut RequestBody, content_type: &str) -> Result<Self> {
        let boundary = parse_boundary(content_type)?;

        Ok(Self {
            body,
            boundary,
            buffer: Buffer::new(),
            limits: MultipartLimits::default(),
            started: false,
            open_part: false,
            parts: 0,
            done: false,
        })
    }

    pub fn with_limits(mut self, limits: MultipartLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn boundary(&self) -> &str {
        std::str::from_utf8(&self.boundary).unwrap_or_default()
    }

    pub async fn next_part<'p>(&'p mut self) -> Result<Option<Part<'p, 'a>>> {
        if self.done {
            return Ok(None);
        }

        if self.open_part {
            self.drain_part().await?;
        }

        if !self.started {
            self.skip_preamble().await?;
            self.started = true;
        }

        self.after_boundary().await
    }

    async fn drain_part(&mut self) -> Result<()> {
        while self.read_part_chunk().await?.is_some() {}
        Ok(())
    }

    async fn skip_preamble(&mut self) -> Result<()> {
        let marker = self.marker();
        let mut searched = 0;

        loop {
            let found = {
                let available = self.buffer.available();
                let mut found = None;
                let mut index = searched;

                while index + marker.len() <= available.len() {
                    let at_start =
                        index == 0 || (index >= 2 && &available[index - 2..index] == b"\r\n");
                    if at_start && &available[index..index + marker.len()] == marker.as_slice() {
                        found = Some(index);
                        break;
                    }
                    index += 1;
                }

                (found, available.len())
            };

            let (index, available_len) = found;
            if let Some(index) = index {
                self.buffer.consume(index + marker.len());
                return Ok(());
            }

            let keep = marker.len() + 1;
            if available_len > keep {
                let discard = available_len - keep;
                self.buffer.consume(discard);
                searched = 0;
            } else {
                searched = available_len.saturating_sub(marker.len().saturating_sub(1));
            }

            if !self.buffer.fill(self.body).await? {
                return Err(invalid("the body does not contain the multipart boundary"));
            }
        }
    }

    async fn after_boundary(&mut self) -> Result<Option<Part<'_, 'a>>> {
        while self.buffer.available().len() < 2 {
            if !self.buffer.fill(self.body).await? {
                return Err(invalid("the body ended after the multipart boundary"));
            }
        }

        let next = {
            let available = self.buffer.available();
            [available[0], available[1]]
        };

        match &next {
            b"\r\n" => {
                self.buffer.consume(2);
                let head = self.read_part_head().await?;
                let headers = parse_part_headers(&head, &self.limits)?;
                reject_transfer_encoding(&headers)?;
                let disposition = headers
                    .get("content-disposition")
                    .ok_or_else(|| invalid("a part is missing Content-Disposition"))?;
                let disposition = parse_disposition(disposition)?;

                self.parts += 1;
                if self.parts > self.limits.max_parts {
                    return Err(invalid("too many parts"));
                }

                self.open_part = true;
                Ok(Some(Part {
                    multipart: self,
                    headers,
                    disposition,
                    finished: false,
                }))
            }
            b"--" => {
                self.buffer.consume(2);
                self.done = true;
                self.drain_epilogue().await?;
                Ok(None)
            }
            _ => Err(invalid(
                "the multipart boundary must be followed by CRLF or \"--\"",
            )),
        }
    }

    async fn read_part_head(&mut self) -> Result<Vec<u8>> {
        loop {
            let limit = self.limits.max_part_head_bytes;

            if let Some(index) = self.buffer.find(b"\r\n\r\n") {
                if index > limit {
                    return Err(invalid("the part headers are too large"));
                }
                let head = self.buffer.take(index);
                self.buffer.consume(4);
                return Ok(head);
            }

            if self.buffer.available().len() > limit + 2 {
                return Err(invalid("the part headers are too large"));
            }

            if !self.buffer.fill(self.body).await? {
                return Err(invalid("the body ended inside the part headers"));
            }
        }
    }

    async fn read_part_chunk(&mut self) -> Result<Option<BodyChunk>> {
        if !self.open_part {
            return Ok(None);
        }

        let delimiter = self.delimiter();

        loop {
            if let Some(index) = self.buffer.find(&delimiter) {
                let data = self.buffer.take(index);
                self.buffer.consume(delimiter.len());
                self.open_part = false;
                return Ok((!data.is_empty()).then_some(data));
            }

            let keep = delimiter.len() - 1;
            let available = self.buffer.available().len();
            if available > keep {
                let take = available - keep;
                return Ok(Some(self.buffer.take(take)));
            }

            if !self.buffer.fill(self.body).await? {
                return Err(invalid(
                    "the body ended before the closing multipart boundary",
                ));
            }
        }
    }

    async fn drain_epilogue(&mut self) -> Result<()> {
        self.buffer.clear();
        while self.body.read_chunk().await?.is_some() {}
        Ok(())
    }

    fn marker(&self) -> Vec<u8> {
        let mut marker = Vec::with_capacity(self.boundary.len() + 2);
        marker.extend_from_slice(b"--");
        marker.extend_from_slice(&self.boundary);
        marker
    }

    fn delimiter(&self) -> Vec<u8> {
        let mut delimiter = Vec::with_capacity(self.boundary.len() + 4);
        delimiter.extend_from_slice(b"\r\n--");
        delimiter.extend_from_slice(&self.boundary);
        delimiter
    }
}

impl fmt::Debug for Multipart<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Multipart")
            .field("boundary", &self.boundary())
            .field("parts", &self.parts)
            .finish()
    }
}

pub struct Part<'p, 'a> {
    multipart: &'p mut Multipart<'a>,
    headers: HeaderMap,
    disposition: Disposition,
    finished: bool,
}

impl Part<'_, '_> {
    pub fn name(&self) -> Option<&str> {
        self.disposition.name.as_deref()
    }

    pub fn file_name(&self) -> Option<&str> {
        self.disposition.file_name.as_deref()
    }

    pub fn safe_file_name(&self) -> Option<String> {
        sanitize_file_name(self.file_name()?)
    }

    pub fn content_type(&self) -> Option<&str> {
        self.headers.get("content-type")
    }

    pub fn is_file(&self) -> bool {
        self.disposition.file_name.is_some()
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub async fn read_chunk(&mut self) -> Result<Option<BodyChunk>> {
        if self.finished {
            return Ok(None);
        }

        let chunk = self.multipart.read_part_chunk().await?;
        if chunk.is_none() {
            self.finished = true;
        }
        Ok(chunk)
    }

    pub async fn bytes(&mut self) -> Result<Vec<u8>> {
        let mut collected = Vec::new();
        while let Some(chunk) = self.read_chunk().await? {
            collected.extend_from_slice(&chunk);
        }
        Ok(collected)
    }

    pub async fn text(&mut self) -> Result<String> {
        let bytes = self.bytes().await?;
        String::from_utf8(bytes).map_err(|_| invalid("the part is not valid UTF-8"))
    }
}

impl fmt::Debug for Part<'_, '_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Part")
            .field("name", &self.name())
            .field("file_name", &self.file_name())
            .field("content_type", &self.content_type())
            .finish()
    }
}

pub fn sanitize_file_name(raw: &str) -> Option<String> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let cleaned: String = base
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    let trimmed = cleaned.trim();

    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return None;
    }

    Some(truncate_utf8(trimmed, 255))
}

struct Buffer {
    data: Vec<u8>,
    start: usize,
}

impl Buffer {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            start: 0,
        }
    }

    fn available(&self) -> &[u8] {
        &self.data[self.start..]
    }

    fn consume(&mut self, count: usize) {
        self.start += count;

        if self.start == self.data.len() {
            self.data.clear();
            self.start = 0;
        } else if self.start >= 64 * 1024 {
            self.data.drain(..self.start);
            self.start = 0;
        }
    }

    fn take(&mut self, count: usize) -> Vec<u8> {
        let bytes = self.available()[..count].to_vec();
        self.consume(count);
        bytes
    }

    fn find(&self, needle: &[u8]) -> Option<usize> {
        find_from(self.available(), needle, 0)
    }

    async fn fill(&mut self, body: &mut RequestBody) -> Result<bool> {
        match body.read_chunk().await? {
            Some(chunk) => {
                self.data.extend_from_slice(&chunk);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn clear(&mut self) {
        self.data.clear();
        self.start = 0;
    }
}

fn find_from(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }

    let from = from.min(haystack.len() - needle.len());
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|index| index + from)
}

fn parse_boundary(content_type: &str) -> Result<Vec<u8>> {
    let mut parameters = content_type.split(';');
    let media_type = parameters.next().unwrap_or_default().trim();

    if !media_type.eq_ignore_ascii_case(MULTIPART_CONTENT_TYPE) {
        return Err(Error::Http(HttpError::UnsupportedContentType {
            actual: content_type.to_owned(),
            expected: MULTIPART_CONTENT_TYPE,
        }));
    }

    let mut boundary = None;
    for parameter in parameters {
        let Some((name, value)) = parameter.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("boundary") {
            boundary = Some(unquote(value.trim())?);
            break;
        }
    }

    let Some(boundary) = boundary else {
        return Err(invalid("multipart/form-data requires a boundary parameter"));
    };

    validate_boundary(&boundary)?;
    Ok(boundary.into_bytes())
}

fn validate_boundary(boundary: &str) -> Result<()> {
    let bytes = boundary.as_bytes();

    if bytes.is_empty() || bytes.len() > MAX_BOUNDARY_LENGTH {
        return Err(invalid("the multipart boundary length is invalid"));
    }

    if !bytes.iter().all(|&byte| is_boundary_byte(byte)) || bytes.ends_with(b" ") {
        return Err(invalid(
            "the multipart boundary contains invalid characters",
        ));
    }

    Ok(())
}

fn is_boundary_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'\''
                | b'('
                | b')'
                | b'+'
                | b'_'
                | b','
                | b'-'
                | b'.'
                | b'/'
                | b':'
                | b'='
                | b'?'
                | b' '
        )
}

fn unquote(raw: &str) -> Result<String> {
    let Some(rest) = raw.strip_prefix('"') else {
        return Ok(raw.to_owned());
    };

    let mut value = String::new();
    let mut characters = rest.char_indices();

    while let Some((index, character)) = characters.next() {
        match character {
            '"' => {
                if !rest[index + 1..].trim().is_empty() {
                    return Err(invalid("unexpected characters after a quoted parameter"));
                }
                return Ok(value);
            }
            '\\' => match characters.next() {
                Some((_, escaped)) => value.push(escaped),
                None => return Err(invalid("a quoted parameter ends with a backslash")),
            },
            character => value.push(character),
        }
    }

    Err(invalid("unterminated quoted parameter"))
}

fn split_parameters(value: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;

    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        match character {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ';' if !quoted => {
                segments.push(&value[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }

    segments.push(&value[start..]);
    segments
}

struct Disposition {
    name: Option<String>,
    file_name: Option<String>,
}

fn parse_disposition(value: &str) -> Result<Disposition> {
    let mut segments = split_parameters(value).into_iter();
    let kind = segments.next().unwrap_or_default().trim();

    if !kind.eq_ignore_ascii_case("form-data") {
        return Err(invalid("Content-Disposition must be form-data"));
    }

    let mut disposition = Disposition {
        name: None,
        file_name: None,
    };

    for segment in segments {
        let Some((name, raw)) = segment.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = unquote(raw.trim())?;

        if name.eq_ignore_ascii_case("name") {
            disposition.name = Some(value);
        } else if name.eq_ignore_ascii_case("filename") {
            disposition.file_name = Some(value);
        }
    }

    Ok(disposition)
}

fn parse_part_headers(head: &[u8], limits: &MultipartLimits) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let mut count = 0;

    for line in head.split(|&byte| byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }

        count += 1;
        if count > limits.max_part_headers {
            return Err(invalid("too many part headers"));
        }

        let (name, value) = parse_header_line(line)?;
        headers.append(name, value);
    }

    Ok(headers)
}

fn reject_transfer_encoding(headers: &HeaderMap) -> Result<()> {
    let Some(encoding) = headers.get("content-transfer-encoding") else {
        return Ok(());
    };

    let encoding = encoding.trim();
    let plain = encoding.eq_ignore_ascii_case("binary")
        || encoding.eq_ignore_ascii_case("8bit")
        || encoding.eq_ignore_ascii_case("7bit");

    if plain {
        Ok(())
    } else {
        Err(invalid("Content-Transfer-Encoding is not supported"))
    }
}

fn truncate_utf8(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }

    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn invalid(reason: &str) -> Error {
    Error::Http(HttpError::InvalidMultipart {
        reason: reason.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::task::{Context, Poll, Waker};

        let mut future = std::pin::pin!(future);
        let mut context = Context::from_waker(Waker::noop());

        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn multipart_body(boundary: &str, parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();

        for (name, file_name, data) in parts {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());

            match file_name {
                Some(file_name) => body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
                    )
                    .as_bytes(),
                ),
                None => body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
                ),
            }

            body.extend_from_slice(data);
            body.extend_from_slice(b"\r\n");
        }

        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        body
    }

    fn parser<'a>(body: &'a mut RequestBody, boundary: &str) -> Multipart<'a> {
        Multipart::new(body, &format!("multipart/form-data; boundary={boundary}"))
            .expect("parse the boundary")
    }

    #[test]
    fn rejects_invalid_content_types() {
        for content_type in [
            "application/json",
            "multipart/form-data",
            "multipart/form-data; boundary=",
            "multipart/form-data; boundary=\"unterminated",
            "multipart/form-data; boundary=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "multipart/form-data; boundary=bad\"boundary",
        ] {
            let mut body = RequestBody::empty();
            assert!(
                Multipart::new(&mut body, content_type).is_err(),
                "{content_type}"
            );
        }
    }

    #[test]
    fn parses_field_and_file_parts() {
        let boundary = "----zhs";
        let raw = multipart_body(
            boundary,
            &[
                ("title", None, b"hello"),
                ("avatar", Some("a.txt"), b"\x00binary\xff"),
            ],
        );

        let mut body = RequestBody::from_bytes(raw);
        let mut multipart = parser(&mut body, boundary);

        block_on(async {
            let mut title = multipart.next_part().await.expect("part").expect("title");
            assert_eq!(title.name(), Some("title"));
            assert!(!title.is_file());
            assert_eq!(title.text().await.expect("text"), "hello");

            let mut avatar = multipart.next_part().await.expect("part").expect("avatar");
            assert_eq!(avatar.name(), Some("avatar"));
            assert_eq!(avatar.file_name(), Some("a.txt"));
            assert_eq!(avatar.safe_file_name().as_deref(), Some("a.txt"));
            assert_eq!(avatar.content_type(), Some("application/octet-stream"));
            assert_eq!(avatar.bytes().await.expect("bytes"), b"\x00binary\xff");

            assert!(multipart.next_part().await.expect("next").is_none());
        });
    }

    #[test]
    fn reads_file_data_across_chunks() {
        let boundary = "----zhs";

        let data: Vec<u8> = (0..200_000u32).map(|value| value as u8).collect();
        let raw = multipart_body(boundary, &[("file", Some("big.bin"), &data)]);

        let mut body = RequestBody::from_bytes(raw);
        let mut multipart = parser(&mut body, boundary);

        block_on(async {
            let mut part = multipart.next_part().await.expect("part").expect("file");
            assert_eq!(part.file_name(), Some("big.bin"));

            let mut collected = Vec::new();
            let mut chunks = 0;
            while let Some(chunk) = part.read_chunk().await.expect("chunk") {
                chunks += 1;
                collected.extend_from_slice(&chunk);
            }

            assert!(chunks > 1, "should be read in several chunks");
            assert_eq!(collected, data);
            assert!(multipart.next_part().await.expect("next").is_none());
        });
    }

    #[test]
    fn skips_parts_that_were_not_read() {
        let boundary = "----zhs";
        let raw = multipart_body(
            boundary,
            &[("skipped", Some("a.bin"), b"12345"), ("kept", None, b"ok")],
        );

        let mut body = RequestBody::from_bytes(raw);
        let mut multipart = parser(&mut body, boundary);

        block_on(async {
            let first = multipart.next_part().await.expect("part").expect("first");
            drop(first);

            let mut second = multipart.next_part().await.expect("part").expect("second");
            assert_eq!(second.name(), Some("kept"));
            assert_eq!(second.text().await.expect("text"), "ok");
            assert!(multipart.next_part().await.expect("next").is_none());
        });
    }

    #[test]
    fn rejects_truncated_bodies() {
        let boundary = "----zhs";
        let raw = multipart_body(boundary, &[("file", Some("a.bin"), b"payload")]);
        let truncated = raw[..raw.len() - 10].to_vec();

        let mut body = RequestBody::from_bytes(truncated);
        let mut multipart = parser(&mut body, boundary);

        block_on(async {
            let mut part = multipart.next_part().await.expect("part").expect("file");
            assert!(part.bytes().await.is_err());
        });
    }

    #[test]
    fn sanitizes_file_names() {
        assert_eq!(
            sanitize_file_name("../../etc/passwd").as_deref(),
            Some("passwd")
        );
        assert_eq!(
            sanitize_file_name(r"C:\Windows\win.ini").as_deref(),
            Some("win.ini")
        );
        assert_eq!(
            sanitize_file_name("  photo.png  ").as_deref(),
            Some("photo.png")
        );
        assert_eq!(sanitize_file_name("a\u{0}b.txt").as_deref(), Some("ab.txt"));
        assert_eq!(sanitize_file_name(".."), None);
        assert_eq!(sanitize_file_name("."), None);
        assert_eq!(sanitize_file_name(""), None);
        assert_eq!(sanitize_file_name("/"), None);

        let long = "a".repeat(300);
        assert_eq!(sanitize_file_name(&long).unwrap().len(), 255);
    }

    #[test]
    fn rejects_base64_transfer_encoding() {
        let boundary = "----zhs";
        let raw = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\nContent-Transfer-Encoding: base64\r\n\r\nYQ==\r\n--{boundary}--\r\n"
        );

        let mut body = RequestBody::from_bytes(raw.into_bytes());
        let mut multipart = parser(&mut body, boundary);

        block_on(async {
            assert!(multipart.next_part().await.is_err());
        });
    }

    #[test]
    fn parses_quoted_parameters() {
        let disposition =
            parse_disposition(r#"form-data; name="file"; filename="a;b.txt""#).expect("parse");
        assert_eq!(disposition.name.as_deref(), Some("file"));
        assert_eq!(disposition.file_name.as_deref(), Some("a;b.txt"));

        let disposition = parse_disposition(r#"form-data; name="a\"b""#).expect("parse");
        assert_eq!(disposition.name.as_deref(), Some(r#"a"b"#));

        assert!(parse_disposition("attachment; name=x").is_err());
        assert!(parse_disposition("form-data; name=\"unterminated").is_err());
    }

    #[test]
    fn ignores_a_preamble_before_the_first_boundary() {
        let boundary = "----zhs";
        let mut raw = b"this is a preamble\r\n--not-the-boundary\r\n".to_vec();
        raw.extend_from_slice(&multipart_body(boundary, &[("field", None, b"value")]));

        let mut body = RequestBody::from_bytes(raw);
        let mut multipart = parser(&mut body, boundary);

        block_on(async {
            let mut part = multipart.next_part().await.expect("part").expect("field");
            assert_eq!(part.text().await.expect("text"), "value");
        });
    }
}
