use zhs_entity::DataError;
use zhs_entity::pool::Connector;

use crate::connection::{ConnectOptions, Connection};

pub struct SqlServerConnector {
    options: ConnectOptions,
}

impl SqlServerConnector {
    pub(crate) fn new(options: ConnectOptions) -> Self {
        Self { options }
    }
}

impl Connector for SqlServerConnector {
    type Connection = Connection;

    fn connect(&self) -> Result<Connection, DataError> {
        Connection::connect(&self.options)
    }

    fn probe(&self, connection: &mut Connection) -> Result<(), DataError> {
        connection.ping()
    }
}

pub type Pool = zhs_entity::pool::Pool<SqlServerConnector>;

pub type PooledConnection = zhs_entity::pool::PooledConnection<SqlServerConnector>;

pub use zhs_entity::pool::{PoolOptions, PoolStats};
