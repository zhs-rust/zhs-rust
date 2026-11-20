use std::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Version {
    Http10,

    #[default]
    Http11,
}

impl Version {
    pub const fn as_str(self) -> &'static str {
        match self {
            Version::Http10 => "HTTP/1.0",
            Version::Http11 => "HTTP/1.1",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "HTTP/1.0" => Some(Version::Http10),
            "HTTP/1.1" => Some(Version::Http11),
            _ => None,
        }
    }

    pub const fn is_http11(self) -> bool {
        matches!(self, Version::Http11)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
