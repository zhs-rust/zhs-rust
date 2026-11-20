use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zhs_entity::DataError;
use zhs_entity::{
    CompiledDelete, CompiledInsert, CompiledQuery, CompiledUpdate, Transaction, WriteOutcome,
};
use zhs_json::{Object, Value};

use crate::connection::Connection;
use crate::ffi::{Environment, Oci};
use crate::pool::{OracleConnector, Pool, PoolStats, PooledConnection};

pub use zhs_entity::pool::PoolOptions;

use crate::render;

#[derive(Clone, PartialEq, Eq)]
pub struct ConnectOptions {
    pub host: String,

    pub port: u16,

    pub service_name: String,

    pub user: String,

    pub password: String,
}

impl ConnectOptions {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        service_name: impl Into<String>,
        user: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            service_name: service_name.into(),
            user: user.into(),
            password: password.into(),
        }
    }

    pub fn database_name(&self) -> String {
        format!("//{}:{}/{}", self.host, self.port, self.service_name)
    }
}

impl std::fmt::Debug for ConnectOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectOptions")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("service_name", &self.service_name)
            .field("user", &self.user)
            .field("password", &"***")
            .finish()
    }
}

pub struct Backend {
    pool: Arc<Pool>,
    identity: Arc<Mutex<HashMap<String, Option<String>>>>,
}

impl Backend {
    pub fn connect(options: ConnectOptions, capacity: usize) -> Result<Self, DataError> {
        Self::connect_with(options, PoolOptions::new(capacity))
    }

    pub fn connect_with(
        options: ConnectOptions,
        pool_options: PoolOptions,
    ) -> Result<Self, DataError> {
        let oci = Arc::new(Oci::load()?);
        let environment = Arc::new(Environment::create(oci)?);
        let pool = Arc::new(Pool::new(
            OracleConnector::new(environment, options),
            pool_options,
        )?);
        Ok(Self {
            pool,
            identity: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    #[allow(dead_code)]
    pub fn pool_stats(&self) -> PoolStats {
        self.pool.stats()
    }

    #[allow(dead_code)]
    pub fn reap_idle(&self) -> usize {
        self.pool.reap()
    }

    fn with_connection<T>(
        &self,
        job: impl FnOnce(&mut Connection) -> Result<T, DataError>,
    ) -> Result<T, DataError> {
        let mut connection = self.pool.get()?;
        job(&mut connection)
    }
}

impl zhs_entity::Backend for Backend {
    fn capabilities(&self) -> zhs_entity::Capabilities {
        zhs_entity::Capabilities::new()
            .with_transactions()
            .with_identity()
            .with_raw_sql()
            .with_pool()
    }

    fn fetch(&self, query: &CompiledQuery) -> Result<Vec<Object>, DataError> {
        let statement = render::render(query)?;
        self.with_connection(|connection| connection.fetch(&statement.sql, &statement.params, true))
    }

    fn count(&self, query: &CompiledQuery) -> Result<u64, DataError> {
        let statement = render::render_count(query)?;
        let rows = self.with_connection(|connection| {
            connection.fetch(&statement.sql, &statement.params, true)
        })?;
        count_from_rows(&rows)
    }

    fn insert(&self, insert: &CompiledInsert) -> Result<WriteOutcome, DataError> {
        let statement = render::render_insert(insert)?;
        self.with_connection(|connection| {
            let identity = lookup_identity(&self.identity, connection, insert.table);
            connection.insert(&statement.sql, &statement.params, identity.as_deref(), true)
        })
    }

    fn update(&self, update: &CompiledUpdate) -> Result<u64, DataError> {
        let statement = render::render_update(update)?;
        self.with_connection(|connection| {
            connection.execute(&statement.sql, &statement.params, true)
        })
    }

    fn delete(&self, delete: &CompiledDelete) -> Result<u64, DataError> {
        let statement = render::render_delete(delete)?;
        self.with_connection(|connection| {
            connection.execute(&statement.sql, &statement.params, true)
        })
    }

    fn fetch_sql(&self, sql: &str, params: &[crate::JsonValue]) -> Result<Vec<Object>, DataError> {
        self.with_connection(|connection| connection.fetch(sql, params, true))
    }

    fn execute_sql(&self, sql: &str, params: &[crate::JsonValue]) -> Result<u64, DataError> {
        self.with_connection(|connection| connection.execute(sql, params, true))
    }

    fn begin(&self) -> Result<Box<dyn Transaction>, DataError> {
        let connection = self.pool.get()?;
        Ok(Box::new(OracleTransaction {
            connection: Some(connection),
            identity: Arc::clone(&self.identity),
        }))
    }
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Backend").finish_non_exhaustive()
    }
}

struct OracleTransaction {
    connection: Option<PooledConnection>,
    identity: Arc<Mutex<HashMap<String, Option<String>>>>,
}

impl OracleTransaction {
    fn connection(&mut self) -> Result<&mut Connection, DataError> {
        self.connection.as_deref_mut().ok_or(DataError::Closed)
    }
}

impl Transaction for OracleTransaction {
    fn fetch(&mut self, query: &CompiledQuery) -> Result<Vec<Object>, DataError> {
        let statement = render::render(query)?;
        self.connection()?
            .fetch(&statement.sql, &statement.params, false)
    }

    fn insert(&mut self, insert: &CompiledInsert) -> Result<WriteOutcome, DataError> {
        let statement = render::render_insert(insert)?;
        let cache = Arc::clone(&self.identity);
        let identity = lookup_identity(&cache, self.connection()?, insert.table);
        self.connection()?.insert(
            &statement.sql,
            &statement.params,
            identity.as_deref(),
            false,
        )
    }

    fn update(&mut self, update: &CompiledUpdate) -> Result<u64, DataError> {
        let statement = render::render_update(update)?;
        self.connection()?
            .execute(&statement.sql, &statement.params, false)
    }

    fn delete(&mut self, delete: &CompiledDelete) -> Result<u64, DataError> {
        let statement = render::render_delete(delete)?;
        self.connection()?
            .execute(&statement.sql, &statement.params, false)
    }

    fn fetch_sql(
        &mut self,
        sql: &str,
        params: &[crate::JsonValue],
    ) -> Result<Vec<Object>, DataError> {
        self.connection()?.fetch(sql, params, false)
    }

    fn execute_sql(&mut self, sql: &str, params: &[crate::JsonValue]) -> Result<u64, DataError> {
        self.connection()?.execute(sql, params, false)
    }

    fn commit(mut self: Box<Self>) -> Result<(), DataError> {
        if let Some(connection) = self.connection.take() {
            connection.commit()?;
        }
        Ok(())
    }

    fn rollback(mut self: Box<Self>) -> Result<(), DataError> {
        if let Some(connection) = self.connection.take() {
            connection.rollback()?;
        }
        Ok(())
    }
}

impl Drop for OracleTransaction {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            let _ = connection.rollback();
        }
    }
}

fn lookup_identity(
    cache: &Arc<Mutex<HashMap<String, Option<String>>>>,
    connection: &mut Connection,
    table: &str,
) -> Option<String> {
    let key = table.to_ascii_uppercase();
    {
        let cache = match cache.lock() {
            Ok(cache) => cache,
            Err(poisoned) => {
                eprintln!("identity cache mutex poisoned; recovering");
                poisoned.into_inner()
            }
        };
        if let Some(cached) = cache.get(&key) {
            return cached.clone();
        }
    }

    let rows = connection
        .fetch(
            "SELECT column_name FROM user_tab_identity_cols WHERE table_name = :1",
            &[Value::from(key.clone())],
            false,
        )
        .ok();

    let column = rows.as_deref().and_then(|rows| match rows {
        [row] => row
            .get("column_name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    });

    cache
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(key, column.clone());
    column
}

fn count_from_rows(rows: &[Object]) -> Result<u64, DataError> {
    rows.first()
        .and_then(|row| row.get("count"))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_i64().map(|count| count.max(0) as u64))
        })
        .ok_or_else(|| DataError::protocol("the count query returned no count"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easy_connect_and_debug_hides_the_password() {
        let options = ConnectOptions::new("db", 1521, "XEPDB1", "app", "s3cret");
        assert_eq!(options.database_name(), "//db:1521/XEPDB1");
        let debug = format!("{options:?}");
        assert!(!debug.contains("s3cret"), "password leaked: {debug}");
    }
}
