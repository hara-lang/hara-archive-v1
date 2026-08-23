/// Value-level primitive operations shared by the tree-walking evaluator and
/// the experimental bytecode VM (issue #195, notes/rust-bytecode-vm.md).
/// All arithmetic and comparison semantics live here exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Primitive {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Equal,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Count,
    Get,
    Meta,
    Nth,
    Assoc,
    First,
    Rest,
    Second,
    ToMutable,
    ToPersistent,
    NumberPredicate,
    ArrayNew,
    ArrayGet,
    ArraySet,
    ObjectNew,
    ObjectGet,
    ObjectSet,
}

impl Primitive {
    #[cfg(test)]
    pub(crate) const ALL: &[Primitive] = &[
        Primitive::Add,
        Primitive::Subtract,
        Primitive::Multiply,
        Primitive::Divide,
        Primitive::Remainder,
        Primitive::Equal,
        Primitive::Less,
        Primitive::LessOrEqual,
        Primitive::Greater,
        Primitive::GreaterOrEqual,
        Primitive::Count,
        Primitive::Get,
        Primitive::Meta,
        Primitive::Nth,
        Primitive::Assoc,
        Primitive::First,
        Primitive::Rest,
        Primitive::Second,
        Primitive::ToMutable,
        Primitive::ToPersistent,
        Primitive::NumberPredicate,
        Primitive::ArrayNew,
        Primitive::ArrayGet,
        Primitive::ArraySet,
        Primitive::ObjectNew,
        Primitive::ObjectGet,
        Primitive::ObjectSet,
    ];

    pub fn from_symbol(symbol: &str) -> Option<Primitive> {
        Some(match symbol {
            "+" => Primitive::Add,
            "-" => Primitive::Subtract,
            "*" => Primitive::Multiply,
            "/" => Primitive::Divide,
            "%" => Primitive::Remainder,
            "=" => Primitive::Equal,
            "<" => Primitive::Less,
            "<=" => Primitive::LessOrEqual,
            ">" => Primitive::Greater,
            ">=" => Primitive::GreaterOrEqual,
            // Evaluator builtin collection/metadata operators (the
            // structural arms in `eval`, not vars); the VM reaches them
            // through the same value-level functions.
            "count" => Primitive::Count,
            "get" => Primitive::Get,
            "meta" => Primitive::Meta,
            "nth" => Primitive::Nth,
            "assoc" => Primitive::Assoc,
            "first" => Primitive::First,
            "rest" => Primitive::Rest,
            "second" => Primitive::Second,
            "to-mutable" => Primitive::ToMutable,
            "to-persistent" => Primitive::ToPersistent,
            "number?" => Primitive::NumberPredicate,
            "std.native.Arr/new" => Primitive::ArrayNew,
            "std.native.Arr/get" => Primitive::ArrayGet,
            "std.native.Arr/set" => Primitive::ArraySet,
            "std.native.Obj/new" => Primitive::ObjectNew,
            "std.native.Obj/get" => Primitive::ObjectGet,
            "std.native.Obj/set" => Primitive::ObjectSet,
            _ => return None,
        })
    }

    /// Operator spelling used in error messages; `mod` reports as `%`,
    /// matching the existing evaluator.
    pub fn operator(self) -> &'static str {
        match self {
            Primitive::Add => "+",
            Primitive::Subtract => "-",
            Primitive::Multiply => "*",
            Primitive::Divide => "/",
            Primitive::Remainder => "%",
            Primitive::Equal => "=",
            Primitive::Less => "<",
            Primitive::LessOrEqual => "<=",
            Primitive::Greater => ">",
            Primitive::GreaterOrEqual => ">=",
            Primitive::Count => "count",
            Primitive::Get => "get",
            Primitive::Meta => "meta",
            Primitive::Nth => "nth",
            Primitive::Assoc => "assoc",
            Primitive::First => "first",
            Primitive::Rest => "rest",
            Primitive::Second => "second",
            Primitive::ToMutable => "to-mutable",
            Primitive::ToPersistent => "to-persistent",
            Primitive::NumberPredicate => "number?",
            Primitive::ArrayNew => "std.native.Arr/new",
            Primitive::ArrayGet => "std.native.Arr/get",
            Primitive::ArraySet => "std.native.Arr/set",
            Primitive::ObjectNew => "std.native.Obj/new",
            Primitive::ObjectGet => "std.native.Obj/get",
            Primitive::ObjectSet => "std.native.Obj/set",
        }
    }
}

/// Applies a primitive to already-evaluated arguments. The evaluator calls
/// this after evaluating argument forms; the bytecode VM calls it directly
/// from the operand stack.
pub(crate) fn apply_primitive(primitive: Primitive, arguments: &[Value]) -> Result<Value, String> {
    let op = primitive.operator();
    if let [left, right] = arguments {
        return apply_binary_primitive(primitive, left, right);
    }
    match primitive {
        Primitive::Add
        | Primitive::Subtract
        | Primitive::Multiply
        | Primitive::Divide
        | Primitive::Remainder => {
            if arguments.is_empty() {
                return Err(format!("{op} expects arguments"));
            }
            if primitive == Primitive::Remainder && arguments.len() != 2 {
                return Err("% expects two numbers".into());
            }
            if arguments.len() == 1 {
                if primitive == Primitive::Subtract {
                    return numeric::numeric_negate(&arguments[0]);
                }
                if primitive == Primitive::Divide {
                    return apply_binary_primitive(
                        Primitive::Divide,
                        &Value::Number(1),
                        &arguments[0],
                    );
                }
                if !numeric::is_numeric_value(&arguments[0]) {
                    return Err(format!("{op} expects numbers"));
                }
                return Ok(arguments[0].clone());
            }
            let mut result = arguments[0].clone();
            for argument in &arguments[1..] {
                result = apply_binary_primitive(primitive, &result, argument)?;
            }
            Ok(result)
        }
        Primitive::Equal => {
            if arguments.len() < 2 {
                return Err("= expects at least 2 arguments".into());
            }
            let first = &arguments[0];
            Ok(Value::Bool(
                arguments[1..].iter().all(|value| value == first),
            ))
        }
        Primitive::Less
        | Primitive::LessOrEqual
        | Primitive::Greater
        | Primitive::GreaterOrEqual => {
            if arguments.len() < 2 {
                return Err(format!("{op} expects at least two arguments"));
            }
            for pair in arguments.windows(2) {
                let Some(ordering) = numeric::numeric_compare(&pair[0], &pair[1])? else {
                    return Err(format!("{} expects numbers", primitive.operator()));
                };
                let matches = match primitive {
                    Primitive::Less => ordering == std::cmp::Ordering::Less,
                    Primitive::LessOrEqual => ordering != std::cmp::Ordering::Greater,
                    Primitive::Greater => ordering == std::cmp::Ordering::Greater,
                    Primitive::GreaterOrEqual => ordering != std::cmp::Ordering::Less,
                    _ => unreachable!(),
                };
                if !matches {
                    return Ok(Value::Bool(false));
                }
            }
            Ok(Value::Bool(true))
        }
        // The evaluator's structural collection/metadata arms, sharing the
        // same value-level functions and arity messages.
        Primitive::Count => {
            if arguments.len() != 1 {
                return Err("count expects one argument".into());
            }
            collection_count(&arguments[0])
        }
        Primitive::Get => {
            if arguments.len() != 2 && arguments.len() != 3 {
                return Err("get expects 2 or 3 arguments".into());
            }
            if matches!(arguments[0], Value::Bytes(_) | Value::ByteBuffer(_)) {
                return byte_get(&arguments[0], &arguments[1], arguments.get(2).cloned());
            }
            let default = arguments.get(2).cloned().unwrap_or(Value::Nil);
            collection_get(&arguments[0], &arguments[1], default)
        }
        Primitive::Meta => {
            if arguments.len() != 1 {
                return Err("meta expects one value".into());
            }
            protocol_meta(arguments)
        }
        Primitive::Nth => {
            if arguments.len() != 2 {
                return Err("nth expects two arguments".into());
            }
            collection_nth(&arguments[0], &arguments[1])
        }
        Primitive::Assoc => {
            if arguments.len() < 3 || arguments.len() % 2 == 0 {
                return Err("assoc expects a collection and key/value pairs".into());
            }
            let mut value = arguments[0].clone();
            for pair in arguments[1..].chunks(2) {
                value = collection_assoc(&value, &pair[0], pair[1].clone())?;
            }
            Ok(value)
        }
        Primitive::First => {
            if arguments.len() != 1 {
                return Err("first expects one argument".into());
            }
            collection_first(arguments[0].clone())
        }
        Primitive::Rest => {
            if arguments.len() != 1 {
                return Err("rest expects one argument".into());
            }
            collection_rest(arguments[0].clone())
        }
        Primitive::Second => {
            if arguments.len() != 1 {
                return Err("second expects one argument".into());
            }
            collection_second(arguments[0].clone())
        }
        Primitive::ToMutable => {
            if arguments.len() != 1 {
                return Err("to-mutable expects one argument".into());
            }
            collection_to_mutable(&arguments[0])
        }
        Primitive::ToPersistent => {
            if arguments.len() != 1 {
                return Err("to-persistent expects one argument".into());
            }
            collection_to_persistent(&arguments[0])
        }
        Primitive::NumberPredicate => {
            if arguments.len() != 1 {
                return Err("number? expects one argument".into());
            }
            Ok(Value::Bool(matches!(
                arguments[0],
                Value::Number(_) | Value::Float(_) | Value::BigInteger(_)
            )))
        }
        Primitive::ArrayNew => Ok(Value::Array(Rc::new(RefCell::new(arguments.to_vec())))),
        Primitive::ArrayGet => {
            if arguments.len() != 2 {
                return Err("std.native.Arr/get expects an array and index".into());
            }
            match &arguments[0] {
                Value::Array(values) => values
                    .borrow()
                    .get(value_index(&arguments[1])?)
                    .cloned()
                    .ok_or_else(|| "array/get index out of bounds".into()),
                _ => Err("std.native.Arr/get expects an array".into()),
            }
        }
        Primitive::ArraySet => {
            if arguments.len() != 3 {
                return Err("std.native.Arr/set expects an array, index, and value".into());
            }
            match &arguments[0] {
                Value::Array(values) => {
                    let index = value_index(&arguments[1])?;
                    let mut values = values.borrow_mut();
                    if index >= values.len() {
                        return Err("array/set index out of bounds".into());
                    }
                    values[index] = arguments[2].clone();
                    drop(values);
                    Ok(arguments[0].clone())
                }
                _ => Err("std.native.Arr/set expects an array".into()),
            }
        }
        Primitive::ObjectNew => {
            if arguments.len() % 2 != 0 {
                return Err("std.native.Obj/new expects key/value pairs".into());
            }
            let mut entries = Vec::with_capacity(arguments.len() / 2);
            for pair in arguments.chunks(2) {
                entries.push((marker_key(&pair[0], "object")?, pair[1].clone()));
            }
            Ok(Value::Object(Rc::new(RefCell::new(entries))))
        }
        Primitive::ObjectGet => {
            if arguments.len() != 2 {
                return Err("std.native.Obj/get expects an object and key".into());
            }
            match &arguments[0] {
                Value::Object(entries) => {
                    let key = marker_key(&arguments[1], "object")?;
                    Ok(entries
                        .borrow()
                        .iter()
                        .find(|(candidate, _)| candidate == &key)
                        .map(|(_, value)| value.clone())
                        .unwrap_or(Value::Nil))
                }
                _ => Err("std.native.Obj/get expects an object".into()),
            }
        }
        Primitive::ObjectSet => {
            if arguments.len() != 3 {
                return Err("std.native.Obj/set expects an object, key, and value".into());
            }
            match &arguments[0] {
                Value::Object(entries) => {
                    let key = marker_key(&arguments[1], "object")?;
                    let mut entries = entries.borrow_mut();
                    if let Some((_, value)) =
                        entries.iter_mut().find(|(candidate, _)| candidate == &key)
                    {
                        *value = arguments[2].clone();
                    } else {
                        entries.push((key, arguments[2].clone()));
                    }
                    drop(entries);
                    Ok(arguments[0].clone())
                }
                _ => Err("std.native.Obj/set expects an object".into()),
            }
        }
    }
}

/// Applies the common fixed-arity primitive case without constructing an
/// argument slice. The bytecode VM uses this directly on its operand stack;
/// the general evaluator reaches the same helper through [`apply_primitive`].
pub(crate) fn apply_binary_primitive(
    primitive: Primitive,
    left: &Value,
    right: &Value,
) -> Result<Value, String> {
    let op = primitive.operator();
    if let (Value::Number(left), Value::Number(right)) = (left, right) {
        return apply_binary_numbers(primitive, *left, *right);
    }
    match primitive {
        Primitive::Equal => return Ok(Value::Bool(left == right)),
        Primitive::Less
        | Primitive::LessOrEqual
        | Primitive::Greater
        | Primitive::GreaterOrEqual => {
            let Some(ordering) = numeric::numeric_compare(left, right)? else {
                return Err(format!("{op} expects numbers"));
            };
            return Ok(Value::Bool(match primitive {
                Primitive::Less => ordering == std::cmp::Ordering::Less,
                Primitive::LessOrEqual => ordering != std::cmp::Ordering::Greater,
                Primitive::Greater => ordering == std::cmp::Ordering::Greater,
                Primitive::GreaterOrEqual => ordering != std::cmp::Ordering::Less,
                _ => unreachable!(),
            }));
        }
        Primitive::Add
        | Primitive::Subtract
        | Primitive::Multiply
        | Primitive::Divide
        | Primitive::Remainder => {
            let operation = match primitive {
                Primitive::Add => ArithmeticOp::Add,
                Primitive::Subtract => ArithmeticOp::Subtract,
                Primitive::Multiply => ArithmeticOp::Multiply,
                Primitive::Divide => ArithmeticOp::Divide,
                Primitive::Remainder => ArithmeticOp::Remainder,
                _ => unreachable!(),
            };
            return numeric::numeric_binary(operation, left, right).map_err(|error| {
                if error == "expected numeric values" {
                    format!("{op} expects numbers")
                } else {
                    error
                }
            });
        }
        _ => {}
    }
    match primitive {
        Primitive::Get if matches!(left, Value::Bytes(_) | Value::ByteBuffer(_)) => {
            byte_get(left, right, None)
        }
        Primitive::Get => collection_get(left, right, Value::Nil),
        Primitive::Count => Err("count expects one argument".into()),
        Primitive::Meta => Err("meta expects one value".into()),
        Primitive::Nth => collection_nth(left, right),
        Primitive::Assoc => Err("assoc expects a collection and key/value pairs".into()),
        Primitive::First => Err("first expects one argument".into()),
        Primitive::Rest => Err("rest expects one argument".into()),
        Primitive::Second => Err("second expects one argument".into()),
        Primitive::ToMutable => Err("to-mutable expects one argument".into()),
        Primitive::ToPersistent => Err("to-persistent expects one argument".into()),
        Primitive::NumberPredicate => Err("number? expects one argument".into()),
        Primitive::ArrayNew => unreachable!("array constructor is variadic"),
        Primitive::ArrayGet => match left {
            Value::Array(values) => values
                .borrow()
                .get(value_index(right)?)
                .cloned()
                .ok_or_else(|| "array/get index out of bounds".into()),
            _ => Err("std.native.Arr/get expects an array".into()),
        },
        Primitive::ArraySet => Err("std.native.Arr/set expects three arguments".into()),
        Primitive::ObjectNew => Ok(Value::Object(Rc::new(RefCell::new(vec![(
            marker_key(left, "object")?,
            right.clone(),
        )])))),
        Primitive::ObjectGet => match left {
            Value::Object(entries) => {
                let key = marker_key(right, "object")?;
                Ok(entries
                    .borrow()
                    .iter()
                    .find(|(candidate, _)| candidate == &key)
                    .map(|(_, value)| value.clone())
                    .unwrap_or(Value::Nil))
            }
            _ => Err("std.native.Obj/get expects an object".into()),
        },
        Primitive::ObjectSet => Err("std.native.Obj/set expects three arguments".into()),
        Primitive::Add
        | Primitive::Subtract
        | Primitive::Multiply
        | Primitive::Divide
        | Primitive::Remainder
        | Primitive::Equal
        | Primitive::Less
        | Primitive::LessOrEqual
        | Primitive::Greater
        | Primitive::GreaterOrEqual => {
            unreachable!("numeric primitives return before collection dispatch")
        }
    }
}

pub(crate) fn apply_binary_numbers(
    primitive: Primitive,
    left: i64,
    right: i64,
) -> Result<Value, String> {
    let value = apply_binary_numbers_promoting(primitive, left, right)?;
    if matches!(value, Value::BigInteger(_)) {
        Err("integer overflow".into())
    } else {
        Ok(value)
    }
}

fn apply_binary_numbers_promoting(
    primitive: Primitive,
    left: i64,
    right: i64,
) -> Result<Value, String> {
    let result = match primitive {
        Primitive::Add => match left.checked_add(right) {
            Some(value) => Value::Number(value),
            None => {
                return numeric::numeric_binary(
                    ArithmeticOp::Add,
                    &Value::Number(left),
                    &Value::Number(right),
                )
            }
        },
        Primitive::Subtract => match left.checked_sub(right) {
            Some(value) => Value::Number(value),
            None => {
                return numeric::numeric_binary(
                    ArithmeticOp::Subtract,
                    &Value::Number(left),
                    &Value::Number(right),
                )
            }
        },
        Primitive::Multiply => match left.checked_mul(right) {
            Some(value) => Value::Number(value),
            None => {
                return numeric::numeric_binary(
                    ArithmeticOp::Multiply,
                    &Value::Number(left),
                    &Value::Number(right),
                )
            }
        },
        Primitive::Divide | Primitive::Remainder if right == 0 => {
            return Err("division by zero".into())
        }
        Primitive::Divide => match left.checked_div(right) {
            Some(value) => Value::Number(value),
            None => {
                return numeric::numeric_binary(
                    ArithmeticOp::Divide,
                    &Value::Number(left),
                    &Value::Number(right),
                )
            }
        },
        Primitive::Remainder => match left.checked_rem(right) {
            Some(value) => Value::Number(value),
            None => {
                return numeric::numeric_binary(
                    ArithmeticOp::Remainder,
                    &Value::Number(left),
                    &Value::Number(right),
                )
            }
        },
        Primitive::Equal => Value::Bool(left == right),
        Primitive::Less => Value::Bool(left < right),
        Primitive::LessOrEqual => Value::Bool(left <= right),
        Primitive::Greater => Value::Bool(left > right),
        Primitive::GreaterOrEqual => Value::Bool(left >= right),
        Primitive::Get => return Err("get expects an associative value".into()),
        Primitive::Count => return Err("count expects one argument".into()),
        Primitive::Meta => return Err("meta expects one value".into()),
        Primitive::Nth => return Err("nth expects a collection and index".into()),
        Primitive::Assoc => return Err("assoc expects a collection and key/value pairs".into()),
        Primitive::First => return Err("first expects one argument".into()),
        Primitive::Rest => return Err("rest expects one argument".into()),
        Primitive::Second => return Err("second expects one argument".into()),
        Primitive::ToMutable => return Err("to-mutable expects one argument".into()),
        Primitive::ToPersistent => return Err("to-persistent expects one argument".into()),
        Primitive::NumberPredicate => return Err("number? expects one argument".into()),
        Primitive::ArrayNew => Value::Array(Rc::new(RefCell::new(vec![
            Value::Number(left),
            Value::Number(right),
        ]))),
        Primitive::ArrayGet
        | Primitive::ArraySet
        | Primitive::ObjectNew
        | Primitive::ObjectGet
        | Primitive::ObjectSet => {
            return Err(format!("{} expects native values", primitive.operator()))
        }
    };
    Ok(result)
}

fn arithmetic(op: &str, args: &[Form], env: &mut HashMap<String, Value>) -> Result<Value, String> {
    let primitive = Primitive::from_symbol(op).expect("arithmetic operator");
    let mut values = Vec::with_capacity(args.len());
    for form in args {
        let value = eval(form, env)?;
        if !numeric::is_numeric_value(&value) {
            return Err(format!("{} expects numbers", primitive.operator()));
        }
        values.push(value);
    }
    apply_primitive(primitive, &values)
}

fn bit_operation(
    op: &str,
    args: &[Form],
    env: &mut HashMap<String, Value>,
) -> Result<Value, String> {
    let op = match op.strip_prefix("std.native.Bits/").unwrap_or(op) {
        "and" => "bit-and",
        "or" => "bit-or",
        "xor" => "bit-xor",
        "not" => "bit-not",
        "shift-left" => "bit-shift-left",
        "shift-right" => "bit-shift-right",
        operation => operation,
    };
    let values = args
        .iter()
        .map(|form| eval(form, env))
        .collect::<Result<Vec<_>, _>>()?;
    bit_values(op, &values)
}

fn bit_values(op: &str, values: &[Value]) -> Result<Value, String> {
    let op = match op.strip_prefix("std.native.Bits/").unwrap_or(op) {
        "and" => "bit-and",
        "or" => "bit-or",
        "xor" => "bit-xor",
        "not" => "bit-not",
        "shift-left" => "bit-shift-left",
        "shift-right" => "bit-shift-right",
        operation => operation,
    };
    match op {
        "bit-not" => {
            if values.len() != 1 {
                return Err("bit-not expects one integer".into());
            }
            numeric::bit_not(&values[0]).map_err(|_| "bit-not expects one integer".to_string())
        }
        "bit-and" | "bit-or" | "bit-xor" => {
            if values.len() != 2 {
                return Err(format!("{op} expects two integers"));
            }
            numeric::bit_binary(op, &values[0], &values[1]).map_err(|error| {
                if error == "expected an integer" {
                    format!("{op} expects integers")
                } else {
                    error
                }
            })
        }
        "bit-shift-left" | "bit-shift-right" => {
            if values.len() != 2 {
                return Err(format!("{op} expects an integer and distance"));
            }
            numeric::bit_shift(op == "bit-shift-left", &values[0], &values[1])
        }
        _ => Err(format!("unknown bit operation: {op}")),
    }
}

pub(crate) fn number_conversion_value(operation: &str, value: Value) -> Result<Value, String> {
    let operation = operation
        .strip_prefix("std.native.Num/")
        .unwrap_or(operation);
    match operation {
        "long" => Ok(Value::Number(
            numeric::to_i64_truncating(&value).map_err(|error| format!("long: {error}"))?,
        )),
        "double" => Ok(Value::Float(
            numeric::to_f64_explicit(&value).map_err(|error| format!("double: {error}"))?,
        )),
        "parse-long" => match value {
            Value::String(value) if !value.is_empty() && value.trim() == value => Ok(value
                .parse::<i64>()
                .map(Value::Number)
                .unwrap_or(Value::Nil)),
            Value::String(_) => Ok(Value::Nil),
            _ => Err("parse-long expects a string".into()),
        },
        "parse-double" => match value {
            Value::String(value) if !value.is_empty() && value.trim() == value => {
                let parsed = match value.as_str() {
                    "NaN" => Some(f64::NAN),
                    "Infinity" | "+Infinity" => Some(f64::INFINITY),
                    "-Infinity" => Some(f64::NEG_INFINITY),
                    _ if decimal_double_text(&value) => value.parse::<f64>().ok(),
                    _ => None,
                };
                Ok(parsed.map(Value::Float).unwrap_or(Value::Nil))
            }
            Value::String(_) => Ok(Value::Nil),
            _ => Err("parse-double expects a string".into()),
        },
        _ => Err(format!("unknown number conversion: {operation}")),
    }
}

fn decimal_double_text(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = usize::from(matches!(bytes.first(), Some(b'+') | Some(b'-')));
    let mut digits = 0usize;
    while matches!(bytes.get(index), Some(b'0'..=b'9')) {
        digits += 1;
        index += 1;
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        while matches!(bytes.get(index), Some(b'0'..=b'9')) {
            digits += 1;
            index += 1;
        }
    }
    if digits == 0 {
        return false;
    }
    if matches!(bytes.get(index), Some(b'e') | Some(b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+') | Some(b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while matches!(bytes.get(index), Some(b'0'..=b'9')) {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }
    index == bytes.len()
}

fn numeric_to_f64(value: &Value, operation: &str) -> Result<f64, String> {
    numeric::to_f64_explicit(value).map_err(|error| format!("{operation}: {error}"))
}

fn numeric_abs(value: Value) -> Result<Value, String> {
    numeric::numeric_abs(&value).map_err(|_| "abs expects a numeric value".to_string())
}

fn math_values(operation: &str, values: Vec<Value>) -> Result<Value, String> {
    let operation = operation
        .strip_prefix("std.native.Maths/")
        .unwrap_or(operation);
    let expected = if matches!(operation, "atan2" | "pow") {
        2
    } else {
        1
    };
    if values.len() != expected {
        return Err(format!(
            "{operation} expects {} numeric {}",
            if expected == 1 { "one" } else { "two" },
            if expected == 1 { "value" } else { "values" }
        ));
    }
    if operation == "abs" {
        return numeric_abs(values.into_iter().next().unwrap());
    }
    let first = numeric_to_f64(&values[0], operation)?;
    let result = match operation {
        "acos" => first.acos(),
        "acosh" => first.acosh(),
        "asin" => first.asin(),
        "asinh" => first.asinh(),
        "atan" => first.atan(),
        "atan2" => first.atan2(numeric_to_f64(&values[1], operation)?),
        "atanh" => first.atanh(),
        "ceil" => first.ceil(),
        "cos" => first.cos(),
        "cosh" => first.cosh(),
        "exp" => first.exp(),
        "floor" => first.floor(),
        "pow" => first.powf(numeric_to_f64(&values[1], operation)?),
        "sin" => first.sin(),
        "sinh" => first.sinh(),
        "sqrt" => first.sqrt(),
        "tan" => first.tan(),
        "tanh" => first.tanh(),
        _ => return Err(format!("unknown math operation: {operation}")),
    };
    Ok(Value::Float(result))
}

#[derive(Clone, Debug)]
enum DocumentOp {
    Text(String, usize),
    Pass(String),
    Escaped(String),
    Line(String, String),
    Break,
    Begin(usize),
    End,
    Nest(i64),
    Align(i64),
    Outdent,
}

#[derive(Clone)]
enum DocumentTask {
    Visit(Value),
    Emit(DocumentOp),
}

fn document_tag(name: &str, children: Vec<Value>) -> Result<Value, String> {
    let mut values = Vec::with_capacity(children.len() + 1);
    values.push(Value::Keyword(Keyword::parse(name)?));
    values.extend(children);
    Ok(Value::Vector(values.into()))
}

fn document_values(value: &Value) -> Option<Vec<Value>> {
    match value {
        Value::Vector(values) => Some(values.iter().cloned().collect()),
        Value::Tuple(values) => Some(values.iter().cloned().collect()),
        Value::List(values) => Some(values.iter().cloned().collect()),
        Value::Cons(values) => Some(values.iter().collect()),
        _ => None,
    }
}

fn document_text(values: &[Value], operation: &str) -> Result<String, String> {
    let mut output = String::new();
    for value in values {
        match value {
            Value::String(text) => output.push_str(text),
            Value::Character(character) => output.push(*character),
            _ => {
                return Err(format!(
                    "std.native.Document/{operation} expects text values"
                ))
            }
        }
    }
    Ok(output)
}

fn document_offset(values: &[Value], fallback: i64) -> (i64, &[Value]) {
    match values.first() {
        Some(Value::Number(offset)) => (*offset, &values[1..]),
        _ => (fallback, values),
    }
}

fn push_document_children(stack: &mut Vec<DocumentTask>, values: &[Value]) {
    for child in values.iter().rev() {
        stack.push(DocumentTask::Visit(child.clone()));
    }
}

fn serialize_document(document: &Value) -> Result<Vec<DocumentOp>, String> {
    let mut stack = vec![DocumentTask::Visit(document.clone())];
    let mut operations = Vec::new();
    while let Some(task) = stack.pop() {
        match task {
            DocumentTask::Emit(operation) => operations.push(operation),
            DocumentTask::Visit(Value::Nil) => {}
            DocumentTask::Visit(Value::String(text)) => {
                let width = text.chars().count();
                operations.push(DocumentOp::Text(text, width));
            }
            DocumentTask::Visit(Value::Keyword(tag))
                if matches!(tag.as_str(), "line" | "document/line") =>
            {
                operations.push(DocumentOp::Line(" ".into(), "".into()));
            }
            DocumentTask::Visit(value) => {
                let values = document_values(&value)
                    .ok_or_else(|| "Document expects strings or element vectors".to_string())?;
                if values.is_empty() {
                    continue;
                }
                let tag = match &values[0] {
                    Value::Keyword(tag) => tag.as_str(),
                    _ => {
                        push_document_children(&mut stack, &values);
                        continue;
                    }
                };
                let body = &values[1..];
                match tag {
                    "text" | "document/text" => {
                        let text = document_text(body, "text")?;
                        let width = text.chars().count();
                        operations.push(DocumentOp::Text(text, width));
                    }
                    "pass" | "document/pass" => {
                        operations.push(DocumentOp::Pass(document_text(body, "pass")?));
                    }
                    "escaped" | "document/escaped" => {
                        if body.len() != 1 {
                            return Err("std.native.Document/escaped expects one string".into());
                        }
                        operations.push(DocumentOp::Escaped(document_text(body, "escaped")?));
                    }
                    "span" | "document/span" | "document/fragment" => {
                        push_document_children(&mut stack, body);
                    }
                    "annotate" | "document/annotate" => {
                        if body.is_empty() {
                            return Err("std.native.Document/annotate expects an annotation".into());
                        }
                        push_document_children(&mut stack, &body[1..]);
                    }
                    "line" | "document/line" => {
                        if body.len() > 2 {
                            return Err(
                                "std.native.Document/line expects optional inline and terminate text"
                                    .into(),
                            );
                        }
                        let inline = if body.is_empty() {
                            " ".into()
                        } else {
                            document_text(&body[..1], "line")?
                        };
                        let terminate = if body.len() < 2 {
                            "".into()
                        } else {
                            document_text(&body[1..2], "line")?
                        };
                        operations.push(DocumentOp::Line(inline, terminate));
                    }
                    "break" | "document/break" => {
                        if !body.is_empty() {
                            return Err("std.native.Document/break expects no arguments".into());
                        }
                        operations.push(DocumentOp::Break);
                    }
                    "group" | "document/group" => {
                        stack.push(DocumentTask::Emit(DocumentOp::End));
                        push_document_children(&mut stack, body);
                        stack.push(DocumentTask::Emit(DocumentOp::Begin(0)));
                    }
                    "nest" | "document/nest" => {
                        let (offset, children) = document_offset(body, 2);
                        stack.push(DocumentTask::Emit(DocumentOp::Outdent));
                        push_document_children(&mut stack, children);
                        stack.push(DocumentTask::Emit(DocumentOp::Nest(offset)));
                    }
                    "align" | "document/align" => {
                        let (offset, children) = document_offset(body, 0);
                        stack.push(DocumentTask::Emit(DocumentOp::Outdent));
                        push_document_children(&mut stack, children);
                        stack.push(DocumentTask::Emit(DocumentOp::Align(offset)));
                    }
                    _ => {
                        return Err(format!(
                            "Document text renderer does not support element tag :{tag}"
                        ))
                    }
                }
            }
        }
    }
    Ok(operations)
}

fn annotate_document_groups(operations: &mut [DocumentOp]) -> Result<Vec<usize>, String> {
    let mut right = 0usize;
    let mut rights = Vec::with_capacity(operations.len());
    let mut groups = Vec::new();
    for index in 0..operations.len() {
        let operation = operations[index].clone();
        match operation {
            DocumentOp::Text(_, width) => right = right.saturating_add(width),
            DocumentOp::Escaped(_) => right = right.saturating_add(1),
            DocumentOp::Line(inline, _) => right = right.saturating_add(inline.chars().count()),
            DocumentOp::Begin(_) => groups.push(index),
            DocumentOp::End => {
                let begin = groups
                    .pop()
                    .ok_or_else(|| "Document contains an unmatched group end".to_string())?;
                operations[begin] = DocumentOp::Begin(right);
            }
            _ => {}
        }
        rights.push(right);
    }
    if !groups.is_empty() {
        return Err("Document contains an unmatched group begin".into());
    }
    Ok(rights)
}

fn render_document_text(document: &Value, width: usize) -> Result<String, String> {
    let mut operations = serialize_document(document)?;
    let rights = annotate_document_groups(&mut operations)?;
    let mut output = String::new();
    let mut fits = 0usize;
    let mut length = width;
    let mut tabs = vec![0i64];
    let mut column = 0i64;
    for (index, operation) in operations.into_iter().enumerate() {
        let indent = *tabs.last().unwrap_or(&0);
        match operation {
            DocumentOp::Text(text, visible) => {
                if column == 0 && indent > 0 {
                    output.push_str(&" ".repeat(indent as usize));
                    column += indent;
                }
                output.push_str(&text);
                column += visible as i64;
            }
            DocumentOp::Escaped(text) => {
                if column == 0 && indent > 0 {
                    output.push_str(&" ".repeat(indent as usize));
                    column += indent;
                }
                output.push_str(&text);
                column += 1;
            }
            DocumentOp::Pass(text) => output.push_str(&text),
            DocumentOp::Line(inline, terminate) => {
                if fits == 0 {
                    output.push_str(&terminate);
                    output.push('\n');
                    column = 0;
                    length = rights[index]
                        .saturating_add(width)
                        .saturating_sub(indent.max(0) as usize);
                } else {
                    column += inline.chars().count() as i64;
                    output.push_str(&inline);
                }
            }
            DocumentOp::Break => {
                output.push('\n');
                column = 0;
                length = rights[index]
                    .saturating_add(width)
                    .saturating_sub(indent.max(0) as usize);
            }
            DocumentOp::Nest(offset) => tabs.push(indent + offset),
            DocumentOp::Align(offset) => tabs.push(column + offset),
            DocumentOp::Outdent => {
                if tabs.len() == 1 {
                    return Err("Document contains an unmatched outdent".into());
                }
                tabs.pop();
            }
            DocumentOp::Begin(end) => {
                fits = if fits > 0 {
                    fits + 1
                } else if end <= length {
                    1
                } else {
                    0
                };
            }
            DocumentOp::End => fits = fits.saturating_sub(1),
        }
    }
    if tabs.len() != 1 {
        return Err("Document contains an unmatched indentation scope".into());
    }
    Ok(output)
}

fn document_map_option(options: &Value, name: &str) -> Option<Value> {
    let key = Value::Keyword(Keyword::parse(name).ok()?);
    map_entries(options)?
        .into_iter()
        .find_map(|(candidate, value)| (candidate == key).then_some(value))
}

fn document_operation(operation: &str, values: Vec<Value>) -> Result<Value, String> {
    let operation = operation
        .strip_prefix("std.native.Document/")
        .unwrap_or(operation);
    match operation {
        "element" => {
            if values.is_empty() || !matches!(values[0], Value::Keyword(_)) {
                return Err("std.native.Document/element expects a keyword tag".into());
            }
            Ok(Value::Vector(values.into()))
        }
        "text" => Ok(Value::String(document_text(&values, "text")?)),
        "fragment" | "group" | "pass" => document_tag(&format!("document/{operation}"), values),
        "annotate" => {
            if values.is_empty() {
                return Err("std.native.Document/annotate expects an annotation".into());
            }
            document_tag("document/annotate", values)
        }
        "escaped" => {
            if values.len() != 1 || !matches!(values[0], Value::String(_)) {
                return Err("std.native.Document/escaped expects one string".into());
            }
            document_tag("document/escaped", values)
        }
        "line" => {
            if values.len() > 2
                || values
                    .iter()
                    .any(|value| !matches!(value, Value::String(_)))
            {
                return Err(
                    "std.native.Document/line expects optional inline and terminate strings".into(),
                );
            }
            document_tag("document/line", values)
        }
        "break" => {
            if !values.is_empty() {
                return Err("std.native.Document/break expects no arguments".into());
            }
            document_tag("document/break", values)
        }
        "nest" | "align" => document_tag(&format!("document/{operation}"), values),
        "normalize" => {
            if values.len() != 1 {
                return Err("std.native.Document/normalize expects one document".into());
            }
            serialize_document(&values[0])?;
            Ok(values[0].clone())
        }
        "valid?" => {
            if values.len() != 1 {
                return Err("std.native.Document/valid? expects one value".into());
            }
            Ok(Value::Bool(serialize_document(&values[0]).is_ok()))
        }
        "render" => {
            if !(1..=2).contains(&values.len()) {
                return Err(
                    "std.native.Document/render expects a document and optional options map".into(),
                );
            }
            let default_options = Value::Map(PMap::new());
            let options = values.get(1).unwrap_or(&default_options);
            if map_entries(options).is_none() {
                return Err("std.native.Document/render expects an options map".into());
            }
            match document_map_option(options, "format") {
                None => {}
                Some(Value::Keyword(value)) if value.as_str() == "text" => {}
                _ => return Err("std.native.Document/render currently supports only :text".into()),
            }
            let width = match document_map_option(options, "width") {
                None => 80usize,
                Some(Value::Number(value)) if value >= 0 => value as usize,
                Some(_) => {
                    return Err(
                        "std.native.Document/render width must be a non-negative integer".into(),
                    )
                }
            };
            Ok(Value::String(render_document_text(&values[0], width)?))
        }
        _ => Err(format!("unknown Document operation: {operation}")),
    }
}

fn result_context(value: Option<Value>) -> Result<Value, String> {
    let context = value.unwrap_or_else(|| Value::Map(PMap::new()));
    map_entries(&context)
        .is_some()
        .then_some(context)
        .ok_or_else(|| "Result context must be a map".into())
}

fn result_synchronize_options(options: Option<Value>) -> Result<(Option<u64>, Value), String> {
    let Some(options) = options else {
        return Ok((None, Value::Map(PMap::new())));
    };
    if map_entries(&options).is_none() {
        return Err("std.native.Result/synchronize expects an options map".into());
    }
    let timeout_key = Value::Keyword(Keyword::from("timeout"));
    let context_key = Value::Keyword(Keyword::from("context"));
    let timeout = match map_value(&options, &timeout_key) {
        None | Some(Value::Nil) => None,
        Some(value) => Some(
            value_u64_integer(value, "std.native.Result/synchronize").map_err(|_| {
                "std.native.Result/synchronize timeout must be a non-negative integer".to_string()
            })?,
        ),
    };
    let context = result_context(map_value(&options, &context_key).cloned())?;
    Ok((timeout, context))
}

fn native_result_values(operation: &str, values: Vec<Value>) -> Result<Value, String> {
    let operation = operation
        .strip_prefix("std.native.Result/")
        .unwrap_or(operation);
    match operation {
        "create" => {
            if !(2..=3).contains(&values.len()) {
                return Err(
                    "std.native.Result/create expects status, value, and optional context".into(),
                );
            }
            let status = values[0].clone();
            let value = values[1].clone();
            let context = result_context(values.get(2).cloned())?;
            match status {
                Value::Keyword(status) if status.as_str() == "success" => Ok(Value::Result(
                    Rc::new(ResultValue::success(value, context)?),
                )),
                Value::Keyword(status) if status.as_str() == "error" => {
                    Ok(Value::Result(Rc::new(ResultValue::error(value, context)?)))
                }
                _ => Err("std.native.Result/create status must be :success or :error".into()),
            }
        }
        "synchronize" => {
            if !(1..=2).contains(&values.len()) {
                return Err(
                    "std.native.Result/synchronize expects a value and optional options map".into(),
                );
            }
            let value = values[0].clone();
            let options = values.get(1).cloned();
            let (timeout, context) = result_synchronize_options(options)?;
            native_result::synchronize_value(value, timeout, context)
        }
        "instance?" | "success?" | "error?" | "status" | "data" | "error-value" | "context" => {
            if values.len() != 1 {
                return Err(format!("std.native.Result/{operation} expects one value"));
            }
            let value = values[0].clone();
            if operation == "instance?" {
                return Ok(Value::Bool(matches!(value, Value::Result(_))));
            }
            let Value::Result(result) = value else {
                if matches!(operation, "success?" | "error?") {
                    return Ok(Value::Bool(false));
                }
                return Err(format!("std.native.Result/{operation} expects a Result"));
            };
            Ok(match operation {
                "success?" => Value::Bool(result.is_success()),
                "error?" => Value::Bool(result.is_error()),
                "status" => result.status_value(),
                "data" => result.data.clone(),
                "error-value" => result.error_value(),
                "context" => {
                    if map_entries(&result.context).is_some_and(|entries| entries.is_empty()) {
                        Value::Nil
                    } else {
                        result.context.clone()
                    }
                }
                _ => unreachable!(),
            })
        }
        "with-context" => {
            if values.len() != 2 {
                return Err("std.native.Result/with-context expects a Result and context".into());
            }
            let value = values[0].clone();
            let Value::Result(result) = value else {
                return Err("std.native.Result/with-context expects a Result".into());
            };
            let context = values[1].clone();
            Ok(Value::Result(Rc::new(result.with_context(context)?)))
        }
        _ => Err(format!("unknown std.native.Result operation: {operation}")),
    }
}

fn native_error_values(operation: &str, values: Vec<Value>) -> Result<Value, String> {
    let operation = operation
        .strip_prefix("std.native.Error/")
        .unwrap_or(operation);
    match operation {
        "new" => {
            if !(2..=3).contains(&values.len()) {
                return Err(
                    "std.native.Error/new expects a message, data map, and optional cause".into(),
                );
            }
            let Value::String(message) = &values[0] else {
                return Err("std.native.Error/new expects a string message".into());
            };
            if map_entries(&values[1]).is_none() {
                return Err("std.native.Error/new expects a data map".into());
            }
            Ok(Value::ExceptionInfo(Rc::new(ExceptionInfo {
                message: message.clone(),
                data: Box::new(values[1].clone()),
                cause: values.get(2).cloned().map(Box::new),
                provenance: Rc::new(RefCell::new(Default::default())),
            })))
        }
        "message" => {
            if values.len() != 1 {
                return Err("std.native.Error/message expects one value".into());
            }
            Ok(match &values[0] {
                Value::ExceptionInfo(value) => Value::String(value.message.clone()),
                Value::String(value) => Value::String(value.clone()),
                value => Value::String(value.display()),
            })
        }
        "class" => {
            if values.len() != 1 {
                return Err("std.native.Error/class expects one value".into());
            }
            Ok(Value::String(portable_type_name(&values[0]).into()))
        }
        _ => Err(format!("unknown native error operation: {operation}")),
    }
}

fn comparison(op: &str, args: &[Form], env: &mut HashMap<String, Value>) -> Result<Value, String> {
    let values = args
        .iter()
        .map(|form| eval(form, env))
        .collect::<Result<Vec<_>, _>>()?;
    apply_primitive(
        Primitive::from_symbol(op).expect("comparison operator"),
        &values,
    )
}

fn value_index(value: &Value) -> Result<usize, String> {
    numeric::to_usize_exact(value)
        .map_err(|_| "index must be a non-negative host-sized integer".into())
}

fn value_u64_integer(value: &Value, operation: &str) -> Result<u64, String> {
    numeric::to_u64_exact(value)
        .map_err(|_| format!("{operation} expects a non-negative 64-bit integer"))
}

fn value_u16_integer(value: &Value, operation: &str, allow_zero: bool) -> Result<u16, String> {
    let value =
        numeric::to_u16_exact(value).map_err(|_| format!("{operation} expects a valid port"))?;
    if !allow_zero && value == 0 {
        return Err(format!("{operation} expects a valid port"));
    }
    Ok(value)
}
