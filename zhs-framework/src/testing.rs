use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};

use zhs_http::Method;
use zhs_json::{FromJson, ToJson};

use crate::{HostHandle, ServiceCollection, WebApplication};

pub use zhs_http::executor::block_on;

pub struct TestApplication {
    _host: HostHandle,
    address: SocketAddr,
}

impl TestApplication {
    pub fn new<F>(configure_services: F) -> Self
    where
        F: FnOnce(&mut ServiceCollection) + 'static,
    {
        Self::builder().services(configure_services).build()
    }

    pub fn builder() -> TestApplicationBuilder {
        TestApplicationBuilder::default()
    }

    pub fn client(&self) -> TestClient {
        TestClient {
            address: self.address,
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }
}

type ConfigureServices = Box<dyn FnOnce(&mut ServiceCollection)>;
type ConfigureApp = Box<dyn FnOnce(&mut WebApplication)>;

#[derive(Default)]
pub struct TestApplicationBuilder {
    configure_services: Option<ConfigureServices>,
    configure_app: Option<ConfigureApp>,
}

impl TestApplicationBuilder {
    pub fn services<F>(mut self, configure: F) -> Self
    where
        F: FnOnce(&mut ServiceCollection) + 'static,
    {
        self.configure_services = Some(Box::new(configure));
        self
    }

    pub fn configure<F>(mut self, configure: F) -> Self
    where
        F: FnOnce(&mut WebApplication) + 'static,
    {
        self.configure_app = Some(Box::new(configure));
        self
    }

    pub fn build(self) -> TestApplication {
        let mut builder = WebApplication::builder(Vec::<String>::new()).address("127.0.0.1:0");

        if let Some(configure) = self.configure_services {
            configure(&mut builder.services);
        }

        let mut app = builder.build().expect("test application failed to build");

        if let Some(configure) = self.configure_app {
            configure(&mut app);
        }

        let host = app.spawn().expect("test application failed to start");
        let address = host.local_addr();

        TestApplication {
            _host: host,
            address,
        }
    }
}

#[derive(Clone)]
pub struct TestClient {
    address: SocketAddr,
}

impl TestClient {
    pub fn get(&self, path: &str) -> TestRequestBuilder {
        self.request(Method::Get, path)
    }

    pub fn post(&self, path: &str) -> TestRequestBuilder {
        self.request(Method::Post, path)
    }

    pub fn put(&self, path: &str) -> TestRequestBuilder {
        self.request(Method::Put, path)
    }

    pub fn patch(&self, path: &str) -> TestRequestBuilder {
        self.request(Method::Patch, path)
    }

    pub fn delete(&self, path: &str) -> TestRequestBuilder {
        self.request(Method::Delete, path)
    }

    pub fn request(&self, method: Method, path: &str) -> TestRequestBuilder {
        TestRequestBuilder {
            address: self.address,
            method,
            path: path.to_owned(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
}

pub struct TestRequestBuilder {
    address: SocketAddr,
    method: Method,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl TestRequestBuilder {
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn text(self, body: impl Into<String>) -> Self {
        self.body(body.into().into_bytes())
    }

    pub fn json<T>(mut self, value: &T) -> Self
    where
        T: ToJson,
    {
        self.body = zhs_json::to_string(value).into_bytes();
        self.header("content-type", "application/json")
    }

    pub fn send(self) -> TestResponse {
        let mut request = format!(
            "{} {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n",
            self.method, self.path
        );

        for (name, value) in &self.headers {
            request.push_str(name);
            request.push_str(": ");
            request.push_str(value);
            request.push_str("\r\n");
        }

        if !self.body.is_empty() {
            request.push_str(&format!("content-length: {}\r\n", self.body.len()));
        }

        request.push_str("\r\n");

        let mut stream =
            TcpStream::connect(self.address).expect("failed to connect to test server");
        stream
            .write_all(request.as_bytes())
            .expect("failed to write request headers");
        stream
            .write_all(&self.body)
            .expect("failed to write request body");
        stream.flush().expect("failed to flush request");

        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .expect("failed to read response");

        TestResponse::parse(response)
    }
}

#[derive(Debug, Clone)]
pub struct TestResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl TestResponse {
    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json<T>(&self) -> Result<T, zhs_json::Error>
    where
        T: FromJson,
    {
        zhs_json::from_str(&self.text())
    }

    pub fn assert_status(&self, status: u16) -> &Self {
        assert_eq!(self.status, status, "body: {}", self.text());
        self
    }

    fn parse(response: Vec<u8>) -> Self {
        let header_end = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|position| position + 4)
            .expect("response has no header terminator");

        let head = std::str::from_utf8(&response[..header_end]).expect("headers are not UTF-8");
        let mut lines = head.split("\r\n");
        let status_line = lines.next().unwrap_or_default();
        let status = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse::<u16>().ok())
            .expect("invalid status line");

        let mut headers = HashMap::new();
        for line in lines {
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
            }
        }

        Self {
            status,
            headers,
            body: response[header_end..].to_vec(),
        }
    }
}
