use super::LogRecord;

pub trait LogFormatter: Send + Sync {
    fn format(&self, record: &LogRecord) -> String;
}

impl<F> LogFormatter for F
where
    F: Fn(&LogRecord) -> String + Send + Sync,
{
    fn format(&self, record: &LogRecord) -> String {
        self(record)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextFormatter {
    utc_offset_seconds: i32,
}

impl TextFormatter {
    pub const fn new() -> Self {
        Self {
            utc_offset_seconds: 0,
        }
    }

    pub const fn with_utc_offset(mut self, utc_offset_seconds: i32) -> Self {
        self.utc_offset_seconds = utc_offset_seconds;
        self
    }

    pub const fn utc_offset_seconds(self) -> i32 {
        self.utc_offset_seconds
    }
}

impl Default for TextFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl LogFormatter for TextFormatter {
    fn format(&self, record: &LogRecord) -> String {
        let mut line = format!(
            "{} {:<5} {}: {}",
            record.timestamp().format(self.utc_offset_seconds),
            record.level().as_str(),
            escape_controls(record.target()),
            escape_controls(record.message())
        );

        for (key, value) in record.fields() {
            line.push(' ');
            line.push_str(&escape_controls(key));
            line.push('=');
            push_value(&mut line, &escape_controls(value));
        }

        line
    }
}

fn escape_controls(value: &str) -> String {
    let mut output = String::with_capacity(value.len());

    for character in value.chars() {
        match character {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            character if character.is_control() => {
                output.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => output.push(character),
        }
    }

    output
}

fn push_value(line: &mut String, value: &str) {
    let needs_quotes = value.is_empty()
        || value
            .chars()
            .any(|character| character.is_whitespace() || character == '=' || character == '"');

    if !needs_quotes {
        line.push_str(value);
        return;
    }

    line.push('"');
    for character in value.chars() {
        match character {
            '\\' => line.push_str("\\\\"),
            '"' => line.push_str("\\\""),
            other => line.push(other),
        }
    }
    line.push('"');
}
