#![allow(unsafe_code)]

use std::ptr;
use std::slice;
use std::sync::Arc;

use zhs_entity::DataError;
use zhs_entity::WriteOutcome;
use zhs_json::{Object, Value};

use crate::backend::ConnectOptions;
use crate::ffi::{
    self, Environment, Handle, OCI_ATTR_DATA_SIZE, OCI_ATTR_DATA_TYPE, OCI_ATTR_NAME,
    OCI_ATTR_PARAM_COUNT, OCI_ATTR_ROW_COUNT, OCI_ATTR_STMT_TYPE, OCI_DEFAULT, OCI_FETCH_NEXT,
    OCI_HTYPE_ERROR, OCI_NO_DATA, OCI_NTV_SYNTAX, OCI_STMT_SELECT, SQLT_CHR,
};

pub(crate) struct Connection {
    environment: Arc<Environment>,
    error: Handle,
    service: Handle,
}

unsafe impl Send for Connection {}

impl Connection {
    pub(crate) fn open(
        environment: Arc<Environment>,
        options: &ConnectOptions,
    ) -> Result<Self, DataError> {
        let error = environment.allocate(OCI_HTYPE_ERROR)?;
        let mut service: Handle = ptr::null_mut();

        let user = options.user.as_bytes();
        let password = options.password.as_bytes();
        let database = options.database_name().into_bytes();
        let functions = environment.oci().functions();

        let status = unsafe {
            (functions.logon2)(
                environment.handle(),
                error,
                &mut service,
                user.as_ptr(),
                user.len() as u32,
                password.as_ptr(),
                password.len() as u32,
                database.as_ptr(),
                database.len() as u32,
                OCI_DEFAULT,
            )
        };

        if status != ffi::OCI_SUCCESS {
            let cause = environment.error(error, status);
            environment.free(error, OCI_HTYPE_ERROR);
            return Err(cause);
        }

        let connection = Self {
            environment,
            error,
            service,
        };

        for statement in [
            "ALTER SESSION SET NLS_DATE_FORMAT = 'YYYY-MM-DD\"T\"HH24:MI:SS'",
            "ALTER SESSION SET NLS_TIMESTAMP_FORMAT = 'YYYY-MM-DD\"T\"HH24:MI:SS.FF6'",
            "ALTER SESSION SET NLS_TIMESTAMP_TZ_FORMAT = 'YYYY-MM-DD\"T\"HH24:MI:SS.FF6 TZH:TZM'",
        ] {
            let _ = connection.execute(statement, &[], false);
        }

        Ok(connection)
    }

    pub fn fetch(
        &self,
        sql: &str,
        params: &[Value],
        commit: bool,
    ) -> Result<Vec<Object>, DataError> {
        let statement = self.prepare(sql)?;
        let statement_type = self.statement_type(&statement)?;

        if statement_type != OCI_STMT_SELECT {
            self.execute_dml(&statement, params, commit)?;
            return Ok(Vec::new());
        }

        let binds = self.bind(&statement, params)?;

        self.execute_statement(&statement, 0)?;
        let mut columns = self.define(&statement)?;

        let mut rows = Vec::new();
        loop {
            let status = unsafe {
                (self.environment.oci().functions().stmt_fetch2)(
                    statement.handle,
                    self.error,
                    1,
                    OCI_FETCH_NEXT,
                    0,
                    OCI_DEFAULT,
                )
            };

            if status == OCI_NO_DATA {
                break;
            }
            if status != ffi::OCI_SUCCESS && status != ffi::OCI_SUCCESS_WITH_INFO {
                return Err(self.environment.error(self.error, status));
            }

            rows.push(self.read_row(&mut columns)?);
        }

        drop(binds);

        drop(statement);
        Ok(rows)
    }

    pub fn execute(&self, sql: &str, params: &[Value], commit: bool) -> Result<u64, DataError> {
        let statement = self.prepare(sql)?;
        self.execute_dml(&statement, params, commit)
    }

    pub fn insert(
        &self,
        sql: &str,
        params: &[Value],
        identity: Option<&str>,
        commit: bool,
    ) -> Result<WriteOutcome, DataError> {
        let Some(identity) = identity else {
            let affected_rows = self.execute(sql, params, commit)?;
            return Ok(WriteOutcome {
                affected_rows,
                last_insert_id: None,
            });
        };

        let returning = crate::render::quote_identifier(identity)?;
        let sql = format!("{sql} RETURNING {returning} INTO :{}", params.len() + 1);
        let statement = self.prepare(&sql)?;
        let _binds = self.bind(&statement, params)?;

        let mut output = vec![0u8; 64];
        let mut indicator: i16 = 0;
        let mut length: u16 = 0;
        let mut bind_handle: Handle = ptr::null_mut();

        let status = unsafe {
            (self.environment.oci().functions().bind_by_pos)(
                statement.handle,
                &mut bind_handle,
                self.error,
                (params.len() + 1) as u32,
                output.as_mut_ptr().cast(),
                output.len() as i32,
                SQLT_CHR,
                (&mut indicator as *mut i16).cast(),
                &mut length,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                OCI_DEFAULT,
            )
        };
        self.environment.check(status, self.error)?;
        self.execute_statement(&statement, 1)?;

        let mut affected: u32 = 0;
        let mut size: u32 = 0;

        let status = unsafe {
            (self.environment.oci().functions().attr_get)(
                statement.handle,
                ffi::OCI_HTYPE_STMT,
                (&mut affected as *mut u32).cast(),
                &mut size,
                OCI_ATTR_ROW_COUNT,
                self.error,
            )
        };
        self.environment.check(status, self.error)?;
        if commit {
            self.commit()?;
        }

        let last_insert_id = if indicator < 0 {
            None
        } else {
            let length = usize::from(length).min(output.len());
            std::str::from_utf8(&output[..length])
                .ok()
                .and_then(|text| text.trim().parse::<u64>().ok())
        };

        Ok(WriteOutcome {
            affected_rows: u64::from(affected),
            last_insert_id,
        })
    }

    pub fn commit(&self) -> Result<(), DataError> {
        let functions = self.environment.oci().functions();

        let status = unsafe { (functions.trans_commit)(self.service, self.error, OCI_DEFAULT) };
        self.environment.check(status, self.error)
    }

    pub fn rollback(&self) -> Result<(), DataError> {
        let functions = self.environment.oci().functions();

        let status = unsafe { (functions.trans_rollback)(self.service, self.error, OCI_DEFAULT) };
        self.environment.check(status, self.error)
    }

    fn prepare(&self, sql: &str) -> Result<Statement, DataError> {
        let functions = self.environment.oci().functions();
        let handle = self.environment.allocate(ffi::OCI_HTYPE_STMT)?;

        let status = unsafe {
            (functions.stmt_prepare)(
                handle,
                self.error,
                sql.as_ptr(),
                sql.len() as u32,
                OCI_NTV_SYNTAX,
                OCI_DEFAULT,
            )
        };
        if status != ffi::OCI_SUCCESS && status != ffi::OCI_SUCCESS_WITH_INFO {
            let error = self.environment.error(self.error, status);
            self.environment.free(handle, ffi::OCI_HTYPE_STMT);
            return Err(error);
        }
        Ok(Statement {
            environment: Arc::clone(&self.environment),
            handle,
        })
    }

    fn statement_type(&self, statement: &Statement) -> Result<u16, DataError> {
        let mut statement_type: u16 = 0;
        let mut size: u32 = 0;

        let status = unsafe {
            (self.environment.oci().functions().attr_get)(
                statement.handle,
                ffi::OCI_HTYPE_STMT,
                (&mut statement_type as *mut u16).cast(),
                &mut size,
                OCI_ATTR_STMT_TYPE,
                self.error,
            )
        };
        self.environment.check(status, self.error)?;
        Ok(statement_type)
    }

    fn bind(&self, statement: &Statement, params: &[Value]) -> Result<Vec<BindBuffer>, DataError> {
        let mut buffers = params
            .iter()
            .map(BindBuffer::encode)
            .collect::<Result<Vec<_>, _>>()?;

        let functions = self.environment.oci().functions();
        for (index, buffer) in buffers.iter_mut().enumerate() {
            let mut bind_handle: Handle = ptr::null_mut();

            let status = unsafe {
                (functions.bind_by_pos)(
                    statement.handle,
                    &mut bind_handle,
                    self.error,
                    (index + 1) as u32,
                    buffer.bytes.as_mut_ptr().cast(),
                    buffer.bytes.len().max(1) as i32,
                    SQLT_CHR,
                    (&mut buffer.indicator as *mut i16).cast(),
                    &mut buffer.length,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    OCI_DEFAULT,
                )
            };
            self.environment.check(status, self.error)?;
        }

        Ok(buffers)
    }

    fn define(&self, statement: &Statement) -> Result<Vec<ColumnBuffer>, DataError> {
        let functions = self.environment.oci().functions();
        let mut count: u32 = 0;
        let mut size: u32 = 0;

        let status = unsafe {
            (functions.attr_get)(
                statement.handle,
                ffi::OCI_HTYPE_STMT,
                (&mut count as *mut u32).cast(),
                &mut size,
                OCI_ATTR_PARAM_COUNT,
                self.error,
            )
        };
        self.environment.check(status, self.error)?;
        if count == 0 {
            return Err(DataError::protocol(
                "the Oracle statement has no result columns",
            ));
        }

        let mut metas = Vec::with_capacity(count as usize);
        for position in 1..=count {
            metas.push(self.column_metadata(statement, position)?);
        }

        let mut columns = Vec::with_capacity(count as usize);
        for meta in metas {
            columns.push(self.new_column(meta)?);
        }

        for (index, column) in columns.iter_mut().enumerate() {
            self.define_column(statement, (index + 1) as u32, column)?;
        }

        Ok(columns)
    }

    fn new_column(&self, meta: ColumnMeta) -> Result<ColumnBuffer, DataError> {
        let storage = if ffi::is_lob_type(meta.data_type) {
            ColumnStorage::Lob {
                locator: self.environment.descriptor(ffi::OCI_DTYPE_LOB)?,
                data_type: meta.data_type,
            }
        } else if meta.data_type == ffi::SQLT_BIN {
            ColumnStorage::Raw {
                // One spare byte so a value exactly filling the declared
                // size is distinguishable from a truncated fetch.
                buffer: vec![0u8; usize::from(meta.data_size).saturating_add(1)],
            }
        } else {
            let numeric = ffi::is_numeric_type(meta.data_type);
            let capacity = if numeric {
                256
            } else {
                (meta.data_size as usize)
                    .saturating_add(1)
                    .clamp(64, 32_768)
            };
            ColumnStorage::Text {
                buffer: vec![0u8; capacity],
                numeric,
            }
        };

        Ok(ColumnBuffer {
            name: meta.name,
            declared_size: meta.data_size,
            storage,
            indicator: 0,
            length: 0,
            environment: Arc::clone(&self.environment),
        })
    }

    fn define_column(
        &self,
        statement: &Statement,
        position: u32,
        column: &mut ColumnBuffer,
    ) -> Result<(), DataError> {
        let functions = self.environment.oci().functions();
        let mut define_handle: Handle = ptr::null_mut();

        let (value, size, data_type, length) = match &mut column.storage {
            ColumnStorage::Text { buffer, .. } => (
                buffer.as_mut_ptr().cast(),
                buffer.len() as i32,
                SQLT_CHR,
                &mut column.length as *mut u16,
            ),
            ColumnStorage::Raw { buffer } => (
                buffer.as_mut_ptr().cast(),
                buffer.len() as i32,
                ffi::SQLT_BIN,
                &mut column.length as *mut u16,
            ),
            ColumnStorage::Lob { locator, data_type } => (
                (locator as *mut Handle).cast(),
                0,
                *data_type,
                ptr::null_mut(),
            ),
        };

        let status = unsafe {
            (functions.define_by_pos)(
                statement.handle,
                &mut define_handle,
                self.error,
                position,
                value,
                size,
                data_type,
                (&mut column.indicator as *mut i16).cast(),
                length,
                ptr::null_mut(),
                OCI_DEFAULT,
            )
        };
        self.environment.check(status, self.error)
    }

    fn column_metadata(
        &self,
        statement: &Statement,
        position: u32,
    ) -> Result<ColumnMeta, DataError> {
        let functions = self.environment.oci().functions();
        let mut parameter: Handle = ptr::null_mut();

        let status = unsafe {
            (functions.param_get)(
                statement.handle,
                ffi::OCI_HTYPE_STMT,
                self.error,
                &mut parameter,
                position,
            )
        };
        self.environment.check(status, self.error)?;
        let _parameter = DescriptorGuard::new(Arc::clone(&self.environment), parameter);

        let mut data_type: u16 = 0;
        let mut size: u32 = 0;

        let status = unsafe {
            (functions.attr_get)(
                parameter,
                ffi::OCI_DTYPE_PARAM,
                (&mut data_type as *mut u16).cast(),
                &mut size,
                OCI_ATTR_DATA_TYPE,
                self.error,
            )
        };
        self.environment.check(status, self.error)?;

        let mut data_size: u16 = 0;

        let status = unsafe {
            (functions.attr_get)(
                parameter,
                ffi::OCI_DTYPE_PARAM,
                (&mut data_size as *mut u16).cast(),
                &mut size,
                OCI_ATTR_DATA_SIZE,
                self.error,
            )
        };
        self.environment.check(status, self.error)?;

        let mut name: *const u8 = ptr::null();
        let mut name_length: u32 = 0;

        let status = unsafe {
            (functions.attr_get)(
                parameter,
                ffi::OCI_DTYPE_PARAM,
                (&mut name as *mut *const u8).cast(),
                &mut name_length,
                OCI_ATTR_NAME,
                self.error,
            )
        };
        self.environment.check(status, self.error)?;

        let name = if name.is_null() || name_length == 0 {
            format!("column_{position}")
        } else {
            String::from_utf8_lossy(unsafe { slice::from_raw_parts(name, name_length as usize) })
                .to_ascii_lowercase()
        };

        Ok(ColumnMeta {
            name,
            data_type,
            data_size,
        })
    }

    fn execute_dml(
        &self,
        statement: &Statement,
        params: &[Value],
        commit: bool,
    ) -> Result<u64, DataError> {
        let _binds = self.bind(statement, params)?;
        self.execute_statement(statement, 1)?;

        let mut affected: u32 = 0;
        let mut size: u32 = 0;

        let status = unsafe {
            (self.environment.oci().functions().attr_get)(
                statement.handle,
                ffi::OCI_HTYPE_STMT,
                (&mut affected as *mut u32).cast(),
                &mut size,
                OCI_ATTR_ROW_COUNT,
                self.error,
            )
        };
        self.environment.check(status, self.error)?;

        if commit {
            self.commit()?;
        }

        Ok(u64::from(affected))
    }

    fn execute_statement(&self, statement: &Statement, iterations: u32) -> Result<(), DataError> {
        let functions = self.environment.oci().functions();

        let status = unsafe {
            (functions.stmt_execute)(
                self.service,
                statement.handle,
                self.error,
                iterations,
                0,
                ptr::null(),
                ptr::null_mut(),
                OCI_DEFAULT,
            )
        };
        self.environment.check(status, self.error)
    }

    fn read_row(&self, columns: &mut [ColumnBuffer]) -> Result<Object, DataError> {
        let mut row = Object::new();
        for column in columns {
            let value = if column.indicator < 0 {
                Value::Null
            } else {
                match &mut column.storage {
                    ColumnStorage::Text { buffer, numeric } => {
                        let length = usize::from(column.length);
                        // OCI reports a truncated fetch through a positive
                        // indicator (the true value length) or by filling the
                        // whole define buffer; fail instead of returning
                        // silently truncated text.
                        if column.indicator > 0 || length >= buffer.len() {
                            return Err(truncation_error(
                                &column.name,
                                length,
                                buffer.len(),
                                column.declared_size,
                            ));
                        }
                        let text = std::str::from_utf8(&buffer[..length])
                            .map_err(|error| {
                                DataError::Decode(format!("invalid UTF-8 in Oracle text: {error}"))
                            })?
                            .to_owned();
                        if *numeric {
                            parse_number(&text)
                        } else {
                            Value::String(text)
                        }
                    }
                    ColumnStorage::Raw { buffer } => {
                        let length = usize::from(column.length);
                        // The buffer holds one byte more than the declared
                        // size, so filling it means the value was truncated.
                        if column.indicator > 0 || length >= buffer.len() {
                            return Err(truncation_error(
                                &column.name,
                                length,
                                buffer.len(),
                                column.declared_size,
                            ));
                        }
                        bytes_to_value(&buffer[..length])
                    }
                    ColumnStorage::Lob { locator, data_type } => {
                        self.read_lob(*locator, *data_type)?
                    }
                }
            };
            row.insert(column.name.clone(), value);
        }
        Ok(row)
    }

    fn read_lob(&self, locator: Handle, data_type: u16) -> Result<Value, DataError> {
        const MAX_LOB_BYTES: u32 = 16 * 1024 * 1024;

        let functions = self.environment.oci().functions();
        let mut length: u32 = 0;

        let status =
            unsafe { (functions.lob_get_length)(self.service, self.error, locator, &mut length) };
        self.environment.check(status, self.error)?;

        if length == 0 {
            return Ok(if data_type == ffi::SQLT_CLOB {
                Value::String(String::new())
            } else {
                bytes_to_value(&[])
            });
        }
        if length > MAX_LOB_BYTES {
            return Err(DataError::Unsupported(
                "Oracle LOBs larger than 16 MiB are not supported yet",
            ));
        }

        let character_length = u64::from(length);
        let capacity = if data_type == ffi::SQLT_CLOB {
            (length as usize).saturating_mul(4).saturating_add(4)
        } else {
            length as usize
        };
        let mut buffer = vec![0u8; capacity];
        let mut byte_amount = capacity as u64;
        let mut char_amount = character_length;

        let status = unsafe {
            (functions.lob_read2)(
                self.service,
                self.error,
                locator,
                &mut byte_amount,
                &mut char_amount,
                1,
                buffer.as_mut_ptr().cast(),
                capacity as u64,
                ffi::OCI_ONE_PIECE,
                ptr::null_mut(),
                ptr::null(),
                0,
                ffi::SQLCS_IMPLICIT,
            )
        };
        self.environment.check(status, self.error)?;

        let read = usize::try_from(byte_amount)
            .unwrap_or(buffer.len())
            .min(buffer.len());
        if data_type == ffi::SQLT_CLOB {
            Ok(Value::String(
                std::str::from_utf8(&buffer[..read])
                    .map_err(|error| {
                        DataError::Decode(format!("invalid UTF-8 in Oracle CLOB: {error}"))
                    })?
                    .to_owned(),
            ))
        } else {
            Ok(bytes_to_value(&buffer[..read]))
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.service.is_null() {
            let functions = self.environment.oci().functions();

            unsafe {
                let _ = (functions.logoff)(self.service, self.error);
            }
        }
        self.environment.free(self.error, OCI_HTYPE_ERROR);
    }
}

struct Statement {
    environment: Arc<Environment>,
    handle: Handle,
}

impl Drop for Statement {
    fn drop(&mut self) {
        self.environment.free(self.handle, ffi::OCI_HTYPE_STMT);
    }
}

struct BindBuffer {
    bytes: Vec<u8>,
    indicator: i16,
    length: u16,
}

impl BindBuffer {
    fn encode(value: &Value) -> Result<Self, DataError> {
        let text = match value {
            Value::Null => {
                return Ok(Self {
                    bytes: vec![0],
                    indicator: -1,
                    length: 0,
                });
            }
            Value::Bool(value) => {
                if *value {
                    "1".to_owned()
                } else {
                    "0".to_owned()
                }
            }
            Value::Number(number) => {
                if let Some(value) = number.as_i64() {
                    value.to_string()
                } else if let Some(value) = number.as_u64() {
                    value.to_string()
                } else {
                    format_number(number.as_f64())?
                }
            }
            Value::String(value) => value.clone(),
            Value::Array(_) | Value::Object(_) => {
                return Err(DataError::Unsupported(
                    "Oracle binds only support scalar JSON values",
                ));
            }
        };

        let mut bytes = text.into_bytes();
        if bytes.len() > 32_767 {
            return Err(DataError::query(
                "the bound value exceeds Oracle's 32767-byte VARCHAR2 limit",
            ));
        }
        let length = bytes.len() as u16;
        bytes.push(0);
        Ok(Self {
            bytes,
            indicator: 0,
            length,
        })
    }
}

struct ColumnMeta {
    name: String,
    data_type: u16,
    data_size: u16,
}

enum ColumnStorage {
    Text { buffer: Vec<u8>, numeric: bool },
    Raw { buffer: Vec<u8> },
    Lob { locator: Handle, data_type: u16 },
}

struct ColumnBuffer {
    name: String,
    declared_size: u16,
    storage: ColumnStorage,
    indicator: i16,
    length: u16,
    environment: Arc<Environment>,
}

impl Drop for ColumnBuffer {
    fn drop(&mut self) {
        if let ColumnStorage::Lob { locator, .. } = &self.storage {
            self.environment
                .free_descriptor(*locator, ffi::OCI_DTYPE_LOB);
        }
    }
}

struct DescriptorGuard {
    environment: Arc<Environment>,
    handle: Handle,
}

impl DescriptorGuard {
    fn new(environment: Arc<Environment>, handle: Handle) -> Self {
        Self { environment, handle }
    }
}

impl Drop for DescriptorGuard {
    fn drop(&mut self) {
        self.environment
            .free_descriptor(self.handle, ffi::OCI_DTYPE_PARAM);
    }
}

fn truncation_error(column: &str, fetched: usize, capacity: usize, declared: u16) -> DataError {
    DataError::decode(format!(
        "the value of Oracle column `{column}` was truncated: the fetch reported {fetched} bytes \
         for a {capacity}-byte define buffer (declared data size {declared})"
    ))
}

fn bytes_to_value(bytes: &[u8]) -> Value {
    Value::Array(
        bytes
            .iter()
            .map(|byte| Value::from(i64::from(*byte)))
            .collect(),
    )
}

/// Renders a floating-point bind value as plain decimal text.
///
/// Bind values are sent as `SQLT_CHR`, so Oracle parses them through its
/// implicit string→NUMBER conversion, which never accepts scientific
/// notation, `inf`, or `NaN`. Integral floats render as integers; magnitudes
/// that cannot be represented as a sane decimal literal within Oracle
/// NUMBER's usable range (`>= 1e15`, or non-zero `< 1e-15`) are rejected
/// with a protocol error instead of being converted incorrectly.
fn format_number(value: f64) -> Result<String, DataError> {
    if !value.is_finite() {
        return Err(DataError::protocol(
            "Oracle NUMBER cannot represent NaN or infinity",
        ));
    }

    let magnitude = value.abs();
    if magnitude >= 1e15 || (magnitude != 0.0 && magnitude < 1e-15) {
        return Err(DataError::protocol(format!(
            "the floating-point value {value:e} is outside the decimal range Oracle NUMBER binds support"
        )));
    }

    if value == value.trunc() {
        return Ok(format!("{value:.0}"));
    }

    let text = format!("{value:.10}");
    let trimmed = text.trim_end_matches('0');
    if trimmed.ends_with('.') {
        // The fixed-precision form hid every fractional digit (e.g. 1e-15
        // or 1.00000000001); Rust's shortest round-trip representation is
        // plain decimal, so fall back to it rather than losing the value.
        return Ok(value.to_string());
    }
    Ok(trimmed.to_owned())
}

fn parse_number(text: &str) -> Value {
    let text = text.trim();
    if let Ok(value) = text.parse::<i64>() {
        Value::from(value)
    } else if let Ok(value) = text.parse::<u64>() {
        Value::from(value)
    } else if let Ok(value) = text.parse::<f64>() {
        Value::from(value)
    } else {
        Value::String(text.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{format_number, BindBuffer};
    use zhs_json::{Number, Value};

    #[test]
    fn formats_floats_as_plain_decimals() {
        assert_eq!(format_number(0.5).unwrap(), "0.5");
        assert_eq!(format_number(-2.25).unwrap(), "-2.25");
        assert_eq!(format_number(0.1).unwrap(), "0.1");
        assert_eq!(format_number(123456.789).unwrap(), "123456.789");
        assert_eq!(format_number(3.0).unwrap(), "3");
        assert_eq!(format_number(-7.0).unwrap(), "-7");
        assert_eq!(format_number(0.0).unwrap(), "0");
        assert_eq!(format_number(1e-15).unwrap(), "0.000000000000001");
        assert_eq!(format_number(1.00000000001).unwrap(), "1.00000000001");
        assert_eq!(
            format_number(999_999_999_999_999.0).unwrap(),
            "999999999999999"
        );
    }

    #[test]
    fn formatted_floats_never_use_scientific_notation_and_round_trip() {
        for value in [
            0.5_f64,
            -2.25,
            0.1,
            1e-10,
            1e-15,
            -1e-15,
            123456.789,
            1.00000000001,
            999_999_999_999_999.5,
        ] {
            let text = format_number(value).unwrap();
            assert!(
                !text.contains('e') && !text.contains('E'),
                "scientific notation for {value}: {text}"
            );
            assert!(
                !text.contains("inf") && !text.contains("NaN"),
                "non-numeric literal for {value}: {text}"
            );
            assert_eq!(text.parse::<f64>().unwrap(), value, "{text}");
        }
    }

    #[test]
    fn rejects_values_oracle_number_cannot_represent() {
        for value in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            1e300_f64,
            -1e300,
            1e-300,
            -5e-16,
            1e15,
            1.5e15,
        ] {
            assert!(format_number(value).is_err(), "value {value}");
        }
    }

    #[test]
    fn binds_numbers_as_plain_text() {
        let bind = BindBuffer::encode(&Value::from(42_i64)).unwrap();
        assert_eq!(bind.bytes.as_slice(), &b"42\0"[..]);
        assert_eq!(bind.indicator, 0);
        assert_eq!(bind.length, 2);

        let bind = BindBuffer::encode(&Value::from(u64::MAX)).unwrap();
        let expected = format!("{u64max}\0", u64max = u64::MAX);
        assert_eq!(bind.bytes.as_slice(), expected.as_bytes());

        let bind = BindBuffer::encode(&Value::from(1.5_f64)).unwrap();
        assert_eq!(bind.bytes.as_slice(), &b"1.5\0"[..]);

        let bind = BindBuffer::encode(&Value::from(5.0_f64)).unwrap();
        assert_eq!(bind.bytes.as_slice(), &b"5\0"[..]);

        assert!(BindBuffer::encode(&Value::from(1e300_f64)).is_err());
        // `Value::from(f64)` maps non-finite floats to Null, so build the
        // Number directly to exercise the NaN/infinity rejection.
        assert!(BindBuffer::encode(&Value::from(Number::from_f64(f64::NAN))).is_err());
        assert!(BindBuffer::encode(&Value::from(Number::from_f64(f64::INFINITY))).is_err());

        let null = BindBuffer::encode(&Value::Null).unwrap();
        assert_eq!(null.indicator, -1);
    }
}
