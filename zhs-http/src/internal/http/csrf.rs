use super::auth::cookie_value;
use super::context::Context;
use super::method::Method;
use super::pipeline::{Middleware, Next, problem_response};
use super::random::random_hex;
use super::service::ServiceFuture;
use super::status::StatusCode;

const TOKEN_BYTES: usize = 32;

#[derive(Clone, Debug)]
pub struct Csrf {
    cookie_name: String,
    header_name: String,
    secure: bool,
}

impl Csrf {
    pub fn new(cookie_name: impl Into<String>, header_name: impl Into<String>) -> Self {
        Self {
            cookie_name: cookie_name.into(),
            header_name: header_name.into(),
            secure: true,
        }
    }

    pub fn with_secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    pub fn cookie_name(&self) -> &str {
        &self.cookie_name
    }

    pub fn header_name(&self) -> &str {
        &self.header_name
    }

    fn is_safe_method(method: &Method) -> bool {
        matches!(method, Method::Get | Method::Head | Method::Options)
    }

    fn set_cookie(&self, token: &str) -> String {
        let mut cookie = format!("{}={token}; SameSite=Lax; Path=/", self.cookie_name);
        if self.secure {
            cookie.push_str("; Secure");
        }
        // Deliberately no HttpOnly: the double-submit pattern requires
        // client-side JavaScript to read the cookie and echo it in a header.
        cookie
    }
}

impl Middleware for Csrf {
    fn call(&self, context: Context, next: Next) -> ServiceFuture {
        if Self::is_safe_method(context.method()) {
            let existing = cookie_value(context.headers(), &self.cookie_name)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            let issued = existing.is_none();
            let token = existing.unwrap_or_else(|| random_hex(TOKEN_BYTES));
            let cookie = if issued {
                Some(self.set_cookie(&token))
            } else {
                None
            };

            return Box::pin(async move {
                let mut response = next.run(context).await?;
                if let Some(cookie) = cookie {
                    response.headers_mut().append("Set-Cookie", cookie);
                }
                Ok(response)
            });
        }

        let cookie = cookie_value(context.headers(), &self.cookie_name);
        let header = context.headers().get(&self.header_name);

        match (cookie, header) {
            (Some(cookie), Some(header))
                if constant_time_eq(cookie.as_bytes(), header.as_bytes()) =>
            {
                next.run(context)
            }
            _ => Box::pin(async move {
                Ok(problem_response(
                    StatusCode::FORBIDDEN,
                    Some("CSRF token is missing or does not match"),
                ))
            }),
        }
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let length = left.len().max(right.len());
    let mut difference = left.len() ^ right.len();

    for index in 0..length {
        let left = left.get(index).copied().unwrap_or(0);
        let right = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left ^ right);
    }

    difference == 0
}

#[cfg(test)]
mod tests {
    use super::{Csrf, constant_time_eq};

    #[test]
    fn compares_without_short_circuiting() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"short", b"longer"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn cookie_is_secure_by_default() {
        let csrf = Csrf::new("csrf_token", "X-CSRF-Token");
        assert_eq!(
            csrf.set_cookie("abc123"),
            "csrf_token=abc123; SameSite=Lax; Path=/; Secure"
        );
        assert!(!csrf.set_cookie("abc123").contains("HttpOnly"));
    }

    #[test]
    fn secure_flag_can_be_disabled() {
        let csrf = Csrf::new("csrf_token", "X-CSRF-Token").with_secure(false);
        assert_eq!(
            csrf.set_cookie("abc123"),
            "csrf_token=abc123; SameSite=Lax; Path=/"
        );
    }
}
