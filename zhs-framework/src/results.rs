use zhs_http::{IntoResponse, Response};
use zhs_json::ToJson;

use crate::error::ApiError;

pub struct Json<T>(pub T);

impl<T> IntoResponse for Json<T>
where
    T: ToJson,
{
    fn into_response(self) -> Response {
        Response::json_raw(zhs_json::to_string(&self.0))
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

pub fn into_response<T>(result: ApiResult<T>) -> Response
where
    T: IntoResponse,
{
    match result {
        Ok(value) => value.into_response(),
        Err(error) => error.into_response(),
    }
}

pub struct Results;

impl Results {
    pub fn ok<T>(value: T) -> ApiResult<T>
    where
        T: IntoResponse,
    {
        Ok(value)
    }

    pub fn json<T>(value: T) -> Json<T> {
        Json(value)
    }

    pub fn not_found() -> ApiError {
        ApiError::not_found()
    }

    pub fn bad_request(detail: impl Into<String>) -> ApiError {
        ApiError::bad_request(detail)
    }

    pub fn internal(detail: impl Into<String>) -> ApiError {
        ApiError::internal(detail)
    }
}
