use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zhs_entity::DataError;
use zhs_entity::{
    CompiledDelete, CompiledInsert, CompiledQuery, CompiledUpdate, Transaction, WriteOutcome,
};
use zhs_json::{Object, Value};

use crate::pool::{Pool, PoolStats, PooledConnection, SqlServerConnector};

pub use crate::connection::{ConnectOptions, ExecOutcome};
pub use crate::pool::PoolOptions;

pub struct Backend {
    pool: Pool,

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
        Ok(Self {
            pool: Pool::new(SqlServerConnector::new(options), pool_options)?,
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

    #[allow(dead_code)]
    pub fn pool(&self) -> &Pool {
        &self.pool
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
        let statement = crate::render::render(query)?;
        let mut connection = self.pool.get()?;
        let outcome = connection.fetch(&statement.sql, &statement.params);

        if outcome.is_err() {
            connection.discard();
        }

        outcome
    }

    fn count(&self, query: &CompiledQuery) -> Result<u64, DataError> {
        let statement = crate::render::render_count(query)?;
        let mut connection = self.pool.get()?;
        let rows = connection.fetch(&statement.sql, &statement.params);

        if rows.is_err() {
            connection.discard();
        }

        count_from_rows(&rows?)
    }

    fn insert(&self, insert: &CompiledInsert) -> Result<WriteOutcome, DataError> {
        let statement = crate::render::render_insert(insert)?;
        let mut connection = self.pool.get()?;
        let identity = lookup_identity(&self.identity, &mut connection, insert.table);

        let statement = if identity.is_some() {
            crate::render::append_identity_select(statement)
        } else {
            statement
        };

        let outcome = connection.execute(&statement.sql, &statement.params);

        if outcome.is_err() {
            connection.discard();
        }

        outcome.map(to_write)
    }

    fn update(&self, statement: &CompiledUpdate) -> Result<u64, DataError> {
        let rendered = crate::render::render_update(statement)?;
        let mut connection = self.pool.get()?;
        let outcome = connection.execute(&rendered.sql, &rendered.params);

        if outcome.is_err() {
            connection.discard();
            return outcome.map(|outcome| outcome.affected_rows);
        }

        outcome.map(|outcome| outcome.affected_rows)
    }

    fn delete(&self, statement: &CompiledDelete) -> Result<u64, DataError> {
        let rendered = crate::render::render_delete(statement)?;
        let mut connection = self.pool.get()?;
        let outcome = connection.execute(&rendered.sql, &rendered.params);

        if outcome.is_err() {
            connection.discard();
            return outcome.map(|outcome| outcome.affected_rows);
        }

        outcome.map(|outcome| outcome.affected_rows)
    }

    fn fetch_sql(&self, sql: &str, params: &[Value]) -> Result<Vec<Object>, DataError> {
        let mut connection = self.pool.get()?;
        let outcome = connection.fetch(sql, params);

        if outcome.is_err() {
            connection.discard();
        }

        outcome
    }

    fn execute_sql(&self, sql: &str, params: &[Value]) -> Result<u64, DataError> {
        let mut connection = self.pool.get()?;
        let outcome = connection.execute(sql, params);

        if outcome.is_err() {
            connection.discard();
            return outcome.map(|outcome| outcome.affected_rows);
        }

        outcome.map(|outcome| outcome.affected_rows)
    }

    fn begin(&self) -> Result<Box<dyn Transaction>, DataError> {
        let mut connection = self.pool.get()?;
        connection.execute("BEGIN TRANSACTION", &[])?;
        Ok(Box::new(SqlTransaction {
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

struct SqlTransaction {
    connection: Option<PooledConnection>,
    identity: Arc<Mutex<HashMap<String, Option<String>>>>,
}

impl SqlTransaction {
    fn connection(&mut self) -> Result<&mut PooledConnection, DataError> {
        self.connection.as_mut().ok_or(DataError::Closed)
    }

    fn run(
        &mut self,
        sql: &str,
        params: &[Value],
    ) -> Result<crate::connection::ExecOutcome, DataError> {
        let Some(connection) = self.connection.as_mut() else {
            return Err(DataError::Closed);
        };

        let outcome = connection.execute(sql, params);

        if outcome.is_err()
            && let Some(broken) = self.connection.take()
        {
            broken.discard();
        }

        outcome
    }
}

impl Transaction for SqlTransaction {
    fn fetch(&mut self, query: &CompiledQuery) -> Result<Vec<Object>, DataError> {
        let statement = crate::render::render(query)?;
        let connection = self.connection()?;
        connection.fetch(&statement.sql, &statement.params)
    }

    fn insert(&mut self, insert: &CompiledInsert) -> Result<WriteOutcome, DataError> {
        let statement = crate::render::render_insert(insert)?;
        let cache = Arc::clone(&self.identity);
        let identity = lookup_identity(&cache, self.connection()?, insert.table);

        let statement = if identity.is_some() {
            crate::render::append_identity_select(statement)
        } else {
            statement
        };

        self.run(&statement.sql, &statement.params).map(to_write)
    }

    fn update(&mut self, statement: &CompiledUpdate) -> Result<u64, DataError> {
        let rendered = crate::render::render_update(statement)?;
        Ok(self.run(&rendered.sql, &rendered.params)?.affected_rows)
    }

    fn delete(&mut self, statement: &CompiledDelete) -> Result<u64, DataError> {
        let rendered = crate::render::render_delete(statement)?;
        Ok(self.run(&rendered.sql, &rendered.params)?.affected_rows)
    }

    fn fetch_sql(&mut self, sql: &str, params: &[Value]) -> Result<Vec<Object>, DataError> {
        let connection = self.connection()?;
        connection.fetch(sql, params)
    }

    fn execute_sql(&mut self, sql: &str, params: &[Value]) -> Result<u64, DataError> {
        Ok(self.run(sql, params)?.affected_rows)
    }

    fn commit(mut self: Box<Self>) -> Result<(), DataError> {
        let mut connection = self.connection.take().ok_or(DataError::Closed)?;
        match connection.execute("COMMIT TRANSACTION", &[]) {
            Ok(_) => Ok(()),
            Err(error) => {
                connection.discard();
                Err(error)
            }
        }
    }

    fn rollback(mut self: Box<Self>) -> Result<(), DataError> {
        let Some(mut connection) = self.connection.take() else {
            return Ok(());
        };
        match connection.execute("ROLLBACK TRANSACTION", &[]) {
            Ok(_) => Ok(()),
            Err(error) => {
                connection.discard();
                Err(error)
            }
        }
    }
}

impl Drop for SqlTransaction {
    fn drop(&mut self) {
        if let Some(mut connection) = self.connection.take()
            && connection.execute("ROLLBACK TRANSACTION", &[]).is_err()
        {
            connection.discard();
        }
    }
}

fn lookup_identity(
    cache: &Arc<Mutex<HashMap<String, Option<String>>>>,
    connection: &mut PooledConnection,
    table: &str,
) -> Option<String> {
    {
        let cache = lock(cache);
        if let Some(cached) = cache.get(table) {
            return cached.clone();
        }
    }

    let rows = connection
        .fetch(
            "SELECT TOP 1 name FROM sys.identity_columns WHERE object_id = OBJECT_ID(@P1)",
            &[Value::from(table.to_owned())],
        )
        .ok();

    let column = rows.as_deref().and_then(|rows| match rows {
        [row] => row.get("name").and_then(Value::as_str).map(str::to_owned),
        _ => None,
    });

    lock(cache).insert(table.to_owned(), column.clone());
    column
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            eprintln!("identity cache mutex poisoned; recovering");
            poisoned.into_inner()
        }
    }
}

fn to_write(outcome: ExecOutcome) -> WriteOutcome {
    WriteOutcome {
        affected_rows: outcome.affected_rows,
        last_insert_id: outcome.last_insert_id,
    }
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
