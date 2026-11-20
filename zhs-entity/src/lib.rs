mod capabilities;
mod column;
mod db;
mod entity;
mod error;
mod expr;
pub mod pool;
mod query;
mod write;

use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, Default)]
pub struct Params {
    values: Vec<Value>,
}

impl Params {
    pub fn new(values: impl IntoIterator<Item = Value>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }

    pub fn values(&self) -> &[Value] {
        &self.values
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Row {
    columns: Vec<String>,
    values: Vec<Value>,
}

impl Row {
    pub fn new(columns: Vec<String>, values: Vec<Value>) -> Self {
        Self { columns, values }
    }

    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    pub fn values(&self) -> &[Value] {
        &self.values
    }

    pub fn get<T>(&self, column: &str) -> Result<T, Error>
    where
        T: FromValue,
    {
        let index = self
            .columns
            .iter()
            .position(|name| name.eq_ignore_ascii_case(column))
            .ok_or_else(|| Error::MissingColumn(column.to_owned()))?;

        let value = self
            .values
            .get(index)
            .ok_or_else(|| Error::MissingColumn(column.to_owned()))?;

        T::from_value(value)
    }
}

pub trait ToValue {
    fn to_value(&self) -> Value;
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::Int(value)
    }
}

impl From<u64> for Value {
    fn from(value: u64) -> Self {
        Value::UInt(value)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Value::Float(value)
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Bool(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::Text(value.to_owned())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Value::Text(value)
    }
}

pub trait FromValue: Sized {
    fn from_value(value: &Value) -> Result<Self, Error>;
}

pub trait FromRow: Sized {
    fn from_row(row: &Row) -> Result<Self, Error>;
}

pub trait SqlDatabase: Send + Sync {
    fn execute(&self, sql: &str, params: &Params) -> Result<u64, Error>;
    fn query(&self, sql: &str, params: &Params) -> Result<Vec<Row>, Error>;
}

#[derive(Clone)]
pub struct Database {
    inner: Arc<dyn SqlDatabase>,
}

impl Database {
    pub fn new(inner: impl SqlDatabase + 'static) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn execute(&self, sql: &str, params: &Params) -> Result<u64, Error> {
        self.inner.execute(sql, params)
    }

    pub fn query(&self, sql: &str, params: &Params) -> Result<Vec<Row>, Error> {
        self.inner.query(sql, params)
    }

    pub fn query_as<T>(&self, sql: &str, params: &Params) -> Result<Vec<T>, Error>
    where
        T: FromRow,
    {
        self.query(sql, params)?.iter().map(T::from_row).collect()
    }

    pub fn query_one<T>(&self, sql: &str, params: &Params) -> Result<Option<T>, Error>
    where
        T: FromRow,
    {
        self.query_as(sql, params).map(|mut rows| {
            if rows.is_empty() {
                None
            } else {
                Some(rows.remove(0))
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Database(DataError),
    MissingColumn(String),
    TypeMismatch {
        expected: &'static str,
        actual: &'static str,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Database(error) => error.fmt(formatter),
            Error::MissingColumn(column) => write!(formatter, "missing column `{column}`"),
            Error::TypeMismatch { expected, actual } => {
                write!(formatter, "expected {expected}, found {actual}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Database(error) => Some(error),
            Error::MissingColumn(_) | Error::TypeMismatch { .. } => None,
        }
    }
}

impl From<DataError> for Error {
    fn from(error: DataError) -> Self {
        Error::Database(error)
    }
}

impl Error {
    pub fn is_conflict(&self) -> bool {
        matches!(self, Error::Database(error) if error.is_conflict())
    }
}

impl ToValue for Value {
    fn to_value(&self) -> Value {
        self.clone()
    }
}

impl<T> ToValue for &T
where
    T: ToValue + ?Sized,
{
    fn to_value(&self) -> Value {
        (*self).to_value()
    }
}

impl FromValue for Value {
    fn from_value(value: &Value) -> Result<Self, Error> {
        Ok(value.clone())
    }
}

macro_rules! impl_integer_value {
    ($($type:ty),* $(,)?) => {
        $(
            impl ToValue for $type {
                fn to_value(&self) -> Value {
                    Value::Int(*self as i64)
                }
            }

            impl FromValue for $type {
                fn from_value(value: &Value) -> Result<Self, Error> {
                    match value {
                        Value::Int(value) => <$type>::try_from(*value).map_err(|_| {
                            Error::TypeMismatch {
                                expected: stringify!($type),
                                actual: "integer out of range",
                            }
                        }),
                        Value::UInt(value) => <$type>::try_from(*value).map_err(|_| {
                            Error::TypeMismatch {
                                expected: stringify!($type),
                                actual: "integer out of range",
                            }
                        }),
                        other => Err(Error::TypeMismatch {
                            expected: stringify!($type),
                            actual: other.type_name(),
                        }),
                    }
                }
            }
        )*
    };
}

impl_integer_value!(i8, i16, i32, i64, u8, u16, u32);

impl ToValue for u64 {
    fn to_value(&self) -> Value {
        Value::UInt(*self)
    }
}

impl FromValue for u64 {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Int(value) => u64::try_from(*value).map_err(|_| Error::TypeMismatch {
                expected: "u64",
                actual: "integer out of range",
            }),
            Value::UInt(value) => Ok(*value),
            other => Err(Error::TypeMismatch {
                expected: "u64",
                actual: other.type_name(),
            }),
        }
    }
}

impl ToValue for f64 {
    fn to_value(&self) -> Value {
        Value::Float(*self)
    }
}

impl FromValue for f64 {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Float(value) => Ok(*value),
            Value::Int(value) => Ok(*value as f64),
            other => Err(Error::TypeMismatch {
                expected: "f64",
                actual: other.type_name(),
            }),
        }
    }
}

impl ToValue for bool {
    fn to_value(&self) -> Value {
        Value::Bool(*self)
    }
}

impl FromValue for bool {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Bool(value) => Ok(*value),
            Value::Int(value) => Ok(*value != 0),
            other => Err(Error::TypeMismatch {
                expected: "bool",
                actual: other.type_name(),
            }),
        }
    }
}

impl ToValue for str {
    fn to_value(&self) -> Value {
        Value::Text(self.to_owned())
    }
}

impl ToValue for String {
    fn to_value(&self) -> Value {
        Value::Text(self.clone())
    }
}

impl FromValue for String {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Text(value) => Ok(value.clone()),
            other => Err(Error::TypeMismatch {
                expected: "String",
                actual: other.type_name(),
            }),
        }
    }
}

impl ToValue for Vec<u8> {
    fn to_value(&self) -> Value {
        Value::Bytes(self.clone())
    }
}

impl FromValue for Vec<u8> {
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Bytes(value) => Ok(value.clone()),
            other => Err(Error::TypeMismatch {
                expected: "Vec<u8>",
                actual: other.type_name(),
            }),
        }
    }
}

impl<T> ToValue for Option<T>
where
    T: ToValue,
{
    fn to_value(&self) -> Value {
        match self {
            Some(value) => value.to_value(),
            None => Value::Null,
        }
    }
}

impl<T> FromValue for Option<T>
where
    T: FromValue,
{
    fn from_value(value: &Value) -> Result<Self, Error> {
        match value {
            Value::Null => Ok(None),
            value => T::from_value(value).map(Some),
        }
    }
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::UInt(_) => "uint",
            Value::Float(_) => "float",
            Value::Text(_) => "text",
            Value::Bytes(_) => "bytes",
        }
    }
}

#[macro_export]
macro_rules! params {
    ($($value:expr),* $(,)?) => {
        $crate::Params::new([$($crate::ToValue::to_value(&$value)),*])
    };
}

pub use capabilities::Capabilities;
pub use column::{Column, Direction, Order};
pub use db::{Backend, Db, Transaction};
pub use entity::{Entity, FromValue as JsonFromValue, RowExt, decode_rows};
pub use error::DataError;
pub use expr::{Expr, Node, Op};
pub use pool::{Connector, Pool, PoolOptions, PoolStats, PooledConnection};
pub use query::{CompiledQuery, Query};
pub use write::{
    CompiledDelete, CompiledInsert, CompiledUpdate, Delete, Insert, Update, WriteOutcome,
};
pub use zhs_json::{Object as JsonObject, Value as JsonValue};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_errors_keep_their_kind() {
        let error = Error::from(DataError::Server {
            code: 1062,
            state: "23000".to_owned(),
            message: "duplicate".to_owned(),
        });

        assert!(error.is_conflict());
        assert!(matches!(
            error,
            Error::Database(DataError::Server { code: 1062, .. })
        ));
    }

    #[test]
    fn u64_values_keep_their_range() {
        let value = u64::MAX.to_value();
        assert_eq!(value, Value::UInt(u64::MAX));
        assert_eq!(<u64 as FromValue>::from_value(&value).unwrap(), u64::MAX);
    }
}
