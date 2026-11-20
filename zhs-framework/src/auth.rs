use std::collections::HashMap;
use std::sync::Arc;

use zhs_http::auth::{ClaimsPrincipal, JwtBearer, JwtCodec};
use zhs_http::{HttpContext, ServiceResolver};

use crate::error::ApiError;

#[derive(Debug, Clone, Default)]
pub struct Policy {
    roles: Vec<String>,
}

impl Policy {
    pub fn require_role(mut self, role: impl Into<String>) -> Self {
        self.roles.push(role.into());
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct AuthorizationOptions {
    policies: HashMap<String, Policy>,
}

impl AuthorizationOptions {
    pub fn add_policy(
        &mut self,
        name: impl Into<String>,
        configure: impl FnOnce(Policy) -> Policy,
    ) -> &mut Self {
        self.policies
            .insert(name.into(), configure(Policy::default()));
        self
    }

    pub fn policy(&self, name: &str) -> Option<&Policy> {
        self.policies.get(name)
    }
}

pub fn authorize(
    context: &HttpContext,
    policy: Option<&str>,
    roles: &[&str],
) -> Result<(), ApiError> {
    if !context.user.is_authenticated() {
        return Err(ApiError::unauthorized());
    }

    for role in roles {
        if !context.user.is_in_role(role) {
            return Err(ApiError::forbidden());
        }
    }

    if let Some(policy_name) = policy {
        let options = resolve_options(&context.services)?;
        let policy = options
            .policy(policy_name)
            .ok_or_else(|| ApiError::internal(format!("unknown policy `{policy_name}`")))?;

        for role in &policy.roles {
            if !context.user.is_in_role(role) {
                return Err(ApiError::forbidden());
            }
        }
    }

    Ok(())
}

pub fn authenticate(context: &mut HttpContext, bearer: &JwtBearer) {
    if let Some(principal) = bearer.authenticate(context.request.header("authorization")) {
        context.user = principal;
    }
}

pub fn current_user(context: &HttpContext) -> &ClaimsPrincipal {
    &context.user
}

pub fn jwt_bearer(codec: JwtCodec) -> JwtBearer {
    JwtBearer::from_codec(codec)
}

fn resolve_options(
    services: &Arc<dyn ServiceResolver>,
) -> Result<Arc<AuthorizationOptions>, ApiError> {
    services
        .resolve(std::any::TypeId::of::<AuthorizationOptions>())
        .and_then(|value| value.downcast::<AuthorizationOptions>().ok())
        .ok_or_else(|| {
            ApiError::internal("authorization is used but `add_authorization` was not called")
        })
}
