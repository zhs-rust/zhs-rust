use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use crate::internal::http::{
    Request as ZhsRequest, Response as ZhsResponse, Server as ZhsServer,
    ServerHandle as ZhsServerHandle,
};

use crate::context::{HttpContext, ServiceResolver};
use crate::method::Method;
use crate::request::Request;
use crate::response::{Response, StatusCode};
use crate::router::{HandlerFn, Router};

pub struct Server {
    inner: ZhsServer,
    address: SocketAddr,
}

impl Server {
    pub fn bind(address: &str) -> std::io::Result<Self> {
        let inner = ZhsServer::bind(address)?;
        let address = inner.local_addr()?;

        Ok(Self { inner, address })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }

    pub fn serve(self, router: Router, services: Arc<dyn ServiceResolver>) -> std::io::Result<()> {
        let router = Arc::new(router);
        let handler: HandlerFn = Arc::new(move |context| router.dispatch(context));

        self.serve_with(handler, services)
    }

    pub fn serve_with(
        self,
        handler: HandlerFn,
        services: Arc<dyn ServiceResolver>,
    ) -> std::io::Result<()> {
        self.inner.run(move |request| {
            let handler = Arc::clone(&handler);
            let services = Arc::clone(&services);

            async move { Ok(bridge_request(request, handler, services).await) }
        })
    }

    pub fn spawn_with(
        self,
        handler: HandlerFn,
        services: Arc<dyn ServiceResolver>,
    ) -> std::io::Result<ServerHandle> {
        let address = self.address;
        let inner = self.inner.spawn(move |request| {
            let handler = Arc::clone(&handler);
            let services = Arc::clone(&services);

            async move { Ok(bridge_request(request, handler, services).await) }
        })?;

        Ok(ServerHandle { inner, address })
    }
}

pub struct ServerHandle {
    inner: ZhsServerHandle,
    address: SocketAddr,
}

impl ServerHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }

    pub fn is_shutdown(&self) -> bool {
        self.inner.is_shutdown()
    }

    pub fn shutdown(&self) -> std::io::Result<()> {
        self.inner.shutdown()
    }

    pub fn wait(self) -> std::io::Result<()> {
        while !self.inner.is_shutdown() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Ok(())
    }
}

async fn bridge_request(
    request: ZhsRequest,
    handler: HandlerFn,
    services: Arc<dyn ServiceResolver>,
) -> ZhsResponse {
    let method_text = request.method().as_str().to_owned();
    let path = request.path().to_owned();
    let query = parse_query(request.query());
    let http_version = request.version().as_str().to_owned();
    let peer_addr = request.peer_addr();
    let headers = request
        .headers()
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
        .collect::<HashMap<_, _>>();

    let mut request = request;
    let body = match request.body_mut().bytes().await {
        Ok(body) => body,
        Err(error) => {
            return to_zhs_response(
                Response::text(error.to_string()).with_status(StatusCode::BAD_REQUEST),
            );
        }
    };

    let request = Request {
        method: Method::from_str(&method_text).unwrap_or(Method::Get),
        path,
        http_version,
        query,
        headers,
        body,
        peer_addr,
    };

    let context = HttpContext::new(request, services.create_scope());
    let response = match handler(context).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };

    to_zhs_response(response)
}

fn to_zhs_response(response: Response) -> ZhsResponse {
    let status = crate::internal::http::StatusCode::new(response.status.0)
        .unwrap_or(crate::internal::http::StatusCode::INTERNAL_SERVER_ERROR);

    let mut result = ZhsResponse::new(status);

    for (name, value) in response.headers {
        result = result.with_header(name, value);
    }

    result.with_body(response.body)
}

fn parse_query(query: &str) -> HashMap<String, Vec<String>> {
    let mut values: HashMap<String, Vec<String>> = HashMap::new();

    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }

        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(key);
        let value = percent_decode(value);

        values.entry(key).or_default().push(value);
    }

    values
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let high = hex_value(bytes[index + 1]);
                let low = hex_value(bytes[index + 2]);

                if let (Some(high), Some(low)) = (high, low) {
                    output.push(high << 4 | low);
                    index += 3;
                    continue;
                }

                output.push(b'%');
                index += 1;
            }
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8_lossy(&output).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
