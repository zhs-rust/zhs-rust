#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderMap {
    entries: Vec<(String, String)>,
}

impl HeaderMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case(name))
    }

    pub fn get_all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.entries
            .iter()
            .filter(move |(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        self.entries
            .retain(|(key, _)| !key.eq_ignore_ascii_case(&name));
        self.entries.push((name, value.into()));
    }

    pub fn append(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.entries.push((name.into(), value.into()));
    }

    pub fn remove(&mut self, name: &str) -> Option<String> {
        let mut removed = None;
        self.entries.retain(|(key, value)| {
            if key.eq_ignore_ascii_case(name) {
                if removed.is_none() {
                    removed = Some(value.clone());
                }
                false
            } else {
                true
            }
        });
        removed
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

impl<'a> IntoIterator for &'a HeaderMap {
    type Item = (&'a str, &'a str);
    type IntoIter = Box<dyn Iterator<Item = (&'a str, &'a str)> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

pub(crate) fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

pub(crate) fn is_valid_value_byte(byte: u8) -> bool {
    byte == b'\t' || (0x20..=0x7E).contains(&byte) || byte >= 0x80
}

pub(crate) fn has_token<'a>(values: impl Iterator<Item = &'a str>, token: &str) -> bool {
    values
        .flat_map(|value| value.split(','))
        .any(|value| value.trim().eq_ignore_ascii_case(token))
}
