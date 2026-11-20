use std::fmt;
use std::ops::{Index, IndexMut};

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Null,

    Bool(bool),

    Number(Number),

    String(String),

    Array(Vec<Value>),

    Object(Object),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn is_bool(&self) -> bool {
        matches!(self, Value::Bool(_))
    }

    pub fn is_number(&self) -> bool {
        matches!(self, Value::Number(_))
    }

    pub fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<&Number> {
        match self {
            Value::Number(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        self.as_number()?.as_i64()
    }

    pub fn as_u64(&self) -> Option<u64> {
        self.as_number()?.as_u64()
    }

    pub fn as_f64(&self) -> Option<f64> {
        self.as_number().map(Number::as_f64)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::Array(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Value::Object(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut Object> {
        match self {
            Value::Object(value) => Some(value),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.as_object_mut()?.get_mut(key)
    }

    pub fn get_index(&self, index: usize) -> Option<&Value> {
        self.as_array()?.get(index)
    }

    pub fn get_index_mut(&mut self, index: usize) -> Option<&mut Value> {
        self.as_array_mut()?.get_mut(index)
    }
}

impl Index<&str> for Value {
    type Output = Value;

    fn index(&self, key: &str) -> &Self::Output {
        self.get(key)
            .unwrap_or_else(|| panic!("JSON object has no key {key:?}"))
    }
}

impl IndexMut<&str> for Value {
    fn index_mut(&mut self, key: &str) -> &mut Self::Output {
        self.get_mut(key)
            .unwrap_or_else(|| panic!("JSON object has no key {key:?}"))
    }
}

impl Index<usize> for Value {
    type Output = Value;

    fn index(&self, index: usize) -> &Self::Output {
        self.get_index(index)
            .unwrap_or_else(|| panic!("JSON array has no index {index}"))
    }
}

impl IndexMut<usize> for Value {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_index_mut(index)
            .unwrap_or_else(|| panic!("JSON array has no index {index}"))
    }
}

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&super::to_string(self))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Number {
    repr: Repr,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Repr {
    I64(i64),
    U64(u64),
    F64(f64),
}

impl Number {
    pub const fn from_i64(value: i64) -> Self {
        Self {
            repr: Repr::I64(value),
        }
    }

    pub const fn from_u64(value: u64) -> Self {
        if value <= i64::MAX as u64 {
            Self {
                repr: Repr::I64(value as i64),
            }
        } else {
            Self {
                repr: Repr::U64(value),
            }
        }
    }

    pub const fn from_f64(value: f64) -> Self {
        Self {
            repr: Repr::F64(value),
        }
    }

    pub fn is_i64(&self) -> bool {
        matches!(self.repr, Repr::I64(_))
    }

    pub fn is_u64(&self) -> bool {
        matches!(self.repr, Repr::U64(_))
    }

    pub fn is_f64(&self) -> bool {
        matches!(self.repr, Repr::F64(_))
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self.repr {
            Repr::I64(value) => Some(value),
            Repr::U64(value) => i64::try_from(value).ok(),
            Repr::F64(_) => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self.repr {
            Repr::I64(value) => u64::try_from(value).ok(),
            Repr::U64(value) => Some(value),
            Repr::F64(_) => None,
        }
    }

    pub fn as_f64(&self) -> f64 {
        match self.repr {
            Repr::I64(value) => value as f64,
            Repr::U64(value) => value as f64,
            Repr::F64(value) => value,
        }
    }

    pub(crate) fn write_to(&self, output: &mut String) {
        match self.repr {
            Repr::I64(value) => output.push_str(&value.to_string()),
            Repr::U64(value) => output.push_str(&value.to_string()),
            Repr::F64(value) => {
                if !value.is_finite() {
                    output.push_str("null");
                    return;
                }
                let text = value.to_string();
                output.push_str(&text);

                if !text.contains('.') && !text.contains('e') && !text.contains('E') {
                    output.push_str(".0");
                }
            }
        }
    }
}

impl fmt::Display for Number {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut text = String::new();
        self.write_to(&mut text);
        formatter.write_str(&text)
    }
}

impl From<i64> for Number {
    fn from(value: i64) -> Self {
        Self::from_i64(value)
    }
}

impl From<u64> for Number {
    fn from(value: u64) -> Self {
        Self::from_u64(value)
    }
}

impl From<f64> for Number {
    fn from(value: f64) -> Self {
        Self::from_f64(value)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Object {
    entries: Vec<(String, Value)>,
}

impl Object {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(existing, _)| existing == key)
            .map(|(_, value)| value)
    }

    /// 大小写不敏感的回退查找:返回第一个 ASCII 大小写不敏感匹配 `key` 的条目。
    ///
    /// 语义约定:`get` 精确匹配优先,本方法仅供派生宏生成的 `FromJson`
    /// 绑定代码做兜底(对齐 ASP.NET Core web 默认的
    /// `PropertyNameCaseInsensitive = true`),不要在需要区分大小写的
    /// 场景使用。JSON 同时存在 `"Host"` 与 `"host"` 时,精确匹配决定胜负。
    pub fn get_ignore_ascii_case(&self, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(existing, _)| existing.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.entries
            .iter_mut()
            .find(|(existing, _)| existing == key)
            .map(|(_, value)| value)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn insert(&mut self, key: impl Into<String>, value: Value) -> Option<Value> {
        let key = key.into();
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|(existing, _)| existing == &key)
        {
            return Some(std::mem::replace(&mut entry.1, value));
        }
        self.entries.push((key, value));
        None
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let index = self
            .entries
            .iter()
            .position(|(existing, _)| existing == key)?;
        Some(self.entries.remove(index).1)
    }

    pub fn iter(&self) -> Iter<'_> {
        Iter {
            inner: self.entries.iter(),
        }
    }

    pub fn iter_mut(&mut self) -> IterMut<'_> {
        IterMut {
            inner: self.entries.iter_mut(),
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> + '_ {
        self.entries.iter().map(|(key, _)| key.as_str())
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> + '_ {
        self.entries.iter().map(|(_, value)| value)
    }
}

impl FromIterator<(String, Value)> for Object {
    fn from_iter<T: IntoIterator<Item = (String, Value)>>(iterable: T) -> Self {
        let mut object = Self::new();
        for (key, value) in iterable {
            object.insert(key, value);
        }
        object
    }
}

impl Extend<(String, Value)> for Object {
    fn extend<T: IntoIterator<Item = (String, Value)>>(&mut self, iterable: T) {
        for (key, value) in iterable {
            self.insert(key, value);
        }
    }
}

impl IntoIterator for Object {
    type Item = (String, Value);
    type IntoIter = std::vec::IntoIter<(String, Value)>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

#[derive(Clone, Debug)]
pub struct Iter<'a> {
    inner: std::slice::Iter<'a, (String, Value)>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a str, &'a Value);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(key, value)| (key.as_str(), value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for Iter<'_> {}
impl std::iter::FusedIterator for Iter<'_> {}

#[derive(Debug)]
pub struct IterMut<'a> {
    inner: std::slice::IterMut<'a, (String, Value)>,
}

impl<'a> Iterator for IterMut<'a> {
    type Item = (&'a str, &'a mut Value);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(key, value)| (key.as_str(), value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for IterMut<'_> {}
impl std::iter::FusedIterator for IterMut<'_> {}

impl<'a> IntoIterator for &'a Object {
    type Item = (&'a str, &'a Value);
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut Object {
    type Item = (&'a str, &'a mut Value);
    type IntoIter = IterMut<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Bool(value)
    }
}

macro_rules! impl_from_integer {
    ($($ty:ty => $method:ident, $target:ty;)*) => {
        $(
            impl From<$ty> for Value {
                fn from(value: $ty) -> Self {
                    Value::Number(Number::$method(value as $target))
                }
            }
        )*
    };
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::Number(Number::from_i64(value))
    }
}

impl From<u64> for Value {
    fn from(value: u64) -> Self {
        Value::Number(Number::from_u64(value))
    }
}

impl_from_integer! {
    i8 => from_i64, i64;
    i16 => from_i64, i64;
    i32 => from_i64, i64;
    isize => from_i64, i64;
    u8 => from_u64, u64;
    u16 => from_u64, u64;
    u32 => from_u64, u64;
    usize => from_u64, u64;
}

impl From<f32> for Value {
    fn from(value: f32) -> Self {
        Value::from(f64::from(value))
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        if value.is_finite() {
            Value::Number(Number::from_f64(value))
        } else {
            Value::Null
        }
    }
}

impl From<Number> for Value {
    fn from(value: Number) -> Self {
        Value::Number(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::String(value.to_owned())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Value::String(value)
    }
}

impl From<Vec<Value>> for Value {
    fn from(value: Vec<Value>) -> Self {
        Value::Array(value)
    }
}

impl<const N: usize> From<[Value; N]> for Value {
    fn from(value: [Value; N]) -> Self {
        Value::Array(value.into_iter().collect())
    }
}

impl From<Object> for Value {
    fn from(value: Object) -> Self {
        Value::Object(value)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(value: Option<T>) -> Self {
        match value {
            Some(value) => value.into(),
            None => Value::Null,
        }
    }
}

impl FromIterator<Value> for Value {
    fn from_iter<T: IntoIterator<Item = Value>>(iterable: T) -> Self {
        Value::Array(iterable.into_iter().collect())
    }
}
