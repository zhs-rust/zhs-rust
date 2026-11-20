use super::error::{JsonError, JsonErrorKind};
use super::parser::MAX_DEPTH;
use super::value::{Object, Value};

/// Serializes `value` to a compact JSON string.
///
/// Values nested deeper than [`MAX_DEPTH`] cannot be serialized; in that
/// (pathological, hand-built-only) case the result is the valid JSON literal
/// `null` instead of malformed truncated output or a stack overflow. Use
/// [`try_to_string`] to detect this as an error.
pub(crate) fn to_string(value: &Value) -> String {
    match try_to_string(value) {
        Ok(output) => output,
        Err(_) => "null".to_owned(),
    }
}

/// Serializes `value` to an indented JSON string.
///
/// Behaves like [`to_string`] (falls back to `null`) when the value is nested
/// deeper than [`MAX_DEPTH`]; use [`try_to_string_pretty`] for the strict
/// variant.
pub(crate) fn to_string_pretty(value: &Value) -> String {
    match try_to_string_pretty(value) {
        Ok(output) => output,
        Err(_) => "null".to_owned(),
    }
}

/// Serializes `value` to a compact JSON string, returning
/// [`JsonErrorKind::RecursionLimitExceeded`] when it is nested deeper than
/// [`MAX_DEPTH`].
pub(crate) fn try_to_string(value: &Value) -> Result<String, JsonError> {
    let mut output = String::new();
    write_value(value, 0, None, &mut output)?;
    Ok(output)
}

/// Strict variant of [`to_string_pretty`].
pub(crate) fn try_to_string_pretty(value: &Value) -> Result<String, JsonError> {
    let mut output = String::new();
    write_value(value, 0, Some(2), &mut output)?;
    Ok(output)
}

/// Serializes `value` into `output`, tracking nesting `depth`.
///
/// Returns [`JsonErrorKind::RecursionLimitExceeded`] (mirroring the parser's
/// `MAX_DEPTH` cap) for containers nested deeper than the limit, so a
/// hand-built deeply nested [`Value`] degrades instead of overflowing the
/// stack.
fn write_value(
    value: &Value,
    depth: usize,
    indent: Option<usize>,
    output: &mut String,
) -> Result<(), JsonError> {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(true) => output.push_str("true"),
        Value::Bool(false) => output.push_str("false"),
        Value::Number(number) => number.write_to(output),
        Value::String(text) => write_escaped(text, output),
        Value::Array(items) => {
            check_depth(depth)?;
            write_array(items, depth, indent, output)?;
        }
        Value::Object(object) => {
            check_depth(depth)?;
            write_object(object, depth, indent, output)?;
        }
    }

    Ok(())
}

fn check_depth(depth: usize) -> Result<(), JsonError> {
    if depth >= MAX_DEPTH {
        return Err(JsonError::at(JsonErrorKind::RecursionLimitExceeded, 0, ""));
    }
    Ok(())
}

fn write_array(
    items: &[Value],
    depth: usize,
    indent: Option<usize>,
    output: &mut String,
) -> Result<(), JsonError> {
    if items.is_empty() {
        output.push_str("[]");
        return Ok(());
    }

    output.push('[');
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        newline(output, depth + 1, indent);
        write_value(item, depth + 1, indent, output)?;
    }
    newline(output, depth, indent);
    output.push(']');
    Ok(())
}

fn write_object(
    object: &Object,
    depth: usize,
    indent: Option<usize>,
    output: &mut String,
) -> Result<(), JsonError> {
    if object.is_empty() {
        output.push_str("{}");
        return Ok(());
    }

    output.push('{');
    for (index, (key, value)) in object.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        newline(output, depth + 1, indent);
        write_escaped(key, output);
        output.push(':');
        if indent.is_some() {
            output.push(' ');
        }
        write_value(value, depth + 1, indent, output)?;
    }
    newline(output, depth, indent);
    output.push('}');
    Ok(())
}

fn newline(output: &mut String, depth: usize, indent: Option<usize>) {
    if let Some(step) = indent {
        output.push('\n');
        for _ in 0..depth * step {
            output.push(' ');
        }
    }
}

fn write_escaped(text: &str, output: &mut String) {
    output.push('"');
    for character in text.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000C}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                output.push_str("\\u00");
                let byte = control as u8;
                output.push(hex_digit(byte >> 4));
                output.push(hex_digit(byte & 0x0F));
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

fn hex_digit(value: u8) -> char {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    char::from(DIGITS[usize::from(value & 0x0F)])
}
