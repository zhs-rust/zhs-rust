//! 数据库实体(FromRow 行映射)。
//!
//! 注意:`zhs-entity-mariadb` 的二进制协议解码里,DECIMAL / DATETIME
//! 都以文本(`Value::String`)返回,所以 `Product.price` 用 `String` 承载,
//! 转 DTO 时再解析成 `f64`。

use zhs_framework::prelude::FromRow;

/// products 表的一行。`#[table]` 生成 `Product::TABLE` 常量。
#[derive(Debug, FromRow)]
#[table("products")]
pub struct Product {
    pub id: i64,
    pub name: String,
    pub category: String,
    /// DECIMAL(10,2) 经驱动解码为文本,如 "12.50"。
    pub price: String,
    pub quantity: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// users 表的一行(登录校验用)。
#[derive(Debug, FromRow)]
#[table("users")]
pub struct UserRow {
    #[allow(dead_code)]
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub display_name: String,
    pub role: String,
}

/// `SELECT COUNT(*) AS total` 的结果行。
#[derive(Debug, FromRow)]
pub struct CountRow {
    pub total: i64,
}
