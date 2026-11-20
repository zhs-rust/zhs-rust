use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::time::Duration;

use zhs_crypto::{base64, constant_time, random};
use zhs_entity::DataError;
use zhs_json::Object;

use crate::bson::{Bson, Document};
use crate::find::Find;
use crate::scram;
use crate::wire;

#[derive(Clone, Debug)]
pub struct ConnectOptions {
    pub host: String,

    pub port: u16,

    pub user: String,

    pub password: String,

    pub auth_database: String,

    pub database: String,

    pub connect_timeout: Duration,

    pub read_timeout: Option<Duration>,

    pub write_timeout: Option<Duration>,

    pub max_scram_iterations: u32,
}

impl ConnectOptions {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        let host = host.into();
        Self {
            port,
            host,
            user: user.into(),
            password: password.into(),
            auth_database: "admin".to_owned(),
            database: "admin".to_owned(),
            connect_timeout: Duration::from_secs(5),
            read_timeout: Some(Duration::from_secs(30)),
            write_timeout: Some(Duration::from_secs(30)),
            max_scram_iterations: 20_000,
        }
    }

    pub fn database(mut self, name: impl Into<String>) -> Self {
        self.database = name.into();
        self
    }

    pub fn auth_database(mut self, name: impl Into<String>) -> Self {
        self.auth_database = name.into();
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

pub struct Connection {
    stream: TcpStream,
    request_id: i32,
    database: String,
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
            request_id: 0,
            database: options.database.clone(),
        };
        connection.handshake(options)?;
        Ok(connection)
    }

    pub fn database(&self) -> &str {
        &self.database
    }

    pub(crate) fn command(
        &mut self,
        database: &str,
        mut body: Document,
    ) -> Result<Document, DataError> {
        body.set("$db", Bson::String(database.to_owned()));

        self.request_id = self.request_id.wrapping_add(1);
        let message = wire::encode(self.request_id, &body);
        self.stream
            .write_all(&message)
            .map_err(|error| DataError::protocol(format!("writing a command: {error}")))?;
        self.stream
            .flush()
            .map_err(|error| DataError::protocol(format!("flushing a command: {error}")))?;

        let reply = self.read_message()?;
        if reply.response_to != self.request_id {
            return Err(DataError::protocol(
                "reply does not answer the last request",
            ));
        }

        check_ok(&reply.document)?;
        Ok(reply.document)
    }

    pub fn run_command(&mut self, database: &str, command: &Object) -> Result<Object, DataError> {
        let document = Document::from_object(command)?;
        let reply = self.command(database, document)?;
        Ok(reply.to_object())
    }

    pub fn run_command_documents(
        &mut self,
        database: &str,
        command: &Object,
    ) -> Result<Vec<Object>, DataError> {
        let document = Document::from_object(command)?;
        let reply = self.command(database, document)?;
        self.drain_cursor(database, reply)
    }

    fn drain_cursor(&mut self, database: &str, reply: Document) -> Result<Vec<Object>, DataError> {
        let Some(Bson::Document(cursor)) = reply.get("cursor") else {
            return Ok(vec![reply.to_object()]);
        };

        let mut id = integer(cursor.get("id")).unwrap_or(0);
        let collection = match cursor.get("ns") {
            Some(Bson::String(namespace)) => {
                namespace.split('.').nth(1).unwrap_or_default().to_owned()
            }
            _ => String::new(),
        };

        let mut documents: Vec<Object> = batch_documents(cursor)
            .iter()
            .map(Document::to_object)
            .collect();

        while id != 0 {
            let mut get_more = Document::new();
            get_more.insert("getMore", Bson::Int64(id));
            get_more.insert("collection", Bson::String(collection.clone()));

            let next = match self.command(database, get_more) {
                Ok(next) => next,
                Err(error) => {
                    if self.kill_cursor(database, &collection, id).is_err() {
                        eprintln!("failed to kill MongoDB cursor");
                    }
                    return Err(error);
                }
            };

            let Some(Bson::Document(cursor)) = next.get("cursor") else {
                break;
            };
            id = integer(cursor.get("id")).unwrap_or(0);
            documents.extend(batch_documents(cursor).iter().map(Document::to_object));
        }

        Ok(documents)
    }

    pub fn find(&mut self, collection: &str, find: &Find) -> Result<Vec<Object>, DataError> {
        let database = self.database.clone();

        let mut command = Document::new();
        command.insert("find", Bson::String(collection.to_owned()));
        command.insert(
            "filter",
            Bson::Document(Document::from_object(&find.filter)?),
        );
        command.insert("batchSize", Bson::Int32(1000));

        if let Some(sort) = &find.sort {
            command.insert("sort", Bson::Document(Document::from_object(sort)?));
        }
        if let Some(projection) = &find.projection {
            command.insert(
                "projection",
                Bson::Document(Document::from_object(projection)?),
            );
        }
        if let Some(limit) = find.limit {
            command.insert("limit", Bson::Int64(limit as i64));
        }
        if let Some(skip) = find.skip {
            command.insert("skip", Bson::Int64(skip as i64));
        }

        let reply = self.command(&database, command)?;
        let (mut cursor, batch) = cursor_of(&reply)?;
        let mut documents: Vec<Object> = batch.iter().map(Document::to_object).collect();

        while cursor != 0 {
            let mut get_more = Document::new();
            get_more.insert("getMore", Bson::Int64(cursor));
            get_more.insert("collection", Bson::String(collection.to_owned()));
            get_more.insert("batchSize", Bson::Int32(1000));

            let reply = match self.command(&database, get_more) {
                Ok(reply) => reply,
                Err(error) => {
                    if self.kill_cursor(&database, collection, cursor).is_err() {
                        eprintln!("failed to kill MongoDB cursor");
                    }
                    return Err(error);
                }
            };

            let (next, batch) = cursor_of(&reply)?;
            documents.extend(batch.iter().map(Document::to_object));
            cursor = next;
        }

        Ok(documents)
    }

    pub fn ping(&mut self) -> Result<(), DataError> {
        let database = self.database.clone();
        let mut command = Document::new();
        command.insert("ping", Bson::Int32(1));
        self.command(&database, command)?;
        Ok(())
    }

    pub fn insert_one(&mut self, collection: &str, document: &Object) -> Result<(), DataError> {
        let database = self.database.clone();

        let mut command = Document::new();
        command.insert("insert", Bson::String(collection.to_owned()));
        command.insert(
            "documents",
            Bson::Array(vec![Bson::Document(Document::from_object(document)?)]),
        );

        let reply = self.command(&database, command)?;
        if let Some(Bson::Array(errors)) = reply.get("writeErrors")
            && let Some(Bson::Document(error)) = errors.first()
        {
            let code = numeric(error.get("code")).unwrap_or(0.0) as u32;
            let message = match error.get("errmsg") {
                Some(Bson::String(text)) => text.clone(),
                _ => "write failed".to_owned(),
            };
            return Err(DataError::Server {
                code,
                state: String::new(),
                message,
            });
        }

        Ok(())
    }

    pub fn drop_collection(&mut self, collection: &str) -> Result<(), DataError> {
        let database = self.database.clone();
        let mut command = Document::new();
        command.insert("drop", Bson::String(collection.to_owned()));

        match self.command(&database, command) {
            Ok(_) => Ok(()),
            Err(DataError::Server { code: 26, .. }) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub fn close(self) -> Result<(), DataError> {
        self.stream
            .shutdown(Shutdown::Both)
            .map_err(|error| DataError::protocol(format!("closing the socket: {error}")))
    }

    fn handshake(&mut self, options: &ConnectOptions) -> Result<(), DataError> {
        let auth_database = options.auth_database.clone();

        let mut hello = Document::new();
        hello.insert("hello", Bson::Int32(1));
        hello.insert("helloOk", Bson::Bool(true));
        hello.insert("client", Bson::Document(client_metadata()));
        if !options.user.is_empty() {
            hello.insert(
                "saslSupportedMechs",
                Bson::String(format!("{auth_database}.{}", options.user)),
            );
        }

        let reply = self.command(&auth_database, hello)?;

        let wire_version = numeric(reply.get("maxWireVersion")).unwrap_or(0.0);
        if wire_version < 6.0 {
            return Err(DataError::Unsupported(
                "MongoDB older than 3.6 (no OP_MSG support)",
            ));
        }

        if options.user.is_empty() {
            return Ok(());
        }

        let mut mechanisms = Vec::new();
        if let Some(Bson::Array(items)) = reply.get("saslSupportedMechs") {
            for item in items {
                if let Bson::String(name) = item {
                    mechanisms.push(name.clone());
                }
            }
        }

        if !mechanisms.is_empty() && !mechanisms.iter().any(|name| name == "SCRAM-SHA-256") {
            return Err(DataError::Unsupported(
                "only `SCRAM-SHA-256` authentication is implemented",
            ));
        }

        self.authenticate(options, &auth_database)
    }

    fn authenticate(
        &mut self,
        options: &ConnectOptions,
        auth_database: &str,
    ) -> Result<(), DataError> {
        let nonce = base64::encode_url(
            &random::bytes(18)
                .map_err(|error| DataError::protocol(format!("generating a nonce: {error}")))?,
        );
        let first = scram::client_first(&options.user, &nonce);

        let mut start = Document::new();
        start.insert("saslStart", Bson::Int32(1));
        start.insert("mechanism", Bson::String("SCRAM-SHA-256".to_owned()));
        start.insert("payload", Bson::Binary(first.message.into_bytes()));
        start.insert("autoAuthorize", Bson::Int32(1));
        start.insert("options", Bson::Document(Document::new()));

        let reply = self.command(auth_database, start)?;
        let conversation = integer(reply.get("conversationId"))
            .ok_or_else(|| DataError::protocol("saslStart reply has no conversationId"))?;
        let server_first = text_of(binary(reply.get("payload"))?)?;
        let last = scram::client_final(
            &options.password,
            &nonce,
            &first.bare,
            server_first,
            options.max_scram_iterations,
        )?;

        let mut reply = self.send_scram(auth_database, conversation, last.message.as_bytes())?;
        let mut payload = Vec::new();
        let mut done = false;

        for _ in 0..3 {
            let current = binary(reply.get("payload"))?;
            if !current.is_empty() {
                payload = current.to_vec();
            }
            if flag(reply.get("done")) {
                done = true;
                break;
            }
            reply = self.send_scram(auth_database, conversation, b"")?;
        }

        if !done {
            return Err(DataError::protocol("SCRAM conversation did not finish"));
        }
        if payload.is_empty() {
            return Err(DataError::protocol(
                "SCRAM conversation ended without a server signature",
            ));
        }

        let signature = scram::server_signature(text_of(&payload)?)?;
        if !constant_time::eq(&signature, &last.server_signature) {
            return Err(DataError::protocol("SCRAM server signature did not match"));
        }

        Ok(())
    }

    fn send_scram(
        &mut self,
        auth_database: &str,
        conversation: i64,
        payload: &[u8],
    ) -> Result<Document, DataError> {
        let mut continue_message = Document::new();
        continue_message.insert("saslContinue", Bson::Int32(1));
        continue_message.insert(
            "conversationId",
            Bson::Int32(
                i32::try_from(conversation).map_err(|_| {
                    DataError::protocol("SCRAM conversation id does not fit in i32")
                })?,
            ),
        );
        continue_message.insert("payload", Bson::Binary(payload.to_vec()));
        self.command(auth_database, continue_message)
    }

    fn kill_cursor(
        &mut self,
        database: &str,
        collection: &str,
        cursor: i64,
    ) -> Result<(), DataError> {
        let mut command = Document::new();
        command.insert("killCursors", Bson::String(collection.to_owned()));
        command.insert("cursors", Bson::Array(vec![Bson::Int64(cursor)]));
        self.command(database, command)?;
        Ok(())
    }

    fn read_message(&mut self) -> Result<wire::Reply, DataError> {
        let mut header = [0_u8; 16];
        self.stream
            .read_exact(&mut header)
            .map_err(|error| DataError::protocol(format!("reading a message header: {error}")))?;

        let length = i32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        if length < 21 || length as usize > wire::max_message_length() {
            return Err(DataError::protocol(format!(
                "message length {length} is out of range"
            )));
        }

        let mut buffer = vec![0_u8; length as usize];
        buffer[..16].copy_from_slice(&header);
        self.stream
            .read_exact(&mut buffer[16..])
            .map_err(|error| DataError::protocol(format!("reading a message body: {error}")))?;

        wire::decode(&buffer)
    }
}

fn client_metadata() -> Document {
    let mut driver = Document::new();
    driver.insert("name", Bson::String("zhs".to_owned()));
    driver.insert("version", Bson::String("0.1.0".to_owned()));

    let mut os = Document::new();
    os.insert("type", Bson::String(std::env::consts::OS.to_owned()));

    let mut client = Document::new();
    client.insert("driver", Bson::Document(driver));
    client.insert("os", Bson::Document(os));
    client
}

fn check_ok(document: &Document) -> Result<(), DataError> {
    match document.get("ok") {
        Some(value) if numeric(Some(value)) == Some(1.0) => Ok(()),
        Some(_) => {
            let code = numeric(document.get("code")).unwrap_or(0.0) as u32;
            let state = match document.get("codeName") {
                Some(Bson::String(name)) => name.clone(),
                _ => String::new(),
            };
            let message = match document.get("errmsg") {
                Some(Bson::String(text)) => text.clone(),
                _ => "command failed".to_owned(),
            };
            Err(DataError::Server {
                code,
                state,
                message,
            })
        }
        None => Err(DataError::protocol("reply has no `ok` field")),
    }
}

fn numeric(value: Option<&Bson>) -> Option<f64> {
    match value? {
        Bson::Double(value) => Some(*value),
        Bson::Int32(value) => Some(f64::from(*value)),
        Bson::Int64(value) => Some(*value as f64),
        _ => None,
    }
}

fn integer(value: Option<&Bson>) -> Option<i64> {
    match value? {
        Bson::Int32(value) => Some(i64::from(*value)),
        Bson::Int64(value) => Some(*value),
        _ => None,
    }
}

fn flag(value: Option<&Bson>) -> bool {
    matches!(value, Some(Bson::Bool(true)))
}

fn binary(value: Option<&Bson>) -> Result<&[u8], DataError> {
    match value {
        Some(Bson::Binary(bytes)) => Ok(bytes),
        _ => Err(DataError::protocol("expected a binary payload")),
    }
}

fn text_of(bytes: &[u8]) -> Result<&str, DataError> {
    std::str::from_utf8(bytes)
        .map_err(|error| DataError::protocol(format!("payload is not UTF-8: {error}")))
}

fn batch_documents(cursor: &Document) -> Vec<Document> {
    let batch = match cursor.get("firstBatch").or_else(|| cursor.get("nextBatch")) {
        Some(Bson::Array(items)) => items,
        _ => return Vec::new(),
    };

    batch
        .iter()
        .filter_map(|item| match item {
            Bson::Document(document) => Some(document.clone()),
            _ => None,
        })
        .collect()
}

fn cursor_of(document: &Document) -> Result<(i64, Vec<Document>), DataError> {
    let cursor = match document.get("cursor") {
        Some(Bson::Document(cursor)) => cursor,
        _ => return Err(DataError::protocol("reply has no cursor document")),
    };

    let id = integer(cursor.get("id")).ok_or_else(|| DataError::protocol("cursor has no id"))?;

    let batch = match cursor.get("firstBatch").or_else(|| cursor.get("nextBatch")) {
        Some(Bson::Array(items)) => items,
        _ => return Err(DataError::protocol("cursor has no batch")),
    };

    let documents = batch
        .iter()
        .filter_map(|item| match item {
            Bson::Document(document) => Some(document.clone()),
            _ => None,
        })
        .collect();

    Ok((id, documents))
}
