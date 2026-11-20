use zhs_http::{Client, ClientRequest};
use zhs_json::Value;

#[derive(Debug, Clone)]
pub enum PiAuth {
    None,
    Basic { username: String, password: String },
    Bearer { token: String },
    Header { name: String, value: String },
}

impl PiAuth {
    pub fn basic(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self::Basic {
            username: username.into(),
            password: password.into(),
        }
    }

    pub fn bearer(token: impl Into<String>) -> Self {
        Self::Bearer {
            token: token.into(),
        }
    }

    fn apply(&self, request: ClientRequest) -> ClientRequest {
        match self {
            PiAuth::None => request,
            PiAuth::Basic { username, password } => {
                request.basic_auth(username, Some(password.as_str()))
            }
            PiAuth::Bearer { token } => request.bearer_auth(token),
            PiAuth::Header { name, value } => request.header(name.clone(), value.clone()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PiOptions {
    pub url: String,
    pub auth: PiAuth,
}

impl PiOptions {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            auth: PiAuth::None,
        }
    }

    pub fn auth(mut self, auth: PiAuth) -> Self {
        self.auth = auth;
        self
    }
}

#[derive(Debug, Clone)]
pub struct TimeRange {
    pub start: String,
    pub end: String,
}

impl TimeRange {
    pub fn new(start: impl Into<String>, end: impl Into<String>) -> Self {
        Self {
            start: start.into(),
            end: end.into(),
        }
    }

    pub fn last_hour() -> Self {
        Self::new("-1h", "*")
    }

    pub fn last_minutes(minutes: u32) -> Self {
        Self::new(format!("-{minutes}m"), "*")
    }
}

pub struct PiDatabase {
    client: Client,
    auth: PiAuth,
}

impl PiDatabase {
    pub fn connect(options: PiOptions) -> Result<Self, Error> {
        let client = Client::builder()
            .base_url(options.url)
            .build()
            .map_err(|error| Error::Database(error.to_string()))?;

        Ok(Self {
            client,
            auth: options.auth,
        })
    }

    pub fn point_by_path(&self, path: &str) -> Result<PiStream<'_>, Error> {
        let response = self
            .auth
            .apply(self.client.get("/points").query("path", path));

        let value = self.send_json(response)?;
        let web_id = value
            .get("WebId")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("PI point response has no WebId".to_owned()))?;

        Ok(PiStream {
            database: self,
            web_id: web_id.to_owned(),
        })
    }

    pub fn stream_by_web_id(&self, web_id: impl Into<String>) -> PiStream<'_> {
        PiStream {
            database: self,
            web_id: web_id.into(),
        }
    }

    fn send_json(&self, request: ClientRequest) -> Result<Value, Error> {
        let response = request
            .send_blocking()
            .map_err(|error| Error::Database(error.to_string()))?
            .error_for_status()
            .map_err(|error| Error::Database(error.to_string()))?;

        let text = response
            .text()
            .map_err(|error| Error::Database(error.to_string()))?;

        zhs_json::parse(text).map_err(|error| Error::Protocol(error.to_string()))
    }
}

pub struct PiStream<'a> {
    database: &'a PiDatabase,
    web_id: String,
}

impl<'a> PiStream<'a> {
    pub fn recorded(&'a self, range: TimeRange) -> RecordedQuery<'a> {
        RecordedQuery {
            database: self.database,
            web_id: self.web_id.clone(),
            range,
            max_count: None,
        }
    }
}

pub struct RecordedQuery<'a> {
    database: &'a PiDatabase,
    web_id: String,
    range: TimeRange,
    max_count: Option<u64>,
}

impl<'a> RecordedQuery<'a> {
    pub fn max_count(mut self, max_count: u64) -> Self {
        self.max_count = Some(max_count);
        self
    }

    pub fn to_list(self) -> Result<Vec<PiValue>, Error> {
        let path = format!("/streams/{}/recorded", encode_segment(&self.web_id));
        let mut request = self
            .database
            .client
            .get(path)
            .query("startTime", &self.range.start)
            .query("endTime", &self.range.end);

        if let Some(max_count) = self.max_count {
            request = request.query("maxCount", max_count.to_string());
        }

        let value = self.database.send_json(self.database.auth.apply(request))?;
        let items = value
            .get("Items")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Protocol("PI recorded response has no Items".to_owned()))?;

        Ok(items.iter().filter_map(parse_value).collect())
    }
}

#[derive(Debug, Clone)]
pub struct PiValue {
    pub timestamp: String,
    pub value: Value,
    pub good: bool,
    pub questionable: bool,
    pub substituted: bool,
}

fn parse_value(item: &Value) -> Option<PiValue> {
    let object = item.as_object()?;

    Some(PiValue {
        timestamp: object
            .get("Timestamp")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        value: object.get("Value").cloned().unwrap_or(Value::Null),
        good: object.get("Good").and_then(Value::as_bool).unwrap_or(false),
        questionable: object
            .get("Questionable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        substituted: object
            .get("Substituted")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn encode_segment(value: &str) -> String {
    let mut output = String::new();

    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }

    output
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Database(String),
    Protocol(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Database(message) | Error::Protocol(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_web_id_segments() {
        assert_eq!(encode_segment("a/b+c="), "a%2Fb%2Bc%3D");
    }

    #[test]
    fn parses_recorded_value() {
        let value =
            zhs_json::parse(r#"{"Timestamp":"2026-11-19T00:00:00Z","Value":12.5,"Good":true}"#)
                .unwrap();
        let parsed = parse_value(&value).unwrap();

        assert_eq!(parsed.timestamp, "2026-11-19T00:00:00Z");
        assert!(parsed.good);
        assert_eq!(parsed.value.as_f64(), Some(12.5));
    }
}
