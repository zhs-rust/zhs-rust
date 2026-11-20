//! 商品 CRUD 端点。
//!
//! - GET /api/products?keyword=&page=&page_size=  列表(分页 + 关键字)
//! - GET /api/products/{id} / POST / PUT {id} / DELETE {id}(仅 admin)
//!
//! 除登录外所有接口要求 `#[authorize]`;DELETE 额外要求 admin 角色,
//! 演示基于角色的授权。控制器只做绑定与映射,业务在 `ProductService`。

use std::sync::Arc;

use zhs_framework::controller;
use zhs_framework::error::ApiError;
use zhs_framework::json;
use zhs_framework::prelude::{
    ApiResult, Json, Response, Results, StatusCode,
};

use crate::models::dtos::{ProductDto, ProductInput, ProductListDto};
use crate::services::ProductService;

pub struct ProductsController {
    products: Arc<ProductService>,
}

#[controller("/api/products")]
impl ProductsController {
    #[inject]
    fn new(products: Arc<ProductService>) -> Self {
        Self { products }
    }

    /// 商品列表。
    #[http_get]
    #[authorize]
    async fn list(
        &self,
        #[from_query] keyword: Option<String>,
        #[from_query] page: Option<u32>,
        #[from_query] page_size: Option<u32>,
    ) -> ApiResult<Json<ProductListDto>> {
        let result = self
            .products
            .list(keyword.as_deref(), page, page_size)?;

        Ok(Results::json(ProductListDto::from(result)))
    }

    /// 按 id 取单个商品。
    #[http_get("{id:long}")]
    #[authorize]
    async fn get(&self, #[from_route] id: i64) -> ApiResult<Json<ProductDto>> {
        let product = self.products.get(id)?.ok_or_else(ApiError::not_found)?;

        Ok(Results::json(ProductDto::from(product)))
    }

    /// 新增商品,201 + location。
    #[http_post]
    #[authorize]
    async fn create(&self, #[from_body] input: ProductInput) -> ApiResult<Response> {
        let product = self.products.create(&input)?;

        let location = format!("/api/products/{}", product.id);

        Ok(Response::json_raw(json::to_string(&ProductDto::from(product)))
            .with_status(StatusCode::CREATED)
            .with_header("location", location))
    }

    /// 编辑商品。
    #[http_put("{id:long}")]
    #[authorize]
    async fn update(
        &self,
        #[from_route] id: i64,
        #[from_body] input: ProductInput,
    ) -> ApiResult<Json<ProductDto>> {
        let product = self
            .products
            .update(id, &input)?
            .ok_or_else(ApiError::not_found)?;

        Ok(Results::json(ProductDto::from(product)))
    }

    /// 删除商品(仅 admin),204。
    #[http_delete("{id:long}")]
    #[authorize(roles = "admin")]
    async fn delete(&self, #[from_route] id: i64) -> ApiResult<Response> {
        if !self.products.delete(id)? {
            return Err(ApiError::not_found());
        }

        Ok(Response::new(StatusCode::NO_CONTENT))
    }
}
