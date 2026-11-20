use zhs_json::{Object, Value, to_string};

use super::context::Context;
use super::response::Response;
use super::service::Service;
use super::status::StatusCode;

pub trait HealthProbe: Send + Sync {
    fn name(&self) -> &'static str;

    fn check(&self) -> std::result::Result<(), String>;
}

pub struct Health {
    probes: Vec<Box<dyn HealthProbe>>,
}

impl Health {
    pub fn new() -> Self {
        Self { probes: Vec::new() }
    }

    pub fn probe(mut self, probe: impl HealthProbe + 'static) -> Self {
        self.probes.push(Box::new(probe));
        self
    }

    pub fn probe_boxed(mut self, probe: Box<dyn HealthProbe>) -> Self {
        self.probes.push(probe);
        self
    }

    pub fn len(&self) -> usize {
        self.probes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.probes.is_empty()
    }
}

impl Default for Health {
    fn default() -> Self {
        Self::new()
    }
}

impl Service for Health {
    type Future = std::future::Ready<crate::internal::error::Result<Response>>;

    fn call(&self, _context: Context) -> Self::Future {
        let mut healthy = true;
        let mut checks = Vec::with_capacity(self.probes.len());

        for probe in &self.probes {
            let (ok, detail) = match probe.check() {
                Ok(()) => (true, Value::Null),
                Err(detail) => {
                    healthy = false;
                    (false, Value::from(detail))
                }
            };

            let mut check = Object::new();
            check.insert("name", Value::from(probe.name()));
            check.insert("ok", Value::from(ok));
            check.insert("detail", detail);
            checks.push(Value::from(check));
        }

        let mut body = Object::new();
        body.insert(
            "status",
            Value::from(if healthy { "healthy" } else { "unhealthy" }),
        );
        body.insert("checks", Value::from(checks));

        let status = if healthy {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };

        let response = Response::new(status)
            .with_header("Content-Type", "application/json")
            .with_header("Cache-Control", "no-store")
            .with_body(to_string(&Value::from(body)));

        std::future::ready(Ok(response))
    }
}

impl std::fmt::Debug for Health {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Health")
            .field("probes", &self.probes.len())
            .finish()
    }
}
