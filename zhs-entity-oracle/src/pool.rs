use std::sync::Arc;

use zhs_entity::DataError;
use zhs_entity::pool::Connector;

use crate::connection::Connection;
use crate::ffi::Environment;

use crate::backend::ConnectOptions;

pub(crate) struct OracleConnector {
    environment: Arc<Environment>,
    options: ConnectOptions,
}

impl OracleConnector {
    pub(crate) fn new(environment: Arc<Environment>, options: ConnectOptions) -> Self {
        Self {
            environment,
            options,
        }
    }
}

impl Connector for OracleConnector {
    type Connection = Connection;

    fn connect(&self) -> Result<Connection, DataError> {
        Connection::open(Arc::clone(&self.environment), &self.options)
    }

    fn probe(&self, connection: &mut Connection) -> Result<(), DataError> {
        connection
            .fetch("SELECT 1 FROM DUAL", &[], false)
            .map(|_| ())
    }
}

pub(crate) type Pool = zhs_entity::pool::Pool<OracleConnector>;

pub(crate) type PooledConnection = zhs_entity::pool::PooledConnection<OracleConnector>;

pub use zhs_entity::pool::PoolStats;
