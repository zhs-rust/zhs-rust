use crate::ber::tlv;

const EQUALITY_MATCH: u8 = 0xA3;
const PRESENT: u8 = 0x87;
const AND: u8 = 0xA0;
const OR: u8 = 0xA1;

pub fn escape_filter(value: &str) -> String {
    let mut output = String::with_capacity(value.len());

    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\5c"),
            '*' => output.push_str("\\2a"),
            '(' => output.push_str("\\28"),
            ')' => output.push_str("\\29"),
            '\0' => output.push_str("\\00"),
            character => output.push(character),
        }
    }

    output
}

pub fn equality(attribute: &str, value: &str) -> Vec<u8> {
    let assertion = crate::ber::sequence(
        &[
            crate::ber::octet_string(attribute.as_bytes()),
            crate::ber::octet_string(value.as_bytes()),
        ]
        .concat(),
    );
    tlv(EQUALITY_MATCH, &assertion)
}

pub fn present(attribute: &str) -> Vec<u8> {
    tlv(PRESENT, attribute.as_bytes())
}

pub fn and(filters: &[Vec<u8>]) -> Vec<u8> {
    tlv(AND, &filters.concat())
}

pub fn or(filters: &[Vec<u8>]) -> Vec<u8> {
    tlv(OR, &filters.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_special_characters() {
        assert_eq!(escape_filter("a*b(c)\\d"), "a\\2ab\\28c\\29\\5cd");
    }

    #[test]
    fn builds_equality_filter() {
        let filter = equality("uid", "alice");
        assert_eq!(filter[0], EQUALITY_MATCH);
    }
}
