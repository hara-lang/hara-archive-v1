use super::{PMap, Value};
use ed25519_dalek::{Signature as Ed25519Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hmac::{Hmac, Mac};
use p256::ecdh::diffie_hellman;
use p256::ecdsa::{Signature as P256Signature, SigningKey as P256SigningKey};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey as P256PublicKey, SecretKey as P256SecretKey};
use sha2::{Digest, Sha256, Sha512};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret as X25519Secret};

const MAX_RANDOM_BYTES: usize = 1_048_576;

pub(super) fn operation(operation: &str, arguments: Vec<Value>) -> Result<Value, String> {
    match operation {
        "sha256" => {
            arity(operation, &arguments, 1)?;
            Ok(Value::String(hex(&Sha256::digest(bytes(
                operation,
                &arguments[0],
            )?))))
        }
        "sha512" => {
            arity(operation, &arguments, 1)?;
            Ok(Value::String(hex(&Sha512::digest(bytes(
                operation,
                &arguments[0],
            )?))))
        }
        "hmac-sha256" => hmac_sha256(operation, &arguments),
        "hmac-sha512" => hmac_sha512(operation, &arguments),
        "random-bytes" => random_bytes(operation, &arguments),
        "secure-equal?" => secure_equal(operation, &arguments),
        "ed25519-keypair" => ed25519_keypair(operation, &arguments),
        "ed25519-public" => ed25519_public(operation, &arguments),
        "ed25519-sign" => ed25519_sign(operation, &arguments),
        "ed25519-verify" => ed25519_verify(operation, &arguments),
        "x25519-keypair" => x25519_keypair(operation, &arguments),
        "x25519-public" => x25519_public(operation, &arguments),
        "x25519-shared" => x25519_shared(operation, &arguments),
        "p256-keypair" => p256_keypair(operation, &arguments),
        "p256-public" => p256_public(operation, &arguments),
        "p256-sign" => p256_sign(operation, &arguments),
        "p256-verify" => p256_verify(operation, &arguments),
        "p256-shared" => p256_shared(operation, &arguments),
        _ => Err(error(operation, "is not implemented")),
    }
}

fn hmac_sha256(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 2)?;
    let key = bytes(operation, &arguments[0])?;
    let message = bytes(operation, &arguments[1])?;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key)
        .map_err(|_| error(operation, "expects a valid key"))?;
    mac.update(&message);
    Ok(Value::String(hex(&mac.finalize().into_bytes())))
}

fn hmac_sha512(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 2)?;
    let key = bytes(operation, &arguments[0])?;
    let message = bytes(operation, &arguments[1])?;
    let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(&key)
        .map_err(|_| error(operation, "expects a valid key"))?;
    mac.update(&message);
    Ok(Value::String(hex(&mac.finalize().into_bytes())))
}

fn random_bytes(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 1)?;
    let Value::Number(size) = arguments[0] else {
        return Err(error(operation, "expects an integer size"));
    };
    let size = usize::try_from(size)
        .ok()
        .filter(|size| *size <= MAX_RANDOM_BYTES)
        .ok_or_else(|| {
            error(
                operation,
                &format!("size must be between 0 and {MAX_RANDOM_BYTES}"),
            )
        })?;
    Ok(Value::Bytes(random(operation, size)?))
}

fn secure_equal(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 2)?;
    let left = bytes(operation, &arguments[0])?;
    let right = bytes(operation, &arguments[1])?;
    if left.len() != right.len() {
        return Ok(Value::Bool(false));
    }
    let difference = left
        .iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        });
    Ok(Value::Bool(difference == 0))
}

fn ed25519_keypair(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 0)?;
    let seed = fixed_random::<32>(operation)?;
    let signing = SigningKey::from_bytes(&seed);
    Ok(keypair(
        seed.to_vec(),
        signing.verifying_key().to_bytes().to_vec(),
    ))
}

fn ed25519_public(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 1)?;
    let seed = fixed::<32>(operation, &arguments[0])?;
    Ok(Value::Bytes(
        SigningKey::from_bytes(&seed)
            .verifying_key()
            .to_bytes()
            .to_vec(),
    ))
}

fn ed25519_sign(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 2)?;
    let seed = fixed::<32>(operation, &arguments[0])?;
    let signature: Ed25519Signature =
        SigningKey::from_bytes(&seed).sign(&bytes(operation, &arguments[1])?);
    Ok(Value::Bytes(signature.to_bytes().to_vec()))
}

fn ed25519_verify(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 3)?;
    let public = fixed::<32>(operation, &arguments[0])?;
    let verifying = VerifyingKey::from_bytes(&public)
        .map_err(|_| error(operation, "expects a valid Ed25519 public key"))?;
    let signature_bytes = bytes(operation, &arguments[2])?;
    let Ok(signature) = Ed25519Signature::from_slice(&signature_bytes) else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(
        verifying
            .verify_strict(&bytes(operation, &arguments[1])?, &signature)
            .is_ok(),
    ))
}

fn x25519_keypair(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 0)?;
    let private = fixed_random::<32>(operation)?;
    let secret = X25519Secret::from(private);
    Ok(keypair(
        private.to_vec(),
        X25519PublicKey::from(&secret).as_bytes().to_vec(),
    ))
}

fn x25519_public(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 1)?;
    let secret = X25519Secret::from(fixed::<32>(operation, &arguments[0])?);
    Ok(Value::Bytes(
        X25519PublicKey::from(&secret).as_bytes().to_vec(),
    ))
}

fn x25519_shared(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 2)?;
    let secret = X25519Secret::from(fixed::<32>(operation, &arguments[0])?);
    let public = X25519PublicKey::from(fixed::<32>(operation, &arguments[1])?);
    let shared = secret.diffie_hellman(&public);
    if shared.as_bytes().iter().all(|byte| *byte == 0) {
        return Err(error(
            operation,
            "peer public key produces an invalid shared secret",
        ));
    }
    Ok(Value::Bytes(shared.as_bytes().to_vec()))
}

fn p256_keypair(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 0)?;
    loop {
        let private = fixed_random::<32>(operation)?;
        if let Ok(secret) = P256SecretKey::from_slice(&private) {
            let public = secret.public_key().to_encoded_point(true);
            return Ok(keypair(private.to_vec(), public.as_bytes().to_vec()));
        }
    }
}

fn p256_public(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 1)?;
    let secret = p256_private(operation, &arguments[0])?;
    Ok(Value::Bytes(
        secret
            .public_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    ))
}

fn p256_sign(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 2)?;
    let secret = p256_private(operation, &arguments[0])?;
    let signing = P256SigningKey::from(secret);
    let signature: P256Signature = signing.sign(&bytes(operation, &arguments[1])?);
    Ok(Value::Bytes(signature.to_bytes().to_vec()))
}

fn p256_verify(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 3)?;
    let public_bytes = bytes(operation, &arguments[0])?;
    if public_bytes.len() != 33 {
        return Err(error(operation, "expects a compressed 33-byte public key"));
    }
    let verifying = p256::ecdsa::VerifyingKey::from_sec1_bytes(&public_bytes)
        .map_err(|_| error(operation, "expects a valid P-256 public key"))?;
    let signature_bytes = bytes(operation, &arguments[2])?;
    let Ok(signature) = P256Signature::from_slice(&signature_bytes) else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(
        verifying
            .verify(&bytes(operation, &arguments[1])?, &signature)
            .is_ok(),
    ))
}

fn p256_shared(operation: &str, arguments: &[Value]) -> Result<Value, String> {
    arity(operation, arguments, 2)?;
    let secret = p256_private(operation, &arguments[0])?;
    let public_bytes = bytes(operation, &arguments[1])?;
    if public_bytes.len() != 33 {
        return Err(error(operation, "expects a compressed 33-byte public key"));
    }
    let public = P256PublicKey::from_sec1_bytes(&public_bytes)
        .map_err(|_| error(operation, "expects a valid P-256 public key"))?;
    let shared = diffie_hellman(secret.to_nonzero_scalar(), public.as_affine());
    Ok(Value::Bytes(shared.raw_secret_bytes().to_vec()))
}

fn p256_private(operation: &str, value: &Value) -> Result<P256SecretKey, String> {
    P256SecretKey::from_slice(&bytes(operation, value)?)
        .map_err(|_| error(operation, "expects a valid 32-byte P-256 private key"))
}

fn keypair(private: Vec<u8>, public: Vec<u8>) -> Value {
    Value::Map(PMap::from_iter([
        (Value::Keyword("private".into()), Value::Bytes(private)),
        (Value::Keyword("public".into()), Value::Bytes(public)),
    ]))
}

fn fixed<const SIZE: usize>(operation: &str, value: &Value) -> Result<[u8; SIZE], String> {
    bytes(operation, value)?
        .try_into()
        .map_err(|_| error(operation, &format!("expects {SIZE} bytes")))
}

fn fixed_random<const SIZE: usize>(operation: &str) -> Result<[u8; SIZE], String> {
    random(operation, SIZE)?
        .try_into()
        .map_err(|_| error(operation, "failed to allocate random bytes"))
}

fn random(operation: &str, size: usize) -> Result<Vec<u8>, String> {
    #[cfg(feature = "raw-wasm")]
    {
        let _ = size;
        return Err(error(operation, "secure randomness is unavailable"));
    }
    #[cfg(not(feature = "raw-wasm"))]
    {
        let mut output = vec![0_u8; size];
        getrandom::getrandom(&mut output)
            .map_err(|_| error(operation, "secure randomness is unavailable"))?;
        Ok(output)
    }
}

fn bytes(operation: &str, value: &Value) -> Result<Vec<u8>, String> {
    match value {
        Value::Bytes(value) => Ok(value.clone()),
        Value::ByteBuffer(value) => Ok(value.borrow().clone()),
        _ => Err(error(operation, "expects bytes")),
    }
}

fn arity(operation: &str, arguments: &[Value], expected: usize) -> Result<(), String> {
    if arguments.len() == expected {
        Ok(())
    } else {
        Err(error(
            operation,
            &format!(
                "expects {expected} {}",
                if expected == 1 {
                    "argument"
                } else {
                    "arguments"
                }
            ),
        ))
    }
}

fn hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn error(operation: &str, message: &str) -> String {
    format!("crypto/{operation}: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes_hex(value: &str) -> Value {
        Value::Bytes(
            value
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect(),
        )
    }

    #[test]
    fn standard_digest_hmac_and_ed25519_vectors_match() {
        assert_eq!(
            operation("sha512", vec![Value::Bytes(b"abc".to_vec())]).unwrap(),
            Value::String(
                concat!(
                    "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a",
                    "2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
                )
                .into()
            )
        );
        assert_eq!(
            operation(
                "hmac-sha256",
                vec![
                    Value::Bytes(b"key".to_vec()),
                    Value::Bytes(b"The quick brown fox jumps over the lazy dog".to_vec()),
                ],
            )
            .unwrap(),
            Value::String(
                "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8".into()
            )
        );

        let seed = bytes_hex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let public = bytes_hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        assert_eq!(
            operation("ed25519-public", vec![seed.clone()]).unwrap(),
            public
        );
        let signature = operation("ed25519-sign", vec![seed, Value::Bytes(vec![])]).unwrap();
        assert_eq!(
            operation(
                "ed25519-verify",
                vec![public, Value::Bytes(vec![]), signature],
            )
            .unwrap(),
            Value::Bool(true)
        );
    }

    #[cfg(not(feature = "raw-wasm"))]
    #[test]
    fn evaluator_dispatches_the_native_crypto_static_object() {
        let mut runtime = crate::Runtime::new();
        let result = runtime
            .eval_text("(Crypto/sha512 (bytes 97 98 99))")
            .unwrap();
        assert_eq!(
            result,
            concat!(
                "\"ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a",
                "2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f\""
            )
        );
    }

    #[test]
    fn generated_curve_keys_sign_verify_and_agree() {
        for curve in ["x25519", "p256"] {
            let alice = operation(&format!("{curve}-keypair"), vec![]).unwrap();
            let bob = operation(&format!("{curve}-keypair"), vec![]).unwrap();
            let (alice_private, alice_public) = keypair_parts(&alice);
            let (bob_private, bob_public) = keypair_parts(&bob);
            let left =
                operation(&format!("{curve}-shared"), vec![alice_private, bob_public]).unwrap();
            let right =
                operation(&format!("{curve}-shared"), vec![bob_private, alice_public]).unwrap();
            assert_eq!(left, right);
            assert_eq!(
                operation("secure-equal?", vec![left, right]).unwrap(),
                Value::Bool(true)
            );
        }

        let pair = operation("p256-keypair", vec![]).unwrap();
        let (private, public) = keypair_parts(&pair);
        let message = Value::Bytes(b"portable p256".to_vec());
        let signature = operation("p256-sign", vec![private, message.clone()]).unwrap();
        assert_eq!(
            operation("p256-verify", vec![public, message, signature]).unwrap(),
            Value::Bool(true)
        );

        let Value::Bytes(random) = operation("random-bytes", vec![Value::Number(64)]).unwrap()
        else {
            panic!("random-bytes must return Bytes");
        };
        assert_eq!(random.len(), 64);
    }

    fn keypair_parts(value: &Value) -> (Value, Value) {
        let Value::Map(entries) = value else {
            panic!("keypair must be a map");
        };
        (
            entries
                .get(&Value::Keyword("private".into()))
                .expect("private key")
                .clone(),
            entries
                .get(&Value::Keyword("public".into()))
                .expect("public key")
                .clone(),
        )
    }
}
