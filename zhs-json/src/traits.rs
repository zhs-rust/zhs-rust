use std::collections::HashMap;

use crate::error::JsonDecodeError;

use super::value::{Number, Object, Value};

pub trait ToJson {
    fn to_json(&self) -> Value;
}

pub trait FromJson: Sized {
    fn from_json(value: &Value) -> Result<Self, JsonDecodeError>;
}

impl ToJson for Value {
    fn to_json(&self) -> Value {
        self.clone()
    }
}

impl FromJson for Value {
    fn from_json(value: &Value) -> Result<Self, JsonDecodeError> {
        Ok(value.clone())
    }
}

impl ToJson for Number {
    fn to_json(&self) -> Value {
        Value::Number(*self)
    }
}

impl FromJson for Number {
    fn from_json(value: &Value) -> Result<Self, JsonDecodeError> {
        value
            .as_number()
            .copied()
            .ok_or_else(|| JsonDecodeError::expected("number", value))
    }
}

impl ToJson for Object {
    fn to_json(&self) -> Value {
        Value::Object(self.clone())
    }
}

impl FromJson for Object {
    fn from_json(value: &Value) -> Result<Self, JsonDecodeError> {
        value
            .as_object()
            .cloned()
            .ok_or_else(|| JsonDecodeError::expected("object", value))
    }
}

impl ToJson for bool {
    fn to_json(&self) -> Value {
        Value::Bool(*self)
    }
}

impl FromJson for bool {
    fn from_json(value: &Value) -> Result<Self, JsonDecodeError> {
        value
            .as_bool()
            .ok_or_else(|| JsonDecodeError::expected("boolean", value))
    }
}

impl ToJson for str {
    fn to_json(&self) -> Value {
        Value::String(self.to_owned())
    }
}

impl ToJson for String {
    fn to_json(&self) -> Value {
        Value::String(self.clone())
    }
}

impl FromJson for String {
    fn from_json(value: &Value) -> Result<Self, JsonDecodeError> {
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| JsonDecodeError::expected("string", value))
    }
}

macro_rules! impl_to_json_for_integer {
    ($($ty:ty => $method:ident as $target:ty),* $(,)?) => {
        $(
            impl ToJson for $ty {
                fn to_json(&self) -> Value {
                    Value::Number(Number::$method(*self as $target))
                }
            }
        )*
    };
}

impl_to_json_for_integer! {
    i8 => from_i64 as i64,
    i16 => from_i64 as i64,
    i32 => from_i64 as i64,
    i64 => from_i64 as i64,
    isize => from_i64 as i64,
    u8 => from_u64 as u64,
    u16 => from_u64 as u64,
    u32 => from_u64 as u64,
    u64 => from_u64 as u64,
    usize => from_u64 as u64,
}

macro_rules! impl_from_json_for_signed {
    ($($ty:ty),* $(,)?) => {
        $(
            impl FromJson for $ty {
                fn from_json(value: &Value) -> Result<Self, JsonDecodeError> {
                    value
                        .as_i64()
                        .and_then(|number| <$ty>::try_from(number).ok())
                        .ok_or_else(|| JsonDecodeError::expected(stringify!($ty), value))
                }
            }
        )*
    };
}

macro_rules! impl_from_json_for_unsigned {
    ($($ty:ty),* $(,)?) => {
        $(
            impl FromJson for $ty {
                fn from_json(value: &Value) -> Result<Self, JsonDecodeError> {
                    value
                        .as_u64()
                        .and_then(|number| <$ty>::try_from(number).ok())
                        .ok_or_else(|| JsonDecodeError::expected(stringify!($ty), value))
                }
            }
        )*
    };
}

impl_from_json_for_signed!(i8, i16, i32, i64, isize);
impl_from_json_for_unsigned!(u8, u16, u32, u64, usize);

impl ToJson for f64 {
    fn to_json(&self) -> Value {
        Value::from(*self)
    }
}

impl FromJson for f64 {
    fn from_json(value: &Value) -> Result<Self, JsonDecodeError> {
        value
            .as_f64()
            .ok_or_else(|| JsonDecodeError::expected("number", value))
    }
}

impl ToJson for f32 {
    fn to_json(&self) -> Value {
        Value::from(*self)
    }
}

impl FromJson for f32 {
    fn from_json(value: &Value) -> Result<Self, JsonDecodeError> {
        value
            .as_f64()
            .map(|number| number as f32)
            .filter(|number| number.is_finite())
            .ok_or_else(|| JsonDecodeError::expected("32-bit float", value))
    }
}

impl<T: ToJson> ToJson for Option<T> {
    fn to_json(&self) -> Value {
        match self {
            Some(value) => value.to_json(),
            None => Value::Null,
        }
    }
}

impl<T: FromJson> FromJson for Option<T> {
    fn from_json(value: &Value) -> Result<Self, JsonDecodeError> {
        if value.is_null() {
            Ok(None)
        } else {
            T::from_json(value).map(Some)
        }
    }
}

impl<T: ToJson> ToJson for [T] {
    fn to_json(&self) -> Value {
        Value::Array(self.iter().map(ToJson::to_json).collect())
    }
}

impl<T: ToJson> ToJson for Vec<T> {
    fn to_json(&self) -> Value {
        self.as_slice().to_json()
    }
}

impl<T: FromJson> FromJson for Vec<T> {
    fn from_json(value: &Value) -> Result<Self, JsonDecodeError> {
        let array = value
            .as_array()
            .ok_or_else(|| JsonDecodeError::expected("array", value))?;
        array
            .iter()
            .enumerate()
            .map(|(index, item)| T::from_json(item).map_err(|error| error.prepend_index(index)))
            .collect()
    }
}

impl<T: ToJson, const N: usize> ToJson for [T; N] {
    fn to_json(&self) -> Value {
        Value::Array(self.iter().map(ToJson::to_json).collect())
    }
}

impl<T: FromJson, const N: usize> FromJson for [T; N] {
    fn from_json(value: &Value) -> Result<Self, JsonDecodeError> {
        let array = value
            .as_array()
            .ok_or_else(|| JsonDecodeError::expected("array", value))?;
        if array.len() != N {
            return Err(JsonDecodeError::new(
                "$",
                format!("expected an array of length {N}, found {}", array.len()),
            ));
        }

        let items = array
            .iter()
            .enumerate()
            .map(|(index, item)| T::from_json(item).map_err(|error| error.prepend_index(index)))
            .collect::<Result<Vec<T>, JsonDecodeError>>()?;

        items
            .try_into()
            .map_err(|_| JsonDecodeError::new("$", format!("expected an array of length {N}")))
    }
}

impl<T: ToJson> ToJson for HashMap<String, T> {
    fn to_json(&self) -> Value {
        let mut object = Object::new();
        for (key, value) in self {
            object.insert(key.as_str(), value.to_json());
        }
        Value::Object(object)
    }
}

impl<T: FromJson> FromJson for HashMap<String, T> {
    fn from_json(value: &Value) -> Result<Self, JsonDecodeError> {
        let object = value
            .as_object()
            .ok_or_else(|| JsonDecodeError::expected("object", value))?;
        let mut map = HashMap::with_capacity(object.len());
        for (key, value) in object {
            let decoded = T::from_json(value).map_err(|error| error.prepend_field(key))?;
            map.insert(key.to_owned(), decoded);
        }
        Ok(map)
    }
}

impl ToJson for () {
    fn to_json(&self) -> Value {
        Value::Null
    }
}

impl FromJson for () {
    fn from_json(value: &Value) -> Result<Self, JsonDecodeError> {
        if value.is_null() {
            Ok(())
        } else {
            Err(JsonDecodeError::expected("null", value))
        }
    }
}
