use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use crate::internal::container::{Container, Scope, ServiceScope};
use crate::internal::error::{ContainerError, Error, HttpError, Result};
use zhs_json::FromJson;

use super::body::RequestBody;
use super::fields::{Fields, FromQuery};
use super::headers::HeaderMap;
use super::method::Method;
use super::multipart::Multipart;
use super::params::{FromPath, Params};
use super::request::Request;
use super::version::Version;

#[derive(Debug)]
pub struct Context {
    request: Request,
    params: Params,
    container: Option<Arc<Container>>,
    services: Option<ServiceScope>,
    scope: Scope,
}

impl Context {
    pub fn new(request: Request) -> Self {
        Self {
            request,
            params: Params::new(),
            container: None,
            services: None,
            scope: Scope::new(),
        }
    }

    pub fn with_container(mut self, container: Arc<Container>) -> Self {
        self.services = Some(container.create_scope());
        self.container = Some(container);
        self
    }

    pub fn container(&self) -> Option<&Container> {
        self.container.as_deref()
    }

    pub fn service_scope(&self) -> Option<&ServiceScope> {
        self.services.as_ref()
    }

    pub fn get<T>(&self) -> Option<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.scope.get::<T>().or_else(|| {
            self.services
                .as_ref()
                .and_then(|services| services.get::<T>())
                .or_else(|| {
                    self.container
                        .as_ref()
                        .and_then(|container| container.get::<T>())
                })
        })
    }

    pub fn require<T>(&self) -> Result<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        if let Some(value) = self.scope.get::<T>() {
            return Ok(value);
        }

        if let Some(services) = &self.services {
            return services.require::<T>();
        }

        self.container
            .as_ref()
            .map(|container| container.require::<T>())
            .unwrap_or_else(|| Err(ContainerError::missing::<T>().into()))
    }

    pub fn insert<T>(&mut self, value: T)
    where
        T: Send + Sync + 'static,
    {
        self.scope.insert(value);
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    pub fn scope_mut(&mut self) -> &mut Scope {
        &mut self.scope
    }

    pub(crate) fn inject_container(&mut self, container: Option<&Arc<Container>>) {
        if self.container.is_none() {
            self.container = container.cloned();
            self.services = container.map(|container| container.create_scope());
        }
    }

    pub fn request(&self) -> &Request {
        &self.request
    }

    pub fn request_mut(&mut self) -> &mut Request {
        &mut self.request
    }

    pub fn into_request(self) -> Request {
        self.request
    }

    pub fn params(&self) -> &Params {
        &self.params
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.get(name)
    }

    pub(crate) fn set_params(&mut self, params: Params) {
        self.params = params;
    }

    pub fn method(&self) -> &Method {
        self.request.method()
    }

    pub fn path(&self) -> &str {
        self.request.path()
    }

    pub fn path_as<T>(&self, name: &str) -> Result<T>
    where
        T: FromStr,
    {
        let value = self.param(name).ok_or_else(|| {
            Error::Http(HttpError::InvalidField {
                field: name.to_owned(),
                reason: "the path parameter is missing".to_owned(),
            })
        })?;

        value.parse::<T>().map_err(|_| {
            Error::Http(HttpError::InvalidField {
                field: name.to_owned(),
                reason: "the path parameter could not be parsed".to_owned(),
            })
        })
    }

    pub fn path_params<T>(&self) -> Result<T>
    where
        T: FromPath,
    {
        T::from_path(self.params())
    }

    pub fn query(&self) -> &str {
        self.request.query()
    }

    pub fn query_params(&self) -> Result<Fields> {
        self.request.query_params()
    }

    pub fn query_as<T>(&self) -> Result<T>
    where
        T: FromQuery,
    {
        T::from_query(&self.query_params()?)
    }

    pub fn version(&self) -> Version {
        self.request.version()
    }

    pub fn headers(&self) -> &HeaderMap {
        self.request.headers()
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        self.request.headers_mut()
    }

    pub fn body_mut(&mut self) -> &mut RequestBody {
        self.request.body_mut()
    }

    pub async fn body(&mut self) -> Result<Vec<u8>> {
        self.request.body().await
    }

    pub async fn text(&mut self) -> Result<String> {
        self.request.text().await
    }

    pub async fn json<T: FromJson>(&mut self) -> Result<T> {
        self.request.json().await
    }

    pub async fn form(&mut self) -> Result<Fields> {
        self.request.form().await
    }

    pub fn multipart(&mut self) -> Result<Multipart<'_>> {
        self.request.multipart()
    }

    pub fn peer_addr(&self) -> Option<SocketAddr> {
        self.request.peer_addr()
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.request.local_addr()
    }
}
