//! Compiler: `Form` trees (with parser spans) to a validated `Program`.
//!
//! Supports the milestone-4 synchronous subset: literals, lexical locals,
//! the ten shared primitives, `if`, `do`, `let`, `loop`/`recur`, `fn`
//! closures with capture-by-value upvalues (including variadic
//! parameters), direct calls, exceptions, and the registry-direct global
//! forms — `def`, `defn`/`defn-` (single- and multi-arity, interning real
//! late-bound vars), `var`, `set!`, `declare`, `defstruct`, `defmutable`, `field`, and
//! `instance?` (issue #223; see
//! `specs/01-lang/010-bytecode/draft/hal-bytecode-vm.edn` `:vm/namespaces`).
//! Anything else is a typed [`CompileError`] with source context; the
//! compiler never emits fallback calls into the tree-walking evaluator.
//!
//! Structure: shared state (constants, finished prototypes) plus a stack
//! of function contexts. Each context owns a code buffer, scope stack,
//! loop stack, and capture list. Slot layout per function: parameters at
//! `0..arity-1`, captures at `arity..arity+capture_count-1`, body locals
//! above. Captures are discovered by a free-variable pre-pass over the
//! body, so their slots are reserved (and pre-declared in the function's
//! base scope) before any body-local slot is allocated.

use crate::core::{IntrinsicOp, Value};
use crate::kernel::{Form, Position, Span, SpannedForm};
use crate::lang::data::List as PList;
use std::collections::{HashMap, HashSet};

use super::error::{CompileError, CompileErrorKind};
use super::opcode::Instruction;
use super::program::{
    FunctionPrototype, Program, TryEntry, MAX_CONSTANTS, MAX_PRIMITIVE_ARGUMENTS,
};
use super::source_map::SourceMap;
use super::validate::{self, stack_heights};

#[path = "compiler/bindings.rs"]
mod bindings;
#[path = "compiler/calls.rs"]
mod calls;
#[path = "compiler/coroutines.rs"]
mod coroutines;
#[path = "compiler/destructure.rs"]
mod destructure;
#[path = "compiler/exceptions.rs"]
mod exceptions;
#[path = "compiler/functions.rs"]
mod functions;
#[path = "compiler/literals.rs"]
mod literals;
#[path = "compiler/scope.rs"]
mod scope;
use exceptions::TryContext;
#[path = "compiler/globals.rs"]
mod globals;
#[path = "compiler/recur.rs"]
mod recur;
use recur::LoopContext;
use scope::ScopeStack;

/// Operators that name language forms the VM does not implement. In
/// operator position they report as unsupported rather than as unbound
/// symbols; everything else unbound reports as an unbound symbol,
/// matching the evaluator.
const UNSUPPORTED_OPERATORS: &[&str] = &["in-ns", "await"];

/// Compiles source text into a validated program. Multiple top-level
/// forms compile as an implicit `do`. Without a namespace registry the
/// program must be closed: only the names it declares itself are
/// visible as globals (issue #223).
pub fn compile_source(source: &str) -> Result<Program, CompileError> {
    let forms = crate::kernel::read_forms(source)?;
    compile_spanned_forms(&forms)
}

fn compile_spanned_forms(forms: &[SpannedForm]) -> Result<Program, CompileError> {
    prepare_top_level_namespaces(forms)?;
    compile_spanned_forms_without_namespace_preparation(forms, HashSet::new(), false)
}

fn compile_spanned_forms_without_namespace_preparation(
    forms: &[SpannedForm],
    excluded_foundation_libraries: HashSet<String>,
    allow_unbound_globals: bool,
) -> Result<Program, CompileError> {
    let mut compiler = Compiler::new(excluded_foundation_libraries, allow_unbound_globals);
    compiler.predeclare_top_level(forms);
    let children = compiler.children(forms);
    compiler.compile_sequence(&children, true)?;
    compiler.finish()
}

fn prepare_top_level_namespaces(forms: &[SpannedForm]) -> Result<(), CompileError> {
    fn prepare(form: &Form, position: Position) -> Result<(), CompileError> {
        let Form::List(items) = crate::core::form_without_metadata(form) else {
            return Ok(());
        };
        match items.first() {
            Some(Form::Symbol(operator)) if operator == "ns" || operator == "ns+" => {
                crate::core::prepare_namespace_form(form).map_err(|message| {
                    CompileError::new(CompileErrorKind::UnsupportedForm, message, Some(position))
                })
            }
            Some(Form::Symbol(operator)) if operator == "do" => {
                for child in items.iter().skip(1) {
                    prepare(child, position)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    for form in forms {
        prepare(&form.form, form.span.start)?;
    }
    Ok(())
}

/// Compiles against a caller's namespace registry: registry vars
/// (std.foundation and anything already interned) are visible to the
/// two-phase global check, exactly as they will resolve at execution
/// time through `execute_program_with_globals` (issue #223).
pub fn compile_source_with(
    source: &str,
    registry: &crate::kernel::NamespaceRegistry<crate::core::Value>,
) -> Result<Program, CompileError> {
    let forms = crate::kernel::read_forms(source)?;
    let config = source_namespace_config(&forms)?;
    compile_spanned_forms_with_config(&forms, registry, config, true)
}

/// Variant of [`compile_source_with`] for direct runtime evaluation. It keeps
/// source namespace configuration intact while allowing late-bound globals
/// which a preceding dynamic form may define.
pub fn compile_source_with_allow_unbound_globals(
    source: &str,
    registry: &crate::kernel::NamespaceRegistry<crate::core::Value>,
) -> Result<Program, CompileError> {
    let forms = crate::kernel::read_forms(source)?;
    let config = source_namespace_config(&forms)?;
    compile_spanned_forms_with_config_options(&forms, registry, config, true, true)
}

/// Compiles source against a caller-owned registry and namespace configuration.
/// The configuration is applied to parsed forms before bytecode lowering so
/// aliases remain source-positioned and the VM does not need an evaluator or
/// text round-trip fallback. Runtime callers use this when their namespace
/// declaration has already been loaded and its complete config is available.
pub fn compile_source_with_config(
    source: &str,
    registry: &crate::kernel::NamespaceRegistry<crate::core::Value>,
    config: crate::kernel::GeneratedNamespaceConfig,
) -> Result<Program, CompileError> {
    let forms = crate::kernel::read_forms(source)?;
    compile_spanned_forms_with_config(&forms, registry, config, true)
}

/// Compiles source for a direct-native runtime escape hatch. Dynamic Hara
/// evaluation may define a Var before a later form in the same native frame
/// reads it, so unresolved names are emitted as late-bound global reads. The
/// direct runtime still fails at the read if the Var was not actually
/// materialized; no evaluator fallback is introduced.
pub fn compile_source_with_config_allow_unbound_globals(
    source: &str,
    registry: &crate::kernel::NamespaceRegistry<crate::core::Value>,
    config: crate::kernel::GeneratedNamespaceConfig,
) -> Result<Program, CompileError> {
    let forms = crate::kernel::read_forms(source)?;
    compile_spanned_forms_with_config_options(&forms, registry, config, true, true)
}

/// Compiles an already-read form without printing it back to source first.
/// This is used by the native runtime's ordinary form boundary so metadata
/// such as `^:async` remains part of the bytecode contract.
pub fn compile_form_with_config_allow_unbound_globals(
    form: Form,
    registry: &crate::kernel::NamespaceRegistry<crate::core::Value>,
    config: crate::kernel::GeneratedNamespaceConfig,
) -> Result<Program, CompileError> {
    let form = synthetic_spanned_form(form);
    compile_spanned_form_with_config_allow_unbound_globals(form, registry, config)
}

/// Compiles already-read source forms without discarding their parser spans.
/// Runtime entry points use this boundary so direct-native execution reports
/// the source location of nested exception creation and throw instructions
/// while preserving one program for each contiguous ordinary-form batch.
pub fn compile_spanned_forms_with_config_allow_unbound_globals(
    forms: &[SpannedForm],
    registry: &crate::kernel::NamespaceRegistry<crate::core::Value>,
    config: crate::kernel::GeneratedNamespaceConfig,
) -> Result<Program, CompileError> {
    compile_spanned_forms_with_config_options(forms, registry, config, false, true)
}

/// Compiles an already-read source form without discarding its parser span.
/// Prefer [`compile_spanned_forms_with_config_allow_unbound_globals`] when a
/// runtime entry point has more than one contiguous ordinary source form.
pub fn compile_spanned_form_with_config_allow_unbound_globals(
    form: SpannedForm,
    registry: &crate::kernel::NamespaceRegistry<crate::core::Value>,
    config: crate::kernel::GeneratedNamespaceConfig,
) -> Result<Program, CompileError> {
    compile_spanned_forms_with_config_allow_unbound_globals(&[form], registry, config)
}

/// Reports whether source contains a language-level dynamic evaluation
/// boundary. Direct-native callers use this to permit late-bound reads which
/// may be materialized by `Runtime/eval`, `load-string`, or `defonce` during
/// execution. The scan is deliberately structural so a string containing the
/// word `eval` does not change compilation policy.
pub fn source_uses_dynamic_evaluation(source: &str) -> Result<bool, CompileError> {
    let forms = crate::kernel::read_forms(source)?;

    fn dynamic_symbol(name: &str) -> bool {
        matches!(
            name.rsplit_once('/').map_or(name, |(_, local)| local),
            "defonce" | "eval" | "eval-in" | "eval-in-ns" | "load-string" | "with-ns"
        )
    }

    fn contains(form: &Form) -> bool {
        match crate::core::form_without_metadata(form) {
            Form::List(values) => {
                values.first().is_some_and(
                    |value| matches!(value, Form::Symbol(name) if dynamic_symbol(name)),
                ) || values.iter().any(contains)
            }
            Form::Vector(values) | Form::Set(values) => values.iter().any(contains),
            Form::Map(entries) => entries
                .iter()
                .any(|(key, value)| contains(key) || contains(value)),
            Form::Tagged(_, value) => contains(value),
            _ => false,
        }
    }

    Ok(forms.iter().any(|form| contains(&form.form)))
}

fn compile_spanned_forms_with_config(
    forms: &[SpannedForm],
    registry: &crate::kernel::NamespaceRegistry<crate::core::Value>,
    config: crate::kernel::GeneratedNamespaceConfig,
    prepare_namespaces: bool,
) -> Result<Program, CompileError> {
    compile_spanned_forms_with_config_options(forms, registry, config, prepare_namespaces, false)
}

fn compile_spanned_forms_with_config_options(
    forms: &[SpannedForm],
    registry: &crate::kernel::NamespaceRegistry<crate::core::Value>,
    mut config: crate::kernel::GeneratedNamespaceConfig,
    prepare_namespaces: bool,
    allow_unbound_globals: bool,
) -> Result<Program, CompileError> {
    crate::core::with_namespace_registry(registry, || {
        if prepare_namespaces {
            prepare_top_level_namespaces(forms)?;
        }
        sync_registry_global_aliases(&mut config, registry);
        let rewritten = forms
            .iter()
            .map(|form| rewrite_spanned_form(form, &config))
            .collect::<Vec<_>>();
        compile_spanned_forms_without_namespace_preparation(
            &rewritten,
            config.excluded_foundation_libraries().clone(),
            allow_unbound_globals,
        )
    })
}

pub fn source_namespace_config(
    forms: &[SpannedForm],
) -> Result<crate::kernel::GeneratedNamespaceConfig, CompileError> {
    let mut selected = None;
    for form in forms {
        let crate::kernel::Form::List(items) = crate::core::form_without_metadata(&form.form)
        else {
            continue;
        };
        let Some(crate::kernel::Form::Symbol(operator)) = items.first() else {
            continue;
        };
        let clause_start = match operator.as_str() {
            "ns" => 2,
            "ns+" => 1,
            _ => continue,
        };
        if items.len() < clause_start {
            return Err(CompileError::new(
                CompileErrorKind::UnsupportedForm,
                "namespace declaration is missing its clauses",
                Some(form.span.start),
            ));
        }
        if operator == "ns" || items.len() > clause_start {
            selected = Some(
                crate::kernel::GeneratedNamespaceConfig::configure_with(
                    &items[clause_start..],
                    |_| true,
                )
                .map_err(|message| {
                    CompileError::new(
                        CompileErrorKind::UnsupportedForm,
                        message,
                        Some(form.span.start),
                    )
                })?,
            );
        }
    }
    Ok(selected.unwrap_or_else(crate::kernel::GeneratedNamespaceConfig::defaults))
}

fn sync_registry_global_aliases(
    config: &mut crate::kernel::GeneratedNamespaceConfig,
    registry: &crate::kernel::NamespaceRegistry<crate::core::Value>,
) {
    let excluded_foundation_libraries = config.excluded_foundation_libraries().clone();
    let excluded_foundation = config.excluded_foundation().clone();
    let current = registry.current();
    for library in &excluded_foundation_libraries {
        if let Some(alias) = crate::kernel::generated::foundation_library_alias(library) {
            current.unalias(alias);
        }
    }
    for (alias, target) in current.aliases() {
        let library = target.name().as_str().strip_prefix("std.foundation.");
        if library.is_some_and(|library| excluded_foundation_libraries.contains(library)) {
            current.unalias(alias.as_str());
        }
    }
    for (alias, target) in current.lazy_aliases() {
        let library = target.as_str().strip_prefix("std.foundation.");
        if library.is_some_and(|library| excluded_foundation_libraries.contains(library)) {
            current.unalias(alias.as_str());
        }
    }
    config.set_global_aliases(
        registry
            .global_aliases()
            .into_iter()
            .filter(|(_, namespace)| {
                let library = namespace
                    .as_str()
                    .strip_prefix("std.foundation.")
                    .unwrap_or_default();
                !excluded_foundation_libraries.contains(library)
                    && !excluded_foundation.contains(library)
            })
            .map(|(alias, namespace)| (alias.as_str().to_owned(), namespace.as_str().to_owned())),
    );
}

pub(crate) fn rewrite_spanned_form(
    form: &SpannedForm,
    config: &crate::kernel::GeneratedNamespaceConfig,
) -> SpannedForm {
    SpannedForm {
        form: config.rewrite(form.form.clone()),
        span: form.span.clone(),
        children: form
            .children
            .iter()
            .map(|child| rewrite_spanned_form(child, config))
            .collect(),
    }
}

/// Lowers decoded HALC directly into bytecode while preserving its canonical
/// schema graph in the resulting program. The module's `ns` declaration is
/// loader configuration rather than executable code and is omitted here.
pub fn compile_halc_module(
    module: &crate::kernel::halc::HalcModule,
    registry: &crate::kernel::NamespaceRegistry<crate::core::Value>,
) -> Result<Program, CompileError> {
    let previous = registry.current().name().as_str().to_owned();
    registry.set_current(&module.namespace);
    let forms = module
        .forms
        .iter()
        .filter(|form| !top_level_operator(form, "ns"))
        .cloned()
        .map(synthetic_spanned_form)
        .collect::<Vec<_>>();
    let result = compile_spanned_forms_with_config(
        &forms,
        registry,
        crate::kernel::GeneratedNamespaceConfig::defaults(),
        false,
    );
    registry.set_current(previous);
    let mut program = result?;
    program.namespace = Some(module.namespace.clone());
    program.schema_types = module.schemas.definition_types.clone();
    program.function_types = module.schemas.function_types.clone();
    program.inferred_function_types = crate::kernel::schema::infer_function_types(
        &module.namespace,
        &module.forms,
        &program.function_types,
        &program.schema_types,
    );
    validate_declared_function_arities(&program)?;
    Ok(program)
}

fn validate_declared_function_arities(program: &Program) -> Result<(), CompileError> {
    for (index, prototype) in program.functions.iter().enumerate() {
        let Some(crate::kernel::SchemaType::Function(arities)) =
            program.function_schema(index as u16)
        else {
            continue;
        };
        let compatible = arities.iter().any(|schema| {
            schema.fixed.len() == prototype.arity as usize
                && schema.rest.is_some() == prototype.variadic
        });
        if !compatible {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                format!(
                    "function schema for {} has no {}-argument arity{}",
                    prototype.name.as_deref().unwrap_or("<anonymous>"),
                    prototype.arity,
                    if prototype.variadic {
                        " with rest arguments"
                    } else {
                        ""
                    }
                ),
                None,
            ));
        }
    }
    Ok(())
}

fn top_level_operator(form: &Form, expected: &str) -> bool {
    matches!(
        crate::core::form_without_metadata(form),
        Form::List(items)
            if matches!(items.first(), Some(Form::Symbol(operator)) if operator == expected)
    )
}

pub(crate) fn synthetic_spanned_form(form: Form) -> SpannedForm {
    let position = Position {
        offset: 0,
        line: 1,
        column: 1,
    };
    let children = match &form {
        Form::List(values) | Form::Vector(values) | Form::Set(values) => {
            values.iter().cloned().map(synthetic_spanned_form).collect()
        }
        Form::Map(entries) => entries
            .iter()
            .flat_map(|(key, value)| [key.clone(), value.clone()])
            .map(synthetic_spanned_form)
            .collect(),
        Form::Tagged(_, value) | Form::Metadata(_, value) => {
            vec![synthetic_spanned_form(value.as_ref().clone())]
        }
        _ => Vec::new(),
    };
    SpannedForm {
        form,
        span: Span {
            start: position,
            end: position,
        },
        children,
    }
}

/// A form paired with its span and (when the parser provided matching
/// children) the spans of its elements.
#[derive(Clone, Copy)]
struct Child<'a> {
    form: &'a Form,
    span: &'a Span,
    children: Option<&'a [SpannedForm]>,
}

/// One in-progress function body: code, scopes, loops, and captures.
/// The entry function is context 0 with arity and captures 0.
struct FnContext {
    /// Reserved index into `Compiler::functions`.
    proto_id: usize,
    name: Option<String>,
    /// Fixed parameter count; params occupy slots `0..params-1`.
    params: u16,
    /// Whether the function has a `& rest` parameter (occupying the slot
    /// directly above the fixed params, below the captures).
    variadic: bool,
    /// Whether `std.foundation.coroutine/await` may be emitted in this
    /// function. Every function context resets this flag.
    suspend_allowed: bool,
    /// Whether calls to this prototype return a result promise.
    async_function: bool,
    /// Captured free variables in slot order (slots `params..`); each
    /// entry carries the first-occurrence position for diagnostics.
    captures: Vec<(String, Option<Position>)>,
    code: Vec<Instruction>,
    source_map: SourceMap,
    scopes: ScopeStack,
    loops: Vec<LoopContext>,
    tries: Vec<TryContext>,
    /// Finished handler table entries for this function; entry depths are
    /// patched in after stack analysis in `finish`.
    handlers: Vec<TryEntry>,
    /// Whether control can reach the next emitted instruction. `recur`
    /// clears it; the compiler emits no dead code.
    fallthrough: bool,
}

struct Compiler {
    /// Namespace selected when compilation began. Macro expansion may load
    /// other modules, but global binding must remain relative to this owner.
    namespace: String,
    constants: Vec<Value>,
    constant_index: HashMap<Value, u32>,
    functions: Vec<FunctionPrototype>,
    contexts: Vec<FnContext>,
    /// Names this program defines (`def`/`defn`/`declare`/`defstruct`):
    /// visible to global references compiled after their defining form
    /// (issue #223 two-phase visibility).
    globals: Vec<String>,
    /// Foundation child libraries explicitly removed by the source namespace config.
    /// This is checked before the process-wide global alias registry so an
    /// excluded `str/` (or equivalent) cannot be resurrected by lookup.
    excluded_foundation_libraries: HashSet<String>,
    /// Direct runtime evaluation can materialize a global after the current
    /// source has already been compiled. Such names use the same late-bound
    /// `GetGlobal` instruction but are allowed through the compile-time check.
    allow_unbound_globals: bool,
    /// Source-level forwarding shims opted into call-site lowering with
    /// `^{:inline target/name}`.
    inline_globals: HashMap<String, String>,
    /// Var metadata table indexed by `DefGlobal` operands.
    var_metadata: Vec<std::rc::Rc<crate::lang::data::Metadata>>,
    /// True while compiling a direct child of the top-level sequence;
    /// `defn` and `declare` are only legal there.
    top_level: bool,
    next_destructure_id: u64,
}

/// The reservation placed in `functions` while a body is compiled: the
/// prototype index, arity, and capture count are known up front, the
/// code is filled in when the context closes.
fn placeholder(
    name: Option<String>,
    arity: u16,
    capture_count: u16,
    variadic: bool,
    async_function: bool,
) -> FunctionPrototype {
    FunctionPrototype {
        name,
        async_function,
        arity,
        variadic,
        capture_count,
        local_count: 0,
        max_stack: 0,
        code: Vec::new(),
        source_map: SourceMap::default(),
        handlers: Vec::new(),
    }
}

impl Compiler {
    fn predeclare_top_level(&mut self, forms: &[SpannedForm]) {
        for spanned in forms {
            self.predeclare_nested_definitions(&spanned.form);
            let Form::List(items) = crate::core::form_without_metadata(&spanned.form) else {
                continue;
            };
            let Some(Form::Symbol(operator)) = items.first() else {
                continue;
            };
            if operator == "declare" {
                for item in items.iter().skip(1) {
                    if let Form::Symbol(name) = crate::core::form_without_metadata(item) {
                        if !name.contains('/') {
                            self.declare_program_global(name);
                        }
                    }
                }
                continue;
            }
            if !matches!(
                operator.as_str(),
                "def"
                    | "defonce"
                    | "defn"
                    | "defn-"
                    | "defmacro"
                    | "defstruct"
                    | "defmutable"
                    | "defprotocol"
                    | "defmulti"
            ) {
                continue;
            }
            let Some(name_form) = items.get(1) else {
                continue;
            };
            if let Ok((name, _)) = crate::core::binding_symbol(name_form, "definition name") {
                if !name.contains('/') {
                    self.declare_program_global(&name);
                    if operator == "defstruct" || operator == "defmutable" {
                        self.declare_program_global(&format!("->{name}"));
                        self.declare_program_global(&format!("map->{name}"));
                    }
                }
            }
        }
    }

    /// Nested `defn` forms still intern namespace-owned Vars at runtime. Make
    /// their names visible during the enclosing function's free-variable pass
    /// so calls resolve as late-bound globals instead of being mistaken for
    /// captures. Definitions inside quoted data and syntax templates are data,
    /// not executable declarations.
    fn predeclare_nested_definitions(&mut self, form: &Form) {
        let form = crate::core::form_without_metadata(form);
        match form {
            Form::List(items) => {
                let Some(Form::Symbol(operator)) = items.first() else {
                    return;
                };
                if matches!(operator.as_str(), "quote" | "syntax-quote" | "comment") {
                    return;
                }
                // `defonce` is a Foundation macro which lowers to a runtime
                // `(def ...)`.  Unlike an ordinary `defn`, the definition is
                // hidden behind the macro expansion and therefore cannot be
                // discovered by the normal top-level declaration pass.  It
                // still establishes a namespace-owned global before later
                // forms in the same function execute (the evaluator permits
                // this because it compiles/evaluates one form at a time).
                if matches!(operator.as_str(), "defonce" | "std.foundation/defonce") {
                    if let Some(Form::Symbol(name)) =
                        items.get(1).map(crate::core::form_without_metadata)
                    {
                        if !name.contains('/') {
                            self.declare_program_global(name);
                        }
                    }
                }
                if matches!(operator.as_str(), "defn" | "defn-") {
                    if let Some(Form::Symbol(name)) =
                        items.get(1).map(crate::core::form_without_metadata)
                    {
                        if !name.contains('/') {
                            self.declare_program_global(name);
                        }
                    }
                }
                for child in items {
                    self.predeclare_nested_definitions(child);
                }
            }
            Form::Vector(values) | Form::Set(values) => {
                for child in values {
                    self.predeclare_nested_definitions(child);
                }
            }
            Form::Map(entries) => {
                for (key, value) in entries {
                    self.predeclare_nested_definitions(key);
                    self.predeclare_nested_definitions(value);
                }
            }
            _ => {}
        }
    }

    fn new(
        excluded_foundation_libraries: HashSet<String>,
        allow_unbound_globals: bool,
    ) -> Compiler {
        let mut scopes = ScopeStack::new();
        scopes.push_scope();
        Compiler {
            namespace: crate::core::namespace_registry()
                .map(|registry| registry.current().name().as_str().to_owned())
                .unwrap_or_else(|_| "user".into()),
            constants: Vec::new(),
            constant_index: HashMap::new(),
            functions: vec![placeholder(None, 0, 0, false, false)],
            contexts: vec![FnContext {
                proto_id: 0,
                name: None,
                params: 0,
                variadic: false,
                suspend_allowed: false,
                async_function: false,
                captures: Vec::new(),
                code: Vec::new(),
                source_map: SourceMap::default(),
                scopes,
                loops: Vec::new(),
                tries: Vec::new(),
                handlers: Vec::new(),
                fallthrough: true,
            }],
            globals: Vec::new(),
            excluded_foundation_libraries,
            allow_unbound_globals,
            inline_globals: HashMap::new(),
            var_metadata: Vec::new(),
            top_level: true,
            next_destructure_id: 0,
        }
    }

    fn ctx(&self) -> &FnContext {
        self.contexts.last().expect("function context is open")
    }

    fn ctx_mut(&mut self) -> &mut FnContext {
        self.contexts.last_mut().expect("function context is open")
    }

    /// Resolves coroutine forms by canonical Var identity so aliases and
    /// referred names behave exactly like fully-qualified source.
    fn is_coroutine_var(&self, name: &str, member: &str) -> bool {
        let canonical = format!("std.foundation.coroutine/{member}");
        if name == canonical {
            return true;
        }
        crate::core::namespace_registry()
            .ok()
            .and_then(|registry| {
                let source = registry.resolve(&crate::lang::data::Symbol::parse(name))?;
                let target = registry.resolve(&crate::lang::data::Symbol::parse(&canonical))?;
                Some(source.same_identity(&target))
            })
            .unwrap_or(false)
    }

    fn is_host_call_var(&self, name: &str) -> bool {
        let canonical = "std.native.Host/call";
        if name == canonical {
            return true;
        }
        crate::core::namespace_registry()
            .ok()
            .and_then(|registry| {
                let source = registry.resolve(&crate::lang::data::Symbol::parse(name))?;
                let target = registry.resolve(&crate::lang::data::Symbol::parse(canonical))?;
                Some(source.same_identity(&target))
            })
            .unwrap_or(false)
    }

    fn compile_host_call(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if children.len() != 4 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "std.native.Host/call expects service, method, and an argument vector",
                Some(span.start),
            ));
        }
        self.compile_call_arguments(children, span)?;
        if self.ctx().fallthrough {
            self.emit(Instruction::HostCall, Some(span.start));
        }
        Ok(())
    }

    /// Pairs parsed forms with their spans. When a node's children do not
    /// match its element count (reader macros expand to synthetic lists),
    /// elements inherit the parent span.
    fn children<'a>(&self, nodes: &'a [SpannedForm]) -> Vec<Child<'a>> {
        nodes
            .iter()
            .map(|node| Child {
                form: &node.form,
                span: &node.span,
                children: Some(&node.children),
            })
            .collect()
    }

    fn list_children<'a>(
        &self,
        elements: &'a [Form],
        span: &'a Span,
        children: Option<&'a [SpannedForm]>,
    ) -> Vec<Child<'a>> {
        let usable = children.filter(|nodes| nodes.len() == elements.len());
        elements
            .iter()
            .enumerate()
            .map(
                |(index, form)| match usable.and_then(|nodes| nodes.get(index)) {
                    Some(node) => Child {
                        form: &node.form,
                        span: &node.span,
                        children: Some(&node.children),
                    },
                    None => Child {
                        form,
                        span,
                        children: None,
                    },
                },
            )
            .collect()
    }

    fn emit(&mut self, instruction: Instruction, position: Option<Position>) -> usize {
        let context = self.ctx_mut();
        debug_assert!(context.fallthrough, "no emission after control terminates");
        let index = context.code.len();
        context.code.push(instruction);
        context.source_map.record(position);
        index
    }

    fn patch_jump(&mut self, at: usize, target: usize) {
        let target = target as u32;
        match &mut self.ctx_mut().code[at] {
            Instruction::Jump(operand) | Instruction::JumpIfFalse(operand) => *operand = target,
            other => unreachable!("patching non-jump instruction: {other:?}"),
        }
    }

    fn constant(&mut self, value: Value, span: &Span) -> Result<(), CompileError> {
        let index = self.constant_index_of(value, span)?;
        self.emit(Instruction::Constant(index), Some(span.start));
        Ok(())
    }

    fn unique_constant(&mut self, value: Value, span: &Span) -> Result<(), CompileError> {
        if self.constants.len() >= MAX_CONSTANTS {
            return Err(CompileError::new(
                CompileErrorKind::Limit,
                format!("constant pool exceeds limit of {MAX_CONSTANTS}"),
                Some(span.start),
            ));
        }
        let index = self.constants.len() as u32;
        self.constants.push(value);
        self.emit(Instruction::Constant(index), Some(span.start));
        Ok(())
    }

    /// The pool index for a constant, interning it if new. Used directly
    /// for instruction operands (global names, struct fields); `constant`
    /// additionally emits the load.
    fn constant_index_of(&mut self, value: Value, span: &Span) -> Result<u32, CompileError> {
        match self.constant_index.get(&value) {
            Some(index) => Ok(*index),
            None => {
                if self.constants.len() >= MAX_CONSTANTS {
                    return Err(CompileError::new(
                        CompileErrorKind::Limit,
                        format!("constant pool exceeds limit of {MAX_CONSTANTS}"),
                        Some(span.start),
                    ));
                }
                let index = self.constants.len() as u32;
                self.constants.push(value.clone());
                self.constant_index.insert(value, index);
                Ok(index)
            }
        }
    }

    fn unsupported(&self, form: &Form, span: &Span) -> CompileError {
        let message = match form {
            Form::List(elements) => match elements.first() {
                Some(Form::Symbol(name)) => format!("unsupported operator: {name}"),
                _ => format!("unsupported form: {form}"),
            },
            _ => format!("unsupported form: {form}"),
        };
        CompileError::new(CompileErrorKind::UnsupportedForm, message, Some(span.start))
    }

    /// Compiles a sequence of forms as an implicit `do`: every non-final
    /// result is popped. Dead forms after a terminating `recur` are not
    /// analyzed, matching the evaluator, which never reaches them.
    fn compile_sequence(&mut self, children: &[Child<'_>], tail: bool) -> Result<(), CompileError> {
        if children.is_empty() {
            self.emit(Instruction::Nil, None);
            return Ok(());
        }
        let top = self.top_level && self.contexts.len() == 1;
        let last = children.len() - 1;
        for (index, child) in children.iter().enumerate() {
            if !self.ctx().fallthrough {
                break;
            }
            self.top_level = top;
            self.compile_form(
                child.form,
                child.span,
                child.children,
                tail && index == last,
            )?;
            if index != last && self.ctx().fallthrough {
                self.emit(Instruction::Pop, Some(child.span.start));
            }
        }
        Ok(())
    }

    fn compile_form(
        &mut self,
        form: &Form,
        span: &Span,
        children: Option<&[SpannedForm]>,
        tail: bool,
    ) -> Result<(), CompileError> {
        let top = self.top_level;
        self.top_level = false;
        if !self.ctx().fallthrough {
            // Dead code (e.g. after a nested infinite loop): not analyzed,
            // matching the evaluator, which never reaches it.
            return Ok(());
        }
        if let Some(expanded) =
            destructure::expand(form, &mut self.next_destructure_id).map_err(|message| {
                CompileError::new(CompileErrorKind::UnsupportedForm, message, Some(span.start))
            })?
        {
            self.top_level = top;
            return self.compile_form(&expanded, span, None, tail);
        }
        if let Form::List(values) = crate::core::form_without_metadata(form) {
            let protected = matches!(
                values.first(),
                Some(Form::Symbol(name))
                    if name == "quote" || name == "syntax-quote" || name == "comment"
            );
            if !protected {
                let expanded = crate::core::vm_macroexpand(form).map_err(|message| {
                    CompileError::new(CompileErrorKind::UnsupportedForm, message, Some(span.start))
                })?;
                if expanded != *form {
                    self.top_level = top;
                    return self.compile_form(&expanded, span, None, tail);
                }
            }
        }
        match form {
            Form::Nil => {
                self.emit(Instruction::Nil, Some(span.start));
                Ok(())
            }
            Form::Bool(true) => {
                self.emit(Instruction::True, Some(span.start));
                Ok(())
            }
            Form::Bool(false) => {
                self.emit(Instruction::False, Some(span.start));
                Ok(())
            }
            Form::Number(value) => self.constant(Value::Number(*value), span),
            Form::Float(value) => {
                let value = crate::numeric::finite_float(*value).map_err(|error| {
                    CompileError::new(CompileErrorKind::Parse, error, Some(span.start))
                })?;
                self.constant(Value::Float(value), span)
            }
            Form::String(value) => self.constant(Value::String(value.clone()), span),
            Form::Keyword(value) => self.constant(Value::Keyword(value.clone().into()), span),
            Form::Character(value) => self.constant(Value::Character(*value), span),
            Form::BigInteger(value) => {
                self.constant(crate::numeric::compact_integer(value.clone()), span)
            }
            Form::Regex(value) => self.constant(Value::Regex(value.clone()), span),
            // Collection identity is observable even when language equality
            // is structural across concrete sequential/map/set types.  Do
            // not intern collection literals in the Value-keyed constant
            // pool: `[]` may otherwise alias an earlier `()`, and the HTA
            // constant codec canonicalizes collection values. Literal
            // vectors use a non-interned constant; dynamic collections are
            // built in bytecode so concrete type and order remain intact.
            Form::Vector(values) => {
                if values.iter().all(literal_collection_form) {
                    let value = crate::core::form_to_value(form).map_err(|message| {
                        CompileError::new(
                            CompileErrorKind::UnsupportedForm,
                            message,
                            Some(span.start),
                        )
                    })?;
                    return self.unique_constant(value, span);
                }
                self.compile_collection_values(values.iter(), span)?;
                self.emit(
                    Instruction::BuildVector(self.collection_count(values.len(), span)?),
                    Some(span.start),
                );
                Ok(())
            }
            Form::Map(entries) => {
                // HTA canonicalization sorts hash-map keys. Map literals stay
                // out of the HBC constant pool so runtime construction remains
                // identical across Rust and Truffle runtimes.
                if entries.len() > usize::from(u16::MAX) {
                    return Err(CompileError::new(
                        CompileErrorKind::Limit,
                        "map literal exceeds 65535 entries",
                        Some(span.start),
                    ));
                }
                for (key, value) in entries {
                    self.compile_form(key, span, None, false)?;
                    self.compile_form(value, span, None, false)?;
                }
                self.emit(
                    Instruction::BuildMap(entries.len() as u16),
                    Some(span.start),
                );
                Ok(())
            }
            Form::Set(values) => {
                self.compile_collection_values(values.iter(), span)?;
                self.emit(
                    Instruction::BuildSet(self.collection_count(values.len(), span)?),
                    Some(span.start),
                );
                Ok(())
            }
            Form::Metadata(_, value) => {
                // Metadata wraps the form without changing its lexical or
                // top-level position.  Restore the position after the
                // dispatch prologue above; otherwise a top-level `^{...}
                // (defn ...)` is mistaken for a nested definition.
                self.top_level = top;
                self.compile_form(value, span, None, tail)
            }
            Form::Symbol(name) => match self.ctx().scopes.resolve(name) {
                Some(slot) => {
                    self.emit(Instruction::LoadLocal(slot), Some(span.start));
                    Ok(())
                }
                None if self.visible_global(name) => self.emit_get_global(name, span),
                None if IntrinsicOp::from_symbol(name).is_some() => {
                    let target = self.name_constant(name, span)?;
                    self.emit(Instruction::IntrinsicValue(target), Some(span.start));
                    Ok(())
                }
                None if self.visible_bytecode_callable(name) => {
                    let index = self.name_constant(name, span)?;
                    self.emit(Instruction::BuiltinValue(index), Some(span.start));
                    Ok(())
                }
                None if self.visible_namespace(name) => {
                    let index = self.name_constant(name, span)?;
                    self.emit(Instruction::NamespaceValue(index), Some(span.start));
                    Ok(())
                }
                None => Err(CompileError::new(
                    CompileErrorKind::UnboundSymbol,
                    format!("unbound symbol: {name}"),
                    Some(span.start),
                )),
            },
            Form::List(elements) if elements.is_empty() => {
                self.constant(Value::List(PList::new()), span)
            }
            Form::List(elements) => {
                let children = self.list_children(elements, span, children);
                match &elements[0] {
                    Form::Symbol(name)
                        if matches!(
                            name.as_str(),
                            "defprotocol" | "extend-type" | "defmulti" | "defmethod"
                        ) =>
                    {
                        if matches!(name.as_str(), "defprotocol" | "defmulti") {
                            if let Some(Form::Symbol(declared)) = elements.get(1) {
                                self.declare_program_global(declared);
                            }
                        }
                        let value = crate::core::form_to_value(form).map_err(|message| {
                            CompileError::new(
                                CompileErrorKind::UnsupportedForm,
                                message,
                                Some(span.start),
                            )
                        })?;
                        let index = self.constant_index_of(value, span)?;
                        self.emit(
                            match name.as_str() {
                                "defprotocol" => Instruction::DefProtocol(index),
                                "extend-type" => Instruction::ExtendType(index),
                                "defmulti" => Instruction::DefMulti(index),
                                "defmethod" => Instruction::DefMethod(index),
                                _ => unreachable!("declaration operator was checked"),
                            },
                            Some(span.start),
                        );
                        Ok(())
                    }
                    Form::Symbol(name) if self.is_coroutine_var(name, "await") => {
                        self.compile_await(&children, span)
                    }
                    Form::Symbol(name) if self.is_coroutine_var(name, "yield") => {
                        self.compile_yield(&children, span)
                    }
                    Form::Symbol(name) if self.is_host_call_var(name) => {
                        self.compile_host_call(&children, span)
                    }
                    Form::Symbol(name) if name == "." => {
                        if elements.len() != 3 {
                            return Err(CompileError::new(
                                CompileErrorKind::UnsupportedForm,
                                "dot expects a receiver and method",
                                Some(span.start),
                            ));
                        }
                        let Form::List(method_form) = &elements[2] else {
                            return Err(CompileError::new(
                                CompileErrorKind::UnsupportedForm,
                                "dot call expects a method list",
                                Some(span.start),
                            ));
                        };
                        let Some(Form::Symbol(method_name)) = method_form.first() else {
                            return Err(CompileError::new(
                                CompileErrorKind::UnsupportedForm,
                                "dot method must be a symbol",
                                Some(span.start),
                            ));
                        };
                        self.compile_form(&elements[1], span, None, false)?;
                        for argument in &method_form[1..] {
                            self.compile_form(argument, span, None, false)?;
                        }
                        let method = self.name_constant(method_name, span)?;
                        self.emit(
                            Instruction::DotCall {
                                method,
                                argc: (method_form.len() - 1) as u8,
                            },
                            Some(span.start),
                        );
                        Ok(())
                    }
                    Form::Symbol(name) if name == "if" => self.compile_if(&children, span, tail),
                    Form::Symbol(name) if name == "and" => self.compile_and(&children, span, tail),
                    Form::Symbol(name) if name == "or" => self.compile_or(&children, span, tail),
                    Form::Symbol(name) if name == "cond" => {
                        self.compile_cond(&children, span, tail)
                    }
                    Form::Symbol(name) if name == "quote" => self.compile_quote(&children, span),
                    Form::Symbol(name) if name == "comment" => {
                        self.emit(Instruction::Nil, Some(span.start));
                        Ok(())
                    }
                    Form::Symbol(name) if name == "syntax-quote" => {
                        self.compile_syntax_quote(&children, span)
                    }
                    Form::Symbol(name) if name == "do" => {
                        // A top-level `do` is transparent: its statements
                        // keep top-level position, so `defn` lowering works
                        // inside `(do (defn ...) ...)`.
                        self.top_level = top;
                        self.compile_sequence(&children[1..], tail)
                    }
                    Form::Symbol(name) if name == "let" => self.compile_let(&children, span, tail),
                    Form::Symbol(name) if name == "loop" => {
                        self.compile_loop(&children, span, tail)
                    }
                    Form::Symbol(name) if name == "recur" => {
                        self.compile_recur(&children, span, tail)
                    }
                    Form::Symbol(name) if name == "fn" => self.compile_fn_form(&children, span),
                    Form::Symbol(name) if name == "def" => self.compile_def(&children, span),
                    Form::Symbol(name) if name == "defn" || name == "defn-" => {
                        self.compile_defn(&children, span, top, name == "defn-")
                    }
                    Form::Symbol(name) if name == "defmacro" => {
                        self.compile_defmacro(&children, span, top)
                    }
                    Form::Symbol(name) if name == "declare" => {
                        self.compile_declare(&children, span, top)
                    }
                    Form::Symbol(name) if name == "var" => self.compile_var(&children, span),
                    Form::Symbol(name) if name == "set!" => self.compile_set(&children, span),
                    Form::Symbol(name) if name == "defstruct" || name == "defmutable" => {
                        self.compile_named_definition(&children, span, name == "defmutable")
                    }
                    Form::Symbol(name)
                        if name == "require" || (matches!(name.as_str(), "ns" | "ns+") && !top) =>
                    {
                        let value = crate::core::form_to_value(form).map_err(|message| {
                            CompileError::new(
                                CompileErrorKind::UnsupportedForm,
                                message,
                                Some(span.start),
                            )
                        })?;
                        let index = self.constant_index_of(value, span)?;
                        self.emit(Instruction::NamespaceOperation(index), Some(span.start));
                        Ok(())
                    }
                    Form::Symbol(name) if name == "ns" || name == "ns+" => {
                        if !top {
                            return Err(self.unsupported(form, span));
                        }
                        self.emit(Instruction::Nil, Some(span.start));
                        Ok(())
                    }
                    Form::Symbol(name) if name == "field" => self.compile_field(&children, span),
                    Form::Symbol(name) if name == "instance?" => {
                        self.compile_instance_of(&children, span)
                    }
                    Form::Symbol(name) if name == "try" => self.compile_try(&children, span, tail),
                    Form::Symbol(name) if name == "__dynamic-bind" => {
                        self.compile_dynamic_binding(&children, span, true)
                    }
                    Form::Symbol(name) if name == "__dynamic-unbind" => {
                        self.compile_dynamic_binding(&children, span, false)
                    }
                    Form::Symbol(name) if name == "throw" => self.compile_throw(&children, span),
                    Form::Symbol(name)
                        if UNSUPPORTED_OPERATORS.contains(&name.as_str())
                            && self.ctx().scopes.resolve(name).is_none()
                            && !self.visible_global(name) =>
                    {
                        Err(self.unsupported(form, span))
                    }
                    Form::Symbol(name)
                        if (name.starts_with("std.native.Arr/")
                            || name.starts_with("std.native.Obj/"))
                            && IntrinsicOp::from_symbol(name).is_some() =>
                    {
                        self.compile_primitive(
                            &children,
                            span,
                            IntrinsicOp::from_symbol(name).expect("intrinsic was checked"),
                        )
                    }
                    // Precedence mirrors the evaluator (core.rs operator
                    // dispatch): a bound var wins over the structural
                    // builtin arms, so a program-declared or registry
                    // global compiles to GetGlobal+Call even when it names
                    // a primitive; only otherwise-unbound operator names
                    // lower to intrinsic instructions (issue #223).
                    Form::Symbol(name)
                        if self.ctx().scopes.resolve(name).is_some()
                            || self.visible_global(name) =>
                    {
                        self.compile_named_call(name, &children, span)
                    }
                    Form::Symbol(name) => match IntrinsicOp::from_symbol(name) {
                        Some(op) => self.compile_primitive(&children, span, op),
                        None => self.compile_named_call(name, &children, span),
                    },
                    _ => self.compile_expression_call(&children, span),
                }
            }
            _ => Err(self.unsupported(form, span)),
        }
    }

    fn collection_count(&self, count: usize, span: &Span) -> Result<u16, CompileError> {
        u16::try_from(count).map_err(|_| {
            CompileError::new(
                CompileErrorKind::Limit,
                "collection literal exceeds 65535 items",
                Some(span.start),
            )
        })
    }

    fn compile_collection_values<'a>(
        &mut self,
        values: impl Iterator<Item = &'a Form>,
        span: &Span,
    ) -> Result<(), CompileError> {
        for value in values {
            self.compile_form(value, span, None, false)?;
        }
        Ok(())
    }

    /// Compiles the argument forms of a call (callee already compiled).
    fn compile_call_arguments(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        let argc = children.len() - 1;
        if argc > MAX_PRIMITIVE_ARGUMENTS {
            return Err(CompileError::new(
                CompileErrorKind::Limit,
                format!("calls support at most {MAX_PRIMITIVE_ARGUMENTS} arguments"),
                Some(span.start),
            ));
        }
        for argument in &children[1..] {
            self.compile_form(argument.form, argument.span, argument.children, false)?;
        }
        Ok(())
    }

    fn compile_if(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
        tail: bool,
    ) -> Result<(), CompileError> {
        // The condition is never a tail position; the branches inherit the
        // `if`'s own tail context.
        if children.len() != 3 && children.len() != 4 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "if expects 2 or 3 arguments",
                Some(span.start),
            ));
        }
        let condition = &children[1];
        self.compile_form(condition.form, condition.span, condition.children, false)?;
        if !self.ctx().fallthrough {
            // The condition cannot produce a value (e.g. an infinite inner
            // loop); the branches are dead code.
            return Ok(());
        }
        let jump_else = self.emit(Instruction::JumpIfFalse(0), Some(condition.span.start));
        let then = &children[2];
        self.compile_form(then.form, then.span, then.children, tail)?;
        let then_fell = self.ctx().fallthrough;
        let jump_end = if then_fell {
            Some(self.emit(Instruction::Jump(0), Some(then.span.start)))
        } else {
            None
        };
        // The else branch starts fresh at its label.
        self.ctx_mut().fallthrough = true;
        let else_target = self.ctx().code.len();
        if let Some(else_form) = children.get(3) {
            self.compile_form(else_form.form, else_form.span, else_form.children, tail)?;
        } else {
            self.emit(Instruction::Nil, Some(span.start));
        }
        let else_fell = self.ctx().fallthrough;
        let end = self.ctx().code.len();
        self.patch_jump(jump_else, else_target);
        if let Some(jump_end) = jump_end {
            self.patch_jump(jump_end, end);
        }
        self.ctx_mut().fallthrough = then_fell || else_fell;
        Ok(())
    }

    fn compile_and(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
        tail: bool,
    ) -> Result<(), CompileError> {
        if children.len() == 1 {
            self.emit(Instruction::True, Some(span.start));
            return Ok(());
        }
        let mut false_jumps = Vec::new();
        for child in &children[1..children.len() - 1] {
            self.compile_form(child.form, child.span, child.children, false)?;
            if !self.ctx().fallthrough {
                break;
            }
            self.emit(Instruction::Dup, Some(child.span.start));
            false_jumps.push(self.emit(Instruction::JumpIfFalse(0), Some(child.span.start)));
            self.emit(Instruction::Pop, Some(child.span.start));
        }
        if self.ctx().fallthrough {
            let last = children.last().expect("and has an argument");
            self.compile_form(last.form, last.span, last.children, tail)?;
        }
        let last_fell = self.ctx().fallthrough;
        let end = self.ctx().code.len();
        let short_circuits = !false_jumps.is_empty();
        for jump in false_jumps {
            self.patch_jump(jump, end);
        }
        self.ctx_mut().fallthrough = last_fell || short_circuits;
        Ok(())
    }

    fn compile_or(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
        tail: bool,
    ) -> Result<(), CompileError> {
        if children.len() == 1 {
            self.emit(Instruction::Nil, Some(span.start));
            return Ok(());
        }
        let mut end_jumps = Vec::new();
        for child in &children[1..children.len() - 1] {
            self.compile_form(child.form, child.span, child.children, false)?;
            if !self.ctx().fallthrough {
                break;
            }
            self.emit(Instruction::Dup, Some(child.span.start));
            let false_jump = self.emit(Instruction::JumpIfFalse(0), Some(child.span.start));
            end_jumps.push(self.emit(Instruction::Jump(0), Some(child.span.start)));
            let next = self.ctx().code.len();
            self.patch_jump(false_jump, next);
            self.emit(Instruction::Pop, Some(child.span.start));
        }
        if self.ctx().fallthrough {
            let last = children.last().expect("or has an argument");
            self.compile_form(last.form, last.span, last.children, tail)?;
        }
        let last_fell = self.ctx().fallthrough;
        let end = self.ctx().code.len();
        let short_circuits = !end_jumps.is_empty();
        for jump in end_jumps {
            self.patch_jump(jump, end);
        }
        self.ctx_mut().fallthrough = last_fell || short_circuits;
        Ok(())
    }

    fn compile_cond(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
        tail: bool,
    ) -> Result<(), CompileError> {
        let clauses = &children[1..];
        if clauses.len() % 2 != 0 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "cond expects test/expression pairs",
                Some(span.start),
            ));
        }
        if clauses.is_empty() {
            self.emit(Instruction::Nil, Some(span.start));
            return Ok(());
        }
        let mut end_jumps = Vec::new();
        for pair in clauses.chunks(2) {
            self.compile_form(pair[0].form, pair[0].span, pair[0].children, false)?;
            if !self.ctx().fallthrough {
                let end = self.ctx().code.len();
                for jump in end_jumps {
                    self.patch_jump(jump, end);
                }
                return Ok(());
            }
            let next_jump = self.emit(Instruction::JumpIfFalse(0), Some(pair[0].span.start));
            self.compile_form(pair[1].form, pair[1].span, pair[1].children, tail)?;
            if self.ctx().fallthrough {
                end_jumps.push(self.emit(Instruction::Jump(0), Some(pair[1].span.start)));
            }
            // A false test reaches the next clause even when the preceding
            // expression terminated with recur, throw, or return.
            self.ctx_mut().fallthrough = true;
            let next = self.ctx().code.len();
            self.patch_jump(next_jump, next);
        }
        self.emit(Instruction::Nil, Some(span.start));
        let end = self.ctx().code.len();
        for jump in end_jumps {
            self.patch_jump(jump, end);
        }
        Ok(())
    }

    fn compile_dynamic_binding(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
        bind: bool,
    ) -> Result<(), CompileError> {
        let expected = if bind { 3 } else { 2 };
        if children.len() != expected {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                if bind {
                    "dynamic bind expects a Var and value"
                } else {
                    "dynamic unbind expects a Var"
                },
                Some(span.start),
            ));
        }
        let Form::Symbol(name) = children[1].form else {
            return Err(CompileError::new(
                CompileErrorKind::UnsupportedForm,
                "dynamic binding target must be a symbol",
                Some(children[1].span.start),
            ));
        };
        let name = self.global_name_constant(name, children[1].span)?;
        if bind {
            self.compile_form(
                children[2].form,
                children[2].span,
                children[2].children,
                false,
            )?;
            if self.ctx().fallthrough {
                self.emit(Instruction::DynamicBind(name), Some(span.start));
            }
        } else {
            self.emit(Instruction::DynamicUnbind(name), Some(span.start));
        }
        Ok(())
    }

    /// Compiles `let`-style ordered bindings into fresh slots, returns the
    /// bound slots, and leaves the scope open for the body.
    fn compile_bindings(
        &mut self,
        children: &[Child<'_>],
        form_name: &str,
    ) -> Result<Vec<u16>, CompileError> {
        let bindings = &children[1];
        let pairs: &[Form] = match bindings.form {
            Form::List(values) | Form::Vector(values) => values,
            _ => {
                return Err(CompileError::new(
                    CompileErrorKind::Arity,
                    format!("{form_name} expects a binding list or vector"),
                    Some(bindings.span.start),
                ))
            }
        };
        if pairs.len() % 2 != 0 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                format!("{form_name} bindings require name/value pairs"),
                Some(bindings.span.start),
            ));
        }
        // Binding-pair children keep their own spans when available.
        let pair_children = self.list_children(pairs, bindings.span, bindings.children);
        let mut slots = Vec::with_capacity(pairs.len() / 2);
        for pair in pair_children.chunks(2) {
            let (name, initializer) = (&pair[0], &pair[1]);
            // Binding names are structural: validate before compiling the
            // initializer so destructuring reports on the name.
            let Form::Symbol(symbol) = name.form else {
                return Err(CompileError::new(
                    CompileErrorKind::UnsupportedForm,
                    format!("{form_name} destructuring is not supported"),
                    Some(name.span.start),
                ));
            };
            self.compile_form(
                initializer.form,
                initializer.span,
                initializer.children,
                false,
            )?;
            if !self.ctx().fallthrough {
                return Ok(slots);
            }
            let slot = self.ctx_mut().scopes.declare(symbol).map_err(|error| {
                CompileError::new(error.kind(), error.message(), Some(name.span.start))
            })?;
            self.emit(Instruction::StoreLocal(slot), Some(name.span.start));
            slots.push(slot);
        }
        Ok(slots)
    }

    fn compile_let(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
        tail: bool,
    ) -> Result<(), CompileError> {
        if children.len() < 3 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "let expects bindings and a body",
                Some(span.start),
            ));
        }
        self.ctx_mut().scopes.push_scope();
        let result = self
            .compile_bindings(children, "let")
            .and_then(|_| self.compile_sequence(&children[2..], tail));
        self.ctx_mut().scopes.pop_scope();
        result
    }

    fn compile_loop(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
        _tail: bool,
    ) -> Result<(), CompileError> {
        if children.len() < 3 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "loop expects bindings and a body",
                Some(span.start),
            ));
        }
        self.ctx_mut().scopes.push_scope();
        let result = self.compile_bindings(children, "loop").and_then(|slots| {
            let header = self.ctx().code.len();
            self.ctx_mut().loops.push(LoopContext { header, slots });
            // Multiple body forms sequence like `do`; the last one is
            // the loop's tail (recur) position.
            let result = self.compile_sequence(&children[2..], true);
            self.ctx_mut().loops.pop();
            result
        });
        self.ctx_mut().scopes.pop_scope();
        result
    }

    fn finish(mut self) -> Result<Program, CompileError> {
        if self.ctx().fallthrough {
            self.emit(Instruction::Return, None);
        }
        self.close_context();
        let mut program = Program {
            namespace: None,
            var_metadata: self.var_metadata,
            schema_types: HashMap::new(),
            function_types: HashMap::new(),
            inferred_function_types: HashMap::new(),
            constants: self.constants,
            functions: self.functions,
            entry: 0,
        };
        // The shared analysis computes each operand-stack high-water mark;
        // full validation then runs over the whole program before it is
        // returned. Handler entry depths are patched from the same pass.
        for index in 0..program.functions.len() {
            let heights = stack_heights(&program, &program.functions[index])
                .map_err(|error| internal(error.to_string()))?;
            program.functions[index].max_stack = heights.iter().copied().max().unwrap_or(0);
            for entry_index in 0..program.functions[index].handlers.len() {
                let start = program.functions[index].handlers[entry_index].start as usize;
                program.functions[index].handlers[entry_index].depth = heights[start];
            }
        }
        validate::validate(&program).map_err(|error| internal(error.to_string()))?;
        Ok(program)
    }
}

fn literal_collection_form(form: &Form) -> bool {
    match form {
        Form::Nil
        | Form::Bool(_)
        | Form::Number(_)
        | Form::Float(_)
        | Form::String(_)
        | Form::Keyword(_)
        | Form::Character(_)
        | Form::BigInteger(_)
        | Form::Regex(_) => true,
        Form::Vector(values) | Form::Set(values) => values.iter().all(literal_collection_form),
        Form::Map(entries) => entries
            .iter()
            .all(|(key, value)| literal_collection_form(key) && literal_collection_form(value)),
        _ => false,
    }
}

fn constant_form(form: &Form) -> bool {
    match form {
        Form::Nil
        | Form::Bool(_)
        | Form::Number(_)
        | Form::Float(_)
        | Form::BigInteger(_)
        | Form::Character(_)
        | Form::Regex(_)
        | Form::Keyword(_)
        | Form::String(_) => true,
        Form::Tagged(_, value) | Form::Metadata(_, value) => constant_form(value),
        Form::Vector(values) | Form::Set(values) => values.iter().all(constant_form),
        Form::Map(entries) => entries
            .iter()
            .all(|(key, value)| constant_form(key) && constant_form(value)),
        Form::Symbol(_) | Form::List(_) => false,
    }
}

fn unquote_argument(form: &Form, operator: &str) -> Option<Result<Form, String>> {
    let Form::List(parts) = crate::core::form_without_metadata(form) else {
        return None;
    };
    if !matches!(parts.first(), Some(Form::Symbol(name)) if name == operator) {
        return None;
    }
    Some(if parts.len() == 2 {
        Ok(parts[1].clone())
    } else {
        Err(format!("{operator} expects one argument"))
    })
}

fn internal(message: String) -> CompileError {
    CompileError::new(CompileErrorKind::Internal, message, None)
}

#[cfg(test)]
#[path = "compiler/tests.rs"]
mod tests;
