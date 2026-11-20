use zhs_framework::prelude::*;

#[zhs_framework::test]
async fn host_can_shut_down_gracefully() {
    let builder = WebApplication::builder(std::iter::empty::<String>());
    let builder = builder.address("127.0.0.1:0");

    let app = builder.build().unwrap();
    let host = app.spawn().unwrap();

    host.shutdown().unwrap();

    assert!(!host.is_running());
    host.wait().unwrap();
}
