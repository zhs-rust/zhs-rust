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

    #[http_get("/{name}")]
    async fn greet(&self, #[from_route] name: String) -> ApiResult<String> {
        Ok(format!("Hello, {name}!"))
    }

    #[http_get("/query")]
    async fn query(&self, #[from_query] name: Option<String>) -> ApiResult<String> {
        Ok(format!("query: {}", name.unwrap_or_default()))
    }

    #[http_post]
    async fn echo(&self, #[from_body] body: String) -> ApiResult<String> {
        Ok(body)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = WebApplication::builder(std::env::args());
    builder.services.add_singleton::<AppState>();
    builder.services.add_controllers::<HelloController>();

    let mut app = builder.build()?;
    app.map_controllers();

    println!("listening on http://{}", app.address());
    app.run()?;

    Ok(())
}
