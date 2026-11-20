use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataError {
    Query(String),

    Identifier(String),

    Protocol(String),

    Server {
        code: u32,

        state: String,

        message: String,
    },

    Unsupported(&'static str),

    Decode(String),

    Task,

    Timeout(&'static str),

    Closed,
}

impl DataError {
    pub fn query(message: impl Into<String>) -> Self {
        Self::Query(message.into())
    }

    pub fn identifier(name: impl Into<String>) -> Self {
        Self::Identifier(name.into())
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Self::Protocol(message.into())
    }

    pub fn decode(message: impl Into<String>) -> Self {
        Self::Decode(message.into())
    }

    pub fn is_conflict(&self) -> bool {
        match self {
            DataError::Server { code, state, .. } => match *code {
                1062 | 2627 | 2601 | 11000 => true,

                1 => state == "HY000",
                _ => state.starts_with("23"),
            },

            DataError::Protocol(message) => message.contains("UNIQUE constraint failed"),
            _ => false,
        }
    }
}

impl fmt::Display for DataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataError::Query(message) => write!(formatter, "invalid query: {message}"),
            DataError::Identifier(name) => write!(formatter, "invalid identifier `{name}`"),
            DataError::Protocol(message) => write!(formatter, "database protocol error: {message}"),
            DataError::Server {
                code,
                state,
                message,
            } => write!(formatter, "database error {code} ({state}): {message}"),
            DataError::Unsupported(feature) => {
                write!(formatter, "not supported by this backend: {feature}")
            }
            DataError::Decode(message) => write!(formatter, "failed to decode a row: {message}"),
            DataError::Task => formatter.write_str("the database task did not complete"),
            DataError::Timeout(what) => write!(formatter, "the database timed out {what}"),
            DataError::Closed => formatter.write_str("the database connection pool is closed"),
        }
    }
}

impl std::error::Error for DataError {}

#[cfg(test)]
mod tests {
    use super::DataError;

    #[test]
    fn formats_every_variant() {
        let errors = [
            DataError::Query("empty list".to_owned()),
            DataError::Identifier("bad name".to_owned()),
            DataError::Protocol("short packet".to_owned()),
            DataError::Server {
                code: 1062,
                state: "23000".to_owned(),
                message: "Duplicate entry".to_owned(),
            },
            DataError::Unsupported("transactions"),
            DataError::Decode("column `age`".to_owned()),
            DataError::Task,
            DataError::Timeout("waiting for a pooled connection"),
            DataError::Closed,
        ];

        for error in errors {
            assert!(!error.to_string().is_empty());
        }
    }
}
