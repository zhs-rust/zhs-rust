use crate::error::HttpError;
use crate::request::Request;

#[derive(Debug, Clone)]
pub struct FormFile {
    pub name: String,
    pub file_name: String,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

pub fn parse_files(request: &Request) -> Result<Vec<FormFile>, HttpError> {
    let content_type = request
        .header("content-type")
        .ok_or_else(|| HttpError::BadRequest("missing multipart Content-Type".to_owned()))?;

    if !content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        return Err(HttpError::BadRequest(
            "expected multipart/form-data".to_owned(),
        ));
    }

    let boundary = content_type
        .split(';')
        .find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            if name.trim().eq_ignore_ascii_case("boundary") {
                Some(value.trim().trim_matches('"'))
            } else {
                None
            }
        })
        .filter(|boundary| !boundary.is_empty())
        .ok_or_else(|| HttpError::BadRequest("missing multipart boundary".to_owned()))?;

    // The first delimiter stands alone; every later one must be preceded by
    // a CRLF, so part content containing the bare boundary cannot be
    // mistaken for a delimiter.
    let first_delimiter = format!("--{boundary}").into_bytes();
    let delimiter = format!("\r\n--{boundary}").into_bytes();
    let body = &request.body;
    let mut files = Vec::new();

    let Some(start_offset) = find_boundary(body, &first_delimiter) else {
        return Err(HttpError::BadRequest(
            "multipart boundary not found in body".to_owned(),
        ));
    };
    let mut cursor = start_offset + first_delimiter.len();

    loop {
        match body.get(cursor..cursor + 2) {
            Some(b"--") => break,
            Some(b"\r\n") => {}
            _ => {
                return Err(HttpError::BadRequest(
                    "multipart boundary must be followed by CRLF or \"--\"".to_owned(),
                ))
            }
        }

        let start = cursor + 2;
        let Some(next) = body.get(start..).and_then(|tail| find(tail, &delimiter)) else {
            break;
        };
        let end = start + next;
        let part = &body[start..end];

        if let Some(file) = parse_part(part)? {
            files.push(file);
        }

        cursor = end + delimiter.len();
    }

    Ok(files)
}

/// Finds the first delimiter, which must start the body or follow a CRLF.
fn find_boundary(body: &[u8], delimiter: &[u8]) -> Option<usize> {
    let mut offset = 0;

    while let Some(relative) = find(&body[offset..], delimiter) {
        let index = offset + relative;
        if index == 0 || body.get(index - 2..index) == Some(b"\r\n") {
            return Some(index);
        }
        offset = index + 1;
    }

    None
}

fn parse_part(part: &[u8]) -> Result<Option<FormFile>, HttpError> {
    let Some(header_end) = find(part, b"\r\n\r\n") else {
        return Ok(None);
    };

    let headers = std::str::from_utf8(&part[..header_end])
        .map_err(|_| HttpError::BadRequest("multipart headers are not UTF-8".to_owned()))?;
    let data = part[header_end + 4..].to_vec();

    let mut name = None;
    let mut file_name = None;
    let mut content_type = None;

    for line in headers.split("\r\n") {
        if let Some((header, value)) = line.split_once(':') {
            let header = header.trim().to_ascii_lowercase();
            let value = value.trim();

            if header == "content-disposition" {
                for part in value.split(';') {
                    let part = part.trim();
                    if let Some(value) = part.strip_prefix("name=") {
                        name = Some(value.trim_matches('"').to_owned());
                    }
                    if let Some(value) = part.strip_prefix("filename=") {
                        file_name = Some(value.trim_matches('"').to_owned());
                    }
                }
            } else if header == "content-type" {
                content_type = Some(value.to_owned());
            }
        }
    }

    let Some(file_name) = file_name else {
        return Ok(None);
    };

    Ok(Some(FormFile {
        name: name.unwrap_or_default(),
        file_name,
        content_type,
        data,
    }))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
