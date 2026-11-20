//! 登录认证端点:POST /api/auth/login、GET /api/auth/me。
//! 控制器只做 HTTP 绑定与响应映射,业务在 `AuthService`。

use std::sync::Arc;

use zhs_framework::controller;
use zhs_framework::error::ApiError;
use zhs_framework::prelude::{
    ApiResult, ClaimsPrincipal, Json, Results,
};

use crate::models::dtos::{LoginRequest, LoginResponse, UserInfoDto};
use crate::services::AuthService;

pub struct AuthController {
    auth: Arc<AuthService>,
}

#[controller("/api/auth")]
impl AuthController {
    #[inject]
    fn new(auth: Arc<AuthService>) -> Self {
        Self { auth }
    }

    /// 用户名密码登录,成功返回 JWT。
    #[http_post("/login")]
    async fn login(&self, #[from_body] request: LoginRequest) -> ApiResult<Json<LoginResponse>> {
        let username = request.username.trim();

        if username.is_empty() || request.password.is_empty() {
            return Err(ApiError::bad_request("用户名和密码不能为空"));
        }

        // 用户不存在与密码错误同一个提示,避免暴露账号是否存在
        let user = self
            .auth
            .authenticate(username, &request.password)?
            .ok_or_else(|| ApiError::unauthorized().with_detail("用户名或密码错误"))?;

        let (token, expires_in) = self.auth.issue_token(&user)?;

        Ok(Results::json(LoginResponse {
            token,
            expires_in,
            user: UserInfoDto::from(user),
        }))
    }

    /// 校验当前令牌并返回用户信息(前端可用于令牌自检)。
    #[http_get("/me")]
    #[authorize]
    async fn me(&self, #[from_user] user: ClaimsPrincipal) -> ApiResult<Json<UserInfoDto>> {
        let username = user.name().unwrap_or_default().to_owned();
        let display_name = user
            .find_first("name")
            .map(|claim| claim.value.clone())
            .unwrap_or_else(|| username.clone());
        let role = user
            .find_first("role")
            .map(|claim| claim.value.clone())
            .unwrap_or_default();

        Ok(Results::json(UserInfoDto {
            username,
            display_name,
            role,
        }))
    }
}
