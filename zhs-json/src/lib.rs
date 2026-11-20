mod error;
mod parser;
mod serialize;
mod traits;
mod value;

pub use error::{Error, JsonDecodeError, JsonError, JsonErrorKind, Result};
pub use traits::{FromJson, ToJson};
pub use value::{Iter, IterMut, Number, Object, Value};

pub const MAX_DEPTH: usize = parser::MAX_DEPTH;

pub fn parse(input: &str) -> Result<Value> {
    parser::parse(input).map_err(Error::Json)
}

pub fn parse_bytes(input: &[u8]) -> Result<Value> {
    match std::str::from_utf8(input) {
        Ok(text) => parse(text),
        Err(error) => Err(Error::Json(JsonError::from_utf8(error, input))),
    }
}

pub fn to_value<T: ToJson + ?Sized>(value: &T) -> Value {
    value.to_json()
}

pub fn to_string<T: ToJson + ?Sized>(value: &T) -> String {
    serialize::to_string(&value.to_json())
}

pub fn to_string_pretty<T: ToJson + ?Sized>(value: &T) -> String {
    serialize::to_string_pretty(&value.to_json())
}

/// Like [`to_string`], but returns [`JsonErrorKind::RecursionLimitExceeded`]
/// for values nested deeper than [`MAX_DEPTH`] instead of falling back to
/// the `null` literal.
pub fn try_to_string<T: ToJson + ?Sized>(value: &T) -> Result<String> {
    serialize::try_to_string(&value.to_json()).map_err(Error::Json)
}

/// Strict variant of [`to_string_pretty`].
pub fn try_to_string_pretty<T: ToJson + ?Sized>(value: &T) -> Result<String> {
    serialize::try_to_string_pretty(&value.to_json()).map_err(Error::Json)
}

pub fn from_value<T: FromJson>(value: &Value) -> Result<T> {
    T::from_json(value).map_err(Error::JsonDecode)
}

pub fn from_str<T: FromJson>(input: &str) -> Result<T> {
    let value = parse(input)?;
    from_value(&value)
}

pub fn from_slice<T: FromJson>(input: &[u8]) -> Result<T> {
    let value = parse_bytes(input)?;
    from_value(&value)
}
