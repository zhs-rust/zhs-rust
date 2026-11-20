use std::fmt;

use zhs_http::{HttpError, IntoResponse, Response, StatusCode};

use crate::di::DiError;

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub title: String,
    pub detail: Option<String>,
    pub headers: Vec<(String, String)>,
}

impl ApiError {
    pub fn new(status: StatusCode, title: impl Into<String>) -> Self {
        Self {
            status,
            title: title.into(),
            detail: None,
            headers: Vec::new(),
        }
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "Bad Request").with_detail(detail)
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "Not Found")
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "Unauthorized")
            .with_header("WWW-Authenticate", "Bearer")
    }

    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "Forbidden")
    }

    pub fn conflict(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "Conflict").with_detail(detail)
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").with_detail(detail)
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.status, self.title)?;

        if let Some(detail) = &self.detail {
            write!(formatter, " ({detail})")?;
        }

        Ok(())
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = format!(
            r#"{{"type":"about:blank","title":"{}","status":{}"#,
            json_escape(&self.title),
            self.status.0
        );

        if let Some(detail) = &self.detail {
            if self.status.0 >= 500 {
                // 5xx details can carry Rust type names, file paths or internal
                // service names, so they are logged server-side only and the
                // client receives a generic message.
                eprintln!("[zhs] internal error: {detail}");
                body.push_str(r#","detail":"internal server error""#);
            } else {
                body.push_str(&format!(r#","detail":"{}""#, json_escape(detail)));
            }
        }

        body.push('}');

        let mut response = Response::json_raw(body)
            .with_status(self.status)
            .with_header("content-type", "application/problem+json; charset=utf-8");

        for (name, value) in self.headers {
            response = response.with_header(name, value);
        }

        response
    }
}

impl From<HttpError> for ApiError {
    fn from(error: HttpError) -> Self {
        let status = error.status();
        let detail = error.message();

        Self::new(status, status.reason()).with_detail(detail)
    }
}

impl From<DiError> for ApiError {
    fn from(error: DiError) -> Self {
        ApiError::internal(error.to_string())
    }
}

/// 数据层错误的自动转换,让控制器里的 `?` 直接生效——
/// 相当于 ASP.NET Core 里"抛异常 → 异常过滤器/中间件兜底"的通道。
///
/// - 唯一键冲突等 SQLSTATE 23xxx → 409 Conflict(细节保留)
/// - 其余 → 500,细节只进服务端日志,对客户端隐藏
impl From<zhs_entity::Error> for ApiError {
    fn from(error: zhs_entity::Error) -> Self {
        if error.is_conflict() {
            ApiError::conflict(error.to_string())
        } else {
            ApiError::internal(error.to_string())
        }
    }
}

fn json_escape(input: &str) -> String {
    let mut output = String::with_capacity(input.len());

    for character in input.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character < ' ' => {
                output.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => output.push(character),
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_conflicts_map_to_409() {
        let error = zhs_entity::Error::Database(zhs_entity::DataError::Server {
            code: 1062,
            state: "23000".to_owned(),
            message: "duplicate".to_owned(),
        });

        let api = ApiError::from(error);
        assert_eq!(api.status, StatusCode::CONFLICT);
        assert!(
            api.detail.as_deref().is_some_and(|detail| detail.contains("duplicate")),
            "{api:?}"
        );
    }

    #[test]
    fn other_entity_errors_map_to_500() {
        let error = zhs_entity::Error::Database(zhs_entity::DataError::protocol("boom"));

        let api = ApiError::from(error);
        assert_eq!(api.status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
