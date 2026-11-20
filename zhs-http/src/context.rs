use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use crate::auth::ClaimsPrincipal;
use crate::error::HttpError;
use crate::request::Request;

pub trait ServiceResolver: Send + Sync {
    fn resolve(&self, type_id: TypeId) -> Option<Arc<dyn Any + Send + Sync>>;

    fn create_scope(&self) -> Arc<dyn ServiceResolver>;
}

pub struct HttpContext {
    pub request: Request,
    pub params: HashMap<String, String>,
    pub services: Arc<dyn ServiceResolver>,
    pub user: ClaimsPrincipal,
}

impl HttpContext {
    pub fn new(request: Request, services: Arc<dyn ServiceResolver>) -> Self {
        Self {
            request,
            params: HashMap::new(),
            services,
            user: ClaimsPrincipal::anonymous(),
        }
    }

    pub fn resolve<T>(&self) -> Result<Arc<T>, HttpError>
    where
        T: Any + Send + Sync,
    {
        let type_name = std::any::type_name::<T>();
        let service = self.services.resolve(TypeId::of::<T>()).ok_or_else(|| {
            HttpError::Internal(format!("no service registered for `{type_name}`"))
        })?;

        service.downcast::<T>().map_err(|_| {
            HttpError::Internal(format!("service `{type_name}` has an unexpected type"))
        })
    }

    pub fn route_value<T>(&self, name: &str) -> Result<T, HttpError>
    where
        T: FromStr,
    {
        let raw = self
            .params
            .get(name)
            .ok_or_else(|| HttpError::BadRequest(format!("missing route parameter `{name}`")))?;

        raw.parse::<T>().map_err(|_| {
            HttpError::BadRequest(format!("route parameter `{name}` has an invalid value"))
        })
    }

    pub fn query_value<T>(&self, name: &str) -> Result<Option<T>, HttpError>
    where
        T: FromStr,
    {
        self.request.query_value(name)
    }

    pub fn body_text(&self) -> Result<String, HttpError> {
        self.request.body_text()
    }

    pub fn multipart_files(&self) -> Result<Vec<crate::multipart::FormFile>, HttpError> {
        crate::multipart::parse_files(&self.request)
    }
}

pub fn resolve_required<T>(services: &dyn ServiceResolver) -> Result<Arc<T>, HttpError>
where
    T: Any + Send + Sync,
{
    let type_name = std::any::type_name::<T>();
    let service = services
        .resolve(TypeId::of::<T>())
        .ok_or_else(|| HttpError::Internal(format!("no service registered for `{type_name}`")))?;

    service
        .downcast::<T>()
        .map_err(|_| HttpError::Internal(format!("service `{type_name}` has an unexpected type")))
}
