use crate::DataError;
use zhs_json::{FromJson, Object, Value};

pub trait Entity: Sized + 'static {
    const TABLE: &'static str;

    const COLUMNS: &'static [&'static str];

    const PRIMARY_KEY: Option<&'static str> = None;

    fn from_row(row: &Object) -> Result<Self, DataError>;

    fn values(&self) -> Vec<(&'static str, Value)>;
}

pub trait FromValue: Sized {
    fn from_value(value: &Value) -> Result<Self, DataError>;
}

impl<T> FromValue for T
where
    T: FromJson,
{
    fn from_value(value: &Value) -> Result<Self, DataError> {
        T::from_json(value).map_err(|error| DataError::decode(error.to_string()))
    }
}

pub trait RowExt {
    fn get_field<T: FromValue>(&self, name: &str) -> Result<T, DataError>;
}

impl RowExt for Object {
    fn get_field<T: FromValue>(&self, name: &str) -> Result<T, DataError> {
        let decoded = match self.get(name) {
            Some(value) => T::from_value(value),
            None => T::from_value(&Value::Null),
        };
        decoded.map_err(|error| DataError::decode(format!("column `{name}`: {error}")))
    }
}

pub fn decode_rows<M: Entity>(rows: &[Object]) -> Result<Vec<M>, DataError> {
    rows.iter().map(M::from_row).collect()
}
