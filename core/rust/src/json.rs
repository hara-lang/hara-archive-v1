use crate::core::Value;
use crate::lang::data::{OrderedMap, Vector};
use num_bigint::BigInt;

const MAX_DEPTH: usize = 256;

pub fn read(source: &str) -> Result<Value, String> {
    let mut parser = Parser::new(source);
    let value = parser.value(0)?;
    parser.whitespace();
    if parser.peek().is_some() {
        return Err(parser.error("trailing content after JSON value"));
    }
    Ok(value)
}

pub fn write(value: &Value) -> Result<String, String> {
    let mut out = String::new();
    encode(&mut out, value, 0, false)?;
    Ok(out)
}

pub fn write_pretty(value: &Value) -> Result<String, String> {
    let mut out = String::new();
    encode(&mut out, value, 0, true)?;
    Ok(out)
}

fn encode(out: &mut String, value: &Value, depth: usize, pretty: bool) -> Result<(), String> {
    match value {
        Value::Nil => out.push_str("null"),
        Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => out.push_str(&value.to_string()),
        Value::BigInteger(value) => out.push_str(&value.to_string()),
        Value::String(value) => string(out, value),
        Value::Vector(values) => {
            out.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                if pretty {
                    newline(out, depth + 1);
                }
                encode(out, value, depth + 1, pretty)?;
            }
            if pretty && values.len() > 0 {
                newline(out, depth);
            }
            out.push(']');
        }
        Value::Tuple(values) => {
            out.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                if pretty {
                    newline(out, depth + 1);
                }
                encode(out, value, depth + 1, pretty)?;
            }
            if pretty && !values.is_empty() {
                newline(out, depth);
            }
            out.push(']');
        }
        value @ (Value::Map(_) | Value::OrderedMap(_) | Value::SortedMap(_) | Value::Trie(_)) => {
            let values = crate::core::map_entries(value).expect("map values have entries");
            out.push('{');
            for (index, (key, value)) in values.iter().enumerate() {
                let Value::String(key) = key else {
                    return Err("json/write expects maps with string keys".into());
                };
                if index > 0 {
                    out.push(',');
                }
                if pretty {
                    newline(out, depth + 1);
                }
                string(out, key);
                out.push_str(if pretty { ": " } else { ":" });
                encode(out, value, depth + 1, pretty)?;
            }
            if pretty && !values.is_empty() {
                newline(out, depth);
            }
            out.push('}');
        }
        _ => {
            return Err(
                "json/write accepts nil, booleans, integers, strings, vectors, and string-key maps"
                    .into(),
            )
        }
    }
    Ok(())
}

fn newline(out: &mut String, depth: usize) {
    out.push('\n');
    out.push_str(&"  ".repeat(depth));
}

fn string(out: &mut String, value: &str) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character <= '\u{1f}' => {
                out.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

struct Parser {
    input: Vec<char>,
    offset: usize,
}

impl Parser {
    fn new(source: &str) -> Self {
        Self {
            input: source.chars().collect(),
            offset: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.offset).copied()
    }
    fn take(&mut self) -> Option<char> {
        let value = self.peek();
        if value.is_some() {
            self.offset += 1;
        }
        value
    }
    fn error(&self, message: &str) -> String {
        format!("json/read: {message} at character {}", self.offset)
    }
    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.offset += 1;
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), String> {
        if self.take() == Some(expected) {
            Ok(())
        } else {
            Err(self.error(&format!("expected '{expected}'")))
        }
    }
    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("JSON nesting exceeds 256"));
        }
        self.whitespace();
        match self.peek() {
            Some('n') => {
                self.literal("null")?;
                Ok(Value::Nil)
            }
            Some('t') => {
                self.literal("true")?;
                Ok(Value::Bool(true))
            }
            Some('f') => {
                self.literal("false")?;
                Ok(Value::Bool(false))
            }
            Some('"') => self.string().map(Value::String),
            Some('[') => self.array(depth + 1),
            Some('{') => self.object(depth + 1),
            Some(_) => self.number(),
            None => Err(self.error("expected a JSON value")),
        }
    }
    fn literal(&mut self, literal: &str) -> Result<(), String> {
        for expected in literal.chars() {
            if self.take() != Some(expected) {
                return Err(self.error("invalid JSON token"));
            }
        }
        Ok(())
    }
    fn number(&mut self) -> Result<Value, String> {
        let start = self.offset;
        if self.peek() == Some('-') {
            self.offset += 1;
        }
        match self.peek() {
            Some('0') => {
                self.offset += 1;
                if matches!(self.peek(), Some('0'..='9')) {
                    return Err(self.error("leading zero in JSON number"));
                }
            }
            Some('1'..='9') => {
                while matches!(self.peek(), Some('0'..='9')) {
                    self.offset += 1;
                }
            }
            _ => return Err(self.error("expected a JSON value")),
        }
        if matches!(self.peek(), Some('.' | 'e' | 'E')) {
            return Err(self.error("JSON numbers must be integers"));
        }
        let text = self.input[start..self.offset].iter().collect::<String>();
        let value = BigInt::parse_bytes(text.as_bytes(), 10)
            .ok_or_else(|| self.error("invalid JSON integer"))?;
        Ok(crate::numeric::compact_integer(value))
    }
    fn array(&mut self, depth: usize) -> Result<Value, String> {
        self.expect('[')?;
        self.whitespace();
        let mut values = Vec::new();
        if self.peek() == Some(']') {
            self.offset += 1;
            return Ok(Value::Vector(Vector::from_iter(values)));
        }
        loop {
            values.push(self.value(depth)?);
            self.whitespace();
            if self.peek() == Some(']') {
                self.offset += 1;
                return Ok(Value::Vector(Vector::from_iter(values)));
            }
            self.expect(',')?;
            self.whitespace();
            if self.peek() == Some(']') {
                return Err(self.error("trailing commas are not valid JSON"));
            }
        }
    }
    fn object(&mut self, depth: usize) -> Result<Value, String> {
        self.expect('{')?;
        self.whitespace();
        let mut values: Vec<(Value, Value)> = Vec::new();
        if self.peek() == Some('}') {
            self.offset += 1;
            return Ok(Value::OrderedMap(Box::new(OrderedMap::from_iter(values))));
        }
        loop {
            if self.peek() != Some('"') {
                return Err(self.error("JSON object keys must be strings"));
            }
            let key = self.string()?;
            self.whitespace();
            self.expect(':')?;
            let value = self.value(depth)?;
            if values.iter().any(
                |(existing, _)| matches!(existing, Value::String(existing) if existing == &key),
            ) {
                return Err(self.error("duplicate JSON object key"));
            }
            values.push((Value::String(key), value));
            self.whitespace();
            if self.peek() == Some('}') {
                self.offset += 1;
                return Ok(Value::OrderedMap(Box::new(OrderedMap::from_iter(values))));
            }
            self.expect(',')?;
            self.whitespace();
            if self.peek() == Some('}') {
                return Err(self.error("trailing commas are not valid JSON"));
            }
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            let Some(character) = self.take() else {
                return Err(self.error("unterminated JSON string"));
            };
            match character {
                '"' => return Ok(out),
                character if character < '\u{20}' => {
                    return Err(self.error("unescaped control character"))
                }
                '\\' => match self.take() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('/') => out.push('/'),
                    Some('b') => out.push('\u{08}'),
                    Some('f') => out.push('\u{0c}'),
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some('u') => out.push(self.unicode_escape()?),
                    _ => return Err(self.error("invalid JSON escape")),
                },
                character => out.push(character),
            }
        }
    }
    fn unicode_escape(&mut self) -> Result<char, String> {
        let mut value = 0u32;
        for _ in 0..4 {
            let Some(character) = self.take() else {
                return Err(self.error("incomplete Unicode escape"));
            };
            value = value
                .checked_mul(16)
                .and_then(|value| character.to_digit(16).map(|digit| value + digit))
                .ok_or_else(|| self.error("invalid Unicode escape"))?;
        }
        char::from_u32(value).ok_or_else(|| self.error("invalid Unicode escape"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_json_round_trips_canonical_boundaries() {
        for (source, expected) in [
            (i64::MIN.to_string(), Value::Number(i64::MIN)),
            (i64::MAX.to_string(), Value::Number(i64::MAX)),
            (
                (BigInt::from(i64::MIN) - BigInt::from(1_i64)).to_string(),
                Value::BigInteger(BigInt::from(i64::MIN) - BigInt::from(1_i64)),
            ),
            (
                (BigInt::from(i64::MAX) + BigInt::from(1_i64)).to_string(),
                Value::BigInteger(BigInt::from(i64::MAX) + BigInt::from(1_i64)),
            ),
        ] {
            assert_eq!(read(&source).unwrap(), expected);
            assert_eq!(read(&write(&expected).unwrap()).unwrap(), expected);
        }
    }
}
