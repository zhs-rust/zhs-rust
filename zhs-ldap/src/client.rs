use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::ber::{Reader, boolean, enumerated, integer, octet_string, sequence, tlv};
use crate::{LdapError, filter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    Base,
    OneLevel,
    Subtree,
}

impl SearchScope {
    fn as_u8(self) -> u8 {
        match self {
            SearchScope::Base => 0,
            SearchScope::OneLevel => 1,
            SearchScope::Subtree => 2,
        }
    }
}

#[derive(Clone)]
pub struct LdapOptions {
    pub host: String,
    pub port: u16,
    pub base_dn: String,
    pub bind_dn: String,
    pub bind_password: String,
    pub user_attribute: String,
    pub user_dn_pattern: Option<String>,
    pub group_base_dn: Option<String>,
    pub group_attribute: String,
    pub group_object_class: String,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
}

impl std::fmt::Debug for LdapOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LdapOptions")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("base_dn", &self.base_dn)
            .field("bind_dn", &self.bind_dn)
            .field("bind_password", &"<redacted>")
            .field("user_attribute", &self.user_attribute)
            .field("user_dn_pattern", &self.user_dn_pattern)
            .field("group_base_dn", &self.group_base_dn)
            .field("group_attribute", &self.group_attribute)
            .field("group_object_class", &self.group_object_class)
            .field("connect_timeout", &self.connect_timeout)
            .field("read_timeout", &self.read_timeout)
            .finish()
    }
}

impl LdapOptions {
    pub fn new(host: impl Into<String>, port: u16, base_dn: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
            base_dn: base_dn.into(),
            bind_dn: String::new(),
            bind_password: String::new(),
            user_attribute: "uid".to_owned(),
            user_dn_pattern: None,
            group_base_dn: None,
            group_attribute: "memberOf".to_owned(),
            group_object_class: "group".to_owned(),
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(10),
        }
    }

    pub fn service_account(
        mut self,
        bind_dn: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.bind_dn = bind_dn.into();
        self.bind_password = password.into();
        self
    }

    pub fn user_attribute(mut self, attribute: impl Into<String>) -> Self {
        self.user_attribute = attribute.into();
        self
    }

    pub fn user_dn_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.user_dn_pattern = Some(pattern.into());
        self
    }

    pub fn group_base_dn(mut self, base_dn: impl Into<String>) -> Self {
        self.group_base_dn = Some(base_dn.into());
        self
    }

    pub fn group_attribute(mut self, attribute: impl Into<String>) -> Self {
        self.group_attribute = attribute.into();
        self
    }

    pub fn group_object_class(mut self, object_class: impl Into<String>) -> Self {
        self.group_object_class = object_class.into();
        self
    }

    pub fn timeouts(mut self, connect: Duration, read: Duration) -> Self {
        self.connect_timeout = connect;
        self.read_timeout = read;
        self
    }

    fn address(&self) -> Result<std::net::SocketAddr, LdapError> {
        (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(|error| LdapError::Connect(format!("resolving {}: {error}", self.host)))?
            .next()
            .ok_or_else(|| {
                LdapError::Connect(format!("{}:{} did not resolve", self.host, self.port))
            })
    }
}

#[derive(Debug, Clone, Default)]
pub struct LdapEntry {
    pub dn: String,
    pub attributes: BTreeMap<String, Vec<String>>,
}

impl LdapEntry {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.attributes
            .get(&name.to_ascii_lowercase())
            .and_then(|values| values.first())
            .map(String::as_str)
    }

    pub fn values(&self, name: &str) -> &[String] {
        self.attributes
            .get(&name.to_ascii_lowercase())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

pub struct LdapClient {
    stream: TcpStream,
    message_id: i64,
}

impl LdapClient {
    pub fn connect(options: &LdapOptions) -> Result<Self, LdapError> {
        let address = options.address()?;
        let stream = TcpStream::connect_timeout(&address, options.connect_timeout)
            .map_err(|error| LdapError::Connect(format!("{address}: {error}")))?;
        stream
            .set_read_timeout(Some(options.read_timeout))
            .map_err(|error| LdapError::Io(error.to_string()))?;
        stream
            .set_write_timeout(Some(options.read_timeout))
            .map_err(|error| LdapError::Io(error.to_string()))?;

        Ok(Self {
            stream,
            message_id: 0,
        })
    }

    pub fn bind(&mut self, dn: &str, password: &str) -> Result<(), LdapError> {
        let request = tlv(
            0x60,
            &[
                integer(3),
                octet_string(dn.as_bytes()),
                tlv(0x80, password.as_bytes()),
            ]
            .concat(),
        );

        let (tag, content) = self.exchange(request)?;
        if tag != 0x61 {
            return Err(LdapError::Protocol(format!(
                "expected BindResponse, got {tag:#04x}"
            )));
        }

        let mut reader = Reader::new(&content);
        let code = reader.read_enumerated()?;
        let _matched_dn = reader.read_octet_string()?;
        let diagnostic = String::from_utf8_lossy(reader.read_octet_string()?).into_owned();

        match code {
            0 => Ok(()),
            49 => Err(LdapError::InvalidCredentials),
            32 => Err(LdapError::NoSuchUser),
            _ => Err(LdapError::Search(format!(
                "bind failed with LDAP result {code}: {diagnostic}"
            ))),
        }
    }

    pub fn search(
        &mut self,
        base_dn: &str,
        scope: SearchScope,
        filter: &[u8],
        attributes: &[&str],
    ) -> Result<Vec<LdapEntry>, LdapError> {
        let attributes = sequence(
            &attributes
                .iter()
                .map(|attribute| octet_string(attribute.as_bytes()))
                .collect::<Vec<_>>()
                .concat(),
        );

        let request = tlv(
            0x63,
            &[
                octet_string(base_dn.as_bytes()),
                enumerated(scope.as_u8()),
                enumerated(0),
                integer(0),
                integer(0),
                boolean(false),
                filter.to_vec(),
                attributes,
            ]
            .concat(),
        );

        let request_id = self.next_message_id();
        self.send(request_id, request)?;

        let mut entries = Vec::new();

        loop {
            let (message_id, tag, content) = self.read_message()?;
            if message_id != request_id {
                continue;
            }

            match tag {
                0x64 => entries.push(parse_entry(&content)?),
                0x65 => {
                    let mut reader = Reader::new(&content);
                    let code = reader.read_enumerated()?;
                    let _matched_dn = reader.read_octet_string()?;
                    let diagnostic =
                        String::from_utf8_lossy(reader.read_octet_string()?).into_owned();

                    return match code {
                        0 => Ok(entries),
                        _ => Err(LdapError::Search(format!(
                            "search failed with LDAP result {code}: {diagnostic}"
                        ))),
                    };
                }
                0x73 => {}
                other => {
                    return Err(LdapError::Protocol(format!(
                        "unexpected search response tag {other:#04x}"
                    )));
                }
            }
        }
    }

    pub fn unbind(mut self) -> Result<(), LdapError> {
        let request_id = self.next_message_id();
        self.send(request_id, tlv(0x42, &[]))
    }

    fn next_message_id(&mut self) -> i64 {
        self.message_id += 1;
        self.message_id
    }

    fn send(&mut self, message_id: i64, operation: Vec<u8>) -> Result<(), LdapError> {
        let message = sequence(&[integer(message_id), operation].concat());
        self.stream
            .write_all(&message)
            .map_err(|error| LdapError::Io(error.to_string()))?;
        self.stream
            .flush()
            .map_err(|error| LdapError::Io(error.to_string()))
    }

    fn exchange(&mut self, operation: Vec<u8>) -> Result<(u8, Vec<u8>), LdapError> {
        let message_id = self.next_message_id();
        self.send(message_id, operation)?;

        loop {
            let (id, tag, content) = self.read_message()?;
            if id == message_id {
                return Ok((tag, content));
            }
        }
    }

    fn read_message(&mut self) -> Result<(i64, u8, Vec<u8>), LdapError> {
        let (tag, content) = read_element(&mut self.stream)?;
        if tag != 0x30 {
            return Err(LdapError::Protocol(format!(
                "expected LDAPMessage SEQUENCE, got {tag:#04x}"
            )));
        }

        let mut reader = Reader::new(&content);
        let message_id = reader.read_integer()?;
        let (op_tag, op_content) = reader.read()?;
        Ok((message_id, op_tag, op_content.to_vec()))
    }
}

fn read_element(stream: &mut TcpStream) -> Result<(u8, Vec<u8>), LdapError> {
    let mut tag = [0u8; 1];
    stream
        .read_exact(&mut tag)
        .map_err(|error| LdapError::Io(error.to_string()))?;

    let mut first = [0u8; 1];
    stream
        .read_exact(&mut first)
        .map_err(|error| LdapError::Io(error.to_string()))?;

    let length = if first[0] & 0x80 == 0 {
        first[0] as usize
    } else {
        let count = (first[0] & 0x7F) as usize;
        if count == 0 || count > 4 {
            return Err(LdapError::Protocol(
                "invalid LDAP message length".to_owned(),
            ));
        }

        let mut bytes = vec![0u8; count];
        stream
            .read_exact(&mut bytes)
            .map_err(|error| LdapError::Io(error.to_string()))?;

        let mut length = 0usize;
        for byte in bytes {
            length = (length << 8) | byte as usize;
        }
        length
    };

    if length > 16 * 1024 * 1024 {
        return Err(LdapError::Protocol("LDAP message is too large".to_owned()));
    }

    let mut content = vec![0u8; length];
    stream
        .read_exact(&mut content)
        .map_err(|error| LdapError::Io(error.to_string()))?;

    Ok((tag[0], content))
}

fn parse_entry(content: &[u8]) -> Result<LdapEntry, LdapError> {
    let mut reader = Reader::new(content);
    let dn = String::from_utf8_lossy(reader.read_octet_string()?).into_owned();
    let attributes_content = reader.read_tag(0x30)?;
    let mut attributes_reader = Reader::new(attributes_content);
    let mut attributes = BTreeMap::new();

    while !attributes_reader.is_empty() {
        let partial = attributes_reader.read_tag(0x30)?;
        let mut partial_reader = Reader::new(partial);
        let name =
            String::from_utf8_lossy(partial_reader.read_octet_string()?).to_ascii_lowercase();
        let values_content = partial_reader.read_tag(0x31)?;
        let mut values_reader = Reader::new(values_content);
        let mut values = Vec::new();

        while !values_reader.is_empty() {
            values.push(String::from_utf8_lossy(values_reader.read_octet_string()?).into_owned());
        }

        attributes.insert(name, values);
    }

    Ok(LdapEntry { dn, attributes })
}

pub fn user_filter(attribute: &str, username: &str) -> Vec<u8> {
    filter::equality(attribute, username)
}
