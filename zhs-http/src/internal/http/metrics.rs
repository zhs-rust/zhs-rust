use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use super::context::Context;
use super::pipeline::{Middleware, Next};
use super::response::Response;
use super::server::{ServerStats, ServerStatsSource};
use super::service::{Service, ServiceFuture};
use super::status::StatusCode;

pub const DURATION_BUCKETS: &[f64] = &[
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

#[derive(Clone)]
pub struct Metrics {
    inner: Arc<Inner>,
}

struct Inner {
    started: Instant,
    requests: Mutex<BTreeMap<RequestKey, u64>>,
    durations: Mutex<BTreeMap<String, Histogram>>,
    gauges: Mutex<Vec<Gauge>>,
    server: Mutex<Option<ServerStatsSource>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RequestKey {
    method: String,
    status: u16,
}

#[derive(Clone, Copy, Debug, Default)]
struct Histogram {
    buckets: [u64; DURATION_BUCKETS.len()],
    sum: f64,
    count: u64,
}

impl Histogram {
    fn observe(&mut self, seconds: f64) {
        self.sum += seconds;
        self.count += 1;

        for (index, bound) in DURATION_BUCKETS.iter().enumerate() {
            if seconds <= *bound {
                self.buckets[index] += 1;
            }
        }
    }
}

struct Gauge {
    name: &'static str,
    help: &'static str,
    sample: Arc<dyn Fn() -> f64 + Send + Sync>,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                started: Instant::now(),
                requests: Mutex::new(BTreeMap::new()),
                durations: Mutex::new(BTreeMap::new()),
                gauges: Mutex::new(Vec::new()),
                server: Mutex::new(None),
            }),
        }
    }

    pub fn with_server(self, source: ServerStatsSource) -> Self {
        *lock(&self.inner.server) = Some(source);
        self
    }

    pub fn gauge(
        &self,
        name: &'static str,
        help: &'static str,
        sample: impl Fn() -> f64 + Send + Sync + 'static,
    ) -> &Self {
        lock(&self.inner.gauges).push(Gauge {
            name,
            help,
            sample: Arc::new(sample),
        });
        self
    }

    fn observe(&self, method: &str, status: StatusCode, seconds: f64) {
        let method = normalize_method(method);
        let key = RequestKey {
            method: method.to_owned(),
            status: status.as_u16(),
        };
        *lock(&self.inner.requests).entry(key).or_insert(0) += 1;

        lock(&self.inner.durations)
            .entry(method.to_owned())
            .or_default()
            .observe(seconds);
    }

    pub fn requests(&self, method: &str, status: u16) -> u64 {
        lock(&self.inner.requests)
            .iter()
            .find(|(key, _)| key.method == method && key.status == status)
            .map(|(_, count)| *count)
            .unwrap_or(0)
    }

    pub fn render(&self) -> String {
        let mut output = String::with_capacity(1_024);

        let requests = lock(&self.inner.requests).clone();
        if !requests.is_empty() {
            output.push_str(
                "# HELP zhs_http_requests_total HTTP requests handled by the application.\n",
            );
            output.push_str("# TYPE zhs_http_requests_total counter\n");
            for (key, count) in &requests {
                output.push_str(&format!(
                    "zhs_http_requests_total{{method=\"{}\",status=\"{}\"}} {}\n",
                    key.method, key.status, count
                ));
            }
        }

        let durations = lock(&self.inner.durations).clone();
        if !durations.is_empty() {
            output.push_str(
                "# HELP zhs_http_request_duration_seconds Request duration in seconds.\n",
            );
            output.push_str("# TYPE zhs_http_request_duration_seconds histogram\n");
            for (method, histogram) in &durations {
                Self::render_histogram(&mut output, method, histogram);
            }
        }

        if let Some(stats) = lock(&self.inner.server)
            .as_ref()
            .map(ServerStatsSource::stats)
        {
            Self::render_server(&mut output, &stats);
        }

        output.push_str(
            "# HELP zhs_process_uptime_seconds Seconds since the metrics registry was created.\n",
        );
        output.push_str("# TYPE zhs_process_uptime_seconds gauge\n");
        output.push_str(&format!(
            "zhs_process_uptime_seconds {:.3}\n",
            self.inner.started.elapsed().as_secs_f64()
        ));

        type GaugeRef = (&'static str, &'static str, Arc<dyn Fn() -> f64 + Send + Sync>);

        let gauges: Vec<GaugeRef> = {
            let gauges = lock(&self.inner.gauges);
            gauges
                .iter()
                .map(|gauge| (gauge.name, gauge.help, gauge.sample.clone()))
                .collect()
        };
        for (name, help, sample) in gauges {
            output.push_str(&format!("# HELP {name} {help}\n"));
            output.push_str(&format!("# TYPE {name} gauge\n"));
            output.push_str(&format!("{name} {}\n", sample()));
        }

        output
    }

    fn render_histogram(output: &mut String, method: &str, histogram: &Histogram) {
        for (index, bound) in DURATION_BUCKETS.iter().enumerate() {
            output.push_str(&format!(
                "zhs_http_request_duration_seconds_bucket{{method=\"{method}\",le=\"{bound}\"}} {}\n",
                histogram.buckets[index]
            ));
        }
        output.push_str(&format!(
            "zhs_http_request_duration_seconds_bucket{{method=\"{method}\",le=\"+Inf\"}} {}\n",
            histogram.count
        ));
        output.push_str(&format!(
            "zhs_http_request_duration_seconds_sum{{method=\"{method}\"}} {}\n",
            histogram.sum
        ));
        output.push_str(&format!(
            "zhs_http_request_duration_seconds_count{{method=\"{method}\"}} {}\n",
            histogram.count
        ));
    }

    fn render_server(output: &mut String, stats: &ServerStats) {
        output.push_str("# HELP zhs_http_connections_active Connections currently being served.\n");
        output.push_str("# TYPE zhs_http_connections_active gauge\n");
        output.push_str(&format!(
            "zhs_http_connections_active {}\n",
            stats.active_connections
        ));

        output.push_str(
            "# HELP zhs_http_connections_accepted_total Connections accepted since start.\n",
        );
        output.push_str("# TYPE zhs_http_connections_accepted_total counter\n");
        output.push_str(&format!(
            "zhs_http_connections_accepted_total {}\n",
            stats.accepted_connections
        ));

        output.push_str("# HELP zhs_http_failures_total Connection and accept failures.\n");
        output.push_str("# TYPE zhs_http_failures_total counter\n");
        output.push_str(&format!("zhs_http_failures_total {}\n", stats.failures));
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Middleware for Metrics {
    fn call(&self, context: Context, next: Next) -> ServiceFuture {
        let method = context.method().as_str().to_owned();
        let metrics = self.clone();

        Box::pin(async move {
            let started = Instant::now();
            let result = next.run(context).await;
            let status = match &result {
                Ok(response) => response.status(),
                Err(error) => error.status(),
            };
            metrics.observe(&method, status, started.elapsed().as_secs_f64());
            result
        })
    }
}

impl Service for Metrics {
    type Future = std::future::Ready<crate::internal::error::Result<Response>>;

    fn call(&self, _context: Context) -> Self::Future {
        let mut headers = super::headers::HeaderMap::new();
        headers.append("Content-Type", "text/plain; version=0.0.4; charset=utf-8");
        headers.append("Cache-Control", "no-store");

        let mut response = Response::new(StatusCode::OK).with_body(self.render());
        *response.headers_mut() = headers;

        std::future::ready(Ok(response))
    }
}

impl fmt::Debug for Metrics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Metrics")
            .field("requests", &lock(&self.inner.requests).len())
            .field("methods", &lock(&self.inner.durations).len())
            .finish_non_exhaustive()
    }
}

fn normalize_method(method: &str) -> &str {
    match method {
        "GET" | "HEAD" | "POST" | "PUT" | "DELETE" | "CONNECT" | "OPTIONS" | "TRACE"
        | "PATCH" => method,
        _ => "OTHER",
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => crate::internal::recover_poison(poisoned),
    }
}
