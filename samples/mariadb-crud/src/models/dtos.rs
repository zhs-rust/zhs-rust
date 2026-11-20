//! 对外 JSON 的输入/输出 DTO。

use zhs_framework::prelude::{FromJson, ToJson};

use super::entities::{Product, UserRow};

/// 对外输出的商品 DTO。
#[derive(Debug, ToJson)]
pub struct ProductDto {
    pub id: i64,
    pub name: String,
    pub category: String,
    pub price: f64,
    pub quantity: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl From<Product> for ProductDto {
    fn from(product: Product) -> Self {
        Self {
            id: product.id,
            name: product.name,
            category: product.category,
            price: product.price.parse().unwrap_or(0.0),
            quantity: product.quantity,
            created_at: product.created_at,
            updated_at: product.updated_at,
        }
    }
}

/// 新增/编辑商品请求体。
#[derive(Debug, FromJson)]
pub struct ProductInput {
    pub name: String,
    pub category: String,
    pub price: f64,
    pub quantity: i64,
}

/// 服务层分页结果,由控制器转成响应 DTO。
#[derive(Debug)]
pub struct ProductPage {
    pub items: Vec<Product>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
}

/// 列表查询响应:分页信息 + 当前页数据。
#[derive(Debug, ToJson)]
pub struct ProductListDto {
    pub items: Vec<ProductDto>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
}

impl From<ProductPage> for ProductListDto {
    fn from(page: ProductPage) -> Self {
        Self {
            items: page.items.into_iter().map(ProductDto::from).collect(),
            total: page.total,
            page: page.page,
            page_size: page.page_size,
        }
    }
}

/// 登录请求体。
#[derive(Debug, FromJson)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// 登录响应体。
#[derive(Debug, ToJson)]
pub struct LoginResponse {
    pub token: String,
    pub expires_in: u64,
    pub user: UserInfoDto,
}

/// 当前用户信息。
#[derive(Debug, ToJson)]
pub struct UserInfoDto {
    pub username: String,
    pub display_name: String,
    pub role: String,
}

impl From<UserRow> for UserInfoDto {
    fn from(user: UserRow) -> Self {
        Self {
            username: user.username,
            display_name: user.display_name,
            role: user.role,
        }
    }
}
