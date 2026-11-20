use std::fmt;
use std::str::FromStr;

use crate::internal::error::{Error, HttpError, Result};

pub(crate) const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

pub trait FromQuery: Sized {
    fn from_query(fields: &Fields) -> Result<Self>;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fields {
    entries: Vec<(String, String)>,
}

#[derive(Clone, Copy)]
enum Source {
    Query,
    Form,
}

impl Fields {
    pub fn parse_query(raw: &str) -> Result<Self> {
        Self::parse(raw, Source::Query)
    }

    pub fn parse_form(raw: &str) -> Result<Self> {
        Self::parse(raw, Source::Form)
    }

    fn parse(raw: &str, source: Source) -> Result<Self> {
        let mut entries = Vec::new();

        for pair in raw.split('&') {
            if pair.is_empty() {
                continue;
            }

            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            let name = decode_component(name).ok_or_else(|| source.error())?;
            let value = decode_component(value).ok_or_else(|| source.error())?;
            entries.push((name, value));
        }

        Ok(Self { entries })
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn get_all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.entries
            .iter()
            .filter(move |(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.iter().any(|(key, _)| key == name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    pub fn to_query_string(&self) -> String {
        let mut output = String::new();
        for (index, (name, value)) in self.entries.iter().enumerate() {
            if index > 0 {
                output.push('&');
            }
            encode_component_into(name, &mut output);
            output.push('=');
            encode_component_into(value, &mut output);
        }
        output
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn require(&self, name: &str) -> Result<&str> {
        self.get(name).ok_or_else(|| {
            Error::Http(HttpError::InvalidField {
                field: name.to_owned(),
                reason: "the field is missing".to_owned(),
            })
        })
    }

    pub fn value<T>(&self, name: &str) -> Result<Option<T>>
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        let Some(raw) = self.get(name) else {
            return Ok(None);
        };

        raw.parse::<T>().map(Some).map_err(|error| {
            Error::Http(HttpError::InvalidField {
                field: name.to_owned(),
                reason: format!("{raw:?} is not a valid {}: {error}", short_type_name::<T>()),
            })
        })
    }
}

impl Source {
    fn error(self) -> Error {
        let error = match self {
            Source::Query => HttpError::InvalidQuery,
            Source::Form => HttpError::InvalidForm,
        };

        Error::Http(error)
    }
}

pub(crate) fn encode_component_into(value: &str, output: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    for &byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(byte as char);
        } else {
            output.push('%');
            output.push(HEX[(byte >> 4) as usize] as char);
            output.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
}

fn decode_component(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let high = hex_value(*bytes.get(index + 1)?)?;
                let low = hex_value(*bytes.get(index + 2)?)?;
                output.push((high << 4) | low);
                index += 3;
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

    String::from_utf8(output).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn short_type_name<T>() -> &'static str {
    let name = std::any::type_name::<T>();
    name.rsplit("::").next().unwrap_or(name)
}

pub(crate) fn is_form_content_type(value: &str) -> bool {
    let mut parts = value.split(';');
    let media_type = parts.next().unwrap_or_default().trim();
    if !media_type.eq_ignore_ascii_case(FORM_CONTENT_TYPE) {
        return false;
    }

    for parameter in parts {
        let Some((name, value)) = parameter.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("charset") {
            let charset = value.trim().trim_matches('"');
            return charset.eq_ignore_ascii_case("utf-8");
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_pairs_on_ampersands_and_first_equals() {
        let fields = Fields::parse_query("a=1&b=two=3&&c&d=").expect("parse");

        assert_eq!(fields.get("a"), Some("1"));
        assert_eq!(fields.get("b"), Some("two=3"));
        assert_eq!(fields.get("c"), Some(""));
        assert_eq!(fields.get("d"), Some(""));
        assert_eq!(fields.len(), 4);
    }

    #[test]
    fn decodes_percent_escapes_and_plus() {
        let fields =
            Fields::parse_query("q=hello+world&x=%E4%B8%AD%E6%96%87&y=%2B").expect("parse");

        assert_eq!(fields.get("q"), Some("hello world"));
        assert_eq!(fields.get("x"), Some("中文"));
        assert_eq!(fields.get("y"), Some("+"));
    }

    #[test]
    fn rejects_invalid_escapes() {
        assert!(Fields::parse_query("q=%2").is_err());
        assert!(Fields::parse_query("q=%zz").is_err());
        assert!(Fields::parse_query("q=%E4%B8").is_err());
        assert!(Fields::parse_form("q=%ff").is_err());
    }

    #[test]
    fn encodes_fields_back_to_a_query_string() {
        let fields =
            Fields::parse_query("q=hello+world&x=%E4%B8%AD%E6%96%87&tag=a&tag=b").expect("parse");

        assert_eq!(
            fields.to_query_string(),
            "q=hello%20world&x=%E4%B8%AD%E6%96%87&tag=a&tag=b"
        );

        let round_trip = Fields::parse_query(&fields.to_query_string()).expect("re-parse");
        assert_eq!(round_trip.get("q"), Some("hello world"));
        assert_eq!(round_trip.get("x"), Some("中文"));
        assert_eq!(round_trip.get_all("tag").collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn keeps_repeated_keys_in_order() {
        let fields = Fields::parse_query("tag=a&tag=b&tag=c").expect("parse");

        assert_eq!(fields.get("tag"), Some("a"));
        assert_eq!(fields.get_all("tag").collect::<Vec<_>>(), ["a", "b", "c"]);
    }

    #[test]
    fn require_and_value_report_missing_or_invalid_fields() {
        let fields = Fields::parse_query("page=2&size=big").expect("parse");

        assert_eq!(fields.value::<u32>("page").expect("parse page"), Some(2));
        assert_eq!(
            fields.value::<u32>("missing").expect("missing is fine"),
            None
        );
        assert!(fields.require("missing").is_err());

        let error = fields.value::<u32>("size").expect_err("not a number");
        assert_eq!(
            error.status(),
            crate::internal::http::StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn accepts_only_utf8_urlencoded_content_type() {
        assert!(is_form_content_type("application/x-www-form-urlencoded"));
        assert!(is_form_content_type(
            "Application/X-WWW-Form-Urlencoded; charset=UTF-8"
        ));
        assert!(is_form_content_type(
            "application/x-www-form-urlencoded;charset=\"utf-8\""
        ));

        assert!(!is_form_content_type("application/json"));
        assert!(!is_form_content_type(""));
        assert!(!is_form_content_type(
            "application/x-www-form-urlencoded; charset=iso-8859-1"
        ));
        assert!(!is_form_content_type("multipart/form-data; boundary=x"));
    }
}
