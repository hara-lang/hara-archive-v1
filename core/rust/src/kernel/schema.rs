//! Canonical semantic representation of the portable HAL schema grammar.
//!
//! HALC keeps surface schemas as ordinary forms on the wire. This module is
//! the first compiler-facing lowering step: it turns those forms into a typed
//! graph without evaluating schema Vars or copying nested definitions.

use super::Form;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct SchemaField {
    pub name: Form,
    pub properties: Option<Form>,
    pub value_type: SchemaType,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionSchema {
    pub fixed: Vec<SchemaType>,
    pub rest: Option<Box<SchemaType>>,
    pub output: Box<SchemaType>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SchemaType {
    Primitive(String),
    Reference(String),
    Union(Vec<SchemaType>),
    Vector(Box<SchemaType>),
    Set(Box<SchemaType>),
    Tuple(Vec<SchemaType>),
    Map(Vec<SchemaField>),
    Struct {
        name: String,
        mutable: bool,
        fields: Vec<SchemaField>,
    },
    WithProperties {
        schema: Box<SchemaType>,
        properties: Form,
    },
    Function(Vec<FunctionSchema>),
    Enum(Vec<Form>),
    Extension {
        head: String,
        arguments: Vec<Form>,
    },
    Unknown(Form),
}

impl crate::lang::protocol::IDeref for SchemaType {
    type Output = Form;

    fn deref(&self) -> Form {
        schema_shorthand(self)
    }
}

pub fn schema_shorthand(schema: &SchemaType) -> Form {
    let nested = |value: &SchemaType| schema_shorthand(value);
    match schema {
        SchemaType::Primitive(name) => Form::Vector(vec![Form::Keyword(name.clone())]),
        SchemaType::Reference(name) => Form::Vector(vec![Form::List(vec![
            Form::Symbol("var".into()),
            Form::Symbol(name.clone()),
        ])]),
        SchemaType::Union(types) => Form::Vector(
            std::iter::once(Form::Keyword("or".into()))
                .chain(types.iter().map(nested))
                .collect(),
        ),
        SchemaType::Vector(item) => {
            Form::Vector(vec![Form::Keyword("vector".into()), nested(item)])
        }
        SchemaType::Set(item) => Form::Vector(vec![Form::Keyword("set".into()), nested(item)]),
        SchemaType::Tuple(items) => Form::Vector(
            std::iter::once(Form::Keyword("tuple".into()))
                .chain(items.iter().map(nested))
                .collect(),
        ),
        SchemaType::Map(fields) => Form::Vector(
            std::iter::once(Form::Keyword("map".into()))
                .chain(fields.iter().map(|field| {
                    let mut pair = vec![field.name.clone()];
                    if let Some(properties) = &field.properties {
                        pair.push(properties.clone());
                    }
                    pair.push(nested(&field.value_type));
                    Form::Vector(pair)
                }))
                .collect(),
        ),
        SchemaType::Struct {
            name,
            mutable,
            fields,
        } => {
            let mut values = vec![Form::Keyword("struct".into())];
            if *mutable {
                values.push(Form::Map(vec![(
                    Form::Keyword("mutable?".into()),
                    Form::Bool(true),
                )]));
            }
            values.push(Form::List(vec![
                Form::Symbol("var".into()),
                Form::Symbol(name.clone()),
            ]));
            values.extend(fields.iter().map(|field| {
                let mut pair = vec![field.name.clone()];
                if let Some(properties) = &field.properties {
                    pair.push(properties.clone());
                }
                pair.push(nested(&field.value_type));
                Form::Vector(pair)
            }));
            Form::Vector(values)
        }
        SchemaType::Function(arities) => {
            let function = |arity: &FunctionSchema| {
                let mut inputs = arity.fixed.iter().map(nested).collect::<Vec<_>>();
                if let Some(rest) = &arity.rest {
                    inputs.push(Form::Symbol("&".into()));
                    inputs.push(nested(rest));
                }
                Form::Vector(vec![
                    Form::Keyword("fn".into()),
                    Form::Vector(inputs),
                    nested(&arity.output),
                ])
            };
            if arities.len() == 1 {
                function(&arities[0])
            } else {
                Form::Vector(
                    std::iter::once(Form::Keyword("function".into()))
                        .chain(arities.iter().map(function))
                        .collect(),
                )
            }
        }
        SchemaType::Enum(values) => Form::Vector(
            std::iter::once(Form::Keyword("enum".into()))
                .chain(values.iter().cloned())
                .collect(),
        ),
        SchemaType::WithProperties { schema, properties } => {
            let Form::Vector(mut values) = nested(schema) else {
                return nested(schema);
            };
            values.insert(1, properties.clone());
            Form::Vector(values)
        }
        SchemaType::Extension { head, arguments } => Form::Vector(
            std::iter::once(Form::Keyword(head.clone()))
                .chain(arguments.iter().cloned())
                .collect(),
        ),
        SchemaType::Unknown(Form::Vector(values)) => Form::Vector(values.clone()),
        SchemaType::Unknown(surface) => Form::Vector(vec![surface.clone()]),
    }
}

pub fn normalize_schema(schema: &Form) -> Result<SchemaType, String> {
    match schema {
        Form::Keyword(name) if name == "integer" => Ok(integer_schema()),
        Form::Keyword(name) => Ok(SchemaType::Primitive(name.clone())),
        Form::List(reference)
            if reference.len() == 2
                && matches!(&reference[0], Form::Symbol(operator) if operator == "var") =>
        {
            match &reference[1] {
                Form::Symbol(name) if name.contains('/') => Ok(SchemaType::Reference(name.clone())),
                Form::Symbol(name) => Err(format!(
                    "named schema reference is not fully qualified: {name}"
                )),
                _ => Err("named schema reference must target a symbol".into()),
            }
        }
        Form::Vector(items) if !items.is_empty() => normalize_composite(items),
        Form::Map(entries) => normalize_longhand(entries),
        other => Ok(SchemaType::Unknown(other.clone())),
    }
}

fn longhand_value<'a>(entries: &'a [(Form, Form)], name: &str) -> Option<&'a Form> {
    entries
        .iter()
        .find_map(|(key, value)| matches!(key, Form::Keyword(key) if key == name).then_some(value))
}

fn longhand_children(entries: &[(Form, Form)]) -> Result<&[Form], String> {
    match longhand_value(entries, "children") {
        Some(Form::Vector(values)) => Ok(values),
        None => Ok(&[]),
        _ => Err("schema :children must be a vector".into()),
    }
}

fn longhand_sequence<'a>(
    entries: &'a [(Form, Form)],
    name: &str,
    fallback: &'a [Form],
) -> Result<&'a [Form], String> {
    match longhand_value(entries, name) {
        Some(Form::Vector(values)) => Ok(values),
        Some(_) => Err(format!("schema :{name} must be a vector")),
        None => Ok(fallback),
    }
}

fn normalize_reference_name(value: &Form) -> Result<SchemaType, String> {
    match value {
        Form::Symbol(name) if name.contains('/') => Ok(SchemaType::Reference(name.clone())),
        Form::Symbol(name) => Err(format!(
            "named schema reference is not fully qualified: {name}"
        )),
        _ => Err("named schema reference must target a symbol".into()),
    }
}

fn normalize_union_forms(values: &[Form]) -> Result<SchemaType, String> {
    if values.is_empty() {
        return Err(":or schema requires at least one member".into());
    }
    let mut members = Vec::new();
    for value in values {
        let normalized = normalize_schema(value)?;
        match normalized {
            SchemaType::Union(nested) => {
                for member in nested {
                    push_unique(&mut members, member);
                }
            }
            member => push_unique(&mut members, member),
        }
    }
    Ok(if members.len() == 1 {
        members.pop().unwrap()
    } else {
        SchemaType::Union(members)
    })
}

fn normalize_longhand_field(field: &Form) -> Result<SchemaField, String> {
    let Form::Map(entries) = field else {
        return Err("map schema fields must be {:name name :type schema} maps".into());
    };
    let name = longhand_value(entries, "name")
        .ok_or_else(|| "map schema field requires :name".to_string())?;
    let value_type = longhand_value(entries, "type")
        .ok_or_else(|| "map schema field requires :type".to_string())?;
    let properties = match longhand_value(entries, "properties") {
        None => None,
        Some(Form::Map(values)) => Some(Form::Map(values.clone())),
        Some(_) => return Err("map schema field :properties must be a map".into()),
    };
    Ok(SchemaField {
        name: name.clone(),
        properties,
        value_type: normalize_schema(value_type)?,
    })
}

fn normalize_struct_name(value: &Form) -> Result<String, String> {
    match value {
        Form::List(reference)
            if reference.len() == 2
                && matches!(&reference[0], Form::Symbol(operator) if operator == "var") =>
        {
            match &reference[1] {
                Form::Symbol(name) if name.contains('/') => Ok(name.clone()),
                Form::Symbol(name) => Err(format!(
                    "named struct schema reference is not fully qualified: {name}"
                )),
                _ => Err("named struct schema reference must target a symbol".into()),
            }
        }
        Form::Symbol(name) if name.contains('/') => Ok(name.clone()),
        Form::Symbol(name) => Err(format!(
            "named struct schema reference is not fully qualified: {name}"
        )),
        _ => Err("struct schema name must be a qualified symbol or (var ...) reference".into()),
    }
}

fn normalize_struct_field(argument: &Form) -> Result<SchemaField, String> {
    let Form::Vector(pair) = argument else {
        return Err(":struct schema fields must be [name type] or [name properties type]".into());
    };
    match pair.as_slice() {
        [name, value_type] => Ok(SchemaField {
            name: name.clone(),
            properties: None,
            value_type: normalize_schema(value_type)?,
        }),
        [name, Form::Map(properties), value_type] => Ok(SchemaField {
            name: name.clone(),
            properties: Some(Form::Map(properties.clone())),
            value_type: normalize_schema(value_type)?,
        }),
        _ => Err(":struct schema fields must be [name type] or [name properties type]".into()),
    }
}

fn normalize_struct_forms(arguments: &[Form], mutable: bool) -> Result<SchemaType, String> {
    let Some(name) = arguments.first() else {
        return Err(":struct schema requires a qualified name".into());
    };
    Ok(SchemaType::Struct {
        name: normalize_struct_name(name)?,
        mutable,
        fields: arguments[1..]
            .iter()
            .map(normalize_struct_field)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn struct_mutability(arguments: &[Form]) -> Result<(bool, &[Form]), String> {
    let Some(Form::Map(properties)) = arguments.first() else {
        return Ok((false, arguments));
    };
    let Some(value) = properties.iter().find_map(|(key, value)| {
        matches!(key, Form::Keyword(key) if key == "mutable?").then_some(value)
    }) else {
        return Ok((false, arguments));
    };
    let Form::Bool(mutable) = value else {
        return Err(":struct schema :mutable? must be boolean".into());
    };
    Ok((*mutable, &arguments[1..]))
}

fn normalize_function_inputs(
    inputs: &Form,
) -> Result<(Vec<SchemaType>, Option<Box<SchemaType>>), String> {
    match inputs {
        Form::Map(entries) => {
            let fixed = match longhand_value(entries, "fixed") {
                Some(Form::Vector(values)) => values
                    .iter()
                    .map(normalize_schema)
                    .collect::<Result<Vec<_>, _>>()?,
                None => Vec::new(),
                _ => return Err("function schema :fixed must be a vector".into()),
            };
            let rest = match longhand_value(entries, "rest") {
                None | Some(Form::Nil) => None,
                Some(value) => Some(Box::new(normalize_schema(value)?)),
            };
            Ok((fixed, rest))
        }
        Form::Vector(values) => {
            let mut fixed = Vec::new();
            let mut rest = None;
            let mut index = 0;
            while index < values.len() {
                if matches!(&values[index], Form::Symbol(marker) if marker == "&") {
                    if rest.is_some() || index + 2 != values.len() {
                        return Err(":fn schema & must precede exactly one rest type".into());
                    }
                    rest = Some(Box::new(normalize_schema(&values[index + 1])?));
                    index += 2;
                } else {
                    fixed.push(normalize_schema(&values[index])?);
                    index += 1;
                }
            }
            Ok((fixed, rest))
        }
        _ => Err("function schema :inputs must be a vector or map".into()),
    }
}

fn normalize_longhand_function(entries: &[(Form, Form)]) -> Result<FunctionSchema, String> {
    let inputs = longhand_value(entries, "inputs")
        .ok_or_else(|| "function schema requires :inputs".to_string())?;
    let output = longhand_value(entries, "output")
        .ok_or_else(|| "function schema requires :output".to_string())?;
    let (fixed, rest) = normalize_function_inputs(inputs)?;
    Ok(FunctionSchema {
        fixed,
        rest,
        output: Box::new(normalize_schema(output)?),
    })
}

fn normalize_longhand_functions(values: &[Form]) -> Result<SchemaType, String> {
    if values.is_empty() {
        return Err(":function schema requires at least one :fn schema".into());
    }
    let mut arities = Vec::new();
    for value in values {
        match value {
            Form::Map(entries) if longhand_value(entries, "kind").is_none() => {
                arities.push(normalize_longhand_function(entries)?);
            }
            _ => match normalize_schema(value)? {
                SchemaType::Function(nested) => arities.extend(nested),
                _ => return Err(":function members must be :fn schemas".into()),
            },
        }
    }
    Ok(SchemaType::Function(arities))
}

fn normalize_longhand(entries: &[(Form, Form)]) -> Result<SchemaType, String> {
    let Some(Form::Keyword(kind)) = longhand_value(entries, "kind") else {
        return Ok(SchemaType::Unknown(Form::Map(entries.to_vec())));
    };
    let children = longhand_children(entries)?;
    let normalized = match kind.as_str() {
        "primitive" => {
            let value = longhand_value(entries, "name").or_else(|| children.first());
            match value {
                Some(Form::Keyword(name)) if name == "integer" => Ok(integer_schema()),
                Some(Form::Keyword(name)) => Ok(SchemaType::Primitive(name.clone())),
                _ => Err("primitive schema requires one keyword name".into()),
            }
        }
        "reference" => {
            let value = longhand_value(entries, "name").or_else(|| children.first());
            value
                .ok_or_else(|| "reference schema requires :name".to_string())
                .and_then(normalize_reference_name)
        }
        "union" | "or" => normalize_union_forms(longhand_sequence(entries, "types", children)?),
        "vector" => {
            let value = longhand_value(entries, "item").or_else(|| children.first());
            value
                .ok_or_else(|| "vector schema requires :item".to_string())
                .and_then(normalize_schema)
                .map(|value| SchemaType::Vector(Box::new(value)))
        }
        "set" => {
            let value = longhand_value(entries, "item").or_else(|| children.first());
            value
                .ok_or_else(|| "set schema requires :item".to_string())
                .and_then(normalize_schema)
                .map(|value| SchemaType::Set(Box::new(value)))
        }
        "tuple" => longhand_sequence(entries, "items", children)?
            .iter()
            .map(normalize_schema)
            .collect::<Result<Vec<_>, _>>()
            .map(SchemaType::Tuple),
        "map" => {
            if longhand_value(entries, "fields").is_some() {
                longhand_sequence(entries, "fields", &[])?
                    .iter()
                    .map(normalize_longhand_field)
                    .collect::<Result<Vec<_>, _>>()
                    .map(SchemaType::Map)
            } else {
                children
                    .iter()
                    .map(normalize_map_field)
                    .collect::<Result<Vec<_>, _>>()
                    .map(SchemaType::Map)
            }
        }
        "struct" => {
            let mutable = match longhand_value(entries, "mutable?") {
                None => false,
                Some(Form::Bool(value)) => *value,
                Some(_) => return Err("struct schema :mutable? must be boolean".into()),
            };
            let name = longhand_value(entries, "name").or_else(|| children.first())
                .ok_or_else(|| ":struct schema requires a qualified name".to_string())?;
            let field_fallback = if children.is_empty() {
                &[][..]
            } else {
                &children[1..]
            };
            let fields = longhand_sequence(entries, "fields", field_fallback)?
                .iter()
                .map(normalize_struct_field)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(SchemaType::Struct {
                name: normalize_struct_name(name)?,
                mutable,
                fields,
            })
        }
        "fn" => normalize_longhand_function(entries).map(|arity| SchemaType::Function(vec![arity])),
        "function" => {
            normalize_longhand_functions(longhand_sequence(entries, "arities", children)?)
        }
        "enum" => Ok(SchemaType::Enum(
            longhand_sequence(entries, "values", children)?.to_vec(),
        )),
        "extension" => {
            let head = longhand_value(entries, "head")
                .or_else(|| longhand_value(entries, "name"))
                .ok_or_else(|| "extension schema requires :head".to_string())?;
            let Form::Keyword(head) = head else {
                return Err("extension schema :head must be a keyword".into());
            };
            Ok(SchemaType::Extension {
                head: head.clone(),
                arguments: longhand_sequence(entries, "arguments", children)?.to_vec(),
            })
        }
        "unknown" => Ok(SchemaType::Unknown(
            longhand_value(entries, "surface")
                .or_else(|| children.first())
                .cloned()
                .unwrap_or_else(|| Form::Map(entries.to_vec())),
        )),
        _ => Err(format!("unsupported longhand schema kind: {kind}")),
    }?;
    match longhand_value(entries, "properties") {
        None => Ok(normalized),
        Some(Form::Map(values)) => Ok(SchemaType::WithProperties {
            schema: Box::new(normalized),
            properties: Form::Map(values.clone()),
        }),
        Some(_) => Err("schema :properties must be a map".into()),
    }
}

/// Infers conservative function signatures from executable module forms.
/// Declared schemas seed parameter types, but inferred results remain a
/// separate table: annotations are contracts, while these are optimizer facts.
pub fn infer_function_types(
    namespace: &str,
    forms: &[Form],
    declarations: &HashMap<String, SchemaType>,
    definitions: &HashMap<String, SchemaType>,
) -> HashMap<String, SchemaType> {
    let mut inferred = HashMap::new();
    for form in forms {
        let Form::List(items) = super::super::core::form_without_metadata(form) else {
            continue;
        };
        if !matches!(items.first(), Some(Form::Symbol(operator)) if operator == "defn" || operator == "defn-")
        {
            continue;
        }
        let Some(name) = items.get(1).and_then(binding_name) else {
            continue;
        };
        let qualified = format!("{namespace}/{name}");
        let parameters_at = items.iter().enumerate().skip(2).find_map(|(index, value)| {
            matches!(
                super::super::core::form_without_metadata(value),
                Form::Vector(_)
            )
            .then_some(index)
        });
        let declared = declarations
            .get(&qualified)
            .and_then(|schema| resolve_type(schema, definitions));
        let mut arities = Vec::new();
        if let Some(parameters_at) = parameters_at {
            let Form::Vector(parameters) =
                super::super::core::form_without_metadata(&items[parameters_at])
            else {
                continue;
            };
            arities.push(infer_function_arity(
                parameters,
                &items[parameters_at + 1..],
                declared,
            ));
        } else {
            for clause in items.iter().skip(2) {
                let Form::List(clause) = super::super::core::form_without_metadata(clause) else {
                    continue;
                };
                let Some(Form::Vector(parameters)) = clause.first() else {
                    continue;
                };
                arities.push(infer_function_arity(parameters, &clause[1..], declared));
            }
        }
        if !arities.is_empty() {
            inferred.insert(qualified, SchemaType::Function(arities));
        }
    }
    inferred
}

fn infer_function_arity(
    parameters: &[Form],
    body: &[Form],
    declared: Option<&SchemaType>,
) -> FunctionSchema {
    let declared_arity = match declared {
        Some(SchemaType::Function(arities)) => arities.iter().find(|arity| {
            arity.fixed.len()
                == parameters
                    .iter()
                    .take_while(|form| !matches!(form, Form::Symbol(marker) if marker == "&"))
                    .count()
                && arity.rest.is_some()
                    == parameters
                        .iter()
                        .any(|form| matches!(form, Form::Symbol(marker) if marker == "&"))
        }),
        _ => None,
    };
    let mut environment = HashMap::new();
    let mut fixed = Vec::new();
    let mut rest = None;
    let mut parameter_index = 0;
    let mut variadic = false;
    for parameter in parameters {
        if matches!(parameter, Form::Symbol(marker) if marker == "&") {
            variadic = true;
            continue;
        }
        let Some(parameter_name) = binding_name(parameter) else {
            continue;
        };
        let parameter_type = if variadic {
            declared_arity
                .and_then(|arity| arity.rest.as_deref())
                .cloned()
                .unwrap_or_else(unknown_type)
        } else {
            declared_arity
                .and_then(|arity| arity.fixed.get(parameter_index))
                .cloned()
                .unwrap_or_else(unknown_type)
        };
        environment.insert(parameter_name.to_owned(), parameter_type.clone());
        if variadic {
            rest = Some(Box::new(parameter_type));
        } else {
            fixed.push(parameter_type);
            parameter_index += 1;
        }
    }
    let output = body
        .iter()
        .map(|body| infer_expression(body, &mut environment))
        .last()
        .unwrap_or_else(|| SchemaType::Primitive("nil".into()));
    FunctionSchema {
        fixed,
        rest,
        output: Box::new(output),
    }
}

fn binding_name(form: &Form) -> Option<&str> {
    match form {
        Form::Symbol(name) => Some(name),
        Form::Metadata(_, value) => binding_name(value),
        _ => None,
    }
}

fn resolve_type<'a>(
    schema: &'a SchemaType,
    definitions: &'a HashMap<String, SchemaType>,
) -> Option<&'a SchemaType> {
    let mut current = schema;
    let mut visited = std::collections::HashSet::new();
    loop {
        match current {
            SchemaType::WithProperties { schema, .. } => current = schema,
            SchemaType::Reference(name) => {
                if !visited.insert(name) {
                    return Some(current);
                }
                current = definitions.get(name)?;
            }
            _ => return Some(current),
        }
    }
}

fn unknown_type() -> SchemaType {
    SchemaType::Unknown(Form::Symbol("?".into()))
}

fn infer_expression(form: &Form, environment: &mut HashMap<String, SchemaType>) -> SchemaType {
    match super::super::core::form_without_metadata(form) {
        Form::Nil => SchemaType::Primitive("nil".into()),
        Form::Bool(_) => SchemaType::Primitive("bool".into()),
        Form::Number(_) => SchemaType::Primitive("long".into()),
        Form::Float(_) => SchemaType::Primitive("float".into()),
        Form::BigInteger(_) => SchemaType::Primitive("bigint".into()),
        Form::Character(_) => SchemaType::Primitive("char".into()),
        Form::Regex(_) => SchemaType::Primitive("regex".into()),
        Form::String(_) => SchemaType::Primitive("str".into()),
        Form::Keyword(_) => SchemaType::Primitive("keyword".into()),
        Form::Symbol(name) => environment
            .get(name)
            .map(inference_type)
            .unwrap_or_else(unknown_type),
        Form::Vector(values) => SchemaType::Vector(Box::new(join_types(
            values
                .iter()
                .map(|value| infer_expression(value, environment)),
        ))),
        Form::Map(entries) => SchemaType::Map(
            entries
                .iter()
                .map(|(name, value)| SchemaField {
                    name: name.clone(),
                    properties: None,
                    value_type: infer_expression(value, environment),
                })
                .collect(),
        ),
        Form::Set(values) => SchemaType::Set(Box::new(join_types(
            values
                .iter()
                .map(|value| infer_expression(value, environment)),
        ))),
        Form::List(items) if items.is_empty() => SchemaType::Extension {
            head: "list".into(),
            arguments: Vec::new(),
        },
        Form::List(items) => infer_list(items, environment),
        Form::Tagged(_, value) => infer_expression(value, environment),
        Form::Metadata(_, value) => infer_expression(value, environment),
    }
}

fn infer_list(items: &[Form], environment: &mut HashMap<String, SchemaType>) -> SchemaType {
    let Some(Form::Symbol(operator)) = items.first() else {
        return unknown_type();
    };
    match operator.as_str() {
        "do" => items[1..]
            .iter()
            .map(|value| infer_expression(value, environment))
            .last()
            .unwrap_or_else(|| SchemaType::Primitive("nil".into())),
        "if" => join_types(
            items[2..]
                .iter()
                .map(|value| infer_expression(value, environment)),
        ),
        "let" if items.len() >= 3 => {
            let mut nested = environment.clone();
            if let Form::Vector(bindings) = super::super::core::form_without_metadata(&items[1]) {
                for pair in bindings.chunks(2) {
                    if let [name, value] = pair {
                        if let Some(name) = binding_name(name) {
                            let value_type = infer_expression(value, &mut nested);
                            nested.insert(name.to_owned(), value_type);
                        }
                    }
                }
            }
            items[2..]
                .iter()
                .map(|value| infer_expression(value, &mut nested))
                .last()
                .unwrap_or_else(|| SchemaType::Primitive("nil".into()))
        }
        "+" | "-" | "*" | "mod" => {
            let operands = join_types(
                items[1..]
                    .iter()
                    .map(|value| infer_expression(value, environment)),
            );
            match operands {
                SchemaType::Primitive(name)
                    if matches!(name.as_str(), "int" | "long" | "bigint" | "float") =>
                {
                    SchemaType::Primitive(name)
                }
                SchemaType::Union(members) if members.iter().all(is_long_alias) => {
                    SchemaType::Primitive("long".into())
                }
                _ => SchemaType::Primitive("number".into()),
            }
        }
        "/" => SchemaType::Primitive("number".into()),
        "=" | "<" | "<=" | ">" | ">=" | "instance?" => SchemaType::Primitive("bool".into()),
        "count" => SchemaType::Primitive("long".into()),
        "vector" => SchemaType::Vector(Box::new(join_types(
            items[1..]
                .iter()
                .map(|value| infer_expression(value, environment)),
        ))),
        _ => unknown_type(),
    }
}

fn join_types(types: impl IntoIterator<Item = SchemaType>) -> SchemaType {
    let mut members = Vec::new();
    for value in types {
        match value {
            SchemaType::Union(nested) => {
                for member in nested {
                    push_unique(&mut members, member);
                }
            }
            member => push_unique(&mut members, member),
        }
    }
    match members.len() {
        0 => unknown_type(),
        1 => members.pop().unwrap(),
        _ => SchemaType::Union(members),
    }
}

fn inference_type(schema: &SchemaType) -> SchemaType {
    match schema {
        SchemaType::Primitive(name) if name == "int" => SchemaType::Primitive("long".into()),
        _ => schema.clone(),
    }
}

fn is_long_alias(schema: &SchemaType) -> bool {
    matches!(schema, SchemaType::Primitive(name) if name == "int" || name == "long")
}

fn normalize_map_field(argument: &Form) -> Result<SchemaField, String> {
    let Form::Vector(pair) = argument else {
        return Err(":map schema fields must be [name type] or [name properties type]".into());
    };
    match pair.as_slice() {
        [name, value_type] => Ok(SchemaField {
            name: name.clone(),
            properties: None,
            value_type: normalize_schema(value_type)?,
        }),
        [name, Form::Map(properties), value_type] => Ok(SchemaField {
            name: name.clone(),
            properties: Some(Form::Map(properties.clone())),
            value_type: normalize_schema(value_type)?,
        }),
        _ => Err(":map schema fields must be [name type] or [name properties type]".into()),
    }
}

fn supports_properties(head: &str) -> bool {
    matches!(
        head,
        "str"
            | "string"
            | "keyword"
            | "symbol"
            | "list"
            | "bytes"
            | "int"
            | "long"
            | "bigint"
            | "integer"
            | "num"
            | "number"
            | "any"
            | "vector"
            | "set"
            | "map"
    )
}

fn normalize_composite(items: &[Form]) -> Result<SchemaType, String> {
    let Form::Keyword(head) = &items[0] else {
        return Ok(SchemaType::Unknown(Form::Vector(items.to_vec())));
    };
    let raw_arguments = &items[1..];
    let (properties, arguments) = if supports_properties(head) {
        match raw_arguments.first() {
            Some(Form::Map(values)) => (Some(Form::Map(values.clone())), &raw_arguments[1..]),
            _ => (None, raw_arguments),
        }
    } else {
        (None, raw_arguments)
    };
    let normalized = match head.as_str() {
        "or" => normalize_union_forms(arguments),
        "integer" if arguments.is_empty() => Ok(integer_schema()),
        "maybe" => {
            require_count(head, arguments, 1)?;
            let mut members = Vec::new();
            push_unique(&mut members, normalize_schema(&arguments[0])?);
            push_unique(&mut members, SchemaType::Primitive("nil".into()));
            Ok(SchemaType::Union(members))
        }
        "vector" => {
            require_count(head, arguments, 1)?;
            Ok(SchemaType::Vector(Box::new(normalize_schema(
                &arguments[0],
            )?)))
        }
        "set" => {
            require_count(head, arguments, 1)?;
            Ok(SchemaType::Set(Box::new(normalize_schema(&arguments[0])?)))
        }
        "tuple" => arguments
            .iter()
            .map(normalize_schema)
            .collect::<Result<Vec<_>, _>>()
            .map(SchemaType::Tuple),
        "map" => arguments
            .iter()
            .map(normalize_map_field)
            .collect::<Result<Vec<_>, _>>()
            .map(SchemaType::Map),
        "struct" => {
            let (mutable, arguments) = struct_mutability(arguments)?;
            normalize_struct_forms(arguments, mutable)
        }
        "fn" => normalize_function(items).map(|arity| SchemaType::Function(vec![arity])),
        "function" => {
            if arguments.is_empty() {
                return Err(":function schema requires at least one :fn schema".into());
            }
            arguments
                .iter()
                .map(|argument| {
                    let Form::Vector(function) = argument else {
                        return Err(":function members must be :fn schemas".into());
                    };
                    normalize_function(function)
                })
                .collect::<Result<Vec<_>, _>>()
                .map(SchemaType::Function)
        }
        "enum" => Ok(SchemaType::Enum(arguments.to_vec())),
        _ if arguments.is_empty() => Ok(SchemaType::Primitive(head.clone())),
        _ => Ok(SchemaType::Extension {
            head: head.clone(),
            arguments: arguments.to_vec(),
        }),
    }?;
    Ok(match properties {
        Some(properties) => SchemaType::WithProperties {
            schema: Box::new(normalized),
            properties,
        },
        None => normalized,
    })
}

fn integer_schema() -> SchemaType {
    SchemaType::Union(vec![
        SchemaType::Primitive("long".into()),
        SchemaType::Primitive("bigint".into()),
    ])
}

fn normalize_function(items: &[Form]) -> Result<FunctionSchema, String> {
    if !matches!(items.first(), Some(Form::Keyword(head)) if head == "fn") || items.len() != 3 {
        return Err(":fn schema must be [:fn [inputs ...] output]".into());
    }
    let Form::Vector(inputs) = &items[1] else {
        return Err(":fn schema inputs must be a vector".into());
    };
    let mut fixed = Vec::new();
    let mut rest = None;
    let mut index = 0;
    while index < inputs.len() {
        if matches!(&inputs[index], Form::Symbol(marker) if marker == "&") {
            if rest.is_some() || index + 2 != inputs.len() {
                return Err(":fn schema & must precede exactly one rest type".into());
            }
            rest = Some(Box::new(normalize_schema(&inputs[index + 1])?));
            index += 2;
        } else {
            fixed.push(normalize_schema(&inputs[index])?);
            index += 1;
        }
    }
    Ok(FunctionSchema {
        fixed,
        rest,
        output: Box::new(normalize_schema(&items[2])?),
    })
}

fn require_count(head: &str, arguments: &[Form], expected: usize) -> Result<(), String> {
    if arguments.len() == expected {
        Ok(())
    } else {
        Err(format!(
            ":{head} schema expects {expected} argument{}, got {}",
            if expected == 1 { "" } else { "s" },
            arguments.len()
        ))
    }
}

fn push_unique(output: &mut Vec<SchemaType>, value: SchemaType) {
    if !output.contains(&value) {
        output.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::parse;

    #[test]
    fn normalizes_nested_named_function_schemas() {
        assert_eq!(
            normalize_schema(&parse("[:fn [#'demo/Customer & :int] [:maybe :str]]").unwrap())
                .unwrap(),
            SchemaType::Function(vec![FunctionSchema {
                fixed: vec![SchemaType::Reference("demo/Customer".into())],
                rest: Some(Box::new(SchemaType::Primitive("int".into()))),
                output: Box::new(SchemaType::Union(vec![
                    SchemaType::Primitive("str".into()),
                    SchemaType::Primitive("nil".into()),
                ])),
            }])
        );
    }

    #[test]
    fn rejects_malformed_known_schema_forms() {
        assert!(normalize_schema(&parse("[:map [:name]]").unwrap()).is_err());
        assert!(normalize_schema(&parse("[:fn [:str & :int :bool] :str]").unwrap()).is_err());
        assert!(normalize_schema(&parse("[:maybe]").unwrap()).is_err());
    }

    #[test]
    fn integer_schema_is_the_long_or_big_integer_union() {
        let expected = SchemaType::Union(vec![
            SchemaType::Primitive("long".into()),
            SchemaType::Primitive("bigint".into()),
        ]);
        assert_eq!(normalize_schema(&parse(":integer").unwrap()).unwrap(), expected);
        assert_eq!(
            normalize_schema(&parse("[:integer]").unwrap()).unwrap(),
            expected
        );
    }

    #[test]
    fn struct_schema_is_a_first_class_normal_form() {
        let schema = normalize_schema(
            &parse(
                "[:struct {:mutable? true} (var demo/Cursor) \
                 [:position :int] [:limit {:optional true} [:maybe :int]]]",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            schema,
            SchemaType::Struct {
                name: "demo/Cursor".into(),
                mutable: true,
                fields: vec![
                    SchemaField {
                        name: Form::Symbol("position".into()),
                        properties: None,
                        value_type: SchemaType::Primitive("int".into()),
                    },
                    SchemaField {
                        name: Form::Symbol("limit".into()),
                        properties: Some(Form::Map(vec![(
                            Form::Keyword("optional".into()),
                            Form::Bool(true),
                        )])),
                        value_type: SchemaType::Union(vec![
                            SchemaType::Primitive("int".into()),
                            SchemaType::Primitive("nil".into()),
                        ]),
                    },
                ],
            }
        );
        assert_eq!(normalize_schema(&schema_shorthand(&schema)).unwrap(), schema);
    }

    #[test]
    fn struct_schema_requires_a_qualified_type_name() {
        assert!(normalize_schema(&parse("[:struct (var Cursor) [:value :any]]").unwrap()).is_err());
        assert!(normalize_schema(&parse("[:struct (var demo/Cursor) [:value]]").unwrap()).is_err());
    }

    #[test]
    fn infers_body_results_without_replacing_declared_contracts() {
        let forms = crate::kernel::parse_forms(
            "(ns demo)\n\
             (def Unary [:fn [:int] :number])\n\
             (defn ^{:schema #'Unary} choose [value]\n\
               (let [next (+ value 1)] (if true next 0)))\n\
             (defn labels [] {:name \"Ada\" :active true})\n\
             (defn select ([value] value) ([left right] right))",
        )
        .unwrap();
        let declarations = HashMap::from([(
            "demo/choose".into(),
            SchemaType::Reference("demo/Unary".into()),
        )]);
        let definitions = HashMap::from([(
            "demo/Unary".into(),
            normalize_schema(&parse("[:fn [:int] :number]").unwrap()).unwrap(),
        )]);
        let inferred = infer_function_types("demo", &forms, &declarations, &definitions);

        assert!(matches!(
            inferred.get("demo/choose"),
            Some(SchemaType::Function(arities))
                if arities[0].fixed == vec![SchemaType::Primitive("int".into())]
                    && *arities[0].output == SchemaType::Primitive("long".into())
        ));
        assert!(matches!(
            inferred.get("demo/labels"),
            Some(SchemaType::Function(arities))
                if matches!(arities[0].output.as_ref(), SchemaType::Map(fields) if fields.len() == 2)
        ));
        assert!(matches!(
            inferred.get("demo/select"),
            Some(SchemaType::Function(arities)) if arities.len() == 2
        ));
    }
}
