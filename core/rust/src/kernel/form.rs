#[derive(Debug, Clone, PartialEq)]
pub enum Form {
    Nil,
    Bool(bool),
    Number(i64),
    Float(f64),
    BigInteger(BigInt),
    Character(char),
    Regex(String),
    Tagged(String, Box<Form>),
    Metadata(Box<Form>, Box<Form>),
    Symbol(String),
    Keyword(String),
    String(String),
    Map(Vec<(Form, Form)>),
    Set(Vec<Form>),
    Vector(Vec<Form>),
    List(Vec<Form>),
}

pub(crate) fn display_string(value: &str) -> String {
    let mut output = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000c}' => output.push_str("\\f"),
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            ch if ch.is_control() => output.push_str(&format!("\\u{:04X}", ch as u32)),
            ch => output.push(ch),
        }
    }
    output.push('"');
    output
}

pub(crate) fn display_regex(value: &str) -> String {
    let mut output = String::from("#\"");
    let mut backslashes = 0usize;
    for ch in value.chars() {
        if ch == '"' && backslashes % 2 == 0 {
            output.push('\\');
        }
        output.push(ch);
        backslashes = if ch == '\\' { backslashes + 1 } else { 0 };
    }
    output.push('"');
    output
}

fn display_forms(values: &[Form], start: &str, end: &str) -> String {
    format!(
        "{start}{}{end}",
        values
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    )
}

impl std::fmt::Display for Form {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let output = match self {
            Self::Nil => "nil".into(),
            Self::Bool(value) => value.to_string(),
            Self::Number(value) => value.to_string(),
            Self::Float(value) if value.is_finite() && value.fract() == 0.0 => {
                format!("(double {value:.1})")
            }
            Self::Float(value) if value.is_finite() => format!("(double {value})"),
            Self::Float(_) => panic!("non-finite number"),
            Self::BigInteger(value) => value.to_string(),
            Self::Character('\n') => "\\newline".into(),
            Self::Character(' ') => "\\space".into(),
            Self::Character('\t') => "\\tab".into(),
            Self::Character('\u{0008}') => "\\backspace".into(),
            Self::Character('\u{000c}') => "\\formfeed".into(),
            Self::Character('\r') => "\\return".into(),
            Self::Character(value) if value.is_control() => format!("\\u{:04X}", *value as u32),
            Self::Character(value) => format!("\\{value}"),
            Self::Regex(value) => display_regex(value),
            Self::Tagged(tag, value) => format!("#{tag}{value}"),
            Self::Metadata(_, value) => value.to_string(),
            Self::Symbol(value) => value.clone(),
            Self::Keyword(value) => format!(":{value}"),
            Self::String(value) => display_string(value),
            Self::Map(entries) => {
                let values = entries
                    .iter()
                    .flat_map(|(key, value)| [key.to_string(), value.to_string()])
                    .collect::<Vec<_>>();
                format!("{{{}}}", values.join(" "))
            }
            Self::Set(values) => display_forms(values, "#{", "}"),
            Self::Vector(values) => display_forms(values, "[", "]"),
            Self::List(values) => display_forms(values, "(", ")"),
        };
        formatter.write_str(&output)
    }
}

#[cfg(test)]
mod tests {
    use super::Form;
    use crate::kernel::parse;

    #[test]
    fn canonical_readable_forms_round_trip() {
        let sources = [
            "nil",
            "true",
            "-42",
            "\\newline",
            "\\u0000",
            "\"line\\nvalue\"",
            ":hara/name",
            "hara/name",
            "(quote [1 2 3])",
            "{:message \"value\" :flags #{:a :b}}",
            "#math[:tensor 42]",
            "#\"\\d+\"",
        ];
        for regex in [Form::Regex(r#"\""#.into()), Form::Regex(r#"\\\""#.into())] {
            let readable = regex.to_string();
            assert_eq!(parse(&readable).unwrap(), regex, "{readable}");
        }

        for source in sources {
            let form = parse(source).unwrap();
            let readable = form.to_string();
            assert_eq!(parse(&readable).unwrap(), form, "{source} -> {readable}");
        }
    }

    #[test]
    fn metadata_printing_is_canonical() {
        let metadata = parse("^:private [1]").unwrap();
        assert_eq!(metadata.to_string(), "[1]");
        assert_eq!(parse(&metadata.to_string()).unwrap(), parse("[1]").unwrap());
    }
}
use num_bigint::BigInt;
