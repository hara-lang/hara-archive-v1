//! Java-parity hashing stack for the hara Rust runtime.
//!
//! This module is the Rust analogue of `hara.lang.base.G` plus the
//! collection hash composition in `hara.lang.data.types.IOrderedType` /
//! `IUnOrderedType` / `IStringType` and `hara.lang.data.Trie`.
//!
//! All hash values are Java `long`/`int` semantics: wrapping two's-complement
//! arithmetic. Functions return `i64` (Java `long`); value-level hashes are
//! Java `int` results sign-extended to `i64`, exactly like `G.hashValue`
//! returning `long`. Collection composition accumulates in full 64 bits.
//!
//! DEFAULT_HASH is RAPID (mirrors `G.DEFAULT_HASH`).
//!
//! Documented deviations from the Java runtime (all cases where the Java
//! behaviour is identity-hash based and therefore non-deterministic across
//! JVM runs):
//!
//! - **Keyword**: Java's `IStringType.hashCalc` uses `toString()`, and
//!   `Keyword` does not override `toString()`, so Java hashes
//!   `"::KEYWORD|hara.lang.data.Keyword@<identity>"` — non-deterministic.
//!   This port standardises on the display form:
//!   `"::KEYWORD|:ns/name"`. (Verified empirically; see
//!   `target/hashdump/HashDump.java` and the normative corpus
//!   `hara-specs-registry/01-lang/020-data-structures/draft/conformance/hash-parity.edn`.)
//! - **Symbol**: Java inherits `ObjPersistent.toString()` which is
//!   `class-name + "<" + display() + ">"` — deterministic but
//!   class-qualified. This port mirrors Java exactly:
//!   `"::SYMBOL|hara.lang.data.Symbol<ns/name>"`.
//! - **Pointer**: Java's `Pointer.hashCalc` is `System.identityHashCode`.
//!   This port uses the deterministic string-type hash of
//!   `"::POINTER|" + as_str()`.
//! - **SYSTEM hash of collections**: Java's `G.hashFn(SYSTEM)` degrades to
//!   `Object.hashCode()` (identity) for collection objects. This port uses
//!   the same structural composition as the other hash types so SYSTEM
//!   stays deterministic.
//! - **SIP**: Java never actually routes SipHash through `G` —
//!   `G.hashSip` degrades to `hashValue` (identity for collections) and
//!   `IStringType.hashCalc(SIP)` returns `-1`. This port returns `-1` for
//!   string types (exact mirror) and uses structural composition for
//!   collections (deterministic fallback).
//! - **Regex / arrays / host objects**: Java hashes `java.util.regex.Pattern`
//!   and Java arrays by identity. This port hashes the pattern string and
//!   uses `java.util.Arrays.hashCode(byte[])` for byte vectors (that one IS
//!   deterministic in Java and mirrored exactly).
//! - **f64 formatting**: `G.hashValue(Double)` goes through
//!   `BigDecimal.valueOf` which is defined by `Double.toString`. This port
//!   uses Rust's `{}` formatting; both produce shortest round-trip digits
//!   and canonicalisation (trailing-zero stripping) normalises notation
//!   differences. One KNOWN divergence: the bottom subnormal family, e.g.
//!   `Double.MIN_VALUE` — Java emits `"4.9E-324"` (hash 1844) while Rust's
//!   formatter picks the 1-digit `"5e-324"` form (hash 479). Both strings
//!   round-trip to the same double; the runtimes' digit-selection rules
//!   simply disagree there. That case is excluded from the parity fixture.

pub mod murmur3;
pub mod rapid;
pub mod siphash;

use crate::lang::protocol::HashType;

/// Mirrors `G.DEFAULT_HASH`.
pub const DEFAULT_HASH: HashType = HashType::Rapid;

/// Rust analogue of `G.hashFn(t).apply(o)`: the Java `long` hash of a value
/// under the given hash type. Implemented for `Value` in `core.rs` and for
/// the plain Java-like primitives here. Collection element types in
/// `lang::data` hash through this trait so the `HashType` dispatches all the
/// way down nested structures.
pub trait JavaHash {
    fn java_hash(&self, hash_type: HashType) -> i64;
}

impl JavaHash for bool {
    fn java_hash(&self, _: HashType) -> i64 {
        hash_bool(*self) as i64
    }
}

impl JavaHash for char {
    fn java_hash(&self, _: HashType) -> i64 {
        hash_char(*self) as i64
    }
}

impl JavaHash for i64 {
    fn java_hash(&self, _: HashType) -> i64 {
        hash_long(*self) as i64
    }
}

impl JavaHash for i32 {
    fn java_hash(&self, _: HashType) -> i64 {
        hash_long(*self as i64) as i64
    }
}

impl JavaHash for usize {
    fn java_hash(&self, _: HashType) -> i64 {
        hash_long(*self as i64) as i64
    }
}

impl JavaHash for u64 {
    fn java_hash(&self, _: HashType) -> i64 {
        hash_long(*self as i64) as i64
    }
}

impl JavaHash for f64 {
    fn java_hash(&self, _: HashType) -> i64 {
        hash_double(*self) as i64
    }
}

/// Plain strings hash identically under every hash type: `G.hashFn(t)`
/// only dispatches on `IHash` objects, everything else takes `hashValue`,
/// which for a Java `String` is `String.hashCode`.
impl JavaHash for String {
    fn java_hash(&self, _: HashType) -> i64 {
        java_string_hash(self) as i64
    }
}

impl JavaHash for &str {
    fn java_hash(&self, _: HashType) -> i64 {
        java_string_hash(self) as i64
    }
}

// ---------------------------------------------------------------------------
// plain values (G.hashValue)
// ---------------------------------------------------------------------------

/// Java `String.hashCode`: 31-polynomial over UTF-16 code units, wrapping i32.
pub fn java_string_hash(s: &str) -> i32 {
    let mut h = 0i32;
    for unit in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(unit as i32);
    }
    h
}

/// Java `String.hashCode` of the `IObjType` hash seed `"::" + obj_name`.
pub fn hash_seed(obj_name: &str) -> i32 {
    java_string_hash(&format!("::{obj_name}"))
}

/// Java `Boolean.hashCode`.
pub fn hash_bool(b: bool) -> i32 {
    if b {
        1231
    } else {
        1237
    }
}

/// Java `Character.hashCode` — the UTF-16 unit as int. BMP chars match
/// exactly; supplementary Rust `char`s hash as their full code point
/// (no single Java `char` equivalent exists).
pub fn hash_char(c: char) -> i32 {
    c as i32
}

/// `java.util.Arrays.hashCode(byte[])` — bytes are sign-extended to int.
pub fn hash_bytes(bytes: &[u8]) -> i32 {
    let mut h = 1i32;
    for b in bytes {
        h = h.wrapping_mul(31).wrapping_add(*b as i8 as i32);
    }
    h
}

/// System hash for a long follows `BigDecimal.hashCode` at scale zero. Unlike
/// the canonical decimal path, integer trailing zeroes remain significant.
pub fn hash_long(n: i64) -> i32 {
    hash_long_placement(n)
}

/// CHAMP placement hash for an integral value. Java's node layout retains the
/// scale-zero representation here, including trailing zeroes, even though
/// value/protocol hashing uses the canonical numeric domain above.
pub fn hash_long_placement(n: i64) -> i32 {
    let text = n.to_string();
    let Some((signum, digits, scale)) = parse_decimal(&text) else {
        return java_string_hash(&text);
    };
    bigdecimal_hash(&digits_to_words_be(&digits), signum, scale as i32)
}

/// `G.hashValue(Double)`:
/// - `0.0` (and `-0.0`) → 0
/// - finite → `canonicalDecimal(BigDecimal.valueOf(d)).hashCode()`
pub fn hash_double(d: f64) -> i32 {
    assert!(d.is_finite(), "non-finite number");
    if d == 0.0 {
        return 0;
    }
    // BigDecimal.valueOf(d) is defined via Double.toString; Rust's `{}` also
    // produces shortest round-trip digits (see module deviation notes).
    canonical_decimal_str_hash(&format!("{d}"))
}

/// `canonicalDecimal(new BigDecimal(string)).hashCode()` — parse a Java
/// BigDecimal/BigInteger grammar string, strip trailing zeros, hash.
/// Malformed input falls back to `java_string_hash` (should not happen for
/// runtime-produced numeric strings).
pub fn canonical_decimal_str_hash(s: &str) -> i32 {
    match parse_decimal(s) {
        Some((signum, digits, scale)) => canonical_decimal_hash(signum, digits, scale),
        None => java_string_hash(s),
    }
}

// ---------------------------------------------------------------------------
// string types (IStringType.hashCalc: hashSeed() + "|" + toString())
// ---------------------------------------------------------------------------

/// Per-type hash of an already-composed string-type hash input
/// (`hashSeed + "|" + display`, or the Java-mirrored Symbol form).
/// Mirrors `IStringType.hashCalc`, including the SIP → -1 case.
pub fn hash_string_type(hash_type: HashType, hashed: &str) -> i64 {
    match hash_type {
        HashType::System => java_string_hash(hashed) as i64,
        HashType::Rapid => rapid::hash(hashed.as_bytes()) as i64,
        HashType::Murmur3 => murmur3::hash_chars(hashed) as i64,
        HashType::Sip => -1,
    }
}

// ---------------------------------------------------------------------------
// collection composition (IOrderedType / IUnOrderedType / Trie)
// ---------------------------------------------------------------------------

/// `IOrderedType.hashCalc`: acc starts at `hashSeed().hashCode()` (widened to
/// long), then `acc = acc * 31 + hash(item)` per item, wrapping i64.
pub fn compose_ordered(obj_name: &str, items: impl IntoIterator<Item = i64>) -> i64 {
    let mut acc = hash_seed(obj_name) as i64;
    for h in items {
        acc = acc.wrapping_mul(31).wrapping_add(h);
    }
    acc
}

/// `IUnOrderedType.hashCalc`: acc starts at the seed, then `acc += hash(item)`
/// per item (order-insensitive sum), wrapping i64.
pub fn compose_unordered(obj_name: &str, items: impl IntoIterator<Item = i64>) -> i64 {
    let mut acc = hash_seed(obj_name) as i64;
    for h in items {
        acc = acc.wrapping_add(h);
    }
    acc
}

/// Hash of a map entry. Java map iterators yield `Tuple.Tup2.L` entries,
/// which are `ISequential` — ordered composition with the
/// `"::SEQUENTIAL"` seed: `(seed * 31 + hk) * 31 + hv`.
pub fn compose_entry(key_hash: i64, value_hash: i64) -> i64 {
    compose_ordered("SEQUENTIAL", [key_hash, value_hash])
}

// ---------------------------------------------------------------------------
// canonical number hashing (BigDecimal.hashCode / BigInteger.hashCode)
// ---------------------------------------------------------------------------

/// `BigInteger.hashCode`: 31-fold over big-endian sign-magnitude u32 words
/// (wrapping i32), times signum.
fn biginteger_hash(words_be: &[u32], signum: i32) -> i32 {
    let mut h = 0i32;
    for w in words_be {
        h = h.wrapping_mul(31).wrapping_add(*w as i32);
    }
    h.wrapping_mul(signum)
}

/// `BigDecimal.hashCode` given the unscaled value as big-endian magnitude
/// words, its signum, and the scale. Reproduces both the compact path
/// (unscaled fits in a Java long) and the inflated path:
///
/// ```java
/// if (intCompact != INFLATED) {
///     long val2 = (intCompact < 0)? -intCompact : intCompact;
///     int temp = (int)( ((int)(val2 >>> 32)) * 31  + (val2 & LONG_MASK));
///     return 31*((intCompact < 0) ?-temp:temp) + scale;
/// } else
///     return 31*intVal.hashCode() + scale;
/// ```
fn bigdecimal_hash(words_be: &[u32], signum: i32, scale: i32) -> i32 {
    if signum == 0 {
        // intCompact == 0: val2 = 0, temp = 0 → 31 * 0 + scale
        return scale;
    }
    let mag: u128 = words_be
        .iter()
        .fold(0u128, |acc, w| (acc << 32) | (*w as u128));
    // intCompact == INFLATED (Long.MIN_VALUE sentinel) exactly when the
    // magnitude does not fit in a non-negative long.
    if mag < (1u128 << 63) {
        let val2 = mag as u64;
        let temp = ((((val2 >> 32) as u32) as i32).wrapping_mul(31) as i64
            + (val2 & 0xffff_ffff) as i64) as i32;
        let signed = if signum < 0 {
            temp.wrapping_neg()
        } else {
            temp
        };
        31i32.wrapping_mul(signed).wrapping_add(scale)
    } else {
        31i32
            .wrapping_mul(biginteger_hash(words_be, signum))
            .wrapping_add(scale)
    }
}

/// Parse a Java BigDecimal-grammar string into (signum, digits without
/// leading zeros, scale). Handles optional sign, decimal point and exponent.
fn parse_decimal(s: &str) -> Option<(i32, Vec<u8>, i64)> {
    let b = s.as_bytes();
    let mut i = 0usize;
    let mut neg = false;
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        neg = b[i] == b'-';
        i += 1;
    }
    let mut digits: Vec<u8> = Vec::new();
    let mut seen_dot = false;
    let mut scale: i64 = 0;
    let mut any_digit = false;
    while i < b.len() {
        match b[i] {
            c @ b'0'..=b'9' => {
                digits.push(c - b'0');
                if seen_dot {
                    scale += 1;
                }
                any_digit = true;
            }
            b'.' if !seen_dot => seen_dot = true,
            b'e' | b'E' => {
                i += 1;
                let mut eneg = false;
                if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
                    eneg = b[i] == b'-';
                    i += 1;
                }
                let mut exp: i64 = 0;
                let mut any_exp = false;
                while i < b.len() && b[i].is_ascii_digit() {
                    exp = exp.saturating_mul(10).saturating_add((b[i] - b'0') as i64);
                    any_exp = true;
                    i += 1;
                }
                if !any_exp {
                    return None;
                }
                scale -= if eneg { -exp } else { exp };
                break;
            }
            _ => return None,
        }
        i += 1;
    }
    if !any_digit {
        return None;
    }
    match digits.iter().position(|d| *d != 0) {
        None => Some((0, vec![0], 0)),
        Some(first) => Some((if neg { -1 } else { 1 }, digits[first..].to_vec(), scale)),
    }
}

/// `NumUtils.normalizeDecimal` + hash: zero → `BigDecimal.ZERO` (hash 0);
/// otherwise strip trailing zeros (each stripped zero decrements the scale)
/// and take `BigDecimal.hashCode`.
fn canonical_decimal_hash(signum: i32, mut digits: Vec<u8>, mut scale: i64) -> i32 {
    if signum == 0 {
        return 0;
    }
    while digits.last() == Some(&0) {
        digits.pop();
        scale -= 1;
    }
    let words = digits_to_words_be(&digits);
    bigdecimal_hash(&words, signum, scale as i32)
}

/// Convert a decimal digit sequence (most significant first) to big-endian
/// u32 magnitude words (no leading zero words).
fn digits_to_words_be(digits: &[u8]) -> Vec<u32> {
    let mut le: Vec<u32> = vec![0];
    for d in digits {
        let mut carry = *d as u64;
        for w in le.iter_mut() {
            let v = (*w as u64) * 10 + carry;
            *w = v as u32;
            carry = v >> 32;
        }
        while carry > 0 {
            le.push(carry as u32);
            carry >>= 32;
        }
    }
    while le.len() > 1 && le.last() == Some(&0) {
        le.pop();
    }
    le.iter().rev().copied().collect()
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Value;
    use crate::kernel::parser::parse_forms;
    use crate::kernel::Form;
    use crate::lang::data::{Keyword, Map as PMap, Set as PSet, Symbol, Tuple as PTuple};
    use crate::lang::protocol::{IDisplay, IHash, IObjType};

    /// Locates a repo-relative file from the crate manifest dir (mirrors the
    /// corpus runners in kernel::parser_tests and vm::conformance_tests).
    fn corpus_path(relative: &str) -> Option<std::path::PathBuf> {
        crate::spec_registry::resolve(relative).filter(|candidate| candidate.is_file())
    }

    fn field<'a>(case: &'a Form, key: &str) -> &'a Form {
        match case {
            Form::Map(entries) => entries
                .iter()
                .find(|(k, _)| matches!(k, Form::Keyword(kw) if kw == key))
                .map(|(_, v)| v)
                .unwrap_or_else(|| panic!("case missing :{key}: {case}")),
            other => panic!("case is not a map: {other}"),
        }
    }

    fn kw_of(form: &Form) -> &str {
        match form {
            Form::Keyword(s) => s,
            other => panic!("expected keyword, got {other}"),
        }
    }

    fn num_of(form: &Form) -> i64 {
        match form {
            Form::Number(n) => *n,
            other => panic!("expected number, got {other}"),
        }
    }

    fn str_of(form: &Form) -> &str {
        match form {
            Form::String(s) => s,
            other => panic!("expected string, got {other}"),
        }
    }

    fn hash_type(id: &str) -> HashType {
        match id {
            "system" => HashType::System,
            "rapid" => HashType::Rapid,
            "murmur3" => HashType::Murmur3,
            "sip" => HashType::Sip,
            other => panic!("unknown hash type: {other}"),
        }
    }

    /// Converts a corpus EDN element form to a runtime `Value` (collection
    /// elements only: scalars and nested vector/map/set/list).
    fn element_value(form: &Form) -> Value {
        match form {
            Form::Nil => Value::Nil,
            Form::Bool(b) => Value::Bool(*b),
            Form::Number(n) => Value::Number(*n),
            Form::Float(f) => Value::Float(*f),
            Form::String(s) => Value::String(s.clone().into()),
            Form::Vector(items) => Value::Vector(items.iter().map(element_value).collect()),
            Form::List(items) => Value::List(items.iter().map(element_value).collect()),
            Form::Map(pairs) => Value::Map(
                pairs
                    .iter()
                    .map(|(k, v)| (element_value(k), element_value(v)))
                    .collect::<PMap<Value, Value>>(),
            ),
            Form::Set(items) => {
                Value::Set(items.iter().map(element_value).collect::<PSet<Value>>())
            }
            other => panic!("unsupported collection element: {other}"),
        }
    }

    /// Builds the corpus collection for a `:kind :collection` case, using
    /// `:structure` to interpret the input form (queue and tuple2 inputs are
    /// vector-encoded).
    fn collection_value(structure: &str, input: &Form) -> Value {
        match structure {
            "vector" | "list" | "map" | "set" => element_value(input),
            "queue" => match input {
                Form::Vector(items) => {
                    Value::Queue(Box::new(items.iter().map(element_value).collect()))
                }
                other => panic!("queue input must be a vector: {other}"),
            },
            "tuple2" => match input {
                Form::Vector(items) if items.len() == 2 => Value::Tuple(Box::new(
                    PTuple::from_values(items.iter().map(element_value).collect()).unwrap(),
                )),
                other => panic!("tuple2 input must be a 2-vector: {other}"),
            },
            other => panic!("unknown collection structure: {other}"),
        }
    }

    fn eval_case(case: &Form) -> i64 {
        let hash = kw_of(field(case, "hash"));
        let kind = kw_of(field(case, "kind"));
        let input = field(case, "input");
        match kind {
            "string" => {
                let s = str_of(input);
                match hash {
                    "rapid" => rapid::hash(s.as_bytes()) as i64,
                    "murmur3" => murmur3::hash_chars(s) as i64,
                    "sip" => siphash::hash(&siphash::HARA, s.as_bytes()) as i64,
                    "system" => java_string_hash(s) as i64,
                    other => panic!("unknown string hash type: {other}"),
                }
            }
            "int" => murmur3::hash_int(num_of(input) as i32) as i64,
            "long" => match hash {
                "murmur3" => murmur3::hash_long(num_of(input)) as i64,
                "system" => hash_long(num_of(input)) as i64,
                other => panic!("unknown long hash type: {other}"),
            },
            "double" => match input {
                Form::Float(f) => hash_double(*f) as i64,
                other => panic!("double input must be a float: {other}"),
            },
            "bigint" => canonical_decimal_str_hash(str_of(input)) as i64,
            "bool" => match input {
                Form::Bool(b) => hash_bool(*b) as i64,
                other => panic!("bool input must be a boolean: {other}"),
            },
            "char" => match input {
                Form::Character(c) => hash_char(*c) as i64,
                other => panic!("char input must be a character: {other}"),
            },
            "bytes" => match input {
                Form::Vector(items) => {
                    let bytes: Vec<u8> = items.iter().map(|f| num_of(f) as i8 as u8).collect();
                    hash_bytes(&bytes) as i64
                }
                other => panic!("bytes input must be a vector: {other}"),
            },
            "nil" => 0,
            "seed" => hash_seed(str_of(input)) as i64,
            "keyword" => match input {
                Form::Keyword(s) => Keyword::parse(s).unwrap().hash_calc(hash_type(hash)) as i64,
                other => panic!("keyword input must be a keyword: {other}"),
            },
            "symbol" => match input {
                Form::Symbol(s) => Symbol::parse(s).hash_calc(hash_type(hash)) as i64,
                other => panic!("symbol input must be a symbol: {other}"),
            },
            "collection" => {
                let structure = kw_of(field(case, "structure"));
                let value = collection_value(structure, input);
                match hash {
                    "rapid" => value.stable_hash() as i64,
                    "murmur3" => value.java_hash(HashType::Murmur3),
                    other => panic!("unknown collection hash type: {other}"),
                }
            }
            other => panic!("unknown case kind: {other}"),
        }
    }

    /// Byte-exact parity against the Java runtime's hash values, from the
    /// normative corpus hara-specs-registry/01-lang/020-data-structures/draft/conformance/
    /// hash-parity.edn (generated by target/hashdump/HashDump.java).
    #[test]
    fn java_parity_fixture() {
        let Some(path) =
            corpus_path("01-lang/020-data-structures/draft/conformance/hash-parity.edn")
        else {
            eprintln!(
                "skipping hash-parity corpus: specs checkout not found from {}",
                env!("HARA_SOURCE_ROOT")
            );
            return;
        };
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let forms = parse_forms(&source).expect("hash-parity corpus must parse");
        assert_eq!(forms.len(), 1, "corpus must be a single map form");
        let Form::Vector(cases) = field(&forms[0], "cases") else {
            panic!("corpus :cases must be a vector");
        };
        let mut failures: Vec<String> = Vec::new();
        for case in cases {
            if kw_of(field(case, "kind")) == "decimal" {
                continue;
            }
            let id = kw_of(field(case, "id")).to_string();
            let expected = num_of(field(case, "expect"));
            let actual = eval_case(case);
            if actual != expected {
                failures.push(format!(":{id}: expected {expected}, got {actual}"));
            }
        }
        assert!(
            cases.len() >= 270,
            "only {} hash-parity cases found",
            cases.len()
        );
        if !failures.is_empty() {
            panic!(
                "{} of {} hash-parity cases failed:\n{}",
                failures.len(),
                cases.len(),
                failures.join("\n")
            );
        }
    }

    #[test]
    fn cross_type_numeric_equality() {
        // Canonical numeric representations share one equality/hash domain;
        // `hash_long` is the separate scale-zero system/CHAMP layout hash.
        assert_eq!(hash_double(1.0), canonical_decimal_str_hash("1"));
        assert_eq!(hash_double(1.0), canonical_decimal_str_hash("1.0"));
        assert_eq!(hash_double(2.5), canonical_decimal_str_hash("2.50"));
        assert_eq!(hash_double(100.0), canonical_decimal_str_hash("100"));
        assert_eq!(hash_double(100.0), canonical_decimal_str_hash("100.0"));
    }

    #[test]
    fn subnormal_double_known_divergence() {
        // Pinned deviation (see module docs): Java hashes Double.MIN_VALUE
        // via "4.9E-324" → 1844; this port formats it as Rust does ("5e-324"
        // digits) → 479. Excluded from the parity corpus on purpose.
        assert_eq!(hash_double(f64::from_bits(1)), 479);
        assert_eq!(hash_double(5e-324), 479);
    }

    #[test]
    fn keyword_display_form_deviation() {
        // The corpus keyword/symbol cases already route through hash_calc;
        // pin the composed strings explicitly.
        let kw = Keyword::create(None, "a").unwrap();
        assert_eq!(
            kw.hash_calc(HashType::Rapid) as i64,
            rapid::hash("::KEYWORD|:a".as_bytes()) as i64
        );
        let sym = Symbol::create(None, "a");
        assert_eq!(
            sym.hash_calc(HashType::Rapid) as i64,
            rapid::hash("::SYMBOL|hara.lang.data.Symbol<a>".as_bytes()) as i64
        );
        // IStringType.hashCalc(SIP) == -1 in Java.
        assert_eq!(kw.hash_calc(HashType::Sip) as i64, -1);
        // display forms used in the composition
        assert_eq!(kw.display(), ":a");
        assert_eq!(sym.display(), "a");
        assert_eq!(kw.hash_seed(), "::KEYWORD");
        assert_eq!(sym.hash_seed(), "::SYMBOL");
    }
}
