use zhs_entity::DataError;

use crate::connection::ConnectOptions;
use crate::packet::push_utf16;

pub(crate) const ENCRYPT_OFF: u8 = 0x00;

pub(crate) const ENCRYPT_ON: u8 = 0x01;

pub(crate) const ENCRYPT_NOT_SUP: u8 = 0x02;

pub(crate) const ENCRYPT_REQ: u8 = 0x03;

const LOGIN7_FIXED_LEN: usize = 94;

const CLIENT_LCID: u32 = 0x0000_0409;

const CLIENT_PROG_VER: u32 = 0x0F00_07D0;

pub(crate) fn prelogin_payload() -> Vec<u8> {
    let options: [(u8, &[u8]); 5] = [
        (0x00, &[0x0F, 0x00, 0x07, 0xD0, 0x00, 0x00]),
        (0x01, &[ENCRYPT_NOT_SUP]),
        (0x02, &[0x00]),
        (0x03, &[0x00, 0x00, 0x00, 0x00]),
        (0x04, &[0x00]),
    ];

    let header_len = options.len() * 5 + 1;
    let mut payload = Vec::with_capacity(header_len + 16);
    let mut data = Vec::new();

    for (token, bytes) in options {
        payload.push(token);
        payload.extend_from_slice(&((header_len + data.len()) as u16).to_be_bytes());
        payload.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        data.extend_from_slice(bytes);
    }

    payload.push(0xFF);
    payload.extend_from_slice(&data);
    payload
}

pub(crate) fn encryption_level(payload: &[u8]) -> Result<u8, DataError> {
    let mut position = 0;

    while position < payload.len() {
        let token = payload[position];
        position += 1;

        if token == 0xFF {
            break;
        }

        if position + 4 > payload.len() {
            return Err(DataError::protocol("a truncated PRELOGIN option table"));
        }

        let offset = u16::from_be_bytes([payload[position], payload[position + 1]]) as usize;
        let length = u16::from_be_bytes([payload[position + 2], payload[position + 3]]) as usize;
        position += 4;

        if token == 0x01 {
            if length == 0 {
                return Err(DataError::protocol("an empty PRELOGIN ENCRYPTION option"));
            }
            let value = *payload
                .get(offset)
                .ok_or_else(|| DataError::protocol("a PRELOGIN option offset out of range"))?;
            return Ok(value);
        }
    }

    Err(DataError::protocol(
        "the PRELOGIN reply carries no ENCRYPTION option",
    ))
}

pub(crate) fn login7_payload(
    options: &ConnectOptions,
    packet_size: usize,
) -> Result<Vec<u8>, DataError> {
    let host = host_name();
    let mut strings: Vec<u8> = Vec::new();

    let (host_offset, host_length) = push_field(&mut strings, &host);
    let (user_offset, user_length) = push_field(&mut strings, &options.user);

    let password_offset = (LOGIN7_FIXED_LEN + strings.len()) as u16;
    // cchPassword counts UTF-16 characters, not bytes (MS-TDS 2.2.6.4); the
    // obfuscated password buffer itself is still two bytes per character.
    let password_length = options.password.encode_utf16().count() as u16;
    for unit in options.password.encode_utf16() {
        let bytes = unit.to_le_bytes();
        strings.push(swap_nibbles(bytes[0]) ^ 0xA5);
        strings.push(swap_nibbles(bytes[1]) ^ 0xA5);
    }

    let (app_offset, app_length) = push_field(&mut strings, "zhs");
    let (server_offset, server_length) = push_field(&mut strings, &options.host);

    let extension_offset = (LOGIN7_FIXED_LEN + strings.len()) as u16;

    let (int_offset, int_length) = push_field(&mut strings, "zhs");
    let language_offset = (LOGIN7_FIXED_LEN + strings.len()) as u16;
    let (database_offset, database_length) = match &options.database {
        Some(database) => push_field(&mut strings, database),
        None => ((LOGIN7_FIXED_LEN + strings.len()) as u16, 0),
    };

    let total = LOGIN7_FIXED_LEN + strings.len();
    if total > u16::MAX as usize {
        return Err(DataError::query("the LOGIN7 packet exceeds 64 KiB"));
    }

    let mut body = vec![0u8; LOGIN7_FIXED_LEN];
    body[0..4].copy_from_slice(&(total as u32).to_le_bytes());
    body[4..8].copy_from_slice(&0x7400_0004u32.to_le_bytes());
    body[8..12].copy_from_slice(&(packet_size as u32).to_le_bytes());
    body[12..16].copy_from_slice(&CLIENT_PROG_VER.to_le_bytes());
    body[16..20].copy_from_slice(&std::process::id().to_le_bytes());

    body[24] = 0xE0;
    body[25] = 0x02;
    body[26] = 0x00;
    body[27] = 0x00;
    body[32..36].copy_from_slice(&CLIENT_LCID.to_le_bytes());

    put_pair(&mut body, 36, host_offset, host_length);
    put_pair(&mut body, 40, user_offset, user_length);
    put_pair(&mut body, 44, password_offset, password_length);
    put_pair(&mut body, 48, app_offset, app_length);
    put_pair(&mut body, 52, server_offset, server_length);
    put_pair(&mut body, 56, extension_offset, 0);
    put_pair(&mut body, 60, int_offset, int_length);
    put_pair(&mut body, 64, language_offset, 0);
    put_pair(&mut body, 68, database_offset, database_length);

    body[72..78].copy_from_slice(&[0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);

    put_pair(&mut body, 78, extension_offset, 0);
    put_pair(&mut body, 82, extension_offset, 0);
    put_pair(&mut body, 86, extension_offset, 0);

    body.extend_from_slice(&strings);
    Ok(body)
}

fn push_field(strings: &mut Vec<u8>, text: &str) -> (u16, u16) {
    let offset = (LOGIN7_FIXED_LEN + strings.len()) as u16;
    let length = text.encode_utf16().count() as u16;
    push_utf16(strings, text);
    (offset, length)
}

fn put_pair(body: &mut [u8], position: usize, offset: u16, length: u16) {
    body[position..position + 2].copy_from_slice(&offset.to_le_bytes());
    body[position + 2..position + 4].copy_from_slice(&length.to_le_bytes());
}

fn host_name() -> String {
    let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "zhs".to_owned());
    let filtered: String = name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .take(64)
        .collect();

    if filtered.is_empty() {
        "zhs".to_owned()
    } else {
        filtered
    }
}

fn swap_nibbles(byte: u8) -> u8 {
    byte.rotate_left(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> ConnectOptions {
        ConnectOptions::new("127.0.0.1", 1433, "sa", "s3cret").database("app")
    }

    #[test]
    fn prelogin_advertises_no_encryption() {
        let payload = prelogin_payload();
        assert_eq!(payload[0], 0x00);
        assert_eq!(payload[5], 0x01);
        assert_eq!(payload[payload.len() - 1], 0x00);
        assert_eq!(encryption_level(&payload).unwrap(), ENCRYPT_NOT_SUP);
    }

    #[test]
    fn parses_the_encryption_option_of_a_reply() {
        let mut reply = vec![0x01];
        reply.extend_from_slice(&6u16.to_be_bytes());
        reply.extend_from_slice(&1u16.to_be_bytes());
        reply.push(0xFF);
        reply.push(ENCRYPT_REQ);

        assert_eq!(encryption_level(&reply).unwrap(), ENCRYPT_REQ);
    }

    #[test]
    fn login7_offsets_point_inside_the_packet() {
        let payload = login7_payload(&options(), 4096).unwrap();
        let total = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;

        assert_eq!(total, payload.len());
        assert_eq!(payload[4..8], 0x7400_0004u32.to_le_bytes());

        for pair in [36usize, 40, 44, 48, 52, 60, 68] {
            let offset = u16::from_le_bytes([payload[pair], payload[pair + 1]]) as usize;
            let length = u16::from_le_bytes([payload[pair + 2], payload[pair + 3]]) as usize;
            assert!(offset <= total, "offset {offset} out of range");
            assert!(offset + length * 2 <= total, "field overruns the packet");
        }

        // cchPassword is a character count; the password buffer ends where the
        // app-name field begins.
        let password_offset = u16::from_le_bytes([payload[44], payload[45]]) as usize;
        let password_length = u16::from_le_bytes([payload[46], payload[47]]) as usize;
        let app_offset = u16::from_le_bytes([payload[48], payload[49]]) as usize;
        assert_eq!(password_offset + password_length * 2, app_offset);
        assert_eq!(password_length, "s3cret".encode_utf16().count());

        let plain: Vec<u8> = "s3cret"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert!(!payload.windows(plain.len()).any(|window| window == plain));
    }

    #[test]
    fn login7_omits_the_database_when_not_configured() {
        let mut options = options();
        options.database = None;
        let payload = login7_payload(&options, 4096).unwrap();
        let length = u16::from_le_bytes([payload[70], payload[71]]);
        assert_eq!(length, 0);
    }

    #[test]
    fn obfuscation_swaps_nibbles_and_xors() {
        assert_eq!(swap_nibbles(0x73) ^ 0xA5, 0x92);
        assert_eq!(swap_nibbles(0x61) ^ 0xA5, 0xB3);
    }
}
