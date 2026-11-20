use std::sync::Arc;
use zhs_entity::pool::{Connector, Pool, PoolOptions};

use zhs_json::{Object as ZObject, Value as ZValue};
mod bson;
#[allow(dead_code)]
mod connection;
#[allow(dead_code)]
mod find;
mod scram;
mod wire;
use connection::Connection;
use find::Find;
use zhs_json::{Object, Value};

pub use connection::ConnectOptions;

pub struct MongoDatabase {
    pool: Arc<Pool<MongoConnector>>,
    database: String,
}

struct MongoConnector {
    options: ConnectOptions,
}

impl Connector for MongoConnector {
    type Connection = Connection;

    fn connect(&self) -> Result<Self::Connection, zhs_entity::DataError> {
        Connection::connect(&self.options)
    }

    fn probe(&self, connection: &mut Self::Connection) -> Result<(), zhs_entity::DataError> {
        connection.ping()
    }
}

impl MongoDatabase {
    pub fn connect(options: &ConnectOptions) -> Result<Self, Error> {
        let pool = Pool::new(
            MongoConnector {
                options: options.clone(),
            },
            PoolOptions::new(8),
        )
        .map_err(pool_error)?;

        Ok(Self {
            pool: Arc::new(pool),
            database: options.database.clone(),
        })
    }

    pub fn collection(&self, name: &str) -> Collection {
        Collection {
            pool: Arc::clone(&self.pool),
            database: self.database.clone(),
            name: name.to_owned(),
        }
    }

    pub fn ping(&self) -> Result<(), Error> {
        let mut connection = self.pool.get().map_err(pool_error)?;

        connection
            .ping()
            .map_err(|error| Error::Database(error.to_string()))
    }

    pub fn database_name(&self) -> &str {
        &self.database
    }
}

#[derive(Clone)]
pub struct Collection {
    pool: Arc<Pool<MongoConnector>>,
    database: String,
    name: String,
}

impl Collection {
    pub fn find(&self, filter: Filter) -> FindBuilder {
        FindBuilder {
            collection: self.clone(),
            filter: filter.object,
            sort: None,
            projection: None,
            limit: None,
            skip: None,
        }
    }

    pub fn insert_one(&self, document: Object) -> Result<(), Error> {
        let mut connection = self.pool.get().map_err(pool_error)?;
        let document = to_zhs_object(&document);

        connection
            .insert_one(&self.name, &document)
            .map_err(|error| Error::Database(error.to_string()))
    }

    pub fn update_many(&self, filter: Filter, update: Update) -> Result<u64, Error> {
        let mut updates = Object::new();
        updates.insert("q", Value::Object(filter.object));
        updates.insert("u", Value::Object(update.object));
        updates.insert("multi", Value::Bool(true));
        updates.insert("upsert", Value::Bool(false));

        let mut command = Object::new();
        command.insert("update", Value::from(self.name.as_str()));
        command.insert("updates", Value::Array(vec![Value::Object(updates)]));
        command.insert("ordered", Value::Bool(false));

        let reply = self.command(command)?;
        Ok(reply.get("nModified").and_then(Value::as_u64).unwrap_or(0))
    }

    pub fn delete_many(&self, filter: Filter) -> Result<u64, Error> {
        let mut deletion = Object::new();
        deletion.insert("q", Value::Object(filter.object));
        deletion.insert("limit", Value::from(0_i64));

        let mut command = Object::new();
        command.insert("delete", Value::from(self.name.as_str()));
        command.insert("deletes", Value::Array(vec![Value::Object(deletion)]));
        command.insert("ordered", Value::Bool(false));

        let reply = self.command(command)?;
        Ok(reply.get("n").and_then(Value::as_u64).unwrap_or(0))
    }

    pub fn count(&self, filter: Filter) -> Result<u64, Error> {
        let mut command = Object::new();
        command.insert("count", Value::from(self.name.as_str()));
        command.insert("query", Value::Object(filter.object));

        let reply = self.command(command)?;
        Ok(reply.get("n").and_then(Value::as_u64).unwrap_or(0))
    }

    pub fn command(&self, command: Object) -> Result<Object, Error> {
        let mut connection = self.pool.get().map_err(pool_error)?;
        let command = to_zhs_object(&command);

        let reply = connection
            .run_command(&self.database, &command)
            .map_err(|error| Error::Database(error.to_string()))?;

        Ok(from_zhs_object(&reply))
    }
}

pub struct FindBuilder {
    collection: Collection,
    filter: Object,
    sort: Option<Object>,
    projection: Option<Object>,
    limit: Option<u64>,
    skip: Option<u64>,
}

impl FindBuilder {
    pub fn sort(mut self, sort: Object) -> Self {
        self.sort = Some(sort);
        self
    }

    pub fn projection(mut self, projection: Object) -> Self {
        self.projection = Some(projection);
        self
    }

    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn skip(mut self, skip: u64) -> Self {
        self.skip = Some(skip);
        self
    }

    pub fn to_list(self) -> Result<Vec<Object>, Error> {
        let find = Find {
            filter: to_zhs_object(&self.filter),
            sort: self.sort.as_ref().map(to_zhs_object),
            projection: self.projection.as_ref().map(to_zhs_object),
            limit: self.limit,
            skip: self.skip,
        };

        let mut connection = self.collection.pool.get().map_err(pool_error)?;

        let rows = connection
            .find(&self.collection.name, &find)
            .map_err(|error| Error::Database(error.to_string()))?;

        Ok(rows.iter().map(from_zhs_object).collect())
    }

    pub fn first(self) -> Result<Option<Object>, Error> {
        Ok(self.limit(1).to_list()?.into_iter().next())
    }
}

#[derive(Debug, Clone, Default)]
pub struct Filter {
    object: Object,
}

impl Filter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn raw(object: Object) -> Self {
        Self { object }
    }

    pub fn eq(mut self, field: impl Into<String>, value: impl Into<Value>) -> Self {
        self.object.insert(field, value.into());
        self
    }

    pub fn gt(self, field: impl Into<String>, value: impl Into<Value>) -> Self {
        self.operator(field, "$gt", value.into())
    }

    pub fn gte(self, field: impl Into<String>, value: impl Into<Value>) -> Self {
        self.operator(field, "$gte", value.into())
    }

    pub fn lt(self, field: impl Into<String>, value: impl Into<Value>) -> Self {
        self.operator(field, "$lt", value.into())
    }

    pub fn lte(self, field: impl Into<String>, value: impl Into<Value>) -> Self {
        self.operator(field, "$lte", value.into())
    }

    pub fn in_<I, T>(mut self, field: impl Into<String>, values: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Value>,
    {
        let mut operator = Object::new();
        operator.insert(
            "$in",
            Value::Array(values.into_iter().map(Into::into).collect()),
        );

        self.object.insert(field, Value::Object(operator));
        self
    }

    fn operator(mut self, field: impl Into<String>, name: &str, value: Value) -> Self {
        let mut operator = Object::new();
        operator.insert(name, value);
        self.object.insert(field, Value::Object(operator));
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct Update {
    object: Object,
}

impl Update {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(mut self, field: impl Into<String>, value: impl Into<Value>) -> Self {
        self.insert_operator("$set", field, value.into());
        self
    }

    pub fn inc(mut self, field: impl Into<String>, value: impl Into<Value>) -> Self {
        self.insert_operator("$inc", field, value.into());
        self
    }

    pub fn unset(mut self, field: impl Into<String>) -> Self {
        self.insert_operator("$unset", field, Value::from(1_i64));
        self
    }

    fn insert_operator(&mut self, operator: &str, field: impl Into<String>, value: Value) {
        let entry = self.object.get_mut(operator).and_then(Value::as_object_mut);

        let object = match entry {
            Some(object) => object,
            None => {
                self.object.insert(operator, Value::Object(Object::new()));
                self.object
                    .get_mut(operator)
                    .and_then(Value::as_object_mut)
                    .expect("operator object was just inserted")
            }
        };

        object.insert(field, value);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Database(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Database(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

fn pool_error(error: zhs_entity::DataError) -> Error {
    Error::Database(error.to_string())
}

fn to_zhs_object(object: &Object) -> ZObject {
    let mut result = ZObject::new();
    for (key, value) in object.iter() {
        result.insert(key, to_zhs_value(value));
    }
    result
}

fn to_zhs_value(value: &Value) -> ZValue {
    match value {
        Value::Null => ZValue::Null,
        Value::Bool(value) => ZValue::Bool(*value),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                ZValue::from(value)
            } else if let Some(value) = number.as_u64() {
                ZValue::from(value)
            } else {
                ZValue::from(number.as_f64())
            }
        }
        Value::String(value) => ZValue::from(value.as_str()),
        Value::Array(values) => ZValue::Array(values.iter().map(to_zhs_value).collect()),
        Value::Object(object) => ZValue::Object(to_zhs_object(object)),
    }
}

fn from_zhs_object(object: &ZObject) -> Object {
    let mut result = Object::new();
    for (key, value) in object.iter() {
        result.insert(key, from_zhs_value(value));
    }
    result
}

fn from_zhs_value(value: &ZValue) -> Value {
    match value {
        ZValue::Null => Value::Null,
        ZValue::Bool(value) => Value::Bool(*value),
        ZValue::Number(number) => {
            if let Some(value) = number.as_i64() {
                Value::from(value)
            } else if let Some(value) = number.as_u64() {
                Value::from(value)
            } else {
                Value::from(number.as_f64())
            }
        }
        ZValue::String(value) => Value::from(value.as_str()),
        ZValue::Array(values) => Value::Array(values.iter().map(from_zhs_value).collect()),
        ZValue::Object(object) => Value::Object(from_zhs_object(object)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_filter_documents() {
        let filter = Filter::new().gt("age", 18_i64);
        let age = filter.object.get("age").unwrap();

        assert_eq!(age["$gt"].as_i64(), Some(18));
    }

    #[test]
    fn builds_update_documents() {
        let update = Update::new().set("name", "Ada").inc("age", 1_i64);
        let set = update.object.get("$set").unwrap();

        assert_eq!(set["name"].as_str(), Some("Ada"));
    }
}
