use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use zhs_entity::DataError;
use zhs_json::{Object, Value};

use crate::auth;
use crate::packet::{PacketReader, push_lenenc_bytes, read_packet, write_packet};

const CLIENT_LONG_PASSWORD: u32 = 0x0000_0001;
const CLIENT_LONG_FLAG: u32 = 0x0000_0004;
const CLIENT_CONNECT_WITH_DB: u32 = 0x0000_0008;
const CLIENT_PROTOCOL_41: u32 = 0x0000_0200;
const CLIENT_TRANSACTIONS: u32 = 0x0000_2000;
const CLIENT_SECURE_CONNECTION: u32 = 0x0000_8000;
const CLIENT_PLUGIN_AUTH: u32 = 0x0008_0000;

const COM_QUIT: u8 = 0x01;
const COM_QUERY: u8 = 0x03;
const COM_PING: u8 = 0x0E;
const COM_STMT_PREPARE: u8 = 0x16;
const COM_STMT_EXECUTE: u8 = 0x17;
const COM_STMT_CLOSE: u8 = 0x19;

const MYSQL_TYPE_TINY: u8 = 1;
const MYSQL_TYPE_SHORT: u8 = 2;
const MYSQL_TYPE_LONG: u8 = 3;
const MYSQL_TYPE_FLOAT: u8 = 4;
const MYSQL_TYPE_DOUBLE: u8 = 5;
const MYSQL_TYPE_NULL: u8 = 6;
const MYSQL_TYPE_TIMESTAMP: u8 = 7;
const MYSQL_TYPE_LONGLONG: u8 = 8;
const MYSQL_TYPE_INT24: u8 = 9;
const MYSQL_TYPE_DATE: u8 = 10;
const MYSQL_TYPE_TIME: u8 = 11;
const MYSQL_TYPE_DATETIME: u8 = 12;
const MYSQL_TYPE_YEAR: u8 = 13;
const MYSQL_TYPE_NEWDATE: u8 = 14;
const MYSQL_TYPE_NEWDECIMAL: u8 = 246;
const MYSQL_TYPE_VAR_STRING: u8 = 253;

const CHARSET_UTF8MB4: u8 = 45;

/// Column definition flag bit marking an UNSIGNED numeric column.
const COLUMN_FLAG_UNSIGNED: u16 = 0x0020;

const MAX_PACKET_SIZE: u32 = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecOutcome {
    pub affected_rows: u64,

    pub last_insert_id: Option<u64>,
}

pub struct Connection {
    stream: TcpStream,
    sequence: u8,
    capabilities: u32,
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
            sequence: 0,
            capabilities: 0,
        };
        connection.handshake(options)?;
        Ok(connection)
    }

    pub fn query(&mut self, sql: &str) -> Result<Vec<Object>, DataError> {
        let (_, rows) = self.query_result(sql)?;
        Ok(rows)
    }

    pub fn query_exec(&mut self, sql: &str) -> Result<ExecOutcome, DataError> {
        let (outcome, _) = self.query_result(sql)?;
        Ok(outcome.unwrap_or_default())
    }

    pub fn execute_exec(&mut self, sql: &str, params: &[Value]) -> Result<ExecOutcome, DataError> {
        let prepared = self.prepare(sql)?;
        let outcome = self.execute_prepared_exec(&prepared, params);
        let closed = self.close_statement(prepared.id);

        match outcome {
            Ok(outcome) => {
                closed?;
                Ok(outcome)
            }
            Err(error) => Err(error),
        }
    }

    fn query_result(&mut self, sql: &str) -> Result<(Option<ExecOutcome>, Vec<Object>), DataError> {
        let mut payload = Vec::with_capacity(sql.len() + 1);
        payload.push(COM_QUERY);
        payload.extend_from_slice(sql.as_bytes());

        self.sequence = 0;
        self.write_packet(&payload)?;
        self.read_result(false)
    }

    pub fn execute(&mut self, sql: &str, params: &[Value]) -> Result<Vec<Object>, DataError> {
        let prepared = self.prepare(sql)?;
        let outcome = self.execute_prepared(&prepared, params);
        let closed = self.close_statement(prepared.id);

        match outcome {
            Ok(rows) => {
                closed?;
                Ok(rows)
            }
            Err(error) => Err(error),
        }
    }

    pub fn capabilities(&self) -> u32 {
        self.capabilities
    }

    pub fn ping(&mut self) -> Result<(), DataError> {
        self.sequence = 0;
        self.write_packet(&[COM_PING])?;
        let packet = self.read_packet()?;
        match packet.first() {
            Some(&0x00) => Ok(()),
            Some(&0xFF) => Err(server_error(&packet)),
            _ => Err(DataError::protocol("unexpected reply to `COM_PING`")),
        }
    }

    pub fn close(mut self) -> Result<(), DataError> {
        self.sequence = 0;
        let _ = self.write_packet(&[COM_QUIT]);
        let _ = self.stream.flush();
        Ok(())
    }

    fn handshake(&mut self, options: &ConnectOptions) -> Result<(), DataError> {
        let packet = self.read_packet()?;
        if packet.first() == Some(&0xFF) {
            return Err(server_error(&packet));
        }
        let (server_capabilities, nonce, plugin) = parse_handshake(&packet)?;

        if plugin != "mysql_native_password" {
            return Err(DataError::Unsupported(
                "only `mysql_native_password` authentication is implemented",
            ));
        }

        let mut desired =
            CLIENT_LONG_PASSWORD | CLIENT_LONG_FLAG | CLIENT_PROTOCOL_41 | CLIENT_TRANSACTIONS;
        desired |= CLIENT_SECURE_CONNECTION | CLIENT_PLUGIN_AUTH;
        if options.database.is_some() {
            desired |= CLIENT_CONNECT_WITH_DB;
        }
        let capabilities = desired & server_capabilities;
        self.capabilities = capabilities;

        let response = auth::native_password(&options.password, &nonce);

        let mut payload = Vec::with_capacity(128);
        payload.extend_from_slice(&capabilities.to_le_bytes());
        payload.extend_from_slice(&MAX_PACKET_SIZE.to_le_bytes());
        payload.push(CHARSET_UTF8MB4);
        payload.extend_from_slice(&[0_u8; 23]);
        payload.extend_from_slice(options.user.as_bytes());
        payload.push(0);
        payload.push(response.len() as u8);
        payload.extend_from_slice(&response);
        if let Some(database) = &options.database {
            payload.extend_from_slice(database.as_bytes());
            payload.push(0);
        }
        if capabilities & CLIENT_PLUGIN_AUTH != 0 {
            payload.extend_from_slice(b"mysql_native_password");
            payload.push(0);
        }

        self.write_packet(&payload)?;
        self.finish_authentication(options)
    }

    fn finish_authentication(&mut self, options: &ConnectOptions) -> Result<(), DataError> {
        loop {
            let packet = self.read_packet()?;
            match packet.first().copied() {
                Some(0x00) => return Ok(()),
                Some(0xFF) => return Err(server_error(&packet)),
                Some(0xFE) => {
                    let mut reader = PacketReader::new(&packet[1..]);
                    let plugin = String::from_utf8_lossy(reader.null_terminated()?).into_owned();
                    if plugin != "mysql_native_password" {
                        return Err(DataError::Unsupported(
                            "the server asked for another authentication plugin",
                        ));
                    }
                    let nonce = reader.rest_without_nul();
                    let response = auth::native_password(&options.password, nonce);
                    self.write_packet(&response)?;
                }
                Some(0x01) => {
                    return Err(DataError::Unsupported(
                        "the server requested additional authentication data",
                    ));
                }
                _ => {
                    return Err(DataError::protocol(
                        "unexpected packet during authentication",
                    ));
                }
            }
        }
    }

    fn read_result_set(&mut self, binary: bool) -> Result<Vec<Object>, DataError> {
        let (_, rows) = self.read_result(binary)?;
        Ok(rows)
    }

    fn read_result(
        &mut self,
        binary: bool,
    ) -> Result<(Option<ExecOutcome>, Vec<Object>), DataError> {
        let packet = self.read_packet()?;
        match packet.first().copied() {
            Some(0x00) => return Ok((Some(parse_ok(&packet)?), Vec::new())),
            Some(0xFF) => return Err(server_error(&packet)),
            Some(0xFB) => {
                return Err(DataError::Unsupported("`LOAD DATA LOCAL INFILE`"));
            }
            _ => {}
        }

        let mut reader = PacketReader::new(&packet);
        let column_count = reader
            .lenenc_int()?
            .ok_or_else(|| DataError::protocol("result set has no column count"))?;
        if column_count > 4096 {
            return Err(DataError::protocol(format!(
                "result set column count {column_count} exceeds the limit of 4096"
            )));
        }
        let column_count = column_count as usize;

        let mut columns = Vec::with_capacity(column_count);
        for _ in 0..column_count {
            let definition = self.read_packet()?;
            columns.push(parse_column(&definition)?);
        }
        self.expect_eof()?;

        let mut rows = Vec::new();
        loop {
            let packet = self.read_packet()?;
            if is_eof(&packet) {
                return Ok((None, rows));
            }
            if packet.first() == Some(&0xFF) {
                return Err(server_error(&packet));
            }
            rows.push(if binary {
                decode_binary_row(&packet, &columns)?
            } else {
                decode_text_row(&packet, &columns)?
            });
        }
    }

    fn expect_eof(&mut self) -> Result<(), DataError> {
        let packet = self.read_packet()?;
        if is_eof(&packet) || packet.first() == Some(&0x00) {
            return Ok(());
        }
        if packet.first() == Some(&0xFF) {
            return Err(server_error(&packet));
        }
        Err(DataError::protocol("expected an end-of-result marker"))
    }

    fn prepare(&mut self, sql: &str) -> Result<Prepared, DataError> {
        let mut payload = Vec::with_capacity(sql.len() + 1);
        payload.push(COM_STMT_PREPARE);
        payload.extend_from_slice(sql.as_bytes());

        self.sequence = 0;
        self.write_packet(&payload)?;

        let packet = self.read_packet()?;
        if packet.first() == Some(&0xFF) {
            return Err(server_error(&packet));
        }
        if packet.first() != Some(&0x00) {
            return Err(DataError::protocol("unexpected prepare response"));
        }

        let mut reader = PacketReader::new(&packet[1..]);
        let id = reader.u32_le()?;
        let column_count = reader.u16_le()?;
        let parameter_count = reader.u16_le()?;
        let _filler = reader.byte()?;
        let _warnings = reader.u16_le()?;

        for _ in 0..parameter_count {
            self.read_packet()?;
        }
        if parameter_count > 0 {
            self.expect_eof()?;
        }

        let mut columns = Vec::with_capacity(usize::from(column_count));
        for _ in 0..column_count {
            let definition = self.read_packet()?;
            columns.push(parse_column(&definition)?);
        }
        if column_count > 0 {
            self.expect_eof()?;
        }
        drop(columns);

        Ok(Prepared {
            id,
            parameter_count,
        })
    }

    fn execute_prepared(
        &mut self,
        prepared: &Prepared,
        params: &[Value],
    ) -> Result<Vec<Object>, DataError> {
        self.write_execute(prepared, params)?;
        self.read_result_set(true)
    }

    fn execute_prepared_exec(
        &mut self,
        prepared: &Prepared,
        params: &[Value],
    ) -> Result<ExecOutcome, DataError> {
        self.write_execute(prepared, params)?;
        let (outcome, _) = self.read_result(true)?;
        Ok(outcome.unwrap_or_default())
    }

    fn write_execute(&mut self, prepared: &Prepared, params: &[Value]) -> Result<(), DataError> {
        if params.len() != usize::from(prepared.parameter_count) {
            return Err(DataError::query(format!(
                "statement expects {} parameters, got {}",
                prepared.parameter_count,
                params.len()
            )));
        }

        let mut payload = Vec::with_capacity(16 + params.len() * 12);
        payload.push(COM_STMT_EXECUTE);
        payload.extend_from_slice(&prepared.id.to_le_bytes());
        payload.push(0x00);
        payload.extend_from_slice(&1_u32.to_le_bytes());

        if !params.is_empty() {
            let bitmap_length = params.len().div_ceil(8);
            let mut bitmap = vec![0_u8; bitmap_length];
            for (index, param) in params.iter().enumerate() {
                if param.is_null() {
                    bitmap[index / 8] |= 1 << (index % 8);
                }
            }
            payload.extend_from_slice(&bitmap);
            payload.push(1);

            for param in params {
                payload.push(binary_type(param));
                payload.push(unsigned_flag(param));
            }
            for param in params {
                push_binary_value(&mut payload, param)?;
            }
        }

        self.sequence = 0;
        self.write_packet(&payload)
    }

    fn close_statement(&mut self, id: u32) -> Result<(), DataError> {
        let mut payload = Vec::with_capacity(5);
        payload.push(COM_STMT_CLOSE);
        payload.extend_from_slice(&id.to_le_bytes());
        self.sequence = 0;
        self.write_packet(&payload)
    }

    fn read_packet(&mut self) -> Result<Vec<u8>, DataError> {
        read_packet(&mut self.stream, &mut self.sequence)
    }

    fn write_packet(&mut self, payload: &[u8]) -> Result<(), DataError> {
        write_packet(&mut self.stream, &mut self.sequence, payload)
    }
}

struct Prepared {
    id: u32,
    parameter_count: u16,
}

struct Column {
    name: String,
    column_type: u8,
    unsigned: bool,
}

fn parse_handshake(packet: &[u8]) -> Result<(u32, Vec<u8>, String), DataError> {
    let mut reader = PacketReader::new(packet);

    let protocol = reader.byte()?;
    if protocol != 10 {
        return Err(DataError::protocol(format!(
            "unsupported handshake protocol version {protocol}"
        )));
    }

    let _server_version = reader.null_terminated()?;
    let _connection_id = reader.u32_le()?;
    let first_part = reader.take(8)?;
    let _filler = reader.byte()?;
    let capabilities_lower = u32::from(reader.u16_le()?);

    let mut capabilities = capabilities_lower;
    let mut nonce = first_part.to_vec();
    let mut plugin = String::from("mysql_native_password");

    if reader.remaining() > 0 {
        let _charset = reader.byte()?;
        let _status = reader.u16_le()?;
        let capabilities_upper = u32::from(reader.u16_le()?) << 16;
        capabilities |= capabilities_upper;
        let plugin_length = usize::from(reader.byte()?);
        let _reserved = reader.take(10)?;

        let part_length = plugin_length.saturating_sub(8).max(13);
        let available = reader.remaining();
        let part = reader.take(part_length.min(available))?;
        let part = match part.last() {
            Some(0) => &part[..part.len() - 1],
            _ => part,
        };
        nonce.extend_from_slice(part);

        if capabilities & CLIENT_PLUGIN_AUTH != 0 && !reader.is_empty() {
            plugin = String::from_utf8_lossy(reader.null_terminated()?).into_owned();
        }
    }

    if nonce.len() < 20 {
        return Err(DataError::protocol("handshake nonce is too short"));
    }
    nonce.truncate(20);

    Ok((capabilities, nonce, plugin))
}

fn parse_column(packet: &[u8]) -> Result<Column, DataError> {
    let mut reader = PacketReader::new(packet);
    let _catalog = reader.lenenc_bytes()?;
    let _schema = reader.lenenc_bytes()?;
    let _table = reader.lenenc_bytes()?;
    let _original_table = reader.lenenc_bytes()?;
    let name = reader
        .lenenc_bytes()?
        .ok_or_else(|| DataError::protocol("column definition has no name"))?;
    let _original_name = reader.lenenc_bytes()?;
    let _fixed_length = reader.lenenc_int()?;
    let _character_set = reader.take(2)?;
    let _column_length = reader.take(4)?;
    let column_type = reader.byte()?;
    // The flags field (2 bytes) follows the type; bit 0x0020 is UNSIGNED.
    // Read it leniently so definition packets without the trailing fields
    // keep parsing as before.
    let flags = reader
        .take(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .unwrap_or(0);

    Ok(Column {
        name: String::from_utf8_lossy(name).into_owned(),
        column_type,
        unsigned: flags & COLUMN_FLAG_UNSIGNED != 0,
    })
}

fn is_eof(packet: &[u8]) -> bool {
    packet.first() == Some(&0xFE) && packet.len() < 9
}

fn parse_ok(packet: &[u8]) -> Result<ExecOutcome, DataError> {
    let mut reader = PacketReader::new(packet);
    let _header = reader.byte()?;
    let affected_rows = reader.lenenc_int()?.unwrap_or(0);
    let last_insert_id = reader.lenenc_int()?.unwrap_or(0);

    Ok(ExecOutcome {
        affected_rows,
        last_insert_id: (last_insert_id != 0).then_some(last_insert_id),
    })
}

fn server_error(packet: &[u8]) -> DataError {
    let mut reader = PacketReader::new(packet);
    let _ = reader.byte();
    let code = reader.u16_le().unwrap_or(0);
    let mut state = String::new();

    if packet.get(reader.position()) == Some(&b'#') {
        let _ = reader.byte();
        if let Ok(bytes) = reader.take(5) {
            state = String::from_utf8_lossy(bytes).into_owned();
        }
    }

    DataError::Server {
        code: u32::from(code),
        state,
        message: String::from_utf8_lossy(reader.rest()).into_owned(),
    }
}

fn decode_text_row(packet: &[u8], columns: &[Column]) -> Result<Object, DataError> {
    let mut reader = PacketReader::new(packet);
    let mut row = Object::new();

    for column in columns {
        let value = match reader.lenenc_bytes()? {
            Some(bytes) => text_value(column, bytes)?,
            None => Value::Null,
        };
        row.insert(column.name.clone(), value);
    }

    Ok(row)
}

fn decode_binary_row(packet: &[u8], columns: &[Column]) -> Result<Object, DataError> {
    let mut reader = PacketReader::new(packet);
    if reader.byte()? != 0x00 {
        return Err(DataError::protocol("unexpected binary row header"));
    }

    let bitmap_length = (columns.len() + 2).div_ceil(8);
    let bitmap = reader.take(bitmap_length)?;
    let mut row = Object::new();

    for (index, column) in columns.iter().enumerate() {
        let bit = index + 2;
        let is_null = bitmap[bit / 8] & (1 << (bit % 8)) != 0;
        let value = if is_null {
            Value::Null
        } else {
            binary_value(&mut reader, column)?
        };
        row.insert(column.name.clone(), value);
    }

    Ok(row)
}

fn text_value(column: &Column, bytes: &[u8]) -> Result<Value, DataError> {
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|error| DataError::Decode(format!("invalid UTF-8 in MariaDB text: {error}")))?;

    match column.column_type {
        MYSQL_TYPE_TINY | MYSQL_TYPE_SHORT | MYSQL_TYPE_LONG | MYSQL_TYPE_LONGLONG
        | MYSQL_TYPE_INT24 | MYSQL_TYPE_YEAR => {
            // BIGINT UNSIGNED text can exceed i64, so decode it as u64.
            if column.column_type == MYSQL_TYPE_LONGLONG && column.unsigned {
                let value = text.parse::<u64>().map_err(|_| {
                    DataError::Decode(format!("invalid unsigned integer column value `{text}`"))
                })?;
                return Ok(Value::from(value));
            }
            let value = text
                .parse::<i64>()
                .map_err(|_| DataError::Decode(format!("invalid integer column value `{text}`")))?;
            Ok(Value::from(value))
        }
        MYSQL_TYPE_FLOAT | MYSQL_TYPE_DOUBLE => {
            let value = text
                .parse::<f64>()
                .map_err(|_| DataError::Decode(format!("invalid float column value `{text}`")))?;
            Ok(Value::from(value))
        }
        _ => Ok(Value::String(text)),
    }
}

fn binary_value(reader: &mut PacketReader<'_>, column: &Column) -> Result<Value, DataError> {
    Ok(match column.column_type {
        MYSQL_TYPE_TINY => Value::from(i64::from(reader.byte()? as i8)),
        MYSQL_TYPE_SHORT => {
            let bytes = reader.take(2)?;
            Value::from(i64::from(i16::from_le_bytes([bytes[0], bytes[1]])))
        }
        MYSQL_TYPE_YEAR => {
            let bytes = reader.take(2)?;
            Value::from(i64::from(u16::from_le_bytes([bytes[0], bytes[1]])))
        }
        MYSQL_TYPE_LONG | MYSQL_TYPE_INT24 => {
            let bytes = reader.take(4)?;
            Value::from(i64::from(i32::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3],
            ])))
        }
        MYSQL_TYPE_LONGLONG => {
            let bytes = reader.take(8)?;
            if column.unsigned {
                // BIGINT UNSIGNED values above i64::MAX must not wrap negative;
                // Number::from_u64 keeps them exact and lib.rs maps them to
                // the entity-level Value::UInt.
                Value::from(u64::from_le_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]))
            } else {
                Value::from(i64::from_le_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]))
            }
        }
        MYSQL_TYPE_FLOAT => {
            let bytes = reader.take(4)?;
            Value::from(f64::from(f32::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3],
            ])))
        }
        MYSQL_TYPE_DOUBLE => {
            let bytes = reader.take(8)?;
            Value::from(f64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]))
        }
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE | MYSQL_TYPE_DATETIME | MYSQL_TYPE_TIMESTAMP => {
            Value::String(read_datetime(reader)?)
        }
        MYSQL_TYPE_TIME => Value::String(read_time(reader)?),
        MYSQL_TYPE_NEWDECIMAL => {
            let bytes = reader.lenenc_bytes()?.unwrap_or(&[]);
            Value::String(String::from_utf8_lossy(bytes).into_owned())
        }
        _ => {
            let bytes = reader.lenenc_bytes()?.unwrap_or(&[]);
            Value::String(String::from_utf8_lossy(bytes).into_owned())
        }
    })
}

fn read_datetime(reader: &mut PacketReader<'_>) -> Result<String, DataError> {
    let length = usize::from(reader.byte()?);

    Ok(match length {
        0 => "0000-00-00 00:00:00".to_owned(),
        4 => {
            let bytes = reader.take(4)?;
            format!(
                "{:04}-{:02}-{:02}",
                u16::from_le_bytes([bytes[0], bytes[1]]),
                bytes[2],
                bytes[3]
            )
        }
        7 => {
            let bytes = reader.take(7)?;
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                u16::from_le_bytes([bytes[0], bytes[1]]),
                bytes[2],
                bytes[3],
                bytes[4],
                bytes[5],
                bytes[6]
            )
        }
        11 => {
            let bytes = reader.take(11)?;
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:06}",
                u16::from_le_bytes([bytes[0], bytes[1]]),
                bytes[2],
                bytes[3],
                bytes[4],
                bytes[5],
                bytes[6],
                u32::from_le_bytes([bytes[7], bytes[8], bytes[9], bytes[10]])
            )
        }
        other => {
            return Err(DataError::protocol(format!(
                "unexpected datetime length {other}"
            )));
        }
    })
}

fn read_time(reader: &mut PacketReader<'_>) -> Result<String, DataError> {
    let length = usize::from(reader.byte()?);

    Ok(match length {
        0 => "00:00:00".to_owned(),
        8 | 12 => {
            let bytes = reader.take(length)?;
            let negative = bytes[0] == 1;
            let days = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            let hours = u64::from(days) * 24 + u64::from(bytes[5]);
            let mut text = format!(
                "{}{:02}:{:02}:{:02}",
                if negative { "-" } else { "" },
                hours,
                bytes[6],
                bytes[7]
            );
            if length == 12 {
                let micros = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
                text.push_str(&format!(".{micros:06}"));
            }
            text
        }
        other => {
            return Err(DataError::protocol(format!(
                "unexpected time length {other}"
            )));
        }
    })
}

fn binary_type(value: &Value) -> u8 {
    match value {
        Value::Null => MYSQL_TYPE_NULL,
        Value::Bool(_) => MYSQL_TYPE_TINY,
        Value::Number(number) if number.is_f64() => MYSQL_TYPE_DOUBLE,
        Value::Number(_) => MYSQL_TYPE_LONGLONG,
        _ => MYSQL_TYPE_VAR_STRING,
    }
}

fn unsigned_flag(value: &Value) -> u8 {
    match value {
        Value::Number(number) if number.is_u64() => 0x80,
        _ => 0,
    }
}

fn push_binary_value(buffer: &mut Vec<u8>, value: &Value) -> Result<(), DataError> {
    match value {
        Value::Null => {}
        Value::Bool(flag) => buffer.push(u8::from(*flag)),
        Value::Number(number) => {
            if let Some(number) = number.as_i64() {
                buffer.extend_from_slice(&number.to_le_bytes());
            } else if let Some(number) = number.as_u64() {
                buffer.extend_from_slice(&number.to_le_bytes());
            } else {
                buffer.extend_from_slice(&number.as_f64().to_le_bytes());
            }
        }
        Value::String(text) => push_lenenc_bytes(buffer, text.as_bytes()),
        other => {
            let text = zhs_json::to_string(other);
            push_lenenc_bytes(buffer, text.as_bytes());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        binary_value, parse_column, text_value, Column, COLUMN_FLAG_UNSIGNED, MYSQL_TYPE_LONGLONG,
        MYSQL_TYPE_LONG, MYSQL_TYPE_TIME,
    };
    use crate::packet::{push_lenenc_bytes, PacketReader};

    fn column_definition(name: &str, column_type: u8, flags: u16) -> Vec<u8> {
        let mut packet = Vec::new();
        push_lenenc_bytes(&mut packet, b"def");
        push_lenenc_bytes(&mut packet, b"");
        push_lenenc_bytes(&mut packet, b"");
        push_lenenc_bytes(&mut packet, b"");
        push_lenenc_bytes(&mut packet, name.as_bytes());
        push_lenenc_bytes(&mut packet, b"");
        packet.push(0x0c);
        packet.extend_from_slice(&45_u16.to_le_bytes());
        packet.extend_from_slice(&20_u32.to_le_bytes());
        packet.push(column_type);
        packet.extend_from_slice(&flags.to_le_bytes());
        packet.push(0);
        packet.extend_from_slice(&[0, 0]);
        packet
    }

    #[test]
    fn parses_the_unsigned_column_flag() {
        let unsigned =
            parse_column(&column_definition("id", MYSQL_TYPE_LONGLONG, COLUMN_FLAG_UNSIGNED))
                .unwrap();
        assert_eq!(unsigned.name, "id");
        assert_eq!(unsigned.column_type, MYSQL_TYPE_LONGLONG);
        assert!(unsigned.unsigned);

        let signed = parse_column(&column_definition("id", MYSQL_TYPE_LONGLONG, 0)).unwrap();
        assert!(!signed.unsigned);

        // Other flag bits (NOT_NULL | PRIMARY_KEY) must not imply unsigned.
        let other = parse_column(&column_definition("id", MYSQL_TYPE_LONGLONG, 0x0002 | 0x0004))
            .unwrap();
        assert!(!other.unsigned);

        // Missing trailing fields keep parsing and default to signed.
        let mut truncated = column_definition("id", MYSQL_TYPE_LONGLONG, COLUMN_FLAG_UNSIGNED);
        truncated.truncate(truncated.len() - 5);
        assert!(!parse_column(&truncated).unwrap().unsigned);
    }

    #[test]
    fn decodes_unsigned_bigint_binary_values_as_u64() {
        let payload = u64::MAX.to_le_bytes();

        let unsigned = Column {
            name: "id".to_owned(),
            column_type: MYSQL_TYPE_LONGLONG,
            unsigned: true,
        };
        let value = binary_value(&mut PacketReader::new(&payload), &unsigned).unwrap();
        assert_eq!(value.as_u64(), Some(u64::MAX));
        assert_eq!(value.as_i64(), None);

        let signed = Column {
            name: "id".to_owned(),
            column_type: MYSQL_TYPE_LONGLONG,
            unsigned: false,
        };
        let value = binary_value(&mut PacketReader::new(&payload), &signed).unwrap();
        assert_eq!(value.as_i64(), Some(-1));

        // Signed INT decoding is unchanged by the flag plumbing.
        let int_column = Column {
            name: "n".to_owned(),
            column_type: MYSQL_TYPE_LONG,
            unsigned: true,
        };
        let value = binary_value(&mut PacketReader::new(&(-5_i32).to_le_bytes()), &int_column)
            .unwrap();
        assert_eq!(value.as_i64(), Some(-5));
    }

    #[test]
    fn decodes_unsigned_bigint_text_values_as_u64() {
        let max_text = u64::MAX.to_string();

        let unsigned = Column {
            name: "id".to_owned(),
            column_type: MYSQL_TYPE_LONGLONG,
            unsigned: true,
        };
        let value = text_value(&unsigned, max_text.as_bytes()).unwrap();
        assert_eq!(value.as_u64(), Some(u64::MAX));

        let value = text_value(&unsigned, b"42").unwrap();
        assert_eq!(value.as_i64(), Some(42));

        let signed = Column {
            name: "id".to_owned(),
            column_type: MYSQL_TYPE_LONGLONG,
            unsigned: false,
        };
        let value = text_value(&signed, b"-42").unwrap();
        assert_eq!(value.as_i64(), Some(-42));
        assert!(text_value(&signed, max_text.as_bytes()).is_err());
    }

    #[test]
    fn decodes_time_values_with_huge_day_counts_without_overflow() {
        let mut body = Vec::new();
        body.push(8); // length
        body.push(0); // not negative
        body.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes()); // days
        body.extend_from_slice(&[0, 0, 0, 0]); // hours, minutes, seconds

        let column = Column {
            name: "t".to_owned(),
            column_type: MYSQL_TYPE_TIME,
            unsigned: false,
        };
        let value = binary_value(&mut PacketReader::new(&body), &column).unwrap();
        assert_eq!(value.as_str(), Some("103079215080:00:00"));
    }
}
