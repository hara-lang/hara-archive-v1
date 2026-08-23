use super::{Keyword, Symbol};
use num_bigint::BigInt;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub enum MetadataValue {
    Nil,
    Boolean(bool),
    Number(i64),
    Float(f64),
    BigInteger(BigInt),
    Character(char),
    Regex(String),
    Tagged(String, Box<MetadataValue>),
    String(String),
    Keyword(Keyword),
    Symbol(Symbol),
    Vector(Vec<MetadataValue>),
    List(Vec<MetadataValue>),
    Set(Vec<MetadataValue>),
    Map(Vec<(MetadataValue, MetadataValue)>),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metadata {
    entries: Vec<(MetadataValue, MetadataValue)>,
}

impl Metadata {
    pub fn new(entries: Vec<(MetadataValue, MetadataValue)>) -> Rc<Self> {
        Rc::new(Self { entries })
    }

    pub fn document(value: impl Into<String>) -> Rc<Self> {
        Self::new(vec![(
            MetadataValue::Keyword(Keyword::from("doc")),
            MetadataValue::String(value.into()),
        )])
    }

    pub fn entries(&self) -> &[(MetadataValue, MetadataValue)] {
        &self.entries
    }

    pub fn get(&self, key: &MetadataValue) -> Option<&MetadataValue> {
        self.entries
            .iter()
            .rev()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value)
    }

    pub fn get_keyword(&self, key: &str) -> Option<&MetadataValue> {
        self.get(&MetadataValue::Keyword(Keyword::from(key)))
    }

    pub fn flag(&self, key: &str) -> bool {
        matches!(self.get_keyword(key), Some(MetadataValue::Boolean(true)))
    }

    pub fn doc(&self) -> Option<&str> {
        match self.get_keyword("doc") {
            Some(MetadataValue::String(value)) => Some(value),
            _ => None,
        }
    }
}
