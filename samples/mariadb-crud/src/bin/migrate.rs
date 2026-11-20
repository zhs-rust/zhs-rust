//! 迁移工具:建表 + 种子账号。
//!
//! ```bash
//! cargo run --bin migrate            # 开发环境
//! cargo run --release --bin migrate  # 发布
//! ```
//!
//! 对应 ASP.NET Core 的 `dotnet ef database update`:
//! 与服务进程完全分离,服务启动不做任何 DDL。

use mariadb_crud::migrations;
use mariadb_crud::settings::{self, Settings};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let configuration = settings::standalone_configuration(&arguments);
    let settings = Settings::from(&configuration)?;

    let database = settings.mariadb.connect()?;

    migrations::run(&database)?;
    println!("migrations applied to `{}`", settings.mariadb.database);

    Ok(())
}
