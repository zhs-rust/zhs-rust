use std::fmt;

use crate::internal::error::Result;

pub trait FromPath: Sized {
    fn from_path(params: &Params) -> Result<Self>;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Params {
    entries: Vec<(String, String)>,
}

impl Params {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn push(&mut self, name: String, value: String) {
        self.entries.push((name, value));
    }
}

impl fmt::Display for Params {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (name, value) in self.iter() {
            if !first {
                formatter.write_str(", ")?;
            }
            first = false;
            write!(formatter, "{name}={value}")?;
        }
        Ok(())
    }
}
