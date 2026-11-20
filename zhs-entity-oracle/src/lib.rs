use zhs_entity::Backend as _;
use zhs_json::{Object as JsonObject, Value as JsonValue};
mod backend;
mod client;
mod connection;
mod ffi;
mod pool;
mod render;
use backend::Backend;

use zhs_entity::{DataError, Error, Params, Row, SqlDatabase, Value};

pub use backend::ConnectOptions;

pub struct OracleDatabase {
    backend: Backend,
}

impl OracleDatabase {
    pub fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let backend = Backend::connect(options, 8).map_err(|error| {
            Error::Database(DataError::protocol(format!(
                "failed to connect to Oracle: {error}"
            )))
        })?;

        Ok(Self { backend })
    }
}

impl SqlDatabase for OracleDatabase {
    fn execute(&self, sql: &str, params: &Params) -> Result<u64, Error> {
        let params = to_json_params(params)?;
        self.backend
            .execute_sql(sql, &params)
            .map_err(Error::Database)
    }

    fn query(&self, sql: &str, params: &Params) -> Result<Vec<Row>, Error> {
        let params = to_json_params(params)?;
        let rows = self
            .backend
            .fetch_sql(sql, &params)
            .map_err(Error::Database)?;

        Ok(rows.iter().map(from_json_row).collect())
    }
}

fn to_json_params(params: &Params) -> Result<Vec<JsonValue>, Error> {
    params
        .values()
        .iter()
        .map(|value| match value {
            Value::Null => Ok(JsonValue::Null),
            Value::Bool(value) => Ok(JsonValue::Bool(*value)),
            Value::Int(value) => Ok(JsonValue::from(*value)),
            Value::UInt(value) => Ok(JsonValue::from(*value)),
            Value::Float(value) => Ok(JsonValue::from(*value)),
            Value::Text(value) => Ok(JsonValue::from(value.as_str())),
            Value::Bytes(_) => Err(Error::Database(DataError::protocol(
                "Oracle parameter binding does not support Vec<u8> yet",
            ))),
        })
        .collect()
}

fn from_json_row(row: &JsonObject) -> Row {
    let mut columns = Vec::with_capacity(row.len());
    let mut values = Vec::with_capacity(row.len());

    for (name, value) in row.iter() {
        columns.push(name.to_owned());
        values.push(from_json_value(value));
    }

    Row::new(columns, values)
}

fn from_json_value(value: &JsonValue) -> Value {
    match value {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(value) => Value::Bool(*value),
        JsonValue::Number(number) => {
            if let Some(value) = number.as_i64() {
                Value::Int(value)
            } else if let Some(value) = number.as_u64() {
                Value::UInt(value)
            } else {
                Value::Float(number.as_f64())
            }
        }
        JsonValue::String(value) => Value::Text(value.clone()),
        other => Value::Text(other.to_string()),
    }
}
