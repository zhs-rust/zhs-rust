//! 服务进程入口(等价 Program.cs):配置 → 连库 → DI 注册 → 中间件 → 启动。
//!
//! 建表与种子数据由独立的迁移工具完成:`cargo run --bin migrate`。

use zhs_crypto::password::PasswordHasher;
use zhs_framework::prelude::{StaticFiles, WebApplication};

use mariadb_crud::controllers::{AuthController, ProductsController};
use mariadb_crud::services::{AuthService, ProductService};
use mariadb_crud::settings::Settings;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = WebApplication::builder(std::env::args());
    let settings = Settings::from(&builder.configuration)?;

    // 数据访问 + 认证
    builder.services.add_database(settings.mariadb.connect()?);
    let codec = settings.jwt.codec()?;
    builder
        .services
        .add_singleton_instance(PasswordHasher::recommended());
    builder.services.add_singleton_instance(codec.clone());

    // 业务服务
    builder.services.add_singleton::<AuthService>();
    builder.services.add_singleton::<ProductService>();

    // 控制器
    builder.services.add_controllers::<AuthController>();
    builder.services.add_controllers::<ProductsController>();

    let app = builder.build()?;
    let mut app = app;

    app.use_request_id();
    app.use_request_logging();
    app.use_jwt_bearer(codec);
    // wwwroot/index.html = 登录页, wwwroot/app.html = 商品管理页
    app.use_static_files(StaticFiles::new("wwwroot"));
    app.map_controllers();
    app.map_health_checks("/health");
    app.map_openapi("/openapi/v1.json", "mariadb-crud API", "v1");
    app.use_swagger_ui("/swagger", "/openapi/v1.json");

    println!("listening on http://{}", app.address());
    app.run()?;

    Ok(())
}
