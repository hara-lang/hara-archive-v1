use super::{IDisplay, IMetadata};
use hara_protocol_macros::hara_protocol;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjType {
    Class,
    Type,
    Nil,
    Boolean,
    Number,
    Character,
    String,
    Symbol,
    Keyword,
    Pattern,
    Date,
    Uuid,
    Uri,
    Sequential,
    List,
    Vector,
    MapEntry,
    Map,
    Set,
    Function,
    Atom,
    Meta,
    Object,
    Iterator,
    Future,
    Promise,
    Delay,
    Pending,
    Error,
    Reader,
    Pointer,
}

#[hara_protocol(
    namespace = "std.protocol.iobjtype",
    name = "IObjType",
    parents = ["IHash", "IDisplay"],
    inherited_methods = [("meta", "meta", 1), ("with-meta", "with_meta", 2)]
)]
pub trait IObjType: IDisplay + IMetadata {
    fn obj_type(&self) -> ObjType {
        ObjType::Class
    }

    fn obj_name(&self) -> &'static str {
        match self.obj_type() {
            ObjType::Class => "CLASS",
            ObjType::Type => "TYPE",
            ObjType::Nil => "NIL",
            ObjType::Boolean => "BOOLEAN",
            ObjType::Number => "NUMBER",
            ObjType::Character => "CHARACTER",
            ObjType::String => "STRING",
            ObjType::Symbol => "SYMBOL",
            ObjType::Keyword => "KEYWORD",
            ObjType::Pattern => "PATTERN",
            ObjType::Date => "DATE",
            ObjType::Uuid => "UUID",
            ObjType::Uri => "URI",
            ObjType::Sequential => "SEQUENTIAL",
            ObjType::List => "LIST",
            ObjType::Vector => "VECTOR",
            ObjType::MapEntry => "MAP_ENTRY",
            ObjType::Map => "MAP",
            ObjType::Set => "SET",
            ObjType::Function => "FUNCTION",
            ObjType::Atom => "ATOM",
            ObjType::Meta => "META",
            ObjType::Object => "OBJECT",
            ObjType::Iterator => "ITERATOR",
            ObjType::Future => "FUTURE",
            ObjType::Promise => "PROMISE",
            ObjType::Delay => "DELAY",
            ObjType::Pending => "PENDING",
            ObjType::Error => "ERROR",
            ObjType::Reader => "READER",
            ObjType::Pointer => "POINTER",
        }
    }

    fn hash_seed(&self) -> String {
        format!("::{}", self.obj_name())
    }
}
