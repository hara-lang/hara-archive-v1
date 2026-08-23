use crate::lang::data::MetadataValue;
use crate::Runtime;

#[derive(Clone, Debug, PartialEq)]
pub enum DocumentationValue {
    Nil,
    Boolean(bool),
    Integer(i64),
    String(String),
    Array(Vec<DocumentationValue>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Documentation {
    pub symbol: String,
    pub doc: Option<String>,
    pub arglists: DocumentationValue,
    pub file: Option<String>,
    pub line: Option<i64>,
    pub column: Option<i64>,
}

fn value(metadata: &MetadataValue) -> DocumentationValue {
    match metadata {
        MetadataValue::Nil => DocumentationValue::Nil,
        MetadataValue::Boolean(value) => DocumentationValue::Boolean(*value),
        MetadataValue::Number(value) => DocumentationValue::Integer(*value),
        MetadataValue::Float(value) => DocumentationValue::String(value.to_string()),
        MetadataValue::BigInteger(value) => DocumentationValue::String(value.to_string()),
        MetadataValue::Regex(value) => DocumentationValue::String(value.clone()),
        MetadataValue::Character(value) => DocumentationValue::String(value.to_string()),
        MetadataValue::Tagged(tag, tagged) => {
            DocumentationValue::Array(vec![DocumentationValue::String(tag.clone()), value(tagged)])
        }
        MetadataValue::String(value) => DocumentationValue::String(value.clone()),
        MetadataValue::Keyword(value) => DocumentationValue::String(format!(":{}", value.as_str())),
        MetadataValue::Symbol(value) => DocumentationValue::String(value.as_str().to_owned()),
        MetadataValue::Vector(values)
        | MetadataValue::List(values)
        | MetadataValue::Set(values) => {
            DocumentationValue::Array(values.iter().map(value).collect())
        }
        MetadataValue::Map(values) => DocumentationValue::Array(
            values
                .iter()
                .flat_map(|(key, item)| [value(key), value(item)])
                .collect(),
        ),
    }
}

fn string(value: Option<&MetadataValue>) -> Option<String> {
    match value {
        Some(MetadataValue::String(value)) => Some(value.clone()),
        Some(MetadataValue::Symbol(value)) => Some(value.as_str().to_owned()),
        _ => None,
    }
}

fn integer(value: Option<&MetadataValue>) -> Option<i64> {
    match value {
        Some(MetadataValue::Number(value)) => Some(*value),
        Some(MetadataValue::String(value)) => value.parse().ok(),
        _ => None,
    }
}

pub(super) fn lookup(runtime: &Runtime, symbol: &str) -> Result<Documentation, String> {
    let metadata = runtime
        .var_metadata(symbol)
        .ok_or_else(|| format!("No documentation symbol: {symbol}"))?;
    let hara = metadata.hara.as_deref();
    let doc = hara
        .and_then(|value| value.doc().map(str::to_owned))
        .or(metadata.doc);
    let arglists = hara
        .and_then(|value| value.get_keyword("arglists"))
        .map(value)
        .unwrap_or_else(|| {
            DocumentationValue::Array(
                metadata
                    .arglists
                    .into_iter()
                    .map(DocumentationValue::String)
                    .collect(),
            )
        });
    Ok(Documentation {
        symbol: symbol.into(),
        doc,
        arglists,
        file: string(hara.and_then(|value| value.get_keyword("file"))),
        line: integer(hara.and_then(|value| value.get_keyword("line"))),
        column: integer(hara.and_then(|value| value.get_keyword("column"))),
    })
}
