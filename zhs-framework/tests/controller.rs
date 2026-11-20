use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use zhs_framework::prelude::*;

struct AppState {
    greeting: String,
}

#[injectable]
impl AppState {
    #[inject]
    fn new() -> Self {
        Self {
            greeting: "Hello".to_owned(),
        }
    }
}

struct HelloController {
    state: Arc<AppState>,
    logger: Logger<HelloController>,
}

#[derive(FromJson, ToJson)]
struct Echo {
    message: String,
    count: Option<u32>,
}

#[controller("/api/[controller]")]
impl HelloController {
    #[inject]
    fn new(state: Arc<AppState>, logger: Logger<HelloController>) -> Self {
        Self { state, logger }
    }

    #[http_get]
    async fn index(&self) -> ApiResult<String> {
        self.logger.info("hello index");
        Ok(format!("{} from zhs", self.state.greeting))
    }

    #[http_get("/{name}")]
    async fn greet(&self, #[from_route] name: String) -> ApiResult<String> {
        Ok(format!("Hello, {name}!"))
    }

    #[http_get("/query")]
    async fn query(&self, #[from_query] name: Option<String>) -> ApiResult<String> {
        Ok(format!("query: {}", name.unwrap_or_default()))
    }

    #[http_get("/id/{id:int}")]
    async fn by_id(&self, #[from_route] id: i64) -> ApiResult<String> {
        Ok(format!("id: {id}"))
    }

    #[http_post]
    async fn echo(&self, #[from_body] body: String) -> ApiResult<String> {
        Ok(body)
    }

    #[http_post("/json")]
    async fn json(&self, #[from_body] input: Echo) -> ApiResult<Json<Echo>> {
        Ok(Results::json(input))
    }

    #[http_get("/me")]
    #[authorize]
    async fn me(&self, #[from_user] user: ClaimsPrincipal) -> ApiResult<String> {
        Ok(user.name().unwrap_or("anonymous").to_owned())
    }

    #[http_get("/admin")]
    #[authorize(roles = "admin")]
    async fn admin(&self) -> ApiResult<String> {
        Ok("admin".to_owned())
    }
}

const JWT_SECRET: &[u8] = b"01234567890123456789012345678901";

fn start() -> (zhs_framework::HostHandle, String, String) {
    let codec = JwtCodec::new(JWT_SECRET).unwrap();
    let token = codec
        .issue(&Claims::new("ada").with_roles(["admin"]))
        .unwrap();
    let user_token = codec
        .issue(&Claims::new("bob").with_roles(["user"]))
        .unwrap();

    let mut builder = WebApplication::builder(Vec::<String>::new()).address("127.0.0.1:0");
    builder.services.add_singleton::<AppState>();
    builder.services.add_controllers::<HelloController>();

    let app = builder.build().unwrap();
    let mut app = app;
    app.use_jwt_bearer(codec);
    app.use_fn(|context, next| {
        Box::pin(async move {
            let mut response = next(context).await?;
            response
                .headers
                .push(("x-test-middleware".to_owned(), "1".to_owned()));
            Ok(response)
        })
    });
    (app.spawn().unwrap(), token, user_token)
}

fn request(address: std::net::SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    stream.write_all(request.as_bytes()).unwrap();

    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();

    response
}

fn get(address: std::net::SocketAddr, path: &str) -> String {
    request(
        address,
        &format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"),
    )
}

fn get_with_token(address: std::net::SocketAddr, path: &str, token: &str) -> String {
    request(
        address,
        &format!(
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
        ),
    )
}

#[test]
fn controller_routes_bind_and_return_responses() {
    let (host, token, user_token) = start();
    let address = host.local_addr();

    let response = get(address, "/api/hello");
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("x-test-middleware: 1"), "{response}");
    assert!(response.ends_with("Hello from zhs"), "{response}");

    let response = get(address, "/api/hello/Ada");
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.ends_with("Hello, Ada!"), "{response}");

    let response = get(address, "/api/hello/query?name=Bob");
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.ends_with("query: Bob"), "{response}");

    let response = get(address, "/api/hello/id/42");
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.ends_with("id: 42"), "{response}");

    let response = get(address, "/api/hello/id/not-a-number");
    assert!(response.starts_with("HTTP/1.1 404 Not Found"), "{response}");

    let response = request(
        address,
        "POST /api/hello HTTP/1.1\r\nHost: localhost\r\nContent-Length: 8\r\nConnection: close\r\n\r\nraw body",
    );
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.ends_with("raw body"), "{response}");

    let response = request(
        address,
        "POST /api/hello/json HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 26\r\nConnection: close\r\n\r\n{\"message\":\"hi\",\"count\":2}",
    );
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("application/json"), "{response}");
    assert!(
        response.ends_with(r#"{"message":"hi","count":2}"#),
        "{response}"
    );

    let response = get(address, "/missing");
    assert!(response.starts_with("HTTP/1.1 404 Not Found"), "{response}");

    let response = get(address, "/api/hello/me");
    assert!(
        response.starts_with("HTTP/1.1 401 Unauthorized"),
        "{response}"
    );

    let response = get_with_token(address, "/api/hello/me", &token);
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.ends_with("ada"), "{response}");

    let response = get_with_token(address, "/api/hello/admin", &token);
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");

    let response = get_with_token(address, "/api/hello/admin", &user_token);
    assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");

    let response = get(address, "/api/hello/admin");
    assert!(
        response.starts_with("HTTP/1.1 401 Unauthorized"),
        "{response}"
    );
}
