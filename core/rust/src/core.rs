#![allow(clippy::too_many_lines)] // Temporary compatibility facade during Java-port split.
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};

pub use crate::kernel::Form;
use crate::kernel::{NamespaceLoadState, NamespaceRegistry, Var as KernelVar, VarOrigin};
use crate::lang::data::List as PList;
use crate::lang::data::{
    Atom as PAtom, Cons as PCons, Deque as PDeque, Keyword, Map as PMap, OrderedMap as POrderedMap,
    OrderedSet as POrderedSet, Pointer as PPointer, PriorityMap as PPriorityMap, Queue as PQueue,
    Seq as PSeq, Set as PSet, SortedMap as PSortedMap, SortedSet as PSortedSet, Symbol,
    TaggedLiteral as PTaggedLiteral, Trie as PTrie, Tuple as PTuple, Vector as PVector,
};
use crate::lang::data::{Metadata, MetadataValue};
use crate::lang::data::{
    MutableList, MutableMap, MutableOrderedMap, MutableOrderedSet, MutableQueue, MutableSet,
    MutableSortedMap, MutableSortedSet, MutableTrie, MutableVector,
};
use crate::lang::hash::JavaHash;
use crate::lang::protocol::{
    IDisplay, IEmpty, IFn, IMetadata, INamespaced, IPopFirst, IPopLast, IToMutable, IToPersistent,
};
use crate::numeric::{self, ArithmeticOp};
pub use crate::task::{
    LocalPromiseProvider, Promise, PromiseProvider, PromiseRejection, PromiseState,
};
use num_bigint::BigInt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

thread_local! {
    static ACTIVE_EVALUATION_INTERRUPT: RefCell<Option<Rc<dyn Fn() -> Option<String>>>> =
        const { RefCell::new(None) };
    static ACTIVE_EXCEPTION_SITE: RefCell<Option<ExceptionSite>> = const { RefCell::new(None) };
}

pub(crate) fn with_exception_site<R>(site: ExceptionSite, operation: impl FnOnce() -> R) -> R {
    ACTIVE_EXCEPTION_SITE.with(|active| {
        let previous = active.replace(Some(site));
        let result = operation();
        active.replace(previous);
        result
    })
}

pub(crate) fn current_exception_site() -> Option<ExceptionSite> {
    ACTIVE_EXCEPTION_SITE.with(|active| active.borrow().clone())
}

pub(crate) fn exception_site_at(line: usize, column: usize) -> Option<ExceptionSite> {
    Some(current_exception_site().map_or(
        ExceptionSite {
            namespace: None,
            resource: None,
            line,
            column,
        },
        |mut site| {
            site.line = line;
            site.column = column;
            site
        },
    ))
}

pub fn exception_located_form(node: &crate::kernel::SpannedForm) -> Form {
    let rebuilt = match &node.form {
        Form::List(values)
            if values.first().is_some_and(|value| {
                matches!(value, Form::Symbol(name) if name == "quote" || name == "'")
            }) =>
        {
            Form::List(values.clone())
        }
        Form::List(values) if node.children.len() == values.len() => {
            Form::List(node.children.iter().map(exception_located_form).collect())
        }
        Form::List(values) if node.children.len() + 1 == values.len() => {
            let mut rebuilt = vec![values[0].clone()];
            rebuilt.extend(node.children.iter().map(exception_located_form));
            Form::List(rebuilt)
        }
        Form::Vector(values) if node.children.len() == values.len() => {
            Form::Vector(node.children.iter().map(exception_located_form).collect())
        }
        Form::Set(values) if node.children.len() == values.len() => {
            Form::Set(node.children.iter().map(exception_located_form).collect())
        }
        Form::Map(values) if node.children.len() == values.len() * 2 => Form::Map(
            node.children
                .chunks_exact(2)
                .map(|pair| {
                    (
                        exception_located_form(&pair[0]),
                        exception_located_form(&pair[1]),
                    )
                })
                .collect(),
        ),
        Form::Tagged(tag, _) if node.children.len() == 1 => Form::Tagged(
            tag.clone(),
            Box::new(exception_located_form(&node.children[0])),
        ),
        Form::Metadata(metadata, _) if node.children.len() == 1 => Form::Metadata(
            metadata.clone(),
            Box::new(exception_located_form(&node.children[0])),
        ),
        form => form.clone(),
    };
    let Form::List(mut values) = rebuilt else {
        return rebuilt;
    };
    let Some(Form::Symbol(operator)) = values.first() else {
        return Form::List(values);
    };
    if !matches!(
        operator.as_str(),
        "throw" | "ex" | "ex-info" | "std.foundation/ex" | "std.foundation/ex-info"
    ) {
        return Form::List(values);
    }
    let original_operator = values[0].clone();
    let marker = if operator == "throw" {
        "__throw-at"
    } else {
        "__ex-at"
    };
    values[0] = Form::Symbol(marker.into());
    values.insert(1, Form::Number(node.span.start.line as i64));
    values.insert(2, Form::Number(node.span.start.column as i64));
    if marker == "__ex-at" {
        values.insert(3, original_operator);
    }
    Form::List(values)
}

pub(crate) fn with_evaluation_interrupt<R>(
    interrupt: Rc<dyn Fn() -> Option<String>>,
    operation: impl FnOnce() -> R,
) -> R {
    ACTIVE_EVALUATION_INTERRUPT.with(|active| {
        let previous = active.replace(Some(interrupt));
        let result = operation();
        active.replace(previous);
        result
    })
}

pub(crate) fn check_evaluation_interrupt() -> Result<(), String> {
    ACTIVE_EVALUATION_INTERRUPT.with(|active| {
        active
            .borrow()
            .as_ref()
            .and_then(|interrupt| interrupt())
            .map_or(Ok(()), Err)
    })
}

#[path = "fiber.rs"]
mod fiber;
#[path = "core/native_result.rs"]
mod native_result;
pub use native_result::{ResultStatus, ResultValue};
#[cfg(not(feature = "raw-wasm"))]
#[path = "native_crypto.rs"]
mod native_crypto;
#[cfg(feature = "raw-wasm")]
mod native_crypto {
    use super::Value;

    pub(super) fn operation(_operation: &str, _arguments: Vec<Value>) -> Result<Value, String> {
        Err("std.native.Crypto is unavailable in raw Wasm".into())
    }
}
pub(crate) use fiber::Cont;
pub use fiber::{EvalFiber, EvalFiberState, Step};

include!("core/registry.rs");
include!("core/native_declarations.rs");
include!("core/value.rs");
include!("core/vm_tool.rs");
#[cfg(all(feature = "bytecode-vm", not(feature = "raw-wasm")))]
include!("core/package_tool.rs");
#[cfg(any(not(feature = "bytecode-vm"), feature = "raw-wasm"))]
pub(crate) fn package_tool_provider_values() -> Vec<(&'static str, Value)> {
    Vec::new()
}
include!("core/inspection.rs");
include!("core/environment.rs");
include!("core/native.rs");
include!("core/provider.rs");
include!("core/async_value.rs");
include!("core/primitive.rs");
include!("core/protocol.rs");
include!("core/operation.rs");
include!("core/form.rs");
include!("core/namespace.rs");
// Synchronous declaration and compatibility forms remain isolated from the
// fiber execution target. Runtime source evaluation enters through EvalFiber;
// this module is only the small compatibility seam used by namespace and
// bytecode declaration machinery.
include!("core/special_forms.rs");
include!("core/bootstrap.rs");
