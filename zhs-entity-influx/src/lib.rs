use zhs_entity::{Error as EntityError, Row, Value};
use zhs_http::Client;
use zhs_json::{Object, Value as JsonValue};

#[derive(Debug, Clone)]
pub struct InfluxOptions {
    pub url: String,
    pub token: String,
    pub org: String,
    pub bucket: String,
}

impl InfluxOptions {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            token: String::new(),
            org: String::new(),
            bucket: String::new(),
        }
    }

    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = token.into();
        self
    }

    pub fn org(mut self, org: impl Into<String>) -> Self {
        self.org = org.into();
        self
    }

    pub fn bucket(mut self, bucket: impl Into<String>) -> Self {
        self.bucket = bucket.into();
        self
    }
}

pub struct InfluxDatabase {
    client: Client,
    token: String,
    org: String,
    bucket: String,
}

impl InfluxDatabase {
    pub fn connect(options: InfluxOptions) -> Result<Self, Error> {
        let client = Client::builder()
            .base_url(options.url)
            .build()
            .map_err(|error| Error::Database(error.to_string()))?;

        Ok(Self {
            client,
            token: options.token,
            org: options.org,
            bucket: options.bucket,
        })
    }

    pub fn write(&self, points: &[Point]) -> Result<(), Error> {
        if points.is_empty() {
            return Ok(());
        }

        let body = points
            .iter()
            .map(Point::to_line_protocol)
            .collect::<Vec<_>>()
            .join("\n");

        self.client
            .post("/api/v2/write")
            .query("org", &self.org)
            .query("bucket", &self.bucket)
            .query("precision", "ns")
            .bearer_auth(&self.token)
            .header("content-type", "text/plain; charset=utf-8")
            .text(body)
            .send_blocking()
            .map_err(|error| Error::Database(error.to_string()))?
            .error_for_status()
            .map_err(|error| Error::Database(error.to_string()))?;

        Ok(())
    }

    pub fn query(&self, query: Query) -> Result<Vec<Row>, Error> {
        let body = query.to_json();

        let response = self
            .client
            .post("/api/v2/query")
            .query("org", &self.org)
            .bearer_auth(&self.token)
            .header("content-type", "application/json")
            .text(body)
            .send_blocking()
            .map_err(|error| Error::Database(error.to_string()))?
            .error_for_status()
            .map_err(|error| Error::Database(error.to_string()))?;

        let csv = response
            .text()
            .map_err(|error| Error::Database(error.to_string()))?;
        Ok(parse_flux_csv(csv))
    }

    pub fn query_raw(&self, query: Query) -> Result<String, Error> {
        let response = self
            .client
            .post("/api/v2/query")
            .query("org", &self.org)
            .bearer_auth(&self.token)
            .header("content-type", "application/json")
            .text(query.to_json())
            .send_blocking()
            .map_err(|error| Error::Database(error.to_string()))?
            .error_for_status()
            .map_err(|error| Error::Database(error.to_string()))?;

        response
            .text()
            .map(str::to_owned)
            .map_err(|error| Error::Database(error.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct Query {
    flux: String,
    params: Object,
}

impl Query {
    pub fn flux(flux: impl Into<String>) -> Self {
        Self {
            flux: flux.into(),
            params: Object::new(),
        }
    }

    pub fn param(mut self, name: impl Into<String>, value: impl Into<JsonValue>) -> Self {
        self.params.insert(name, value.into());
        self
    }

    pub fn param_string(self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.param(name, JsonValue::from(value.into()))
    }

    fn to_json(&self) -> String {
        let mut root = Object::new();
        root.insert("query", JsonValue::from(self.flux.as_str()));
        root.insert("type", JsonValue::from("flux"));
        root.insert("params", JsonValue::Object(self.params.clone()));
        zhs_json::to_string(&JsonValue::Object(root))
    }
}

#[derive(Debug, Clone)]
pub struct Point {
    measurement: String,
    tags: Vec<(String, String)>,
    fields: Vec<(String, Value)>,
    timestamp: Option<i64>,
}

impl Point {
    pub fn measurement(measurement: impl Into<String>) -> Self {
        Self {
            measurement: measurement.into(),
            tags: Vec::new(),
            fields: Vec::new(),
            timestamp: None,
        }
    }

    pub fn tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.push((key.into(), value.into()));
        self
    }

    pub fn field(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.fields.push((key.into(), value.into()));
        self
    }

    pub fn timestamp(mut self, nanoseconds: i64) -> Self {
        self.timestamp = Some(nanoseconds);
        self
    }

    pub fn to_line_protocol(&self) -> String {
        let mut output = escape_measurement(&self.measurement);

        for (key, value) in &self.tags {
            output.push(',');
            output.push_str(&escape_key(key));
            output.push('=');
            output.push_str(&escape_tag(value));
        }

        output.push(' ');

        let fields = self
            .fields
            .iter()
            .filter(|(_, value)| !matches!(value, Value::Null))
            .map(|(key, value)| format!("{}={}", escape_key(key), field_value(value)))
            .collect::<Vec<_>>()
            .join(",");

        output.push_str(&fields);

        if let Some(timestamp) = self.timestamp {
            output.push(' ');
            output.push_str(&timestamp.to_string());
        }

        output
    }
}

fn field_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => format!("{value}i"),
        Value::UInt(value) => format!("{value}i"),
        Value::Float(value) => value.to_string(),
        Value::Text(value) => format!(
            "\"{}\"",
            value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
        ),
        Value::Bytes(_) => "\"\"".to_owned(),
    }
}

fn escape_measurement(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(',', "\\,")
        .replace(' ', "\\ ")
        .replace('=', "\\=")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn escape_key(value: &str) -> String {
    escape_measurement(value)
}

fn escape_tag(value: &str) -> String {
    escape_measurement(value)
}

fn parse_flux_csv(csv: &str) -> Vec<Row> {
    let mut lines = csv.lines().filter(|line| !line.starts_with('#'));
    let Some(header) = lines.next() else {
        return Vec::new();
    };

    let columns = split_csv(header);
    let mut rows = Vec::new();

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }

        let values = split_csv(line);
        if values.len() != columns.len() {
            continue;
        }

        let row_values = values
            .iter()
            .map(|value| infer_value(value))
            .collect::<Vec<_>>();
        rows.push(Row::new(columns.clone(), row_values));
    }

    rows
}

fn split_csv(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut index = 0;

    while index < line.len() {
        let Some(character) = line[index..].chars().next() else {
            break;
        };
        index += character.len_utf8();

        match character {
            '"' if quoted && line[index..].starts_with('"') => {
                current.push('"');
                index += 1;
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                values.push(std::mem::take(&mut current));
            }
            character => current.push(character),
        }
    }

    values.push(current);
    values
}

fn infer_value(value: &str) -> Value {
    if value.is_empty() {
        return Value::Null;
    }
    if value.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if value.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(value) = value.parse::<i64>() {
        return Value::Int(value);
    }
    if let Ok(value) = value.parse::<f64>() {
        return Value::Float(value);
    }

    Value::Text(value.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Database(String),
    Entity(EntityError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Database(message) => formatter.write_str(message),
            Error::Entity(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Entity(error) => Some(error),
            Error::Database(_) => None,
        }
    }
}

impl From<EntityError> for Error {
    fn from(error: EntityError) -> Self {
        Error::Entity(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_line_protocol() {
        let point = Point::measurement("cpu")
            .tag("host", "a b")
            .field("usage", 0.5)
            .timestamp(1_000);

        assert_eq!(point.to_line_protocol(), "cpu,host=a\\ b usage=0.5 1000");
    }

    #[test]
    fn unsigned_and_null_fields_render_valid_line_protocol() {
        let point = Point::measurement("cpu")
            .field("cores", 8_u64)
            .field("ignored", Value::Null)
            .field("usage", 0.5);

        assert_eq!(point.to_line_protocol(), "cpu cores=8i,usage=0.5");
    }

    #[test]
    fn parses_flux_csv_rows() {
        let csv = ",result,table,_time,_value\n,,0,2026-11-19T00:00:00Z,42\n";
        let rows = parse_flux_csv(csv);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get::<i64>("_value").unwrap(), 42);
    }
}
