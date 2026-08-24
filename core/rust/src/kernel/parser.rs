use super::form::Form;
use super::reader::{Position, Reader};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpannedForm {
    pub form: Form,
    pub span: Span,
    pub children: Vec<SpannedForm>,
}
impl SpannedForm {
    pub fn descendants(&self) -> Box<dyn Iterator<Item = &SpannedForm> + '_> {
        Box::new(
            self.children
                .iter()
                .flat_map(|child| std::iter::once(child).chain(child.descendants())),
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub position: Position,
}
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} [line {}, column {}]",
            self.message, self.position.line, self.position.column
        )
    }
}
impl std::error::Error for ParseError {}

type Result<T> = std::result::Result<T, ParseError>;

fn anonymous_arguments(
    form: &Form,
    maximum: &mut usize,
    variadic: &mut bool,
) -> std::result::Result<(), String> {
    match form {
        Form::Symbol(name) if name == "%" => *maximum = (*maximum).max(1),
        Form::Symbol(name) if name == "%&" => *variadic = true,
        Form::Symbol(name) if name.starts_with('%') => {
            let index = name[1..]
                .parse::<usize>()
                .map_err(|_| format!("Invalid anonymous function argument: {name}"))?;
            if index == 0 {
                return Err("Anonymous function arguments begin at %1".into());
            }
            *maximum = (*maximum).max(index);
        }
        Form::List(values) | Form::Vector(values) | Form::Set(values) => {
            for value in values {
                anonymous_arguments(value, maximum, variadic)?;
            }
        }
        Form::Map(entries) => {
            for (key, value) in entries {
                anonymous_arguments(key, maximum, variadic)?;
                anonymous_arguments(value, maximum, variadic)?;
            }
        }
        Form::Metadata(metadata, value) => {
            anonymous_arguments(metadata, maximum, variadic)?;
            anonymous_arguments(value, maximum, variadic)?;
        }
        Form::Tagged(_, value) => anonymous_arguments(value, maximum, variadic)?,
        _ => {}
    }
    Ok(())
}

fn rewrite_anonymous_arguments(form: Form, id: u64) -> Form {
    match form {
        Form::Symbol(name) if name == "%" || name == "%1" => {
            Form::Symbol(format!("__reader_fn_{id}_1"))
        }
        Form::Symbol(name) if name == "%&" => Form::Symbol(format!("__reader_fn_{id}_rest")),
        Form::Symbol(name) if name.starts_with('%') => {
            Form::Symbol(format!("__reader_fn_{id}_{}", &name[1..]))
        }
        Form::List(values) => Form::List(
            values
                .into_iter()
                .map(|value| rewrite_anonymous_arguments(value, id))
                .collect(),
        ),
        Form::Vector(values) => Form::Vector(
            values
                .into_iter()
                .map(|value| rewrite_anonymous_arguments(value, id))
                .collect(),
        ),
        Form::Set(values) => Form::Set(
            values
                .into_iter()
                .map(|value| rewrite_anonymous_arguments(value, id))
                .collect(),
        ),
        Form::Map(entries) => Form::Map(
            entries
                .into_iter()
                .map(|(key, value)| {
                    (
                        rewrite_anonymous_arguments(key, id),
                        rewrite_anonymous_arguments(value, id),
                    )
                })
                .collect(),
        ),
        Form::Metadata(metadata, value) => Form::Metadata(
            Box::new(rewrite_anonymous_arguments(*metadata, id)),
            Box::new(rewrite_anonymous_arguments(*value, id)),
        ),
        Form::Tagged(tag, value) => {
            Form::Tagged(tag, Box::new(rewrite_anonymous_arguments(*value, id)))
        }
        value => value,
    }
}

pub struct Parser<'a> {
    reader: Reader<'a>,
    anonymous_function_id: u64,
}
impl<'a> Parser<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            reader: Reader::new(source),
            anonymous_function_id: 0,
        }
    }
    fn error<T>(&self, message: impl Into<String>) -> Result<T> {
        Err(ParseError {
            message: message.into(),
            position: self.reader.position(),
        })
    }
    fn whitespace(&mut self) {
        loop {
            self.reader.read_while(|ch| ch.is_whitespace() || ch == ',');
            if self.reader.peek_char() == Some(';') {
                self.reader.read_until(|ch| ch == '\n');
            } else {
                break;
            }
        }
    }
    fn symbol_token(&mut self, first: char) -> String {
        let mut token = String::from(first);
        token.push_str(&self.reader.read_while(|ch| {
            !ch.is_whitespace()
                && ch as u32 != 44
                && !matches!(
                    ch as u32,
                    34 | 59 | 94 | 40 | 41 | 91 | 93 | 123 | 125 | 92 | 64 | 96 | 126
                )
        }));
        token
    }

    fn number_token(&mut self, first: char) -> String {
        let mut token = String::from(first);
        token.push_str(&self.reader.read_while(|ch| {
            !ch.is_whitespace()
                && ch as u32 != 44
                && !matches!(
                    ch as u32,
                    34 | 59 | 94 | 40 | 41 | 91 | 93 | 123 | 125 | 92 | 35 | 39 | 64 | 96 | 126
                )
        }));
        token
    }
    fn string(&mut self) -> Result<String> {
        let mut out = String::new();
        loop {
            match self.reader.read_char() {
                None => return self.error("EOF while reading string"),
                Some('"') => return Ok(out),
                Some('\\') => {
                    let escaped = self.reader.read_char().ok_or_else(|| ParseError {
                        message: "EOF while reading string escape".into(),
                        position: self.reader.position(),
                    })?;
                    match escaped {
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'b' => out.push('\u{0008}'),
                        'f' => out.push('\u{000c}'),
                        '\\' => out.push('\\'),
                        '"' => out.push('"'),
                        'u' => {
                            let mut digits = String::new();
                            for _ in 0..4 {
                                let digit = self.reader.read_char().ok_or_else(|| ParseError {
                                    message: "EOF in Unicode escape".into(),
                                    position: self.reader.position(),
                                })?;
                                if !digit.is_ascii_hexdigit() {
                                    return self.error(format!("Invalid digit: {digit}"));
                                }
                                digits.push(digit);
                            }
                            let value = u32::from_str_radix(&digits, 16).expect("validated hex");
                            out.push(char::from_u32(value).ok_or_else(|| ParseError {
                                message: "Invalid Unicode scalar".into(),
                                position: self.reader.position(),
                            })?);
                        }
                        first @ '0'..='7' => {
                            let mut digits = String::from(first);
                            for _ in 0..2 {
                                match self.reader.peek_char() {
                                    Some(next @ '0'..='7') => {
                                        self.reader.read_char();
                                        digits.push(next);
                                    }
                                    _ => break,
                                }
                            }
                            let value = u32::from_str_radix(&digits, 8).expect("validated octal");
                            out.push(char::from_u32(value).ok_or_else(|| ParseError {
                                message: "Invalid octal scalar".into(),
                                position: self.reader.position(),
                            })?);
                        }
                        other => {
                            return self.error(format!("Unsupported escape character: \\{other}"))
                        }
                    }
                }
                Some(ch) => out.push(ch),
            }
        }
    }
    fn regex(&mut self) -> Result<String> {
        let mut out = String::new();
        loop {
            match self.reader.read_char() {
                None => return self.error("EOF while reading regex"),
                Some('"') => return Ok(out),
                Some('\\') => {
                    out.push('\\');
                    out.push(self.reader.read_char().ok_or_else(|| ParseError {
                        message: "EOF while reading regex".into(),
                        position: self.reader.position(),
                    })?);
                }
                Some(ch) => out.push(ch),
            }
        }
    }

    fn metadata(&self, meta: Form, value: Form) -> Result<Form> {
        let normalized = match meta {
            Form::Keyword(name) => Form::Map(vec![(Form::Keyword(name), Form::Bool(true))]),
            tag @ (Form::Symbol(_) | Form::String(_)) => {
                Form::Map(vec![(Form::Keyword("tag".into()), tag)])
            }
            map @ Form::Map(_) => map,
            _ => return self.error("Metadata must be Symbol, Keyword, String or Map"),
        };

        if matches!(value, Form::Keyword(_)) {
            return Ok(value);
        }
        if let Form::Metadata(existing, inner) = value {
            let (Form::Map(mut old), Form::Map(new)) = (*existing, normalized) else {
                unreachable!("reader metadata is normalized to a map")
            };
            for (key, value) in new {
                if let Some((_, prior)) = old.iter_mut().find(|(candidate, _)| *candidate == key) {
                    *prior = value;
                } else {
                    old.push((key, value));
                }
            }
            return Ok(Form::Metadata(Box::new(Form::Map(old)), inner));
        }
        if !matches!(
            value,
            Form::Symbol(_) | Form::List(_) | Form::Vector(_) | Form::Map(_) | Form::Set(_)
        ) {
            return self.error("Metadata can only be applied to object forms");
        }
        Ok(Form::Metadata(Box::new(normalized), Box::new(value)))
    }
    fn delimited(&mut self, close: char, kind: &str) -> Result<Vec<SpannedForm>> {
        let mut forms = Vec::new();
        loop {
            self.whitespace();
            match self.reader.peek_char() {
                None => return self.error(format!("EOF while reading {kind}")),
                Some(ch) if ch == close => {
                    self.reader.read_char();
                    return Ok(forms);
                }
                _ => match self.read_one()? {
                    Some(form) => forms.push(form),
                    None => {}
                },
            }
        }
    }
    fn prefixed(&mut self, name: &str) -> Result<(Form, Vec<SpannedForm>)> {
        let value = self.read_required(name)?;
        let form = Form::List(vec![Form::Symbol(name.into()), value.form.clone()]);
        Ok((form, vec![value]))
    }

    fn anonymous_function(&mut self) -> Result<(Form, Vec<SpannedForm>)> {
        let body = Form::List(
            self.delimited(')', "anonymous function")?
                .into_iter()
                .map(|form| form.form)
                .collect(),
        );
        let mut maximum = 0usize;
        let mut variadic = false;
        anonymous_arguments(&body, &mut maximum, &mut variadic).map_err(|message| ParseError {
            message,
            position: self.reader.position(),
        })?;
        let id = self.anonymous_function_id;
        self.anonymous_function_id = self.anonymous_function_id.wrapping_add(1);
        let parameters = (1..=maximum)
            .map(|index| Form::Symbol(format!("__reader_fn_{id}_{index}")))
            .chain(
                variadic
                    .then(|| {
                        [
                            Form::Symbol("&".into()),
                            Form::Symbol(format!("__reader_fn_{id}_rest")),
                        ]
                    })
                    .into_iter()
                    .flatten(),
            )
            .collect();
        Ok((
            Form::List(vec![
                Form::Symbol("fn".into()),
                Form::Vector(parameters),
                rewrite_anonymous_arguments(body, id),
            ]),
            Vec::new(),
        ))
    }
    fn read_required(&mut self, context: &str) -> Result<SpannedForm> {
        self.whitespace();
        self.read_one()?.ok_or_else(|| ParseError {
            message: format!("EOF after {context}"),
            position: self.reader.position(),
        })
    }
    fn dispatch(&mut self) -> Result<Option<(Form, Vec<SpannedForm>)>> {
        match self.reader.read_char() {
            Some('(') => self.anonymous_function().map(Some),
            Some(ch @ (':' | '=' | '?' | '|')) => {
                self.error(format!("No dispatch macro for: {ch}"))
            }
            Some('{') => {
                let children = self.delimited('}', "set")?;
                let forms = children
                    .iter()
                    .map(|child| child.form.clone())
                    .collect::<Vec<_>>();
                if forms
                    .iter()
                    .enumerate()
                    .any(|(i, value)| forms[..i].contains(value))
                {
                    return self.error("Duplicate item");
                }
                Ok(Some((Form::Set(forms), children)))
            }
            Some('_') => {
                self.read_required("#_")?;
                Ok(None)
            }
            Some('\'') => {
                let value = self.read_required("var quote")?;
                if !matches!(value.form, Form::Symbol(_)) {
                    return self.error("Var quote expects a symbol");
                }
                let form = Form::List(vec![Form::Symbol("var".into()), value.form.clone()]);
                Ok(Some((form, vec![value])))
            }
            Some('"') => Ok(Some((Form::Regex(self.regex()?), Vec::new()))),
            Some('<') => self.error("Unreadable form"),
            Some('^') => {
                let meta = self.read_required("metadata")?;
                let value = self.read_required("metadata")?;
                let form = self.metadata(meta.form.clone(), value.form.clone())?;
                Ok(Some((form, vec![meta, value])))
            }
            Some('#') => {
                let value = self.read_required("symbolic value")?;
                match &value.form {
                    Form::Symbol(name) if matches!(name.as_str(), "Inf" | "-Inf" | "NaN") => {
                        self.error("non-finite number")
                    }
                    Form::Symbol(name) => self.error(format!("Unknown symbolic value: ##{name}")),
                    _ => self.error("Invalid symbolic value"),
                }
            }

            Some(ch) => {
                if !ch.is_alphabetic() {
                    return self.error(format!("No dispatch macro for: {ch}"));
                }
                let tag = self.symbol_token(ch);
                if tag.is_empty() {
                    return self.error(format!("No dispatch macro for: {ch}"));
                }
                let value = self.read_required("tagged literal")?;
                let form = Form::Tagged(tag, Box::new(value.form.clone()));
                Ok(Some((form, vec![value])))
            }
            None => self.error("EOF while reading hash dispatch"),
        }
    }
    fn atom(&self, token: String) -> Result<Form> {
        match token.as_str() {
            "nil" => return Ok(Form::Nil),
            "true" => return Ok(Form::Bool(true)),
            "false" => return Ok(Form::Bool(false)),
            _ => {}
        }
        if let Some(keyword) = token.strip_prefix(':') {
            let slashes = keyword.bytes().filter(|byte| *byte == b'/').count();
            let error = if keyword.is_empty() {
                Some("Keyword name cannot be empty.")
            } else if keyword == "/" {
                Some("Keyword name cannot be a single slash.")
            } else if slashes > 1 {
                Some("Keyword name can only contain one slash.")
            } else if keyword.starts_with('/') {
                Some("Keyword name cannot start with a slash.")
            } else if keyword.ends_with('/') {
                Some("Keyword name cannot end with a slash.")
            } else {
                None
            };
            if let Some(error) = error {
                return self.error(error);
            }
            return Ok(Form::Keyword(keyword.into()));
        }
        let body = token.strip_prefix(['+', '-']).unwrap_or(&token);
        if body.contains('/')
            && body.split_once('/').is_some_and(|(n, d)| {
                !n.is_empty()
                    && !d.is_empty()
                    && n.chars().all(|ch| ch.is_ascii_digit())
                    && d.chars().all(|ch| ch.is_ascii_digit())
            })
        {
            return self.error("Ratios are not supported");
        }
        let numeric = body.chars().next().is_some_and(|ch| ch.is_ascii_digit());
        if numeric {
            use crate::numeric::{parse_integer_digits, CanonicalInteger};

            if token.ends_with(['N', 'M']) {
                return self.error(format!(
                    "Legacy numeric suffixes N and M are not supported: {token}"
                ));
            }
            let negative = token.starts_with('-');
            let integer_form = |integer: CanonicalInteger| match integer {
                CanonicalInteger::Small(value) => Form::Number(value),
                CanonicalInteger::Big(value) => Form::BigInteger(value),
            };
            if body.contains(['.', 'e', 'E']) {
                let float = token.parse::<f64>().map_err(|_| ParseError {
                    message: format!("Invalid number: {token}"),
                    position: self.reader.position(),
                })?;
                if !float.is_finite() {
                    return self.error("non-finite number");
                }
                return Ok(Form::Float(float));
            }
            let parsed = if let Some(hex) =
                body.strip_prefix("0x").or_else(|| body.strip_prefix("0X"))
            {
                parse_integer_digits(hex, 16, negative)
            } else if let Some((radix_text, digits)) = body.split_once(['r', 'R']) {
                let radix = radix_text.parse::<u32>().map_err(|_| ParseError {
                    message: format!("Invalid number: {token}"),
                    position: self.reader.position(),
                })?;
                if !(2..=36).contains(&radix) {
                    return self.error(format!("Radix out of range: {radix}"));
                }
                if digits.is_empty() {
                    return self.error(format!("Invalid number: {token}"));
                }
                if let Some(digit) = digits.chars().find(|digit| digit.to_digit(radix).is_none()) {
                    return self.error(format!("Invalid digit {digit} under radix {radix}"));
                }
                parse_integer_digits(digits, radix, negative)
            } else if body.len() > 1
                && body.starts_with('0')
                && body.chars().all(|ch| ch.is_ascii_digit())
            {
                if body.chars().any(|ch| !('0'..='7').contains(&ch)) {
                    None
                } else {
                    parse_integer_digits(body, 8, negative)
                }
            } else if body.chars().all(|ch| ch.is_ascii_digit()) {
                parse_integer_digits(body, 10, negative)
            } else {
                None
            };
            return parsed.map(integer_form).ok_or_else(|| ParseError {
                message: format!("Invalid number: {token}"),
                position: self.reader.position(),
            });
        }
        Ok(Form::Symbol(token))
    }
    fn read_one(&mut self) -> Result<Option<SpannedForm>> {
        self.whitespace();
        if self.reader.is_eof() {
            return Ok(None);
        }
        let start = self.reader.position();
        let ch = self.reader.read_char().expect("checked EOF");
        let form = match ch {
            '(' => {
                let children = self.delimited(')', "list")?;
                let forms = children.iter().map(|child| child.form.clone()).collect();
                Some((Form::List(forms), children))
            }
            '[' => {
                let children = self.delimited(']', "vector")?;
                let forms = children.iter().map(|child| child.form.clone()).collect();
                Some((Form::Vector(forms), children))
            }
            '{' => {
                let children = self.delimited('}', "map")?;
                if children.len() % 2 != 0 {
                    return self.error("Map literal requires an even number of forms");
                }
                {
                    let entries = children
                        .chunks(2)
                        .map(|pair| (pair[0].form.clone(), pair[1].form.clone()))
                        .collect::<Vec<_>>();
                    if entries
                        .iter()
                        .enumerate()
                        .any(|(i, (key, _))| entries[..i].iter().any(|(prior, _)| prior == key))
                    {
                        return self.error("Duplicate key");
                    }
                    Some((Form::Map(entries), children))
                }
            }
            ')' | ']' | '}' => return self.error(format!("Unmatched delimiter: {ch}")),
            '"' => Some((Form::String(self.string()?), Vec::new())),
            '\'' => Some(self.prefixed("quote")?),
            '@' => Some(self.prefixed("deref")?),
            '`' => Some(self.prefixed("syntax-quote")?),
            '~' => {
                if self.reader.peek_char() == Some('@') {
                    self.reader.read_char();
                    Some(self.prefixed("unquote-splicing")?)
                } else {
                    Some(self.prefixed("unquote")?)
                }
            }
            '^' => {
                let meta = self.read_required("metadata")?;
                let value = self.read_required("metadata")?;
                let form = self.metadata(meta.form.clone(), value.form.clone())?;
                Some((form, vec![meta, value]))
            }
            '\\' => {
                let token = if self
                    .reader
                    .peek_char()
                    .is_some_and(|c| "()[]{}".contains(c))
                {
                    self.reader
                        .read_char()
                        .map(|value| value.to_string())
                        .unwrap_or_default()
                } else {
                    self.reader
                        .read_while(|c| !c.is_whitespace() && !"()[]{}".contains(c))
                };
                if let Some(digit) = token
                    .strip_prefix('u')
                    .filter(|digits| digits.len() == 4)
                    .and_then(|digits| digits.chars().find(|ch| !ch.is_ascii_hexdigit()))
                {
                    return self.error(format!("Invalid digit: {digit}"));
                }
                if let Some(digit) = token
                    .strip_prefix('o')
                    .filter(|digits| (1..=3).contains(&digits.len()))
                    .and_then(|digits| digits.chars().find(|ch| !(('0'..='7').contains(ch))))
                {
                    return self.error(format!("Invalid digit: {digit}"));
                }
                let value = match token.as_str() {
                    "newline" => Some('\n'),
                    "space" => Some(' '),
                    "tab" => Some('\t'),
                    "backspace" => Some('\u{0008}'),
                    "formfeed" => Some('\u{000c}'),
                    "return" => Some('\r'),
                    _ if token.starts_with('u') && token.len() == 5 => {
                        u32::from_str_radix(&token[1..], 16)
                            .ok()
                            .and_then(char::from_u32)
                    }
                    _ if token.starts_with('o') && (2..=4).contains(&token.len()) => {
                        u32::from_str_radix(&token[1..], 8)
                            .ok()
                            .and_then(char::from_u32)
                    }
                    _ if token.chars().count() == 1 => token.chars().next(),
                    _ => None,
                };
                Some((
                    Form::Character(value.ok_or_else(|| ParseError {
                        message: format!("Invalid character: \\{token}"),
                        position: self.reader.position(),
                    })?),
                    Vec::new(),
                ))
            }
            '#' => self.dispatch()?,
            other => {
                let numeric = other.is_ascii_digit()
                    || ((other == '+' || other == '-')
                        && self
                            .reader
                            .peek_char()
                            .is_some_and(|ch| ch.is_ascii_digit()));
                let token = if numeric {
                    self.number_token(other)
                } else {
                    self.symbol_token(other)
                };
                Some((self.atom(token)?, Vec::new()))
            }
        };
        Ok(form.map(|(form, children)| SpannedForm {
            form,
            span: Span {
                start,
                end: self.reader.position(),
            },
            children,
        }))
    }
    pub fn read_all(mut self) -> Result<Vec<SpannedForm>> {
        let mut forms = Vec::new();
        loop {
            self.whitespace();
            if self.reader.is_eof() {
                break;
            }
            if let Some(form) = self.read_one()? {
                forms.push(form);
            }
        }
        if forms.is_empty() {
            return self.error("source contains no forms");
        }
        Ok(forms)
    }
}
pub fn read_forms(source: &str) -> Result<Vec<SpannedForm>> {
    Parser::new(source).read_all()
}
pub fn parse_forms(source: &str) -> std::result::Result<Vec<Form>, String> {
    read_forms(source)
        .map(|forms| forms.into_iter().map(|f| f.form).collect())
        .map_err(|e| e.to_string())
}
pub fn parse(source: &str) -> std::result::Result<Form, String> {
    let mut forms = parse_forms(source)?;
    if forms.len() != 1 {
        return Err("source contains multiple forms; use eval_text".into());
    }
    Ok(forms.remove(0))
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;
