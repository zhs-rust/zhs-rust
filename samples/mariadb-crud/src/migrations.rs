//! 数据库迁移:执行 `migrations/*.sql` 建表脚本并写入种子账号。
//!
//! 仅供独立的迁移工具 `cargo run --bin migrate` 调用;
//! 服务进程(main.rs)不做任何 DDL —— 对应 ASP.NET Core 里
//! `dotnet ef database update` 与应用启动分离的约定。

use zhs_crypto::password::PasswordHasher;
use zhs_framework::entity::{Database, Params};
use zhs_framework::prelude::{params, FromRow};

/// 执行全部迁移:建表脚本 + 种子账号(幂等,可反复执行)。
pub fn run(database: &Database) -> Result<(), Box<dyn std::error::Error>> {
    apply_script(database, "migrations/0001_init.sql")?;
    seed_users(database)?;
    Ok(())
}

/// 按分号拆分逐句执行 SQL 脚本(驱动走 prepared 语句,不支持多语句),
/// 要求脚本本身幂等。
fn apply_script(database: &Database, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let sql = std::fs::read_to_string(path)
        .map_err(|error| format!("读取 {path} 失败(请在 sample 目录下运行): {error}"))?;

    for statement in split_statements(&sql) {
        database
            .execute(&statement, &Params::default())
            .map_err(|error| -> Box<dyn std::error::Error> {
                format!("执行迁移失败: {error}\n语句: {statement}").into()
            })?;
    }

    Ok(())
}

/// 去掉 `--` 注释行后按 `;` 拆分,保留非空语句。
fn split_statements(sql: &str) -> Vec<String> {
    let without_comments = sql
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");

    without_comments
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `SELECT id AS user_id` 的结果行(字段仅用于 FromRow 映射)。
#[derive(FromRow)]
struct UserIdRow {
    #[allow(dead_code)]
    user_id: i64,
}

/// 按用户名写入两个演示账号(已存在则跳过):
/// admin/admin123(管理员,可删除)与 demo/demo123(普通用户)。
fn seed_users(database: &Database) -> Result<(), Box<dyn std::error::Error>> {
    let hasher = PasswordHasher::recommended();

    let accounts = [
        ("admin", "admin123", "系统管理员", "admin"),
        ("demo", "demo123", "演示用户", "user"),
    ];

    for (username, password, display_name, role) in accounts {
        let exists: Option<UserIdRow> = database.query_one(
            "SELECT id AS user_id FROM users WHERE username = ?",
            &params![username],
        )?;

        if exists.is_some() {
            continue;
        }

        let hash = hasher.hash(password)?;
        database.execute(
            "INSERT INTO users (username, password_hash, display_name, role) VALUES (?, ?, ?, ?)",
            &params![username, hash, display_name, role],
        )?;
        println!("seeded default account `{username}` (密码 {password})");
    }

    Ok(())
}
