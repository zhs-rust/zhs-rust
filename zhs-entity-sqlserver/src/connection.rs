use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use zhs_entity::DataError;
use zhs_json::{Object, Value};

use crate::login;
use crate::packet::{self, PACKET_SIZE};
use crate::token::{self, Response};
use crate::types::{ParamKind, classify, declarations, write_param};

#[derive(Clone)]
pub struct ConnectOptions {
    pub host: String,

    pub port: u16,

    pub user: String,

    pub password: String,

    pub database: Option<String>,

    pub connect_timeout: Duration,

    pub read_timeout: Option<Duration>,

    pub write_timeout: Option<Duration>,
}

impl ConnectOptions {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            user: user.into(),
            password: password.into(),
            database: None,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Some(Duration::from_secs(30)),
            write_timeout: Some(Duration::from_secs(30)),
        }
    }

    pub fn database(mut self, name: impl Into<String>) -> Self {
        self.database = Some(name.into());
        self
    }

    fn address(&self) -> Result<std::net::SocketAddr, DataError> {
        let text = format!("{}:{}", self.host, self.port);
        text.to_socket_addrs()
            .map_err(|error| DataError::protocol(format!("resolving `{text}`: {error}")))?
            .next()
            .ok_or_else(|| DataError::protocol(format!("`{text}` did not resolve to an address")))
    }
}

impl std::fmt::Debug for ConnectOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectOptions")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &"***")
            .field("database", &self.database)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecOutcome {
    pub affected_rows: u64,

    pub last_insert_id: Option<u64>,
}

pub struct Connection {
    stream: TcpStream,
    packet_size: usize,

    transaction_descriptor: [u8; 8],
}

impl Connection {
    pub fn connect(options: &ConnectOptions) -> Result<Self, DataError> {
        let address = options.address()?;
        let stream = TcpStream::connect_timeout(&address, options.connect_timeout)
            .map_err(|error| DataError::protocol(format!("connecting to {address}: {error}")))?;
        stream
            .set_nodelay(true)
            .map_err(|error| DataError::protocol(format!("configuring the socket: {error}")))?;
        stream
            .set_read_timeout(options.read_timeout)
            .map_err(|error| {
                DataError::protocol(format!("configuring the read timeout: {error}"))
            })?;
        stream
            .set_write_timeout(options.write_timeout)
            .map_err(|error| {
                DataError::protocol(format!("configuring the write timeout: {error}"))
            })?;

        let mut connection = Self {
            stream,
            packet_size: PACKET_SIZE,
            transaction_descriptor: [0; 8],
        };
        connection.handshake(options)?;
        connection.configure(options)?;
        Ok(connection)
    }

    pub fn fetch(&mut self, sql: &str, params: &[Value]) -> Result<Vec<Object>, DataError> {
        let mut response = self.request(sql, params)?;
        Ok(if response.results.is_empty() {
            Vec::new()
        } else {
            response.results.remove(0)
        })
    }

    pub fn execute(&mut self, sql: &str, params: &[Value]) -> Result<ExecOutcome, DataError> {
        let response = self.request(sql, params)?;
        Ok(ExecOutcome {
            affected_rows: response.affected_rows(),
            last_insert_id: response.last_insert_id(),
        })
    }

    pub(crate) fn ping(&mut self) -> Result<(), DataError> {
        self.fetch("SELECT 1", &[])?;
        Ok(())
    }

    fn handshake(&mut self, options: &ConnectOptions) -> Result<(), DataError> {
        let payload = login::prelogin_payload();
        packet::write_message(
            &mut self.stream,
            packet::TYPE_PRELOGIN,
            &payload,
            self.packet_size,
        )
        .map_err(io_error)?;

        let (kind, payload) = packet::read_message(&mut self.stream).map_err(io_error)?;
        if kind != packet::TYPE_REPLY {
            return Err(DataError::protocol(format!(
                "expected a PRELOGIN reply, got TDS packet type {kind:#04x}"
            )));
        }

        match login::encryption_level(&payload)? {
            login::ENCRYPT_OFF | login::ENCRYPT_NOT_SUP => {}
            login::ENCRYPT_ON | login::ENCRYPT_REQ => {
                return Err(DataError::Unsupported(
                    "TLS-encrypted TDS; disable force encryption on the server, or terminate TLS in front of it",
                ));
            }
            other => {
                return Err(DataError::protocol(format!(
                    "unknown PRELOGIN encryption level {other:#04x}"
                )));
            }
        }

        let payload = login::login7_payload(options, self.packet_size)?;
        packet::write_message(
            &mut self.stream,
            packet::TYPE_LOGIN7,
            &payload,
            self.packet_size,
        )
        .map_err(io_error)?;

        let (kind, payload) = packet::read_message(&mut self.stream).map_err(io_error)?;
        if kind != packet::TYPE_REPLY {
            return Err(DataError::protocol(format!(
                "expected a LOGIN7 reply, got TDS packet type {kind:#04x}"
            )));
        }

        let response = token::parse(&payload)?;
        if !response.login_ack {
            return Err(DataError::protocol(
                "the server replied without a LOGINACK token",
            ));
        }

        Ok(())
    }

    fn configure(&mut self, options: &ConnectOptions) -> Result<(), DataError> {
        self.execute("SET XACT_ABORT ON; SET IMPLICIT_TRANSACTIONS OFF", &[])?;

        if let Some(database) = &options.database {
            let identifier = crate::render::quote_identifier(database)?;
            self.execute(&format!("USE {identifier}"), &[])?;
        }

        Ok(())
    }

    fn request(&mut self, sql: &str, params: &[Value]) -> Result<Response, DataError> {
        if params.is_empty() {
            let mut payload = Vec::with_capacity(22 + sql.len() * 2);
            packet::push_all_headers(&mut payload, self.transaction_descriptor);
            packet::push_utf16(&mut payload, sql);
            packet::write_message(
                &mut self.stream,
                packet::TYPE_SQL_BATCH,
                &payload,
                self.packet_size,
            )
            .map_err(io_error)?;
        } else {
            let payload = rpc_payload(sql, params, self.transaction_descriptor)?;
            packet::write_message(
                &mut self.stream,
                packet::TYPE_RPC,
                &payload,
                self.packet_size,
            )
            .map_err(io_error)?;
        }

        let (kind, payload) = packet::read_message(&mut self.stream).map_err(io_error)?;
        if kind != packet::TYPE_REPLY {
            return Err(DataError::protocol(format!(
                "expected a TDS reply, got packet type {kind:#04x}"
            )));
        }

        let response = token::parse(&payload)?;

        match &response.transaction {
            Some(token::TransactionChange::Begin(descriptor)) => {
                self.transaction_descriptor = *descriptor;
            }
            Some(token::TransactionChange::End) => {
                self.transaction_descriptor = [0; 8];
            }
            None => {}
        }

        Ok(response)
    }
}

fn rpc_payload(
    sql: &str,
    params: &[Value],
    transaction_descriptor: [u8; 8],
) -> Result<Vec<u8>, DataError> {
    let mut payload = Vec::new();

    packet::push_all_headers(&mut payload, transaction_descriptor);

    packet::push_u16le(&mut payload, "sp_executesql".encode_utf16().count() as u16);
    packet::push_utf16(&mut payload, "sp_executesql");
    packet::push_u16le(&mut payload, 0x0000);

    let mut kinds = Vec::with_capacity(params.len() + 2);
    kinds.push(ParamKind::Text(sql.to_owned()));
    kinds.push(ParamKind::Text(declarations(params)?));
    for value in params {
        kinds.push(classify(value)?);
    }

    for kind in &kinds {
        packet::push_b_varchar(&mut payload, "")?;
        packet::push_u8(&mut payload, 0x00);
        write_param(&mut payload, kind);
    }

    Ok(payload)
}

fn io_error(error: io::Error) -> DataError {
    match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
            DataError::Timeout("waiting for the SQL Server response")
        }
        _ => DataError::protocol(format!("TDS transport error: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_body(sql: &str, params: &[Value]) -> Vec<u8> {
        rpc_payload(sql, params, [0; 8]).unwrap()
    }

    #[test]
    fn rpc_payload_calls_sp_executesql_by_name() {
        let payload = request_body(
            "SELECT * FROM [users] WHERE [id] = @P1",
            &[Value::from(7i64)],
        );

        assert_eq!(payload[0..4], 22u32.to_le_bytes());
        assert_eq!(payload[4..8], 18u32.to_le_bytes());
        assert_eq!(payload[8..10], 0x0002u16.to_le_bytes());

        let name_length = u16::from_le_bytes([payload[22], payload[23]]) as usize;
        assert_eq!(name_length, 13);
        let units: Vec<u16> = payload[24..24 + name_length * 2]
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(String::from_utf16(&units).unwrap(), "sp_executesql");
        assert_eq!(
            &payload[24 + name_length * 2..26 + name_length * 2],
            &[0u8, 0u8]
        );
    }

    #[test]
    fn rpc_payload_carries_the_statement_and_declarations_as_text() {
        let params = [Value::from("ada")];
        let payload = request_body("SELECT @P1", &params);

        let contains = |needle: &str| {
            let needle: Vec<u8> = needle
                .encode_utf16()
                .flat_map(|unit| unit.to_le_bytes())
                .collect();
            payload.windows(needle.len()).any(|window| window == needle)
        };

        assert!(contains("SELECT @P1"));
        assert!(contains("@P1 nvarchar(4000)"));
    }

    #[test]
    fn options_debug_hides_the_password() {
        let options = ConnectOptions::new("db", 1433, "sa", "s3cret");
        assert!(!format!("{options:?}").contains("s3cret"));
    }
}
