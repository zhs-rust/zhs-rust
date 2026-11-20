//! 商品服务:CRUD 业务逻辑 + 输入校验。
//!
//! SQL 与参数绑定(Dapper 风格)集中在这一层;控制器只做 HTTP 绑定与响应映射。

use std::sync::Arc;

use zhs_framework::entity::Database;
use zhs_framework::error::ApiError;
use zhs_framework::injectable;
use zhs_framework::prelude::{Params, Value, params};

use crate::models::dtos::{ProductInput, ProductPage};
use crate::models::entities::{CountRow, Product};

const MAX_PAGE_SIZE: u32 = 100;
const DEFAULT_PAGE_SIZE: u32 = 10;

pub struct ProductService {
    db: Arc<Database>,
}

#[injectable]
impl ProductService {
    #[inject]
    fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

impl ProductService {
    /// 商品列表:关键字(名称/分类模糊匹配)+ 分页。
    /// 值一律走 `params!`,用户输入永不进 SQL 文本。
    pub fn list(
        &self,
        keyword: Option<&str>,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<ProductPage, ApiError> {
        let keyword = keyword.map(str::trim).filter(|value| !value.is_empty());
        let page = page.unwrap_or(1).max(1);
        let page_size = page_size.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE);
        let offset = (u64::from(page) - 1) * u64::from(page_size);

        let (filter, filter_params) = match keyword {
            Some(keyword) => (
                " WHERE name LIKE CONCAT('%', ?, '%') OR category LIKE CONCAT('%', ?, '%')",
                vec![Value::Text(keyword.to_owned()), Value::Text(keyword.to_owned())],
            ),
            None => ("", Vec::new()),
        };

        let total: i64 = self
            .db
            .query_one::<CountRow>(
                &format!("SELECT COUNT(*) AS total FROM products{filter}"),
                &Params::new(filter_params.iter().cloned()),
            )?
            .map(|row| row.total)
            .unwrap_or(0);

        let sql = format!(
            "SELECT id, name, category, price, quantity, created_at, updated_at \
             FROM products{filter} ORDER BY id DESC LIMIT ? OFFSET ?"
        );
        let mut values = filter_params;
        values.push(Value::from(u64::from(page_size)));
        values.push(Value::from(offset));

        let items: Vec<Product> = self.db.query_as(&sql, &Params::new(values))?;

        Ok(ProductPage {
            items,
            total: total.max(0) as u64,
            page,
            page_size,
        })
    }

    /// 按 id 取单个商品;不存在返回 `None`。
    pub fn get(&self, id: i64) -> Result<Option<Product>, ApiError> {
        Ok(self.db.query_one(
            "SELECT id, name, category, price, quantity, created_at, updated_at \
             FROM products WHERE id = ?",
            &params![id],
        )?)
    }

    /// 新增商品。MariaDB 10.5+ 的 `INSERT ... RETURNING`
    /// 一次拿回含自增 id 的整行,避免 `LAST_INSERT_ID()` 在连接池下
    /// 拿到别的连接的值。
    pub fn create(&self, input: &ProductInput) -> Result<Product, ApiError> {
        Self::validate(input)?;

        let product: Option<Product> = self.db.query_one(
            "INSERT INTO products (name, category, price, quantity) \
             VALUES (?, ?, ?, ?) \
             RETURNING id, name, category, price, quantity, created_at, updated_at",
            &params![
                input.name.trim(),
                input.category.trim(),
                input.price,
                input.quantity
            ],
        )?;

        product.ok_or_else(|| ApiError::internal("INSERT ... RETURNING 未返回数据"))
    }

    /// 编辑商品;目标不存在返回 `None`,成功返回更新后的行。
    pub fn update(&self, id: i64, input: &ProductInput) -> Result<Option<Product>, ApiError> {
        Self::validate(input)?;

        let affected = self.db.execute(
            "UPDATE products \
             SET name = ?, category = ?, price = ?, quantity = ?, updated_at = NOW() \
             WHERE id = ?",
            &params![
                input.name.trim(),
                input.category.trim(),
                input.price,
                input.quantity,
                id
            ],
        )?;

        if affected == 0 {
            return Ok(None);
        }

        self.get(id)
    }

    /// 删除商品;目标不存在返回 `false`。
    pub fn delete(&self, id: i64) -> Result<bool, ApiError> {
        let affected = self.db.execute("DELETE FROM products WHERE id = ?", &params![id])?;

        Ok(affected > 0)
    }

    /// 输入校验;不合法返回 400。
    fn validate(input: &ProductInput) -> Result<(), ApiError> {
        let name = input.name.trim();
        if name.is_empty() {
            return Err(ApiError::bad_request("商品名称不能为空"));
        }
        if name.chars().count() > 100 {
            return Err(ApiError::bad_request("商品名称不能超过 100 个字符"));
        }
        if input.category.trim().is_empty() {
            return Err(ApiError::bad_request("分类不能为空"));
        }
        if input.category.trim().chars().count() > 50 {
            return Err(ApiError::bad_request("分类不能超过 50 个字符"));
        }
        if !input.price.is_finite() || input.price < 0.0 {
            return Err(ApiError::bad_request("单价必须是不小于 0 的数字"));
        }
        if input.quantity < 0 {
            return Err(ApiError::bad_request("数量必须是不小于 0 的整数"));
        }

        Ok(())
    }
}
