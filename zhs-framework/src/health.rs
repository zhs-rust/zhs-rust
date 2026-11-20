use std::sync::Arc;

use zhs_http::{HandlerFn, HttpContext, Response, StatusCode};
use zhs_json::{Object, Value};

pub struct HealthCheck {
    name: String,
    check: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
}

impl HealthCheck {
    pub fn new<F>(name: impl Into<String>, check: F) -> Self
    where
        F: Fn() -> Result<(), String> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            check: Arc::new(check),
        }
    }

    fn run(&self) -> HealthCheckResult {
        match (self.check)() {
            Ok(()) => HealthCheckResult {
                name: self.name.clone(),
                status: "Healthy",
                detail: None,
            },
            Err(detail) => HealthCheckResult {
                name: self.name.clone(),
                status: "Unhealthy",
                detail: Some(detail),
            },
        }
    }
}

pub struct HealthCheckResult {
    pub name: String,
    pub status: &'static str,
    pub detail: Option<String>,
}

#[derive(Default)]
pub struct HealthCheckRegistry {
    checks: Vec<HealthCheck>,
}

impl HealthCheckRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add<F>(&mut self, name: impl Into<String>, check: F) -> &mut Self
    where
        F: Fn() -> Result<(), String> + Send + Sync + 'static,
    {
        self.checks.push(HealthCheck::new(name, check));
        self
    }

    pub fn run(&self) -> HealthReport {
        let checks = self.checks.iter().map(HealthCheck::run).collect::<Vec<_>>();
        let healthy = checks.iter().all(|check| check.status == "Healthy");

        HealthReport { healthy, checks }
    }
}

pub struct HealthReport {
    pub healthy: bool,
    pub checks: Vec<HealthCheckResult>,
}

impl HealthReport {
    pub fn to_json(&self) -> Value {
        let mut root = Object::new();
        root.insert(
            "status",
            Value::from(if self.healthy { "Healthy" } else { "Unhealthy" }),
        );

        let checks = self
            .checks
            .iter()
            .map(|check| {
                let mut item = Object::new();
                item.insert("name", Value::from(check.name.as_str()));
                item.insert("status", Value::from(check.status));

                if let Some(detail) = &check.detail {
                    item.insert("detail", Value::from(detail.as_str()));
                }

                Value::Object(item)
            })
            .collect::<Vec<_>>();

        root.insert("checks", Value::Array(checks));
        Value::Object(root)
    }
}

pub fn health_handler() -> HandlerFn {
    Arc::new(|context: HttpContext| {
        Box::pin(async move {
            let registry = context
                .resolve::<HealthCheckRegistry>()
                .unwrap_or_else(|_| Arc::new(HealthCheckRegistry::new()));
            let report = registry.run();
            let body = zhs_json::to_string(&report.to_json());

            Ok(Response::json_raw(body)
                .with_status(if report.healthy {
                    StatusCode::OK
                } else {
                    StatusCode(503)
                })
                .with_header("cache-control", "no-store"))
        })
    })
}
