use zhs_framework::prelude::*;

#[derive(Default)]
struct HealthController;

#[controller("/health-check")]
impl HealthController {
    #[http_get]
    async fn index(&self) -> ApiResult<String> {
        Ok("ok".to_owned())
    }
}

#[zhs_framework::test]
async fn health_checks_and_cors() {
    let app = TestApplication::builder()
        .services(|services| {
            services.add_controllers::<HealthController>();
            services.add_health_checks(|health| {
                health.add("self", || Ok::<(), String>(()));
            });
        })
        .configure(|app| {
            app.map_health_checks("/health");
            app.map_openapi("/openapi.json", "Test API", "1.0.0");
            app.use_swagger_ui("/swagger", "/openapi.json");
            app.use_cors(
                CorsOptions::new()
                    .allow_any_origin()
                    .allow_methods(["GET", "OPTIONS"])
                    .allow_headers(["content-type"]),
            );
        })
        .build();

    let client = app.client();

    let response = client.get("/health").send();
    response.assert_status(200);
    let report: zhs_framework::json::Value = response.json().unwrap();
    assert_eq!(report["status"].as_str(), Some("Healthy"));

    let response = client
        .get("/health-check")
        .header("origin", "https://example.com")
        .send();
    response.assert_status(200);
    assert_eq!(response.header("access-control-allow-origin"), Some("*"));

    let response = client
        .request(Method::Options, "/health-check")
        .header("origin", "https://example.com")
        .header("access-control-request-method", "GET")
        .send();
    response.assert_status(204);
    assert_eq!(
        response.header("access-control-allow-methods"),
        Some("GET, OPTIONS")
    );

    let response = client.get("/openapi.json").send();
    response.assert_status(200);
    let openapi: zhs_framework::json::Value = response.json().unwrap();
    assert_eq!(openapi["openapi"].as_str(), Some("3.0.0"));
    assert!(openapi["paths"]["/health-check"]["get"].is_object());

    let response = client.get("/swagger").send();
    response.assert_status(200);
    assert!(response.text().contains("SwaggerUIBundle"));
}
