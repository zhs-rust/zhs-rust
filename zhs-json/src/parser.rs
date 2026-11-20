use crate::error::{JsonError, JsonErrorKind};

use super::value::{Number, Object, Value};

pub(crate) const MAX_DEPTH: usize = 128;

pub(crate) fn parse(input: &str) -> Result<Value, JsonError> {
    let mut parser = Parser::new(input);
    parser.skip_whitespace();
    let value = parser.parse_value()?;
    parser.skip_whitespace();
    if parser.pos < parser.bytes.len() {
        return Err(parser.error(JsonErrorKind::TrailingCharacters, parser.pos));
    }
    Ok(value)
}

struct Parser<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            pos: 0,
            depth: 0,
        }
    }

    fn error(&self, kind: JsonErrorKind, index: usize) -> JsonError {
        JsonError::at(kind, index, self.source)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn enter(&mut self) -> Result<(), JsonError> {
        if self.depth >= MAX_DEPTH {
            return Err(self.error(JsonErrorKind::RecursionLimitExceeded, self.pos));
        }
        self.depth += 1;
        Ok(())
    }

    fn parse_value(&mut self) -> Result<Value, JsonError> {
        match self.peek() {
            None => Err(self.error(JsonErrorKind::UnexpectedEnd, self.bytes.len())),
            Some(b'n') => {
                self.expect_literal(b"null")?;
                Ok(Value::Null)
            }
            Some(b't') => {
                self.expect_literal(b"true")?;
                Ok(Value::Bool(true))
            }
            Some(b'f') => {
                self.expect_literal(b"false")?;
                Ok(Value::Bool(false))
            }
            Some(b'"') => Ok(Value::String(self.parse_string()?)),
            Some(b'[') => self.parse_array(),
            Some(b'{') => self.parse_object(),
            Some(b'-' | b'0'..=b'9') => Ok(Value::Number(self.parse_number()?)),
            Some(_) => Err(self.error(JsonErrorKind::ExpectedValue, self.pos)),
        }
    }

    fn expect_literal(&mut self, literal: &[u8]) -> Result<(), JsonError> {
        let end = self.pos + literal.len();
        if end > self.bytes.len() {
            return Err(self.error(JsonErrorKind::UnexpectedEnd, self.bytes.len()));
        }
        if &self.bytes[self.pos..end] != literal {
            return Err(self.error(JsonErrorKind::InvalidLiteral, self.pos));
        }
        self.pos = end;
        Ok(())
    }

    fn parse_array(&mut self) -> Result<Value, JsonError> {
        self.enter()?;
        self.pos += 1;
        self.skip_whitespace();

        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(Value::Array(items));
        }

        loop {
            items.push(self.parse_value()?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_whitespace();
                }
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                Some(_) => return Err(self.error(JsonErrorKind::ExpectedCommaOrEnd, self.pos)),
                None => return Err(self.error(JsonErrorKind::UnexpectedEnd, self.bytes.len())),
            }
        }

        self.depth -= 1;
        Ok(Value::Array(items))
    }

    fn parse_object(&mut self) -> Result<Value, JsonError> {
        self.enter()?;
        self.pos += 1;
        self.skip_whitespace();

        let mut object = Object::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(Value::Object(object));
        }

        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return match self.peek() {
                    None => Err(self.error(JsonErrorKind::UnexpectedEnd, self.bytes.len())),
                    Some(_) => Err(self.error(JsonErrorKind::ExpectedObjectKey, self.pos)),
                };
            }
            let key = self.parse_string()?;

            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return match self.peek() {
                    None => Err(self.error(JsonErrorKind::UnexpectedEnd, self.bytes.len())),
                    Some(_) => Err(self.error(JsonErrorKind::ExpectedColon, self.pos)),
                };
            }
            self.pos += 1;
            self.skip_whitespace();

            let value = self.parse_value()?;

            object.insert(key, value);

            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                Some(_) => return Err(self.error(JsonErrorKind::ExpectedCommaOrEnd, self.pos)),
                None => return Err(self.error(JsonErrorKind::UnexpectedEnd, self.bytes.len())),
            }
        }

        self.depth -= 1;
        Ok(Value::Object(object))
    }

    /// Parses a JSON number.
    ///
    /// Integral numbers are first tried as `i64`, then as `u64`, and kept
    /// exactly when they fit. Integers outside the `i64`/`u64` range silently
    /// fall back to `f64` (matching serde_json's behavior), which may lose
    /// precision for very large integers. Values that do not fit a finite
    /// `f64` are rejected with [`JsonErrorKind::NumberOutOfRange`].
    fn parse_number(&mut self) -> Result<Number, JsonError> {
        let start = self.pos;

        if self.peek() == Some(b'-') {
            self.pos += 1;
        }

        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(self.error(JsonErrorKind::InvalidNumber, self.pos));
                }
            }
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.error(JsonErrorKind::InvalidNumber, self.pos)),
        }

        let mut is_float = false;

        if self.peek() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error(JsonErrorKind::InvalidNumber, self.pos));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }

        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error(JsonErrorKind::InvalidNumber, self.pos));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }

        let text = &self.source[start..self.pos];

        if !is_float {
            if let Ok(value) = text.parse::<i64>() {
                return Ok(Number::from_i64(value));
            }
            if let Ok(value) = text.parse::<u64>() {
                return Ok(Number::from_u64(value));
            }
        }

        match text.parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(Number::from_f64(value)),
            Ok(_) => Err(self.error(JsonErrorKind::NumberOutOfRange, start)),
            Err(_) => Err(self.error(JsonErrorKind::InvalidNumber, start)),
        }
    }

    fn parse_string(&mut self) -> Result<String, JsonError> {
        self.pos += 1;
        let mut text = String::new();
        let mut run_start = self.pos;

        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error(JsonErrorKind::UnexpectedEnd, self.bytes.len()));
            };
            match byte {
                b'"' => {
                    text.push_str(&self.source[run_start..self.pos]);
                    self.pos += 1;
                    return Ok(text);
                }
                b'\\' => {
                    text.push_str(&self.source[run_start..self.pos]);
                    self.pos += 1;
                    self.parse_escape(&mut text)?;
                    run_start = self.pos;
                }
                0x00..=0x1F => {
                    return Err(self.error(
                        JsonErrorKind::ControlCharacterInString(byte as char),
                        self.pos,
                    ));
                }
                _ => {
                    let character = self.source[self.pos..]
                        .chars()
                        .next()
                        .expect("position always sits on a character boundary");
                    self.pos += character.len_utf8();
                }
            }
        }
    }

    fn parse_escape(&mut self, text: &mut String) -> Result<(), JsonError> {
        let escape_start = self.pos - 1;
        let Some(character) = self.source[self.pos..].chars().next() else {
            return Err(self.error(JsonErrorKind::UnexpectedEnd, self.bytes.len()));
        };
        self.pos += character.len_utf8();

        match character {
            '"' => text.push('"'),
            '\\' => text.push('\\'),
            '/' => text.push('/'),
            'b' => text.push('\u{0008}'),
            'f' => text.push('\u{000C}'),
            'n' => text.push('\n'),
            'r' => text.push('\r'),
            't' => text.push('\t'),
            'u' => self.parse_unicode_escape(text, escape_start)?,
            other => return Err(self.error(JsonErrorKind::InvalidEscape(other), escape_start)),
        }
        Ok(())
    }

    fn parse_unicode_escape(
        &mut self,
        text: &mut String,
        escape_start: usize,
    ) -> Result<(), JsonError> {
        let first = self.parse_hex4()?;

        let code = if (0xD800..=0xDBFF).contains(&first) {
            if self.peek() != Some(b'\\') || self.bytes.get(self.pos + 1) != Some(&b'u') {
                return Err(self.error(JsonErrorKind::LoneSurrogate, escape_start));
            }
            self.pos += 2;
            let second = self.parse_hex4()?;
            if !(0xDC00..=0xDFFF).contains(&second) {
                return Err(self.error(JsonErrorKind::LoneSurrogate, escape_start));
            }
            0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
        } else if (0xDC00..=0xDFFF).contains(&first) {
            return Err(self.error(JsonErrorKind::LoneSurrogate, escape_start));
        } else {
            first
        };

        match char::from_u32(code) {
            Some(character) => {
                text.push(character);
                Ok(())
            }
            None => Err(self.error(JsonErrorKind::LoneSurrogate, escape_start)),
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, JsonError> {
        let start = self.pos;
        if start + 4 > self.bytes.len() {
            return Err(self.error(JsonErrorKind::UnexpectedEnd, self.bytes.len()));
        }

        let mut value = 0_u32;
        for offset in 0..4 {
            let digit = match self.bytes[start + offset] {
                byte @ b'0'..=b'9' => u32::from(byte - b'0'),
                byte @ b'a'..=b'f' => u32::from(byte - b'a' + 10),
                byte @ b'A'..=b'F' => u32::from(byte - b'A' + 10),
                _ => return Err(self.error(JsonErrorKind::InvalidUnicodeEscape, start + offset)),
            };
            value = value * 16 + digit;
        }

        self.pos = start + 4;
        Ok(value)
    }
}
