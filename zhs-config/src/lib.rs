use std::fmt;
use std::ops::Deref;
use std::path::Path;
pub mod server;

use std::sync::Arc;

use zhs_json::{FromJson, Object, Value};

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(zhs_json::Error),
    RootIsNotObject,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(error) => write!(formatter, "configuration I/O error: {error}"),
            Error::Json(error) => write!(formatter, "configuration JSON error: {error}"),
            Error::RootIsNotObject => formatter.write_str("configuration root must be an object"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error)
    }
}

impl From<zhs_json::Error> for Error {
    fn from(error: zhs_json::Error) -> Self {
        Error::Json(error)
    }
}

#[derive(Clone, Default)]
pub struct Configuration {
    root: Object,
}

// Configuration values routinely hold connection strings, API keys and other
// secrets, so `Debug` is hand-implemented to print only the top-level key names
// and never the values behind them.
impl fmt::Debug for Configuration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut keys = self.root.keys().collect::<Vec<_>>();
        keys.sort_unstable();

        formatter.write_str("Configuration {")?;

        for (index, key) in keys.iter().enumerate() {
            if index > 0 {
                formatter.write_str(",")?;
            }

            write!(formatter, " {key}: <redacted>")?;
        }

        if !keys.is_empty() {
            formatter.write_str(" ")?;
        }

        formatter.write_str("}")
    }
}

impl Configuration {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_json(value: Value) -> Result<Self, Error> {
        match value {
            Value::Object(root) => Ok(Self { root }),
            _ => Err(Error::RootIsNotObject),
        }
    }

    pub fn load_file(path: impl AsRef<Path>) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path)?;
        Self::from_json(zhs_json::parse(&text)?)
    }

    pub fn merge(&mut self, other: &Configuration) {
        merge_object(&mut self.root, &other.root);
    }

    pub fn get_value(&self, key: &str) -> Option<&Value> {
        let segments = key
            .split(':')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();

        if segments.is_empty() {
            return None;
        }

        let mut current = &self.root;

        for (index, segment) in segments.iter().enumerate() {
            let value = find_key(current, segment)?;

            if index + 1 == segments.len() {
                return Some(value);
            }

            current = value.as_object()?;
        }

        None
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get_value(key).and_then(Value::as_str)
    }

    pub fn get<T>(&self, key: &str) -> Option<T>
    where
        T: FromJson,
    {
        let value = self.get_value(key)?;
        if let Ok(decoded) = T::from_json(value) {
            return Some(decoded);
        }
        if let Value::String(text) = value {
            let parsed = zhs_json::parse(text).ok()?;
            return T::from_json(&parsed).ok();
        }
        None
    }

    pub fn section(&self, key: &str) -> Configuration {
        match self.get_value(key) {
            Some(Value::Object(object)) => Configuration {
                root: object.clone(),
            },
            _ => Configuration::new(),
        }
    }

    pub fn get_connection_string(&self, name: &str) -> Option<&str> {
        self.get_str(&format!("ConnectionStrings:{name}"))
    }

    pub fn set(&mut self, key: &str, value: Value) {
        let segments: Vec<&str> = key
            .split(':')
            .filter(|segment| !segment.is_empty())
            .collect();
        let Some((last, parents)) = segments.split_last() else {
            return;
        };

        let mut current = &mut self.root;

        for segment in parents {
            let key = current
                .keys()
                .find(|key| key.eq_ignore_ascii_case(segment))
                .map(str::to_owned)
                .unwrap_or_else(|| (*segment).to_owned());
            if !current.contains_key(&key) {
                current.insert(key.clone(), Value::Object(Object::new()));
            }

            let entry = current.get_mut(&key).expect("entry was just inserted");

            if !entry.is_object() {
                *entry = Value::Object(Object::new());
            }

            current = entry.as_object_mut().expect("object was just ensured");
        }

        let key = current
            .keys()
            .find(|key| key.eq_ignore_ascii_case(last))
            .map(str::to_owned)
            .unwrap_or_else(|| (*last).to_owned());
        current.insert(key, value);
    }

    pub fn add_environment(&mut self, prefix: &str) {
        let prefix = prefix.to_ascii_uppercase();

        for (key, value) in std::env::vars() {
            let Some(rest) = key.strip_prefix(&prefix) else {
                continue;
            };

            let path = rest
                .trim_start_matches('_')
                .split("__")
                .filter(|segment| !segment.is_empty())
                .map(|segment| segment.trim_start_matches('_').to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join(":");

            if !path.is_empty() {
                self.set(&path, Value::String(value));
            }
        }
    }

    pub fn add_command_line(&mut self, arguments: impl IntoIterator<Item = String>) {
        for argument in arguments {
            if let Some(rest) = argument.strip_prefix("--")
                && let Some((key, value)) = rest.split_once('=')
            {
                self.set(&key.replace('-', ":"), parse_scalar(value));
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options<T>(Arc<T>);

impl<T> Options<T> {
    pub fn new(value: T) -> Self {
        Self(Arc::new(value))
    }
}

impl<T> Deref for Options<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn find_key<'a>(object: &'a Object, key: &str) -> Option<&'a Value> {
    object.get(key).or_else(|| {
        object
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    })
}

fn merge_object(target: &mut Object, source: &Object) {
    for (key, value) in source.iter() {
        // Reuse the existing entry's key casing so case-insensitive lookups
        // (see `find_key` and `set`) see a single merged entry; otherwise the
        // override would be stored under a duplicate key and silently lost.
        let key = target
            .keys()
            .find(|candidate| candidate.eq_ignore_ascii_case(key))
            .map(str::to_owned)
            .unwrap_or_else(|| key.to_owned());

        match (target.get_mut(&key), value) {
            (Some(Value::Object(target_object)), Value::Object(source_object)) => {
                merge_object(target_object, source_object);
            }
            _ => {
                target.insert(key, value.clone());
            }
        }
    }
}

fn parse_scalar(value: &str) -> Value {
    match value {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        _ => zhs_json::parse(value).unwrap_or_else(|_| Value::String(value.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nested_sections() {
        let configuration = Configuration::from_json(
            zhs_json::parse(r#"{"App":{"Name":"zhs","Port":5000}}"#).unwrap(),
        )
        .unwrap();

        assert_eq!(configuration.get_str("App:Name"), Some("zhs"));
        assert_eq!(configuration.section("App").get_str("Name"), Some("zhs"));
    }

    #[test]
    fn merges_objects_deeply() {
        let mut base =
            Configuration::from_json(zhs_json::parse(r#"{"App":{"A":1,"B":2}}"#).unwrap()).unwrap();
        let override_config =
            Configuration::from_json(zhs_json::parse(r#"{"App":{"B":3}}"#).unwrap()).unwrap();

        base.merge(&override_config);
        assert_eq!(base.get::<i64>("App:A"), Some(1));
        assert_eq!(base.get::<i64>("App:B"), Some(3));
    }

    #[test]
    fn merges_case_insensitive_keys() {
        let mut base =
            Configuration::from_json(zhs_json::parse(r#"{"App":{"A":1,"B":2}}"#).unwrap()).unwrap();
        let override_config =
            Configuration::from_json(zhs_json::parse(r#"{"app":{"B":3}}"#).unwrap()).unwrap();

        base.merge(&override_config);
        assert_eq!(base.get::<i64>("App:A"), Some(1));
        assert_eq!(base.get::<i64>("App:B"), Some(3));
    }

    #[test]
    fn sets_dotted_keys() {
        let mut configuration = Configuration::new();
        configuration.set("ConnectionStrings:Default", Value::from("app.db"));

        assert_eq!(
            configuration.get_connection_string("Default"),
            Some("app.db")
        );
    }
}
