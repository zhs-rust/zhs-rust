use std::sync::Arc;

use crate::DataError;
use zhs_json::Object;

use crate::query::CompiledQuery;
use crate::write::{CompiledDelete, CompiledInsert, CompiledUpdate, WriteOutcome};

pub trait Backend: Send + Sync {
    fn capabilities(&self) -> crate::Capabilities {
        crate::Capabilities::NONE
    }

    fn fetch(&self, query: &CompiledQuery) -> Result<Vec<Object>, DataError>;

    fn count(&self, _query: &CompiledQuery) -> Result<u64, DataError> {
        Err(DataError::Unsupported(
            "counting rows is not available on this backend",
        ))
    }

    fn insert(&self, _statement: &CompiledInsert) -> Result<WriteOutcome, DataError> {
        Err(DataError::Unsupported(
            "inserts are not available on this backend",
        ))
    }

    fn update(&self, _statement: &CompiledUpdate) -> Result<u64, DataError> {
        Err(DataError::Unsupported(
            "updates are not available on this backend",
        ))
    }

    fn delete(&self, _statement: &CompiledDelete) -> Result<u64, DataError> {
        Err(DataError::Unsupported(
            "deletes are not available on this backend",
        ))
    }

    fn fetch_sql(
        &self,
        _sql: &str,
        _params: &[crate::JsonValue],
    ) -> Result<Vec<Object>, DataError> {
        Err(DataError::Unsupported(
            "raw SQL is not available on this backend",
        ))
    }

    fn execute_sql(&self, _sql: &str, _params: &[crate::JsonValue]) -> Result<u64, DataError> {
        Err(DataError::Unsupported(
            "raw SQL is not available on this backend",
        ))
    }

    fn fetch_command(&self, _database: &str, _command: &Object) -> Result<Vec<Object>, DataError> {
        Err(DataError::Unsupported(
            "raw commands are not available on this backend",
        ))
    }

    fn begin(&self) -> Result<Box<dyn Transaction>, DataError> {
        Err(DataError::Unsupported(
            "transactions are not available on this backend",
        ))
    }
}

pub trait Transaction: Send {
    fn fetch(&mut self, query: &CompiledQuery) -> Result<Vec<Object>, DataError>;

    fn insert(&mut self, _statement: &CompiledInsert) -> Result<WriteOutcome, DataError> {
        Err(DataError::Unsupported(
            "inserts are not available in this transaction",
        ))
    }

    fn update(&mut self, _statement: &CompiledUpdate) -> Result<u64, DataError> {
        Err(DataError::Unsupported(
            "updates are not available in this transaction",
        ))
    }

    fn delete(&mut self, _statement: &CompiledDelete) -> Result<u64, DataError> {
        Err(DataError::Unsupported(
            "deletes are not available in this transaction",
        ))
    }

    fn fetch_sql(
        &mut self,
        _sql: &str,
        _params: &[crate::JsonValue],
    ) -> Result<Vec<Object>, DataError> {
        Err(DataError::Unsupported(
            "raw SQL is not available in this transaction",
        ))
    }

    fn execute_sql(&mut self, _sql: &str, _params: &[crate::JsonValue]) -> Result<u64, DataError> {
        Err(DataError::Unsupported(
            "raw SQL is not available in this transaction",
        ))
    }

    fn commit(self: Box<Self>) -> Result<(), DataError>;

    fn rollback(self: Box<Self>) -> Result<(), DataError>;
}

#[derive(Clone)]
pub struct Db {
    backend: Arc<dyn Backend>,
}

impl Db {
    pub fn new(backend: Arc<dyn Backend>) -> Self {
        Self { backend }
    }

    pub fn backend(&self) -> &Arc<dyn Backend> {
        &self.backend
    }

    #[allow(clippy::unused_async)]
    pub async fn fetch(&self, query: CompiledQuery) -> Result<Vec<Object>, DataError> {
        self.backend.fetch(&query)
    }

    #[allow(clippy::unused_async)]
    pub async fn count(&self, query: CompiledQuery) -> Result<u64, DataError> {
        self.backend.count(&query)
    }

    #[allow(clippy::unused_async)]
    pub async fn insert_compiled(
        &self,
        statement: CompiledInsert,
    ) -> Result<WriteOutcome, DataError> {
        self.backend.insert(&statement)
    }

    #[allow(clippy::unused_async)]
    pub async fn update_compiled(&self, statement: CompiledUpdate) -> Result<u64, DataError> {
        self.backend.update(&statement)
    }

    #[allow(clippy::unused_async)]
    pub async fn delete_compiled(&self, statement: CompiledDelete) -> Result<u64, DataError> {
        self.backend.delete(&statement)
    }
}
