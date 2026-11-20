use std::fs;

use zhs_framework::prelude::*;

#[derive(Default)]
struct UploadController;

#[controller("/upload")]
impl UploadController {
    #[http_post]
    async fn upload(&self, #[from_files] files: Vec<FormFile>) -> ApiResult<String> {
        Ok(format!("{}:{}", files.len(), files[0].file_name))
    }
}

#[zhs_framework::test]
async fn rate_limit_and_static_files() {
    let root = std::env::temp_dir().join(format!("zhs-static-{}", Guid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("hello.txt"), "static hello").unwrap();

    let static_root = root.clone();

    let app = TestApplication::builder()
        .services(|services| {
            services.add_controllers::<UploadController>();
        })
        .configure(move |app| {
            app.use_rate_limiting(RateLimiter::per_minute(1));
            app.use_static_files(StaticFiles::new(&static_root).prefix("/assets"));
        })
        .build();

    let client = app.client();

    let response = client.get("/assets/hello.txt").send();
    response.assert_status(200);
    assert_eq!(response.text(), "static hello");

    let response = client.get("/assets/hello.txt").send();
    response.assert_status(429);
    assert!(response.header("retry-after").is_some());

    let _ = fs::remove_dir_all(root);
}

#[zhs_framework::test]
async fn multipart_file_upload() {
    let app = TestApplication::builder()
        .services(|services| {
            services.add_controllers::<UploadController>();
        })
        .build();

    let body = concat!(
        "--X\r\n",
        "Content-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "hello\r\n",
        "--X--\r\n",
    );

    let response = app
        .client()
        .post("/upload")
        .header("content-type", "multipart/form-data; boundary=X")
        .body(body)
        .send();

    response.assert_status(200);
    assert_eq!(response.text(), "1:a.txt");
}
