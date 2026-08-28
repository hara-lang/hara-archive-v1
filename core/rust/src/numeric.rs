use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{Signed, ToPrimitive, Zero};
use std::cmp::Ordering;

use crate::core::Value;
use crate::lang::hash::{canonical_decimal_str_hash, hash_double};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CanonicalInteger {
    Small(i64),
    Big(BigInt),
}

impl CanonicalInteger {
    pub(crate) fn from_bigint(value: BigInt) -> Self {
        match value.to_i64() {
            Some(value) => Self::Small(value),
            None => Self::Big(value),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntegerKind {
    Long,
    BigInteger,
}

/// Classifies an integer by its canonical language width.
///
/// In-range BigInts are classified as Long so that callers remain correct if
/// a low-level ingress path has not yet compacted the value. Floating-point
/// values are deliberately excluded even when they are exact integers.
pub(crate) fn integer_kind(value: &Value) -> Option<IntegerKind> {
    match value {
        Value::Number(_) => Some(IntegerKind::Long),
        Value::BigInteger(value) => value
            .to_i64()
            .map(|_| IntegerKind::Long)
            .or(Some(IntegerKind::BigInteger)),
        _ => None,
    }
}

pub(crate) fn is_long_value(value: &Value) -> bool {
    integer_kind(value) == Some(IntegerKind::Long)
}

pub(crate) fn is_big_integer_value(value: &Value) -> bool {
    integer_kind(value) == Some(IntegerKind::BigInteger)
}

pub(crate) fn parse_integer_digits(
    digits: &str,
    radix: u32,
    negative: bool,
) -> Option<CanonicalInteger> {
    let mut value = BigInt::parse_bytes(digits.as_bytes(), radix)?;
    if negative {
        value = -value;
    }
    Some(CanonicalInteger::from_bigint(value))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArithmeticOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Modulo,
}

pub(crate) fn is_numeric_value(value: &Value) -> bool {
    matches!(
        value,
        Value::Number(_) | Value::BigInteger(_) | Value::Float(_)
    )
}

pub(crate) fn integer_value(value: &Value) -> Result<BigInt, String> {
    match value {
        Value::Number(value) => Ok(BigInt::from(*value)),
        Value::BigInteger(value) => Ok(value.clone()),
        _ => Err("expected an integer".into()),
    }
}

pub(crate) fn compact_integer(value: BigInt) -> Value {
    match CanonicalInteger::from_bigint(value) {
        CanonicalInteger::Small(value) => Value::Number(value),
        CanonicalInteger::Big(value) => Value::BigInteger(value),
    }
}

fn float_value(value: &Value) -> Result<f64, String> {
    match value {
        Value::Float(value) => finite_float(*value),
        Value::Number(value) => Ok(*value as f64),
        Value::BigInteger(value) => value
            .to_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| "numeric value is outside double range".to_string()),
        _ => Err("expected a numeric value".into()),
    }
}

pub(crate) fn finite_float(value: f64) -> Result<f64, String> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err("non-finite number".into())
    }
}

fn compare_integer_to_float(integer: &BigInt, floating: f64) -> Option<Ordering> {
    if !floating.is_finite() {
        return None;
    }
    if floating == 0.0 {
        return Some(integer.cmp(&BigInt::zero()));
    }

    let bits = floating.to_bits();
    let negative = bits >> 63 != 0;
    let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & 0x000f_ffff_ffff_ffff;
    let (significand, exponent) = if exponent_bits == 0 {
        (BigInt::from(fraction), -1022 - 52)
    } else {
        (
            BigInt::from(fraction | (1_u64 << 52)),
            exponent_bits - 1023 - 52,
        )
    };
    let signed = if negative { -significand } else { significand };
    if exponent >= 0 {
        Some(integer.cmp(&(signed << exponent as usize)))
    } else {
        Some((integer << (-exponent) as usize).cmp(&signed))
    }
}

fn float_binary(op: ArithmeticOp, left: f64, right: f64) -> Result<Value, String> {
    if matches!(
        op,
        ArithmeticOp::Divide | ArithmeticOp::Remainder | ArithmeticOp::Modulo
    ) && right == 0.0
    {
        return Err("division by zero".into());
    }
    let value = match op {
        ArithmeticOp::Add => left + right,
        ArithmeticOp::Subtract => left - right,
        ArithmeticOp::Multiply => left * right,
        ArithmeticOp::Divide => left / right,
        // `mod` is the canonical named remainder operator in the Hara
        // bytecode contract. Both spellings preserve the dividend sign.
        ArithmeticOp::Remainder | ArithmeticOp::Modulo => left % right,
    };
    Ok(Value::Float(finite_float(value)?))
}

fn integer_binary(op: ArithmeticOp, left: BigInt, right: BigInt) -> Result<Value, String> {
    if matches!(
        op,
        ArithmeticOp::Divide | ArithmeticOp::Remainder | ArithmeticOp::Modulo
    ) && right.is_zero()
    {
        return Err("division by zero".into());
    }
    let value = match op {
        ArithmeticOp::Add => left + right,
        ArithmeticOp::Subtract => left - right,
        ArithmeticOp::Multiply => left * right,
        ArithmeticOp::Divide => left / right,
        // `mod` is named for source compatibility, but has remainder
        // semantics at the shared numeric boundary.
        ArithmeticOp::Remainder | ArithmeticOp::Modulo => left % right,
    };
    Ok(compact_integer(value))
}

pub(crate) fn numeric_binary(
    op: ArithmeticOp,
    left: &Value,
    right: &Value,
) -> Result<Value, String> {
    if !is_numeric_value(left) || !is_numeric_value(right) {
        return Err("expected numeric values".into());
    }
    if matches!(left, Value::Float(_)) || matches!(right, Value::Float(_)) {
        return float_binary(op, float_value(left)?, float_value(right)?);
    }
    integer_binary(op, integer_value(left)?, integer_value(right)?)
}

pub(crate) fn numeric_quotient(left: &Value, right: &Value) -> Result<Value, String> {
    if !is_numeric_value(left) || !is_numeric_value(right) {
        return Err("quot expects numeric values".into());
    }
    if matches!(left, Value::Float(_)) || matches!(right, Value::Float(_)) {
        let left = float_value(left)?;
        let right = float_value(right)?;
        if right == 0.0 {
            return Err("division by zero".into());
        }
        return Ok(Value::Float(finite_float((left / right).trunc())?));
    }
    integer_binary(
        ArithmeticOp::Divide,
        integer_value(left)?,
        integer_value(right)?,
    )
}

pub(crate) fn numeric_compare(left: &Value, right: &Value) -> Result<Option<Ordering>, String> {
    if !is_numeric_value(left) || !is_numeric_value(right) {
        return Ok(None);
    }
    match (left, right) {
        (Value::Float(left), Value::Float(right)) => {
            finite_float(*left)?;
            finite_float(*right)?;
            Ok(left.partial_cmp(right))
        }
        (Value::Float(left), _) => {
            finite_float(*left)?;
            Ok(compare_integer_to_float(&integer_value(right)?, *left)
                .map(|ordering| ordering.reverse()))
        }
        (_, Value::Float(right)) => {
            finite_float(*right)?;
            Ok(compare_integer_to_float(&integer_value(left)?, *right))
        }
        _ => Ok(Some(integer_value(left)?.cmp(&integer_value(right)?))),
    }
}

pub(crate) fn numeric_equal(left: &Value, right: &Value) -> Option<bool> {
    if !is_numeric_value(left) || !is_numeric_value(right) {
        return None;
    }
    Some(matches!(
        numeric_compare(left, right),
        Ok(Some(Ordering::Equal))
    ))
}

pub(crate) fn numeric_total_compare(left: &Value, right: &Value) -> Option<Ordering> {
    if !is_numeric_value(left) || !is_numeric_value(right) {
        return None;
    }
    numeric_compare(left, right).ok().flatten()
}

pub(crate) fn numeric_hash(value: &Value) -> Option<i32> {
    Some(match value {
        Value::Number(value) => canonical_decimal_str_hash(&value.to_string()),
        Value::BigInteger(value) => canonical_decimal_str_hash(&value.to_string()),
        Value::Float(value) => hash_double(*value),
        _ => return None,
    })
}

pub(crate) fn numeric_negate(value: &Value) -> Result<Value, String> {
    match value {
        Value::Number(value) => match value.checked_neg() {
            Some(value) => Ok(Value::Number(value)),
            None => Ok(Value::BigInteger(BigInt::from(*value).abs())),
        },
        Value::BigInteger(value) => Ok(compact_integer(-value.clone())),
        Value::Float(value) => Ok(Value::Float(finite_float(-value)?)),
        _ => Err("expected a numeric value".into()),
    }
}

pub(crate) fn numeric_abs(value: &Value) -> Result<Value, String> {
    match value {
        Value::Number(value) => match value.checked_abs() {
            Some(value) => Ok(Value::Number(value)),
            None => Ok(Value::BigInteger(BigInt::from(*value).abs())),
        },
        Value::BigInteger(value) => Ok(compact_integer(value.clone().abs())),
        Value::Float(value) => Ok(Value::Float(finite_float(value.abs())?)),
        _ => Err("expected a numeric value".into()),
    }
}

pub(crate) fn bit_not(value: &Value) -> Result<Value, String> {
    Ok(compact_integer(!integer_value(value)?))
}

pub(crate) fn bit_binary(operation: &str, left: &Value, right: &Value) -> Result<Value, String> {
    let left = integer_value(left)?;
    let right = integer_value(right)?;
    let value = match operation {
        "bit-and" => left & right,
        "bit-or" => left | right,
        "bit-xor" => left ^ right,
        _ => return Err(format!("unknown bit operation: {operation}")),
    };
    Ok(compact_integer(value))
}

fn shift_distance(value: &Value) -> Result<usize, String> {
    let value = integer_value(value)?;
    if value.is_negative() {
        return Err("shift distance must be a non-negative integer".into());
    }
    value
        .to_usize()
        .ok_or_else(|| "shift distance is outside the host index range".to_string())
}

pub(crate) fn bit_shift(left: bool, value: &Value, distance: &Value) -> Result<Value, String> {
    let value = integer_value(value)?;
    let distance = shift_distance(distance)?;
    Ok(compact_integer(if left {
        value << distance
    } else {
        value >> distance
    }))
}

/// Converts a finite f64 whose value is an exact integer to a BigInt.
fn f64_to_bigint_exact(value: f64) -> Option<BigInt> {
    if !value.is_finite() || value.fract() != 0.0 {
        return None;
    }
    let bits = value.to_bits();
    let sign_negative = bits >> 63 != 0;
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023;
    let mantissa = bits & 0xfffffffffffff;
    if exponent == -1023 {
        // Subnormal: only zero is an integer.
        return if mantissa == 0 {
            Some(BigInt::zero())
        } else {
            None
        };
    }
    let mantissa = BigInt::from(mantissa | (1 << 52));
    let exponent = exponent - 52;
    let mut result = if exponent >= 0 {
        mantissa * BigInt::from(2u8).pow(exponent as u32)
    } else {
        let divisor = BigInt::from(2u8).pow((-exponent) as u32);
        let (quotient, remainder) = mantissa.div_rem(&divisor);
        if !remainder.is_zero() {
            return None;
        }
        quotient
    };
    if sign_negative {
        result = -result;
    }
    Some(result)
}

fn boundary_integer(value: &Value) -> Result<BigInt, String> {
    match value {
        Value::Number(value) => Ok(BigInt::from(*value)),
        Value::BigInteger(value) => Ok(value.clone()),
        Value::Float(value) if value.is_finite() => f64_to_bigint_exact(*value)
            .ok_or_else(|| "floating-point value is not an exact integer".to_string()),
        Value::Float(_) => Err("floating-point value is not an exact integer".into()),
        _ => Err("expected a numeric value".into()),
    }
}

pub(crate) fn to_i64_exact(value: &Value) -> Result<i64, String> {
    boundary_integer(value)?
        .to_i64()
        .ok_or_else(|| "integer is outside signed 64-bit range".to_string())
}

/// Converts a language integer to i64 without accepting an exact float.
pub(crate) fn to_i64_integer(value: &Value) -> Result<i64, String> {
    match value {
        Value::Number(value) => Ok(*value),
        Value::BigInteger(value) => value
            .to_i64()
            .ok_or_else(|| "integer is outside signed 64-bit range".to_string()),
        _ => Err("expected a signed 64-bit integer".into()),
    }
}

pub(crate) fn to_i64_truncating(value: &Value) -> Result<i64, String> {
    let integer = match value {
        Value::Number(value) => return Ok(*value),
        Value::BigInteger(value) => value.clone(),
        Value::Float(value) if value.is_finite() => f64_to_bigint_exact(value.trunc())
            .ok_or_else(|| "floating-point value is outside signed 64-bit range".to_string())?,
        Value::Float(_) => return Err("floating-point value is not finite".into()),
        _ => return Err("expected a numeric value".into()),
    };
    integer
        .to_i64()
        .ok_or_else(|| "integer is outside signed 64-bit range".to_string())
}

pub(crate) fn to_u16_exact(value: &Value) -> Result<u16, String> {
    boundary_integer(value)?
        .to_u16()
        .ok_or_else(|| "integer is outside unsigned 16-bit range".to_string())
}

pub(crate) fn to_u64_exact(value: &Value) -> Result<u64, String> {
    boundary_integer(value)?
        .to_u64()
        .ok_or_else(|| "integer is outside unsigned 64-bit range".to_string())
}

pub(crate) fn to_usize_exact(value: &Value) -> Result<usize, String> {
    boundary_integer(value)?
        .to_usize()
        .ok_or_else(|| "integer is outside the host index range".to_string())
}

pub(crate) fn to_f64_explicit(value: &Value) -> Result<f64, String> {
    float_value(value)
}

#[cfg(test)]
mod tests {
    use super::{
        integer_kind, is_big_integer_value, is_long_value, numeric_compare, parse_integer_digits,
        CanonicalInteger, IntegerKind,
    };
    use crate::core::Value;
    use crate::lang::hash::JavaHash;
    use crate::lang::protocol::HashType;
    use num_bigint::BigInt;
    use std::cmp::Ordering;
    use std::collections::HashSet;

    #[test]
    fn canonicalizes_integer_text() {
        assert_eq!(
            parse_integer_digits("9223372036854775808", 10, false),
            Some(CanonicalInteger::Big(
                BigInt::parse_bytes(b"9223372036854775808", 10).unwrap()
            ))
        );
    }

    #[test]
    fn classifies_only_canonical_integer_widths() {
        let long = Value::Number(42);
        let fitting_big = Value::BigInteger(BigInt::from(42));
        let big = Value::BigInteger(BigInt::from(1_u8) << 63);
        let float = Value::Float(42.0);

        assert_eq!(integer_kind(&long), Some(IntegerKind::Long));
        assert_eq!(integer_kind(&fitting_big), Some(IntegerKind::Long));
        assert_eq!(integer_kind(&big), Some(IntegerKind::BigInteger));
        assert_eq!(integer_kind(&float), None);
        assert!(is_long_value(&fitting_big));
        assert!(is_big_integer_value(&big));
        assert!(!is_long_value(&float));
    }

    #[test]
    fn equal_numeric_representations_share_order_hash_and_keys() {
        let compact = Value::Number(42);
        let promoted = Value::BigInteger(BigInt::from(42));
        let floating = Value::Float(42.0);

        for value in [&promoted, &floating] {
            assert_eq!(compact, *value);
            assert_eq!(compact.cmp(value), Ordering::Equal);
            assert_eq!(
                compact.java_hash(HashType::Rapid),
                value.java_hash(HashType::Rapid)
            );
        }

        let mut keys = HashSet::new();
        keys.insert(compact);
        assert!(keys.contains(&promoted));
        assert!(keys.contains(&floating));
    }

    #[test]
    fn compares_large_integers_to_floats_without_rounding() {
        let floating = Value::Float(9_007_199_254_740_992.0);
        let exact = Value::BigInteger(BigInt::from(9007199254740992_i64));
        let next = Value::BigInteger(BigInt::from(9007199254740993_i64));

        assert_eq!(
            numeric_compare(&exact, &floating).unwrap(),
            Some(Ordering::Equal)
        );
        assert_eq!(
            numeric_compare(&next, &floating).unwrap(),
            Some(Ordering::Greater)
        );
        assert_ne!(next, floating);
        assert_eq!(
            numeric_compare(&floating, &next).unwrap(),
            Some(Ordering::Less)
        );
    }
}
