//! mariadb-crud 库根:控制器 / 服务 / 模型 / 迁移 / 配置。
//!
//! 服务进程(`src/main.rs`)与迁移工具(`src/bin/migrate.rs`)共用本库。

pub mod controllers;
pub mod migrations;
pub mod models;
pub mod services;
pub mod settings;
