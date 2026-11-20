use zhs_framework::prelude::*;

#[derive(Default)]
struct TestController;

#[controller("/test")]
impl TestController {
    #[http_get]
    async fn hello(&self) -> ApiResult<String> {
        Ok("hello from test".to_owned())
    }
}

#[zhs_framework::test]
async fn test_application_and_client() {
    let app = TestApplication::new(|services| {
        services.add_controllers::<TestController>();
    });

    let response = app.client().get("/test").send();

    response.assert_status(200);
    assert_eq!(response.text(), "hello from test");
}
