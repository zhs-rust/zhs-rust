use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use zhs_log::runtime::Logger;
use zhs_mn::scheduler::Scheduler;

use super::context::Context;
use super::pipeline::{Middleware, Next, log_server_error, problem_response};
use super::request_id::{REQUEST_ID_HEADER, RequestId};
use super::service::ServiceFuture;
use super::status::StatusCode;

#[derive(Clone)]
pub struct Timeout {
    duration: Duration,
    scheduler: Arc<Scheduler>,
    logger: Option<Logger>,
}

impl Timeout {
    pub fn new(duration: Duration) -> Result<Self, zhs_mn::SchedulerError> {
        let scheduler = Scheduler::new(1)?;

        Ok(Self {
            duration,
            scheduler: Arc::new(scheduler),
            logger: None,
        })
    }

    pub fn with_logger(mut self, logger: Logger) -> Self {
        self.logger = Some(logger);
        self
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }
}

impl fmt::Debug for Timeout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Timeout")
            .field("duration", &self.duration)
            .field("logger", &self.logger)
            .finish_non_exhaustive()
    }
}

impl Middleware for Timeout {
    fn call(&self, context: Context, next: Next) -> ServiceFuture {
        let method = context.method().clone();
        let path = context.path().to_owned();
        let request_id = RequestId::of(&context)
            .or_else(|| context.headers().get(REQUEST_ID_HEADER).map(str::to_owned));
        let duration = self.duration;
        let scheduler = Arc::clone(&self.scheduler);
        let logger = self.logger.clone();

        Box::pin(async move {
            match scheduler.timeout(duration, next.run(context)).await {
                Ok(result) => result,
                Err(_) => {
                    log_server_error(
                        logger.as_ref(),
                        StatusCode::GATEWAY_TIMEOUT,
                        &method,
                        &path,
                        request_id.as_deref(),
                        Some("the request deadline elapsed"),
                    );

                    Ok(problem_response(
                        StatusCode::GATEWAY_TIMEOUT,
                        Some("the request deadline elapsed"),
                    ))
                }
            }
        })
    }
}
