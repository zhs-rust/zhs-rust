//! 认证服务:用户查询、密码校验、令牌签发。

use std::sync::Arc;

use zhs_crypto::password::PasswordHasher;
use zhs_framework::entity::Database;
use zhs_framework::error::ApiError;
use zhs_framework::http::auth::JwtCodec;
use zhs_framework::injectable;
use zhs_framework::prelude::{params, Claims};

use crate::models::entities::UserRow;

pub struct AuthService {
    db: Arc<Database>,
    hasher: Arc<PasswordHasher>,
    codec: Arc<JwtCodec>,
}

#[injectable]
impl AuthService {
    #[inject]
    fn new(db: Arc<Database>, hasher: Arc<PasswordHasher>, codec: Arc<JwtCodec>) -> Self {
        Self { db, hasher, codec }
    }
}

impl AuthService {
    /// 校验用户名密码;凭据有效返回用户行,无效返回 `None`。
    /// 数据层错误经 `From<zhs_entity::Error> for ApiError` 自动转 409/500。
    pub fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Option<UserRow>, ApiError> {
        let user: Option<UserRow> = self.db.query_one(
            "SELECT id, username, password_hash, display_name, role \
             FROM users WHERE username = ?",
            &params![username],
        )?;

        let Some(user) = user else {
            return Ok(None);
        };

        let matched = self
            .hasher
            .verify(password, &user.password_hash)
            .map_err(|error| ApiError::internal(format!("密码校验失败: {error}")))?;

        Ok(matched.then_some(user))
    }

    /// 为指定用户签发 JWT,返回 `(token, 有效期秒数)`。
    /// codec 的 iss/aud 会在签发时自动补进令牌,无需手动镜像。
    pub fn issue_token(&self, user: &UserRow) -> Result<(String, u64), ApiError> {
        let token = self
            .codec
            .issue(
                &Claims::new(user.username.clone())
                    .with_roles([user.role.clone()])
                    .with_claim("name", user.display_name.clone()),
            )
            .map_err(|error| ApiError::internal(format!("签发令牌失败: {error}")))?;

        Ok((token, self.codec.lifetime()))
    }
}
