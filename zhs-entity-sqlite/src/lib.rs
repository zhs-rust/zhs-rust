use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr;
use std::sync::Mutex;

use zhs_entity::{DataError, Error, Params, Row, SqlDatabase, Value};

#[repr(C)]
struct Sqlite3 {
    _private: [u8; 0],
}

#[repr(C)]
struct Sqlite3Statement {
    _private: [u8; 0],
}

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;
const SQLITE_INTEGER: c_int = 1;
const SQLITE_FLOAT: c_int = 2;
const SQLITE_TEXT: c_int = 3;
const SQLITE_BLOB: c_int = 4;

type Destructor = Option<unsafe extern "C" fn(*mut c_void)>;

#[cfg(windows)]
#[link(name = "winsqlite3", kind = "raw-dylib")]
unsafe extern "C" {
    fn sqlite3_open(filename: *const c_char, database: *mut *mut Sqlite3) -> c_int;
    fn sqlite3_close(database: *mut Sqlite3) -> c_int;
    fn sqlite3_errmsg(database: *mut Sqlite3) -> *const c_char;
    fn sqlite3_prepare_v2(
        database: *mut Sqlite3,
        sql: *const c_char,
        length: c_int,
        statement: *mut *mut Sqlite3Statement,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_finalize(statement: *mut Sqlite3Statement) -> c_int;
    fn sqlite3_bind_null(statement: *mut Sqlite3Statement, index: c_int) -> c_int;
    fn sqlite3_bind_int64(statement: *mut Sqlite3Statement, index: c_int, value: i64) -> c_int;
    fn sqlite3_bind_double(statement: *mut Sqlite3Statement, index: c_int, value: f64) -> c_int;
    fn sqlite3_bind_text(
        statement: *mut Sqlite3Statement,
        index: c_int,
        value: *const c_char,
        length: c_int,
        destructor: Destructor,
    ) -> c_int;
    fn sqlite3_bind_blob(
        statement: *mut Sqlite3Statement,
        index: c_int,
        value: *const c_void,
        length: c_int,
        destructor: Destructor,
    ) -> c_int;
    fn sqlite3_step(statement: *mut Sqlite3Statement) -> c_int;
    fn sqlite3_column_count(statement: *mut Sqlite3Statement) -> c_int;
    fn sqlite3_column_name(statement: *mut Sqlite3Statement, index: c_int) -> *const c_char;
    fn sqlite3_column_type(statement: *mut Sqlite3Statement, index: c_int) -> c_int;
    fn sqlite3_column_int64(statement: *mut Sqlite3Statement, index: c_int) -> i64;
    fn sqlite3_column_double(statement: *mut Sqlite3Statement, index: c_int) -> f64;
    fn sqlite3_column_text(statement: *mut Sqlite3Statement, index: c_int) -> *const u8;
    fn sqlite3_column_blob(statement: *mut Sqlite3Statement, index: c_int) -> *const c_void;
    fn sqlite3_column_bytes(statement: *mut Sqlite3Statement, index: c_int) -> c_int;
    fn sqlite3_changes(database: *mut Sqlite3) -> c_int;
}

#[cfg(not(windows))]
#[link(name = "sqlite3")]
unsafe extern "C" {
    fn sqlite3_open(filename: *const c_char, database: *mut *mut Sqlite3) -> c_int;
    fn sqlite3_close(database: *mut Sqlite3) -> c_int;
    fn sqlite3_errmsg(database: *mut Sqlite3) -> *const c_char;
    fn sqlite3_prepare_v2(
        database: *mut Sqlite3,
        sql: *const c_char,
        length: c_int,
        statement: *mut *mut Sqlite3Statement,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_finalize(statement: *mut Sqlite3Statement) -> c_int;
    fn sqlite3_bind_null(statement: *mut Sqlite3Statement, index: c_int) -> c_int;
    fn sqlite3_bind_int64(statement: *mut Sqlite3Statement, index: c_int, value: i64) -> c_int;
    fn sqlite3_bind_double(statement: *mut Sqlite3Statement, index: c_int, value: f64) -> c_int;
    fn sqlite3_bind_text(
        statement: *mut Sqlite3Statement,
        index: c_int,
        value: *const c_char,
        length: c_int,
        destructor: Destructor,
    ) -> c_int;
    fn sqlite3_bind_blob(
        statement: *mut Sqlite3Statement,
        index: c_int,
        value: *const c_void,
        length: c_int,
        destructor: Destructor,
    ) -> c_int;
    fn sqlite3_step(statement: *mut Sqlite3Statement) -> c_int;
    fn sqlite3_column_count(statement: *mut Sqlite3Statement) -> c_int;
    fn sqlite3_column_name(statement: *mut Sqlite3Statement, index: c_int) -> *const c_char;
    fn sqlite3_column_type(statement: *mut Sqlite3Statement, index: c_int) -> c_int;
    fn sqlite3_column_int64(statement: *mut Sqlite3Statement, index: c_int) -> i64;
    fn sqlite3_column_double(statement: *mut Sqlite3Statement, index: c_int) -> f64;
    fn sqlite3_column_text(statement: *mut Sqlite3Statement, index: c_int) -> *const u8;
    fn sqlite3_column_blob(statement: *mut Sqlite3Statement, index: c_int) -> *const c_void;
    fn sqlite3_column_bytes(statement: *mut Sqlite3Statement, index: c_int) -> c_int;
    fn sqlite3_changes(database: *mut Sqlite3) -> c_int;
}

pub struct SqliteDatabase {
    connection: Mutex<Connection>,
}

struct Connection {
    database: *mut Sqlite3,
}

unsafe impl Send for Connection {}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.database.is_null() {
            unsafe {
                sqlite3_close(self.database);
            }
        }
    }
}

impl Connection {
    fn open(path: &str) -> Result<Self, Error> {
        let path = CString::new(path)
            .map_err(|_| Error::Database(DataError::Decode("invalid database path".into())))?;
        let mut database = ptr::null_mut();

        let result = unsafe { sqlite3_open(path.as_ptr(), &mut database) };

        if result != SQLITE_OK {
            let message = if database.is_null() {
                "failed to open SQLite database".to_owned()
            } else {
                unsafe { error_message(database) }
            };

            if !database.is_null() {
                unsafe {
                    sqlite3_close(database);
                }
            }

            return Err(Error::Database(DataError::Server {
                code: result as u32,
                state: "SQLITE".to_owned(),
                message,
            }));
        }

        Ok(Self { database })
    }

    fn prepare(&self, sql: &str) -> Result<Statement, Error> {
        let sql = CString::new(sql)
            .map_err(|_| Error::Database(DataError::Decode("SQL contains NUL".into())))?;
        let mut statement = ptr::null_mut();

        let result = unsafe {
            sqlite3_prepare_v2(
                self.database,
                sql.as_ptr(),
                -1,
                &mut statement,
                ptr::null_mut(),
            )
        };

        if result != SQLITE_OK {
            return Err(Error::Database(DataError::Server {
                code: result as u32,
                state: "SQLITE".to_owned(),
                message: unsafe { error_message(self.database) },
            }));
        }

        Ok(Statement { statement })
    }
}

struct Statement {
    statement: *mut Sqlite3Statement,
}

unsafe impl Send for Statement {}

impl Drop for Statement {
    fn drop(&mut self) {
        if !self.statement.is_null() {
            unsafe {
                sqlite3_finalize(self.statement);
            }
        }
    }
}

impl Statement {
    fn bind(&mut self, params: &Params) -> Result<(), Error> {
        for (index, value) in params.values().iter().enumerate() {
            let index = index as c_int + 1;

            let result = match value {
                Value::Null => unsafe { sqlite3_bind_null(self.statement, index) },
                Value::Bool(value) => unsafe {
                    sqlite3_bind_int64(self.statement, index, i64::from(*value))
                },
                Value::Int(value) => unsafe { sqlite3_bind_int64(self.statement, index, *value) },
                Value::UInt(value) => {
                    let value = i64::try_from(*value).map_err(|_| {
                        Error::Database(DataError::Decode(
                            "unsigned value does not fit into SQLite INTEGER".into(),
                        ))
                    })?;
                    unsafe { sqlite3_bind_int64(self.statement, index, value) }
                }
                Value::Float(value) => unsafe {
                    sqlite3_bind_double(self.statement, index, *value)
                },
                Value::Text(value) => {
                    let value = CString::new(value.as_str()).map_err(|_| {
                        Error::Database(DataError::Decode("text contains NUL".into()))
                    })?;
                    unsafe {
                        sqlite3_bind_text(
                            self.statement,
                            index,
                            value.as_ptr(),
                            value.as_bytes().len() as c_int,
                            transient(),
                        )
                    }
                }
                Value::Bytes(value) => unsafe {
                    sqlite3_bind_blob(
                        self.statement,
                        index,
                        value.as_ptr() as *const c_void,
                        value.len() as c_int,
                        transient(),
                    )
                },
            };

            if result != SQLITE_OK {
                return Err(Error::Database(DataError::protocol(
                    "failed to bind SQLite parameter",
                )));
            }
        }

        Ok(())
    }

    fn execute(&mut self) -> Result<(), Error> {
        let result = unsafe { sqlite3_step(self.statement) };

        if result == SQLITE_DONE || result == SQLITE_ROW {
            Ok(())
        } else {
            Err(Error::Database(DataError::protocol(
                "failed to execute SQLite statement",
            )))
        }
    }

    fn query(&mut self) -> Result<Vec<Row>, Error> {
        let mut rows = Vec::new();

        loop {
            let result = unsafe { sqlite3_step(self.statement) };

            if result == SQLITE_DONE {
                break;
            }
            if result != SQLITE_ROW {
                return Err(Error::Database(DataError::protocol(
                    "failed to step SQLite statement",
                )));
            }

            let columns = unsafe { sqlite3_column_count(self.statement) } as usize;
            let mut names = Vec::with_capacity(columns);
            let mut values = Vec::with_capacity(columns);

            for index in 0..columns {
                let name = unsafe { sqlite3_column_name(self.statement, index as c_int) };
                names.push(if name.is_null() {
                    String::new()
                } else {
                    unsafe { CStr::from_ptr(name) }
                        .to_string_lossy()
                        .into_owned()
                });

                values.push(read_column(self.statement, index as c_int)?);
            }

            rows.push(Row::new(names, values));
        }

        Ok(rows)
    }
}

fn read_column(statement: *mut Sqlite3Statement, index: c_int) -> Result<Value, Error> {
    let column_type = unsafe { sqlite3_column_type(statement, index) };

    match column_type {
        SQLITE_INTEGER => Ok(Value::Int(unsafe {
            sqlite3_column_int64(statement, index)
        })),
        SQLITE_FLOAT => Ok(Value::Float(unsafe {
            sqlite3_column_double(statement, index)
        })),
        SQLITE_TEXT => {
            let pointer = unsafe { sqlite3_column_text(statement, index) };
            let length = unsafe { sqlite3_column_bytes(statement, index) } as usize;

            if pointer.is_null() {
                Ok(Value::Null)
            } else {
                let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
                Ok(Value::Text(String::from_utf8(bytes.to_vec()).map_err(
                    |error| {
                        Error::Database(DataError::Decode(format!(
                            "invalid UTF-8 in SQLite text: {error}"
                        )))
                    },
                )?))
            }
        }
        SQLITE_BLOB => {
            let pointer = unsafe { sqlite3_column_blob(statement, index) };
            let length = unsafe { sqlite3_column_bytes(statement, index) } as usize;

            // sqlite3_column_blob returns a NULL pointer for zero-length blobs.
            if pointer.is_null() {
                Ok(Value::Bytes(Vec::new()))
            } else {
                Ok(Value::Bytes(unsafe {
                    std::slice::from_raw_parts(pointer as *const u8, length).to_vec()
                }))
            }
        }
        _ => Ok(Value::Null),
    }
}

impl SqliteDatabase {
    pub fn open(path: &str) -> Result<Self, Error> {
        Ok(Self {
            connection: Mutex::new(Connection::open(path)?),
        })
    }

    pub fn in_memory() -> Result<Self, Error> {
        Self::open(":memory:")
    }
}

impl SqlDatabase for SqliteDatabase {
    fn execute(&self, sql: &str, params: &Params) -> Result<u64, Error> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::Database(DataError::Closed))?;

        let mut statement = connection.prepare(sql)?;
        statement.bind(params)?;
        statement.execute()?;

        Ok(unsafe { sqlite3_changes(connection.database) } as u64)
    }

    fn query(&self, sql: &str, params: &Params) -> Result<Vec<Row>, Error> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| Error::Database(DataError::Closed))?;

        let mut statement = connection.prepare(sql)?;
        statement.bind(params)?;
        statement.query()
    }
}

unsafe fn error_message(database: *mut Sqlite3) -> String {
    let message = unsafe { sqlite3_errmsg(database) };

    if message.is_null() {
        "unknown SQLite error".to_owned()
    } else {
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    }
}

fn transient() -> Destructor {
    unsafe { std::mem::transmute::<isize, Destructor>(-1) }
}
