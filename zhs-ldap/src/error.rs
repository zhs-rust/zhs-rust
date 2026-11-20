use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LdapError {
    Connect(String),
    Io(String),
    Protocol(String),
    InvalidCredentials,
    NoSuchUser,
    Search(String),
    Unsupported(&'static str),
}

impl fmt::Display for LdapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LdapError::Connect(message) => write!(formatter, "LDAP connect failed: {message}"),
            LdapError::Io(message) => write!(formatter, "LDAP I/O failed: {message}"),
            LdapError::Protocol(message) => write!(formatter, "LDAP protocol error: {message}"),
            LdapError::InvalidCredentials => formatter.write_str("invalid LDAP credentials"),
            LdapError::NoSuchUser => formatter.write_str("LDAP user not found"),
            LdapError::Search(message) => write!(formatter, "LDAP search failed: {message}"),
            LdapError::Unsupported(feature) => {
                write!(formatter, "LDAP feature is not supported: {feature}")
            }
        }
    }
}

impl std::error::Error for LdapError {}
