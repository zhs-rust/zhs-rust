use zhs_crypto::{base64, hmac, pbkdf2, sha256};
use zhs_entity::DataError;

pub(crate) struct ClientFirst {
    pub(crate) bare: String,

    pub(crate) message: String,
}

pub(crate) struct ClientFinal {
    pub(crate) message: String,

    pub(crate) server_signature: [u8; 32],
}

pub(crate) fn client_first(user: &str, nonce: &str) -> ClientFirst {
    let bare = format!("n={user},r={nonce}");
    ClientFirst {
        bare: bare.clone(),
        message: format!("n,,{bare}"),
    }
}

pub(crate) fn client_final(
    password: &str,
    client_nonce: &str,
    client_bare: &str,
    server_first: &str,
    iterations_limit: u32,
) -> Result<ClientFinal, DataError> {
    let mut nonce = None;
    let mut salt = None;
    let mut iterations = None;

    for part in server_first.split(',') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        match key {
            "r" => nonce = Some(value.to_owned()),
            "s" => salt = Some(value.to_owned()),
            "i" => {
                iterations = Some(value.parse::<u32>().map_err(|_| {
                    DataError::protocol(format!("SCRAM iteration count `{value}` is not a number"))
                })?);
            }
            _ => {}
        }
    }

    let nonce = nonce.ok_or_else(|| DataError::protocol("server-first has no nonce"))?;
    let salt = salt.ok_or_else(|| DataError::protocol("server-first has no salt"))?;
    let iterations =
        iterations.ok_or_else(|| DataError::protocol("server-first has no iteration count"))?;

    if !nonce.starts_with(client_nonce) {
        return Err(DataError::protocol(
            "SCRAM server nonce does not extend the client nonce",
        ));
    }
    if iterations == 0 || iterations > iterations_limit {
        return Err(DataError::protocol(format!(
            "SCRAM iteration count {iterations} is outside the accepted range"
        )));
    }

    let salt = base64::decode(&salt)
        .map_err(|error| DataError::protocol(format!("SCRAM salt is not base64: {error}")))?;

    let mut salted = [0_u8; 32];
    pbkdf2::sha256(password.as_bytes(), &salt, iterations, &mut salted)
        .map_err(|error| DataError::protocol(format!("SCRAM key derivation failed: {error}")))?;

    let client_key = hmac::sha256(&salted, b"Client Key");
    let stored_key = sha256::digest(&client_key);

    let without_proof = format!("c=biws,r={nonce}");
    let auth_message = format!("{client_bare},{server_first},{without_proof}");

    let client_signature = hmac::sha256(&stored_key, auth_message.as_bytes());
    let mut proof = [0_u8; 32];
    for (index, byte) in proof.iter_mut().enumerate() {
        *byte = client_key[index] ^ client_signature[index];
    }

    let server_key = hmac::sha256(&salted, b"Server Key");
    let server_signature = hmac::sha256(&server_key, auth_message.as_bytes());

    Ok(ClientFinal {
        message: format!("{without_proof},p={}", base64::encode(&proof)),
        server_signature,
    })
}

pub(crate) fn server_signature(message: &str) -> Result<[u8; 32], DataError> {
    for part in message.split(',') {
        if let Some(value) = part.strip_prefix("v=") {
            let bytes = base64::decode(value).map_err(|error| {
                DataError::protocol(format!("SCRAM server signature is not base64: {error}"))
            })?;
            if bytes.len() != 32 {
                return Err(DataError::protocol(
                    "SCRAM server signature has the wrong length",
                ));
            }
            let mut signature = [0_u8; 32];
            signature.copy_from_slice(&bytes);
            return Ok(signature);
        }
        if let Some(error) = part.strip_prefix("e=") {
            return Err(DataError::protocol(format!("SCRAM server error: {error}")));
        }
    }

    Err(DataError::protocol("server-final has no signature"))
}

#[cfg(test)]
mod tests {
    use super::{client_final, client_first, server_signature};
    use zhs_crypto::base64;

    #[test]
    fn matches_rfc_7677_test_vector() {
        let client_nonce = "rOprNGfwEbeRWgbNEkqO";
        let server_first = "r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,\
                            s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096";

        let first = client_first("user", client_nonce);
        assert_eq!(first.message, "n,,n=user,r=rOprNGfwEbeRWgbNEkqO");

        let last = client_final("pencil", client_nonce, &first.bare, server_first, 1_000_000)
            .expect("client-final");
        assert_eq!(
            last.message,
            "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,\
             p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ="
        );

        let final_message = "v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=";
        let signature = server_signature(final_message).expect("server signature");
        assert_eq!(signature, last.server_signature);
    }

    #[test]
    fn rejects_a_shortened_nonce_and_bad_iterations() {
        let server_first = "r=abc,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096";
        assert!(
            client_final("pencil", "zzz", "n=user,r=zzz", server_first, 1_000_000).is_err(),
            "nonce must extend the client nonce"
        );

        let server_first = "r=rOprNGfwEbeRWgbNEkqO,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=0";
        assert!(
            client_final(
                "pencil",
                "rOprNGfwEbeRWgbNEkqO",
                "n=user,r=x",
                server_first,
                1_000_000
            )
            .is_err()
        );
    }

    #[test]
    fn reports_server_errors() {
        assert!(server_signature("e=invalid-proof").is_err());
        assert!(server_signature("v=not-base64!").is_err());
        assert!(server_signature("").is_err());
    }

    #[test]
    fn base64_round_trips_the_signature() {
        let signature = [7_u8; 32];
        let text = base64::encode(&signature);
        assert_eq!(base64::decode(&text).unwrap(), signature);
    }
}
