use std::sync::Arc;

use zhs_framework::prelude::*;

struct AppState {
    greeting: String,
}

#[injectable]
impl AppState {
    #[inject]
    fn new() -> Self {
        Self {
            greeting: "Hello".to_owned(),
        }
    }
}

struct HelloController {
    state: Arc<AppState>,
}

#[controller("/api/[controller]")]
impl HelloController {
    #[inject]
    fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    #[http_get]
    async fn index(&self) -> ApiResult<String> {
        Ok(format!("{} from zhs", self.state.greeting))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = WebApplication::builder(std::env::args());

    builder.services.add_singleton::<AppState>();
    builder.services.add_controllers::<HelloController>();

    let mut app = builder.build()?;
    app.use_request_id();
    app.use_request_logging();
    app.map_controllers();
    app.map_health_checks("/health");
    app.map_openapi("/openapi/v1.json", "My API", "v1");
    app.use_swagger_ui("/swagger", "/openapi/v1.json");
    app.run()?;

    Ok(())
}
