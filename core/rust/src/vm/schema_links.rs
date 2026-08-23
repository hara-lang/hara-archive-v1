//! HBC1 exact identified-schema links layered over canonical HBC0 programs.
//!
//! The first identified-schema artifact epoch is external-link-only. The
//! structural program remains one unchanged HBC0 artifact; HBC1 authenticates
//! that artifact together with a canonical vector of exact schema coordinates.
//! Runtime installation must resolve those coordinates through an admitted
//! catalog and must never fall back to an unpinned latest schema.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::vm::artifact::{decode_program, encode_program};
use crate::vm::program::Program;

const MAGIC: &[u8; 4] = b"HBC1";
const DIGEST_BYTES: usize = 32;
const HASH_PREFIX: &str = "sha256:";

/// One immutable identified-schema coordinate used by an HBC1 artifact.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaCoordinate {
    pub id: String,
    pub version: u32,
    pub hash: String,
}

impl SchemaCoordinate {
    pub fn new(
        id: impl Into<String>,
        version: u32,
        hash: impl Into<String>,
    ) -> Result<Self, String> {
        let value = Self {
            id: id.into(),
            version,
            hash: hash.into(),
        };
        validate_coordinate(&value)?;
        Ok(value)
    }
}

/// A decoded HBC1 program plus every exact external schema dependency.
#[derive(Debug, Clone)]
pub struct LinkedProgram {
    pub program: Program,
    pub schema_links: Vec<SchemaCoordinate>,
}

/// Encodes an unchanged HBC0 program and exact linked schema coordinates as
/// one checksummed HBC1 artifact.
pub fn encode_linked_program(
    program: &Program,
    schema_links: &[SchemaCoordinate],
) -> Result<Vec<u8>, String> {
    let schema_links = canonical_links(schema_links)?;
    let nested = encode_program(program)?;
    let mut payload = Writer::default();
    payload.bytes(&nested)?;
    payload.len(schema_links.len())?;
    for coordinate in &schema_links {
        write_coordinate(&mut payload, coordinate)?;
    }
    encode_envelope(&payload.bytes)
}

/// Decodes and authenticates HBC1 without resolving any catalog alias or
/// tooling-oriented latest-version view.
pub fn decode_linked_program(bytes: &[u8]) -> Result<LinkedProgram, String> {
    let payload = decode_envelope(bytes)?;
    let mut reader = Reader::new(payload);
    let nested = reader.bytes()?;
    let program = decode_program(nested)?;
    let schema_links = reader.many(read_coordinate)?;
    reader.finish()?;
    let canonical = canonical_links(&schema_links)?;
    if canonical != schema_links {
        return Err("linked bytecode artifact has non-canonical schema link order".into());
    }
    Ok(LinkedProgram {
        program,
        schema_links,
    })
}

fn canonical_links(schema_links: &[SchemaCoordinate]) -> Result<Vec<SchemaCoordinate>, String> {
    let mut values = schema_links.to_vec();
    values.sort();
    let mut identities = BTreeMap::<(String, u32), String>::new();
    for coordinate in &values {
        validate_coordinate(coordinate)?;
        let identity = (coordinate.id.clone(), coordinate.version);
        if let Some(existing) = identities.insert(identity, coordinate.hash.clone()) {
            if existing == coordinate.hash {
                return Err("linked bytecode artifact contains duplicate schema coordinate".into());
            }
            return Err("linked bytecode artifact contains conflicting schema identity".into());
        }
    }
    Ok(values)
}

fn validate_coordinate(coordinate: &SchemaCoordinate) -> Result<(), String> {
    validate_id(&coordinate.id)?;
    hash_bytes(&coordinate.hash)?;
    Ok(())
}

fn validate_id(id: &str) -> Result<(), String> {
    let mut parts = id.split('/');
    let namespace = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if namespace.is_empty()
        || name.is_empty()
        || parts.next().is_some()
        || id.starts_with(':')
        || id.chars().any(char::is_whitespace)
    {
        return Err("linked bytecode schema id must be a qualified keyword name".into());
    }
    Ok(())
}

fn hash_bytes(hash: &str) -> Result<[u8; DIGEST_BYTES], String> {
    let Some(digest) = hash.strip_prefix(HASH_PREFIX) else {
        return Err("linked bytecode schema hash must use sha256".into());
    };
    if digest.len() != DIGEST_BYTES * 2
        || !digest
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
    {
        return Err("linked bytecode schema hash must be canonical lowercase hex".into());
    }
    let mut output = [0u8; DIGEST_BYTES];
    for (index, byte) in output.iter_mut().enumerate() {
        let offset = index * 2;
        *byte = u8::from_str_radix(&digest[offset..offset + 2], 16)
            .map_err(|_| "linked bytecode schema hash is invalid")?;
    }
    Ok(output)
}

fn display_hash(bytes: &[u8; DIGEST_BYTES]) -> String {
    use std::fmt::Write;

    let mut output = String::with_capacity(HASH_PREFIX.len() + DIGEST_BYTES * 2);
    output.push_str(HASH_PREFIX);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn write_coordinate(out: &mut Writer, coordinate: &SchemaCoordinate) -> Result<(), String> {
    out.string(&coordinate.id)?;
    out.u32(coordinate.version);
    out.raw(&hash_bytes(&coordinate.hash)?);
    Ok(())
}

fn read_coordinate(reader: &mut Reader<'_>) -> Result<SchemaCoordinate, String> {
    let id = reader.string()?;
    let version = reader.u32()?;
    let digest: [u8; DIGEST_BYTES] = reader
        .take(DIGEST_BYTES)?
        .try_into()
        .expect("fixed digest length");
    SchemaCoordinate::new(id, version, display_hash(&digest))
}

fn encode_envelope(payload: &[u8]) -> Result<Vec<u8>, String> {
    let digest = Sha256::digest(payload);
    let mut output = MAGIC.to_vec();
    output.extend_from_slice(
        &u32::try_from(payload.len())
            .map_err(|_| "linked bytecode artifact is too large")?
            .to_be_bytes(),
    );
    output.extend_from_slice(payload);
    output.extend_from_slice(&digest);
    Ok(output)
}

fn decode_envelope(bytes: &[u8]) -> Result<&[u8], String> {
    if !bytes.starts_with(MAGIC) {
        return Err("linked bytecode artifact has invalid magic".into());
    }
    if bytes.len() < 8 + DIGEST_BYTES {
        return Err("linked bytecode artifact is truncated".into());
    }
    let payload_len = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let payload_end = 8usize
        .checked_add(payload_len)
        .ok_or("linked bytecode artifact length overflow")?;
    if payload_end.checked_add(DIGEST_BYTES) != Some(bytes.len()) {
        return Err("linked bytecode artifact length mismatch".into());
    }
    let payload = &bytes[8..payload_end];
    if Sha256::digest(payload)[..] != bytes[payload_end..] {
        return Err("linked bytecode artifact checksum mismatch".into());
    }
    Ok(payload)
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn raw(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    fn u32(&mut self, value: u32) {
        self.raw(&value.to_be_bytes());
    }

    fn len(&mut self, value: usize) -> Result<(), String> {
        self.u32(u32::try_from(value).map_err(|_| "linked bytecode field is too large")?);
        Ok(())
    }

    fn bytes(&mut self, value: &[u8]) -> Result<(), String> {
        self.len(value.len())?;
        self.raw(value);
        Ok(())
    }

    fn string(&mut self, value: &str) -> Result<(), String> {
        self.bytes(value.as_bytes())
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn take(&mut self, size: usize) -> Result<&'a [u8], String> {
        let end = self
            .cursor
            .checked_add(size)
            .ok_or("linked bytecode artifact length overflow")?;
        if end > self.bytes.len() {
            return Err("linked bytecode artifact is truncated".into());
        }
        let value = &self.bytes[self.cursor..end];
        self.cursor = end;
        Ok(value)
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let size = self.u32()? as usize;
        self.take(size)
    }

    fn string(&mut self) -> Result<String, String> {
        String::from_utf8(self.bytes()?.to_vec())
            .map_err(|_| "linked bytecode artifact contains invalid UTF-8".into())
    }

    fn many<T>(
        &mut self,
        mut read: impl FnMut(&mut Reader<'a>) -> Result<T, String>,
    ) -> Result<Vec<T>, String> {
        let size = self.u32()? as usize;
        let mut values = Vec::with_capacity(size.min(4096));
        for _ in 0..size {
            values.push(read(self)?);
        }
        Ok(values)
    }

    fn finish(&self) -> Result<(), String> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err("linked bytecode artifact has trailing payload bytes".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::compile_source;

    fn coordinate(id: &str, version: u32, digit: char) -> SchemaCoordinate {
        SchemaCoordinate::new(
            id,
            version,
            format!("sha256:{}", digit.to_string().repeat(64)),
        )
        .unwrap()
    }

    #[test]
    fn exact_schema_links_round_trip_canonically() {
        let program = compile_source("(+ 19 23)").unwrap();
        let account = coordinate("model/account", 2, '2');
        let identifier = coordinate("model/id", 1, '1');
        let encoded =
            encode_linked_program(&program, &[identifier.clone(), account.clone()]).unwrap();
        assert!(encoded.starts_with(b"HBC1"));
        let decoded = decode_linked_program(&encoded).unwrap();
        assert_eq!(decoded.schema_links, vec![account, identifier]);
        assert_eq!(
            encode_program(&decoded.program).unwrap(),
            encode_program(&program).unwrap()
        );
        assert_eq!(
            encode_linked_program(&decoded.program, &decoded.schema_links).unwrap(),
            encoded
        );
    }

    #[test]
    fn duplicate_and_conflicting_schema_links_are_rejected() {
        let program = compile_source("42").unwrap();
        let first = coordinate("model/id", 1, '1');
        assert_eq!(
            encode_linked_program(&program, &[first.clone(), first.clone()]).unwrap_err(),
            "linked bytecode artifact contains duplicate schema coordinate"
        );
        let conflicting = coordinate("model/id", 1, '2');
        assert_eq!(
            encode_linked_program(&program, &[first, conflicting]).unwrap_err(),
            "linked bytecode artifact contains conflicting schema identity"
        );
    }

    #[test]
    fn malformed_coordinates_are_rejected_before_encoding() {
        assert!(
            SchemaCoordinate::new("unqualified", 1, format!("sha256:{}", "1".repeat(64)),)
                .unwrap_err()
                .contains("qualified keyword name")
        );
        assert!(SchemaCoordinate::new("model/id", 1, "sha256:BAD")
            .unwrap_err()
            .contains("canonical lowercase hex"));
    }

    #[test]
    fn corruption_is_rejected_before_nested_program_decode() {
        let program = compile_source("42").unwrap();
        let mut encoded =
            encode_linked_program(&program, &[coordinate("model/id", 1, '1')]).unwrap();
        encoded[12] ^= 1;
        assert_eq!(
            decode_linked_program(&encoded).unwrap_err(),
            "linked bytecode artifact checksum mismatch"
        );
    }

    #[test]
    fn hbc0_is_not_silently_treated_as_a_linked_artifact() {
        let program = compile_source("42").unwrap();
        let encoded = encode_program(&program).unwrap();
        assert_eq!(
            decode_linked_program(&encoded).unwrap_err(),
            "linked bytecode artifact has invalid magic"
        );
    }
}
