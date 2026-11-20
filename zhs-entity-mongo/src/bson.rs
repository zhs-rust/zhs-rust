use zhs_entity::DataError;
use zhs_json::{Object, Value};

const TYPE_DOUBLE: u8 = 0x01;
const TYPE_STRING: u8 = 0x02;
const TYPE_DOCUMENT: u8 = 0x03;
const TYPE_ARRAY: u8 = 0x04;
const TYPE_BINARY: u8 = 0x05;
const TYPE_UNDEFINED: u8 = 0x06;
const TYPE_OBJECT_ID: u8 = 0x07;
const TYPE_BOOL: u8 = 0x08;
const TYPE_DATETIME: u8 = 0x09;
const TYPE_NULL: u8 = 0x0A;
const TYPE_JAVASCRIPT: u8 = 0x0D;
const TYPE_SYMBOL: u8 = 0x0E;
const TYPE_INT32: u8 = 0x10;
const TYPE_TIMESTAMP: u8 = 0x11;
const TYPE_INT64: u8 = 0x12;

const MAX_BSON_DEPTH: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Bson {
    Double(f64),

    String(String),

    Document(Document),

    Array(Vec<Bson>),

    Binary(Vec<u8>),

    ObjectId([u8; 12]),

    Bool(bool),

    DateTime(i64),

    Null,

    Int32(i32),

    Int64(i64),

    Timestamp(u64),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Document {
    entries: Vec<(String, Bson)>,
}

impl Document {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn insert(&mut self, name: impl Into<String>, value: Bson) {
        self.entries.push((name.into(), value));
    }

    pub(crate) fn set(&mut self, name: impl Into<String>, value: Bson) {
        let name = name.into();
        if let Some(entry) = self.entries.iter_mut().find(|(key, _)| *key == name) {
            entry.1 = value;
        } else {
            self.entries.push((name, value));
        }
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Bson> {
        self.entries
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    pub(crate) fn from_object(object: &Object) -> Result<Self, DataError> {
        let mut document = Document::new();
        for (key, value) in object.iter() {
            document.insert(key, Bson::from_value(value)?);
        }
        Ok(document)
    }

    pub(crate) fn to_object(&self) -> Object {
        self.to_object_depth(0)
    }

    fn to_object_depth(&self, depth: usize) -> Object {
        if depth > MAX_BSON_DEPTH {
            return Object::new();
        }
        let mut object = Object::new();
        for (key, value) in &self.entries {
            object.insert(key.clone(), value.to_value_depth(depth + 1));
        }
        object
    }
}

impl Bson {
    fn from_value(value: &Value) -> Result<Self, DataError> {
        Ok(match value {
            Value::Null => Bson::Null,
            Value::Bool(flag) => Bson::Bool(*flag),
            Value::Number(number) => {
                if let Some(value) = number.as_i64() {
                    if i32::try_from(value).is_ok() {
                        Bson::Int32(value as i32)
                    } else {
                        Bson::Int64(value)
                    }
                } else if let Some(value) = number.as_u64() {
                    if let Ok(value) = i64::try_from(value) {
                        Bson::Int64(value)
                    } else {
                        Bson::Double(value as f64)
                    }
                } else {
                    Bson::Double(number.as_f64())
                }
            }
            Value::String(text) => Bson::String(text.clone()),
            Value::Array(items) => Bson::Array(
                items
                    .iter()
                    .map(Bson::from_value)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Value::Object(object) => Bson::Document(Document::from_object(object)?),
        })
    }

    fn to_value_depth(&self, depth: usize) -> Value {
        if depth > MAX_BSON_DEPTH {
            return Value::Null;
        }
        match self {
            Bson::Double(value) => Value::from(*value),
            Bson::String(text) => Value::from(text.clone()),
            Bson::Document(document) => Value::Object(document.to_object_depth(depth + 1)),
            Bson::Array(items) => Value::Array(
                items.iter().map(|item| item.to_value_depth(depth + 1)).collect(),
            ),
            Bson::Binary(bytes) => {
                Value::Array(bytes.iter().map(|byte| Value::from(*byte)).collect())
            }
            Bson::ObjectId(bytes) => Value::from(hex(bytes)),
            Bson::Bool(flag) => Value::Bool(*flag),
            Bson::DateTime(millis) => Value::from(*millis),
            Bson::Null => Value::Null,
            Bson::Int32(value) => Value::from(i64::from(*value)),
            Bson::Int64(value) => Value::from(*value),
            Bson::Timestamp(value) => Value::from(*value),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

pub(crate) fn encode(document: &Document) -> Vec<u8> {
    let mut buffer = vec![0_u8; 4];

    for (name, value) in &document.entries {
        match value {
            Bson::Double(number) => {
                buffer.push(TYPE_DOUBLE);
                write_cstring(&mut buffer, name);
                buffer.extend_from_slice(&number.to_le_bytes());
            }
            Bson::String(text) => {
                buffer.push(TYPE_STRING);
                write_cstring(&mut buffer, name);
                write_string(&mut buffer, text);
            }
            Bson::Document(child) => {
                buffer.push(TYPE_DOCUMENT);
                write_cstring(&mut buffer, name);
                buffer.extend_from_slice(&encode(child));
            }
            Bson::Array(items) => {
                buffer.push(TYPE_ARRAY);
                write_cstring(&mut buffer, name);
                let mut array = Document::new();
                for (index, item) in items.iter().enumerate() {
                    array.insert(index.to_string(), item.clone());
                }
                buffer.extend_from_slice(&encode(&array));
            }
            Bson::Binary(bytes) => {
                buffer.push(TYPE_BINARY);
                write_cstring(&mut buffer, name);
                buffer.extend_from_slice(&(bytes.len() as i32).to_le_bytes());
                buffer.push(0x00);
                buffer.extend_from_slice(bytes);
            }
            Bson::ObjectId(bytes) => {
                buffer.push(TYPE_OBJECT_ID);
                write_cstring(&mut buffer, name);
                buffer.extend_from_slice(bytes);
            }
            Bson::Bool(flag) => {
                buffer.push(TYPE_BOOL);
                write_cstring(&mut buffer, name);
                buffer.push(u8::from(*flag));
            }
            Bson::DateTime(millis) => {
                buffer.push(TYPE_DATETIME);
                write_cstring(&mut buffer, name);
                buffer.extend_from_slice(&millis.to_le_bytes());
            }
            Bson::Null => {
                buffer.push(TYPE_NULL);
                write_cstring(&mut buffer, name);
            }
            Bson::Int32(value) => {
                buffer.push(TYPE_INT32);
                write_cstring(&mut buffer, name);
                buffer.extend_from_slice(&value.to_le_bytes());
            }
            Bson::Int64(value) => {
                buffer.push(TYPE_INT64);
                write_cstring(&mut buffer, name);
                buffer.extend_from_slice(&value.to_le_bytes());
            }
            Bson::Timestamp(value) => {
                buffer.push(TYPE_TIMESTAMP);
                write_cstring(&mut buffer, name);
                buffer.extend_from_slice(&value.to_le_bytes());
            }
        }
    }

    buffer.push(0);
    let length = (buffer.len() as i32).to_le_bytes();
    buffer[..4].copy_from_slice(&length);
    buffer
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Document, DataError> {
    let mut reader = Reader::new(bytes);
    let document = reader.document()?;
    if reader.remaining() != 0 {
        return Err(DataError::decode(format!(
            "{} trailing bytes after the BSON document",
            reader.remaining()
        )));
    }
    Ok(document)
}

fn write_cstring(buffer: &mut Vec<u8>, text: &str) {
    buffer.extend_from_slice(text.as_bytes());
    buffer.push(0);
}

fn write_string(buffer: &mut Vec<u8>, text: &str) {
    buffer.extend_from_slice(&((text.len() + 1) as i32).to_le_bytes());
    buffer.extend_from_slice(text.as_bytes());
    buffer.push(0);
}

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
    depth: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            position: 0,
            depth: 0,
        }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.position
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], DataError> {
        if self.remaining() < length {
            return Err(DataError::decode(format!(
                "BSON ended after {} of {length} bytes",
                self.remaining()
            )));
        }
        let start = self.position;
        self.position += length;
        Ok(&self.data[start..self.position])
    }

    fn byte(&mut self) -> Result<u8, DataError> {
        Ok(self.take(1)?[0])
    }

    fn i32(&mut self) -> Result<i32, DataError> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn i64(&mut self) -> Result<i64, DataError> {
        let bytes = self.take(8)?;
        Ok(i64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn u64(&mut self) -> Result<u64, DataError> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn f64(&mut self) -> Result<f64, DataError> {
        Ok(f64::from_bits(self.u64()?))
    }

    fn cstring(&mut self) -> Result<&'a str, DataError> {
        let start = self.position;
        while self.position < self.data.len() {
            if self.data[self.position] == 0 {
                let text = std::str::from_utf8(&self.data[start..self.position])
                    .map_err(|error| DataError::decode(format!("invalid BSON key: {error}")))?;
                self.position += 1;
                return Ok(text);
            }
            self.position += 1;
        }
        Err(DataError::decode("BSON ended inside a key"))
    }

    fn string(&mut self) -> Result<String, DataError> {
        let length = self.i32()?;
        if length < 1 {
            return Err(DataError::decode("BSON string has an invalid length"));
        }
        let bytes = self.take(length as usize)?;
        if bytes.last() != Some(&0) {
            return Err(DataError::decode("BSON string is not NUL-terminated"));
        }
        String::from_utf8(bytes[..bytes.len() - 1].to_vec())
            .map_err(|error| DataError::decode(format!("invalid BSON string: {error}")))
    }

    fn document(&mut self) -> Result<Document, DataError> {
        self.depth += 1;
        if self.depth > MAX_BSON_DEPTH {
            return Err(DataError::decode(
                "BSON document nesting exceeds the maximum depth",
            ));
        }

        let total = self.i32()?;
        if total < 5 {
            return Err(DataError::decode(format!(
                "BSON document is {total} bytes long"
            )));
        }
        let start = self.position - 4;
        if start + total as usize > self.data.len() {
            return Err(DataError::decode("BSON document runs past the message"));
        }

        let mut document = Document::new();
        loop {
            let kind = self.byte()?;
            if kind == 0 {
                break;
            }
            let name = self.cstring()?.to_owned();
            let value = self.value(kind)?;
            document.insert(name, value);
        }

        if self.position != start + total as usize {
            return Err(DataError::decode(
                "BSON document length does not match its content",
            ));
        }

        self.depth -= 1;
        Ok(document)
    }

    fn value(&mut self, kind: u8) -> Result<Bson, DataError> {
        Ok(match kind {
            TYPE_DOUBLE => Bson::Double(self.f64()?),
            TYPE_STRING => Bson::String(self.string()?),
            TYPE_DOCUMENT => Bson::Document(self.document()?),
            TYPE_ARRAY => {
                let array = self.document()?;
                Bson::Array(array.entries.into_iter().map(|(_, value)| value).collect())
            }
            TYPE_BINARY => {
                let length = self.i32()?;
                if length < 0 {
                    return Err(DataError::decode("BSON binary has an invalid length"));
                }
                let _subtype = self.byte()?;
                Bson::Binary(self.take(length as usize)?.to_vec())
            }
            TYPE_UNDEFINED => Bson::Null,
            TYPE_OBJECT_ID => {
                let bytes = self.take(12)?;
                let mut id = [0_u8; 12];
                id.copy_from_slice(bytes);
                Bson::ObjectId(id)
            }
            TYPE_BOOL => Bson::Bool(self.byte()? != 0),
            TYPE_DATETIME => Bson::DateTime(self.i64()?),
            TYPE_NULL => Bson::Null,
            TYPE_JAVASCRIPT | TYPE_SYMBOL => Bson::String(self.string()?),
            TYPE_INT32 => Bson::Int32(self.i32()?),
            TYPE_TIMESTAMP => Bson::Timestamp(self.u64()?),
            TYPE_INT64 => Bson::Int64(self.i64()?),
            other => {
                return Err(DataError::decode(format!(
                    "unsupported BSON type 0x{other:02x}"
                )));
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Bson, Document, decode, encode};
    use zhs_json::Value;

    fn sample() -> Document {
        let mut document = Document::new();
        document.insert("hello", Bson::String("world".to_owned()));
        document.insert("count", Bson::Int32(42));
        document.insert("pi", Bson::Double(3.5));
        document.insert("ok", Bson::Bool(true));
        document.insert("none", Bson::Null);
        document.insert("big", Bson::Int64(i64::MAX));

        let mut nested = Document::new();
        nested.insert("inner", Bson::Int32(1));
        document.insert("nested", Bson::Document(nested));
        document.insert(
            "list",
            Bson::Array(vec![Bson::Int32(1), Bson::String("two".to_owned())]),
        );
        document
    }

    #[test]
    fn matches_the_reference_encoding_of_a_simple_document() {
        let mut document = Document::new();
        document.insert("hello", Bson::String("world".to_owned()));

        let bytes = encode(&document);
        assert_eq!(bytes.len(), 22);
        assert_eq!(&bytes[..4], &22_i32.to_le_bytes());
        assert_eq!(bytes[4], 0x02);
        assert_eq!(&bytes[5..11], b"hello\0");
        assert_eq!(&bytes[11..15], &6_i32.to_le_bytes());
        assert_eq!(&bytes[15..21], b"world\0");
        assert_eq!(bytes[21], 0);
    }

    #[test]
    fn round_trips_every_supported_type() {
        let document = sample();
        let bytes = encode(&document);
        let decoded = decode(&bytes).expect("decode");
        assert_eq!(decoded, document);
    }

    #[test]
    fn converts_to_and_from_json_values() {
        let document = sample();
        let object = document.to_object();
        let restored = Document::from_object(&object).expect("from object");
        assert_eq!(restored, document);
    }

    #[test]
    fn decodes_object_ids_as_hex_and_binary_as_byte_arrays() {
        let mut document = Document::new();
        document.insert("_id", Bson::ObjectId([0xab; 12]));
        document.insert("data", Bson::Binary(vec![1, 2, 3]));

        let object = decode(&encode(&document)).expect("decode").to_object();
        assert_eq!(
            object.get("_id"),
            Some(&Value::from("abababababababababababab"))
        );
        assert_eq!(
            object.get("data"),
            Some(&Value::Array(vec![
                Value::from(1_u8),
                Value::from(2_u8),
                Value::from(3_u8)
            ]))
        );
    }

    #[test]
    fn rejects_truncated_and_mislabelled_documents() {
        let bytes = encode(&sample());
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());

        let mut wrong = bytes.clone();
        let wrong_length = wrong.len() as i32 + 1;
        wrong[..4].copy_from_slice(&wrong_length.to_le_bytes());
        assert!(decode(&wrong).is_err());
    }
}
