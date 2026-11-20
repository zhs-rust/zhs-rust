use std::fmt;
use std::time::Instant;

use zhs_log::runtime::{LogLevel, LogRecord, Logger};

use super::context::Context;
use super::pipeline::{Middleware, Next};
use super::request_id::{REQUEST_ID_HEADER, RequestId};
use super::service::ServiceFuture;

#[derive(Clone)]
pub struct RequestLogger {
    logger: Logger,
    level: LogLevel,
    target: String,
    query: bool,
    peer: bool,
    request_id: bool,
}

impl RequestLogger {
    pub fn new(logger: Logger) -> Self {
        Self {
            logger,
            level: LogLevel::Info,
            target: "zhs_framework::http".to_owned(),
            query: false,
            peer: false,
            request_id: false,
        }
    }

    pub fn with_level(mut self, level: LogLevel) -> Self {
        self.level = level;
        self
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = target.into();
        self
    }

    pub fn with_query(mut self, enabled: bool) -> Self {
        self.query = enabled;
        self
    }

    pub fn with_peer(mut self, enabled: bool) -> Self {
        self.peer = enabled;
        self
    }

    pub fn with_request_id(mut self, enabled: bool) -> Self {
        self.request_id = enabled;
        self
    }

    pub fn level(&self) -> LogLevel {
        self.level
    }

    pub fn target(&self) -> &str {
        &self.target
    }
}

impl fmt::Debug for RequestLogger {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestLogger")
            .field("level", &self.level)
            .field("target", &self.target)
            .field("query", &self.query)
            .field("peer", &self.peer)
            .field("request_id", &self.request_id)
            .finish_non_exhaustive()
    }
}

impl Middleware for RequestLogger {
    fn call(&self, context: Context, next: Next) -> ServiceFuture {
        if !self.logger.enabled(self.level) {
            return next.run(context);
        }

        let method = context.method().clone();
        let path = context.path().to_owned();
        let query = if self.query {
            Some(context.query().to_owned())
        } else {
            None
        };
        let peer = if self.peer { context.peer_addr() } else { None };
        let request_id = if self.request_id {
            RequestId::of(&context)
                .or_else(|| context.headers().get(REQUEST_ID_HEADER).map(str::to_owned))
        } else {
            None
        };
        let version = context.version();
        let started = Instant::now();

        let logger = self.logger.clone();
        let level = self.level;
        let target = self.target.clone();

        Box::pin(async move {
            let result = next.run(context).await;
            let elapsed = started.elapsed();
            let milliseconds = elapsed.as_secs_f64() * 1_000.0;
            let status = match &result {
                Ok(response) => response.status(),
                Err(error) => error.status(),
            };

            let message = format!("{method} {path} -> {status} ({milliseconds:.2} ms)");
            let mut record = LogRecord::new(level, target, message)
                .with_field("method", method.as_str().to_owned())
                .with_field("path", path)
                .with_field("status", status.as_u16().to_string())
                .with_field("duration_ms", format!("{milliseconds:.3}"))
                .with_field("version", version.as_str().to_owned());

            if let Some(query) = query {
                record = record.with_field("query", query);
            }
            if let Some(peer) = peer {
                record = record.with_field("peer", peer.to_string());
            }
            if let Some(request_id) = request_id {
                record = record.with_field("request_id", request_id);
            }
            if let Err(error) = &result {
                record = record.with_field("error", error.to_string());
            }

            let _ = logger.log(record);
            result
        })
    }
}
