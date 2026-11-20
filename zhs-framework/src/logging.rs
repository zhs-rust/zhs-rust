pub use zhs_log::*;

use zhs_http::resolve_required;

use crate::di::{DiError, Injectable};

impl<T: 'static> Injectable for zhs_log::Logger<T> {
    fn create(services: &dyn zhs_http::ServiceResolver) -> Result<Self, DiError> {
        let factory = resolve_required::<zhs_log::LoggerFactory>(services)
            .map_err(|error| DiError::MissingService(error.to_string()))?;

        Ok(zhs_log::Logger::from_factory(&factory))
    }
}
