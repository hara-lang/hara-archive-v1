use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Document {
    pub package: Option<String>,
    pub interfaces: BTreeMap<String, Interface>,
    pub worlds: BTreeMap<String, World>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Interface {
    pub types: BTreeMap<String, TypeDecl>,
    pub functions: Vec<Function>,
    pub resources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct World {
    pub exports: Vec<String>,
    pub imports: Vec<String>,
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TypeDecl {
    Alias(Type),
    Record(Vec<(String, Type)>),
    Variant(Vec<(String, Option<Type>)>),
    Resource,
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Function {
    pub name: String,
    pub arguments: Vec<(String, Type)>,
    pub result: Option<Type>,
    pub async_: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Type {
    Atom(String),
    List(Box<Type>),
    Option(Box<Type>),
    Result(Option<Box<Type>>, Option<Box<Type>>),
    Tuple(Vec<Type>),
}

pub(super) fn type_label(ty: &Type) -> String {
    match ty {
        Type::Atom(value) => value.clone(),
        Type::List(value) => format!("list<{}>", type_label(value)),
        Type::Option(value) => format!("option<{}>", type_label(value)),
        Type::Result(ok, error) => format!(
            "result<{}, {}>",
            ok.as_deref()
                .map(type_label)
                .unwrap_or_else(|| "unit".into()),
            error
                .as_deref()
                .map(type_label)
                .unwrap_or_else(|| "unit".into())
        ),
        Type::Tuple(values) => format!(
            "tuple<{}>",
            values.iter().map(type_label).collect::<Vec<_>>().join(", ")
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    String(String),
    Punct(char),
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

pub(crate) fn parse(source: &str) -> Result<Document, String> {
    let tokens = lex(source)?;
    let mut parser = Parser { tokens, index: 0 };
    parser.document()
}

fn lex(source: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = source.chars().peekable();
    while let Some(character) = chars.next() {
        if character.is_whitespace() {
            continue;
        }
        if character == '/' {
            match chars.peek() {
                Some('/') => {
                    chars.next();
                    while !matches!(chars.next(), None | Some('\n')) {}
                    continue;
                }
                Some('*') => {
                    chars.next();
                    let mut closed = false;
                    while let Some(value) = chars.next() {
                        if value == '*' && chars.peek() == Some(&'/') {
                            chars.next();
                            closed = true;
                            break;
                        }
                    }
                    if !closed {
                        return Err("unterminated block comment".into());
                    }
                    continue;
                }
                _ => {}
            }
        }
        if character == '"' {
            let mut value = String::new();
            let mut closed = false;
            while let Some(next) = chars.next() {
                match next {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' => match chars.next() {
                        Some('n') => value.push('\n'),
                        Some('r') => value.push('\r'),
                        Some('t') => value.push('\t'),
                        Some('"') => value.push('"'),
                        Some('\\') => value.push('\\'),
                        Some(other) => {
                            value.push('\\');
                            value.push(other);
                        }
                        None => break,
                    },
                    other => value.push(other),
                }
            }
            if !closed {
                return Err("unterminated string".into());
            }
            tokens.push(Token::String(value));
        } else if "{}()<>:;,=>".contains(character)
            || (character == '-' && chars.peek() == Some(&'>'))
        {
            tokens.push(Token::Punct(character));
        } else {
            let mut value = String::from(character);
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() || "{}()<>:;,=>\"".contains(next) {
                    break;
                }
                if next == '/' && matches!(chars.clone().nth(1), Some('/') | Some('*')) {
                    break;
                }
                value.push(next);
                chars.next();
            }
            tokens.push(Token::Word(value));
        }
    }
    Ok(tokens)
}

impl Parser {
    fn document(&mut self) -> Result<Document, String> {
        let mut package = None;
        let mut interfaces = BTreeMap::new();
        let mut worlds = BTreeMap::new();
        while !self.done() {
            let declaration = self.required_word("top-level declaration")?;
            match declaration.as_str() {
                value if value.starts_with('@') => {
                    self.skip_annotation()?;
                }
                "package" => {
                    if package.is_some() {
                        return Err("duplicate package declaration".into());
                    }
                    package = Some(self.package_id()?);
                    self.semicolon()?;
                }
                "interface" => {
                    let name = self.required_word("interface name")?;
                    insert_unique(&mut interfaces, name, self.interface_body()?, "interface")?;
                }
                "world" => {
                    let name = self.required_word("world name")?;
                    insert_unique(&mut worlds, name, self.world_body()?, "world")?;
                }
                "use" | "include" | "import" | "export" => {
                    self.skip_declaration()?;
                }
                other => return Err(format!("unsupported top-level declaration {other}")),
            }
        }
        if interfaces.is_empty() && worlds.is_empty() {
            return Err("WIT document contains no interface or world".into());
        }
        for (name, world) in &mut worlds {
            if !world.functions.is_empty() && world.exports.is_empty() {
                interfaces.insert(
                    name.clone(),
                    Interface {
                        types: BTreeMap::new(),
                        functions: world.functions.clone(),
                        resources: Vec::new(),
                    },
                );
                world.exports.push(name.clone());
            }
        }
        Ok(Document {
            package,
            interfaces,
            worlds,
        })
    }

    fn interface_body(&mut self) -> Result<Interface, String> {
        self.open_brace()?;
        let mut types = BTreeMap::new();
        let mut functions = Vec::new();
        let mut resources = Vec::new();
        while !self.take_punct('}') {
            let declaration = self.required_word("interface declaration")?;
            if declaration.starts_with('@') {
                self.skip_annotation()?;
                continue;
            }
            match declaration.as_str() {
                "type" => {
                    let name = self.required_word("type name")?;
                    self.expect_punct('=')?;
                    insert_unique(&mut types, name, TypeDecl::Alias(self.type_expr()?), "type")?;
                    self.semicolon()?;
                }
                "record" => {
                    let name = self.required_word("record name")?;
                    insert_unique(&mut types, name, TypeDecl::Record(self.fields()?), "type")?;
                }
                "variant" => {
                    let name = self.required_word("variant name")?;
                    insert_unique(&mut types, name, TypeDecl::Variant(self.cases()?), "type")?;
                }
                "resource" => {
                    let name = self.required_word("resource name")?;
                    insert_unique(&mut types, name.clone(), TypeDecl::Resource, "type")?;
                    if resources.iter().any(|value| value == &name) {
                        return Err(format!("duplicate resource {name}"));
                    }
                    resources.push(name);
                    self.skip_block_or_semicolon()?;
                }
                "enum" => {
                    let name = self.required_word("enum name")?;
                    insert_unique(
                        &mut types,
                        name,
                        TypeDecl::Variant(
                            self.cases()?.into_iter().map(|(n, _)| (n, None)).collect(),
                        ),
                        "type",
                    )?;
                }
                "flags" | "tuple" => {
                    let name = self.required_word("type name")?;
                    insert_unique(
                        &mut types,
                        name,
                        TypeDecl::Unsupported(declaration.clone()),
                        "type",
                    )?;
                    self.skip_block_or_semicolon()?;
                }
                "use" | "include" => self.skip_declaration()?,
                "async" => {
                    let function = self.function(true)?;
                    functions.push(function);
                }
                _ if self.take_punct(':') => {
                    let function = self.function_after_name(declaration, false)?;
                    functions.push(function);
                }
                _ => {
                    self.skip_declaration()?;
                    insert_unique(
                        &mut types,
                        declaration.clone(),
                        TypeDecl::Unsupported(declaration),
                        "type",
                    )?;
                }
            }
        }
        let mut names = BTreeMap::new();
        functions.sort_by(|left, right| left.name.cmp(&right.name));
        for function in &functions {
            if names.insert(function.name.clone(), ()).is_some() {
                return Err(format!("duplicate function {}", function.name));
            }
        }
        Ok(Interface {
            types,
            functions,
            resources,
        })
    }

    fn world_body(&mut self) -> Result<World, String> {
        self.open_brace()?;
        let mut exports = Vec::new();
        let mut imports = Vec::new();
        let mut functions = Vec::new();
        while !self.take_punct('}') {
            let declaration = self.required_word("world declaration")?;
            if declaration.starts_with('@') {
                self.skip_annotation()?;
                continue;
            }
            match declaration.as_str() {
                "export" | "import" => {
                    let name = self.required_word("world interface name")?;
                    let target = if self.take_punct(':') {
                        let mut kind = self.required_word("world interface kind")?;
                        if kind == "func" || kind == "async" {
                            if kind == "async" {
                                self.expect_word("func")?;
                            }
                            let function =
                                self.function_signature(name.clone(), kind == "async")?;
                            if declaration == "export" {
                                functions.push(function);
                            } else {
                                imports.push(name);
                            }
                            continue;
                        }
                        while self.take_punct(':') {
                            kind.push(':');
                            kind.push_str(&self.required_word("world interface reference")?);
                        }
                        if kind == "interface" {
                            name.clone()
                        } else {
                            format!("{name}:{kind}")
                        }
                    } else {
                        name.clone()
                    };
                    self.skip_to_semicolon_or_brace()?;
                    let values = if declaration == "export" {
                        &mut exports
                    } else {
                        &mut imports
                    };
                    if values.iter().any(|value| value == &target) {
                        return Err(format!("duplicate world {declaration} {target}"));
                    }
                    values.push(target);
                }
                "include" | "use" => self.skip_declaration()?,
                _ => self.skip_declaration()?,
            }
        }
        exports.sort();
        imports.sort();
        functions.sort_by(|left, right| left.name.cmp(&right.name));
        let mut names = BTreeMap::new();
        for function in &functions {
            if names.insert(function.name.clone(), ()).is_some() {
                return Err(format!("duplicate world function {}", function.name));
            }
        }
        Ok(World {
            exports,
            imports,
            functions,
        })
    }

    fn function(&mut self, async_: bool) -> Result<Function, String> {
        let first = self.required_word("function name")?;
        let name = if first == "func" {
            self.required_word("function name")?
        } else {
            first
        };
        self.expect_punct(':')?;
        self.function_after_name(name, async_)
    }

    fn function_after_name(&mut self, name: String, async_: bool) -> Result<Function, String> {
        let async_ = async_ || self.take_word("async");
        self.expect_word("func")?;
        self.function_signature(name, async_)
    }

    fn function_signature(&mut self, name: String, async_: bool) -> Result<Function, String> {
        self.expect_punct('(')?;
        let mut arguments = Vec::new();
        while !self.take_punct(')') {
            let argument = self.required_word("function argument name")?;
            self.expect_punct(':')?;
            arguments.push((argument, self.type_expr()?));
            if !self.take_punct(',') {
                self.expect_punct(')')?;
                break;
            }
        }
        let result = if self.take_punct('-') {
            self.expect_punct('>')?;
            Some(self.type_expr()?)
        } else {
            None
        };
        self.semicolon()?;
        Ok(Function {
            name,
            arguments,
            result,
            async_,
        })
    }

    fn fields(&mut self) -> Result<Vec<(String, Type)>, String> {
        self.open_brace()?;
        let mut fields = Vec::new();
        let mut names = BTreeMap::new();
        while !self.take_punct('}') {
            let name = self.required_word("field name")?;
            if names.insert(name.clone(), ()).is_some() {
                return Err(format!("duplicate field {name}"));
            }
            self.expect_punct(':')?;
            fields.push((name, self.type_expr()?));
            self.delimiter()?;
        }
        Ok(fields)
    }

    fn cases(&mut self) -> Result<Vec<(String, Option<Type>)>, String> {
        self.open_brace()?;
        let mut cases = Vec::new();
        let mut names = BTreeMap::new();
        while !self.take_punct('}') {
            let name = self.required_word("variant case name")?;
            if names.insert(name.clone(), ()).is_some() {
                return Err(format!("duplicate variant case {name}"));
            }
            let ty = if self.take_punct('(') {
                let value = self.type_expr()?;
                self.expect_punct(')')?;
                Some(value)
            } else {
                None
            };
            self.delimiter()?;
            cases.push((name, ty));
        }
        Ok(cases)
    }

    fn type_expr(&mut self) -> Result<Type, String> {
        let name = self.qualified_word()?;
        match name.as_str() {
            "list" => {
                self.expect_punct('<')?;
                let value = self.type_expr()?;
                self.expect_punct('>')?;
                Ok(Type::List(Box::new(value)))
            }
            "option" => {
                self.expect_punct('<')?;
                let value = self.type_expr()?;
                self.expect_punct('>')?;
                Ok(Type::Option(Box::new(value)))
            }
            "result" => {
                if !self.take_punct('<') {
                    return Ok(Type::Result(None, None));
                }
                let ok = if self.take_punct(',') {
                    None
                } else {
                    let value = self.type_expr()?;
                    if self.take_punct(',') {
                        Some(Box::new(value))
                    } else {
                        self.expect_punct('>')?;
                        return Ok(Type::Result(Some(Box::new(value)), None));
                    }
                };
                let error = if self.take_punct('>') {
                    None
                } else {
                    let value = self.type_expr()?;
                    self.expect_punct('>')?;
                    Some(Box::new(value))
                };
                Ok(Type::Result(ok, error))
            }
            "tuple" => {
                self.expect_punct('<')?;
                let mut values = Vec::new();
                while !self.take_punct('>') {
                    values.push(self.type_expr()?);
                    if !self.take_punct(',') {
                        self.expect_punct('>')?;
                        break;
                    }
                }
                Ok(Type::Tuple(values))
            }
            _ => Ok(Type::Atom(name)),
        }
    }

    fn package_id(&mut self) -> Result<String, String> {
        let namespace = self.required_word("package namespace")?;
        if self.take_punct(':') {
            Ok(format!(
                "{}:{}",
                namespace,
                self.required_word("package name")?
            ))
        } else {
            Ok(namespace)
        }
    }

    fn skip_declaration(&mut self) -> Result<(), String> {
        self.skip_to_semicolon_or_brace()?;
        Ok(())
    }

    fn skip_block_or_semicolon(&mut self) -> Result<(), String> {
        if self.take_punct(';') {
            return Ok(());
        }
        self.skip_balanced('{', '}')
    }

    fn skip_to_semicolon_or_brace(&mut self) -> Result<(), String> {
        let mut depth = 0;
        while let Some(token) = self.tokens.get(self.index) {
            self.index += 1;
            match token {
                Token::Punct('{') => depth += 1,
                Token::Punct('}') if depth > 0 => depth -= 1,
                Token::Punct(';') if depth == 0 => break,
                Token::Punct('}') if depth == 0 => {
                    self.index -= 1;
                    break;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn skip_balanced(&mut self, open: char, close: char) -> Result<(), String> {
        self.expect_punct(open)?;
        let mut depth = 1;
        while let Some(token) = self.tokens.get(self.index) {
            self.index += 1;
            if *token == Token::Punct(open) {
                depth += 1;
            } else if *token == Token::Punct(close) {
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
        }
        Err("unterminated WIT declaration".into())
    }

    fn skip_annotation(&mut self) -> Result<(), String> {
        if self.take_punct('(') {
            self.skip_balanced_after_open('(', ')')
        } else {
            Ok(())
        }
    }

    fn skip_balanced_after_open(&mut self, open: char, close: char) -> Result<(), String> {
        let mut depth = 1;
        while let Some(token) = self.tokens.get(self.index) {
            self.index += 1;
            if *token == Token::Punct(open) {
                depth += 1;
            } else if *token == Token::Punct(close) {
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
        }
        Err("unterminated WIT annotation".into())
    }

    fn word(&mut self) -> Option<String> {
        match self.tokens.get(self.index) {
            Some(Token::Word(value)) => {
                self.index += 1;
                Some(value.clone())
            }
            Some(Token::String(value)) => {
                self.index += 1;
                Some(value.clone())
            }
            _ => None,
        }
    }

    fn required_word(&mut self, label: &str) -> Result<String, String> {
        self.word().ok_or_else(|| format!("expected {label}"))
    }

    fn qualified_word(&mut self) -> Result<String, String> {
        let mut value = self.required_word("type")?;
        while self.take_punct(':') {
            value.push(':');
            value.push_str(&self.required_word("qualified type")?);
        }
        Ok(value)
    }

    fn take_word(&mut self, expected: &str) -> bool {
        if self.tokens.get(self.index) == Some(&Token::Word(expected.into())) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn expect_word(&mut self, expected: &str) -> Result<(), String> {
        if self.word().as_deref() == Some(expected) {
            Ok(())
        } else {
            Err(format!("expected {expected}"))
        }
    }

    fn expect_punct(&mut self, expected: char) -> Result<(), String> {
        if self.take_punct(expected) {
            Ok(())
        } else {
            Err(format!("expected '{expected}'"))
        }
    }

    fn take_punct(&mut self, expected: char) -> bool {
        if self.tokens.get(self.index) == Some(&Token::Punct(expected)) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn semicolon(&mut self) -> Result<(), String> {
        self.expect_punct(';')
    }

    fn delimiter(&mut self) -> Result<(), String> {
        if self.take_punct(',') || self.take_punct(';') {
            Ok(())
        } else {
            Err("expected ',' or ';'".into())
        }
    }

    fn open_brace(&mut self) -> Result<(), String> {
        self.expect_punct('{')
    }

    fn done(&self) -> bool {
        self.index >= self.tokens.len()
    }
}

fn insert_unique<T>(
    values: &mut BTreeMap<String, T>,
    name: String,
    value: T,
    kind: &str,
) -> Result<(), String> {
    if values.insert(name.clone(), value).is_some() {
        Err(format!("duplicate {kind} {name}"))
    } else {
        Ok(())
    }
}
