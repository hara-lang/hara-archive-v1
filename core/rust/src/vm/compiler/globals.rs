//! Named global-form compilation (issue #223): `defn`/`defn-`, `declare`,
//! `defstruct`, `field`, and `instance?` lower to registry-direct global
//! instructions. Basic `def`, `set!`, and `var` bindings live in `bindings.rs`.
//! Names resolve in two phases:
//! compile-time visibility (program-declared globals plus the
//! compilation namespace registry) decides whether a symbol may compile
//! to a global reference at all; the emitted instructions resolve the
//! var through the registry again at execution time, so globals are
//! late-bound through the shared var cell — the same semantics the JVM
//! runtime and the fixed tree evaluator exhibit. Split from
//! `compiler.rs` to stay under the repository's per-file line cap.

use std::rc::Rc;

use crate::core::{binding_symbol, definition_metadata, schema_var_reference};
use crate::kernel::{Form, Span};
use crate::lang::data::Metadata;
use crate::lang::data::Symbol;
use crate::lang::protocol::INamespaced;
use crate::vm::error::{CompileError, CompileErrorKind};
use crate::vm::opcode::Instruction;

use super::{Child, Compiler};

impl Compiler {
    pub(super) fn excluded_foundation_symbol(&self, name: &str) -> bool {
        let Some((namespace, _)) = name.split_once('/') else {
            return false;
        };
        self.excluded_foundation_libraries.iter().any(|library| {
            namespace == format!("std.foundation.{library}")
                || crate::kernel::generated::foundation_library_alias(library)
                    .is_some_and(|alias| alias == namespace)
        })
    }

    pub(super) fn compile_defmacro(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
        top: bool,
    ) -> Result<(), CompileError> {
        if !top {
            return Err(unsupported(
                "defmacro is only supported as a top-level statement",
                span.start,
            ));
        }
        if children.len() < 4 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "defmacro expects a name, parameters, and a body",
                Some(span.start),
            ));
        }
        let (name, metadata) = binding_symbol(children[1].form, "defmacro name")
            .map_err(|message| unsupported(message, children[1].span.start))?;
        self.require_owned_global(&name, children[1].span)?;
        let raw = children
            .iter()
            .map(|child| child.form.clone())
            .collect::<Vec<_>>();
        let (metadata, rest) = definition_metadata(metadata, &raw[2..], false, true)
            .map_err(|message| unsupported(format!("{name}: {message}"), children[1].span.start))?;
        let offset = children.len() - rest.len();
        let rest_children = &children[offset..];
        if rest_children.is_empty() {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "defmacro expects a name, parameters, and a body",
                Some(span.start),
            ));
        }
        let metadata = self.var_metadata(metadata);
        self.declare_program_global(&name);
        if matches!(
            crate::core::form_without_metadata(rest_children[0].form),
            Form::Vector(_)
        ) {
            let params = macro_params(rest_children[0].form)
                .map_err(|message| unsupported(message, rest_children[0].span.start))?;
            let params_child = Child {
                form: &params,
                span: rest_children[0].span,
                children: None,
            };
            self.compile_function(
                Some(&name),
                &params_child,
                &rest_children[1..],
                span,
                false,
                false,
            )?;
        } else {
            let mut count = 0usize;
            for clause in rest_children {
                let Form::List(parts) = crate::core::form_without_metadata(clause.form) else {
                    return Err(unsupported(
                        "defmacro multi-arity clauses must be lists",
                        clause.span.start,
                    ));
                };
                if parts.len() < 2 {
                    return Err(CompileError::new(
                        CompileErrorKind::Arity,
                        "defmacro clause expects parameters and a body",
                        Some(clause.span.start),
                    ));
                }
                let params = macro_params(&parts[0])
                    .map_err(|message| unsupported(message, clause.span.start))?;
                let params_child = Child {
                    form: &params,
                    span: clause.span,
                    children: None,
                };
                let body = parts[1..]
                    .iter()
                    .map(|form| Child {
                        form,
                        span: clause.span,
                        children: None,
                    })
                    .collect::<Vec<_>>();
                self.compile_function(None, &params_child, &body, span, false, false)?;
                count += 1;
                if count > usize::from(u8::MAX) {
                    return Err(CompileError::new(
                        CompileErrorKind::Limit,
                        "defmacro supports at most 255 arity clauses",
                        Some(span.start),
                    ));
                }
            }
            let name_constant = self.name_constant(&name, children[1].span)?;
            self.emit(
                Instruction::MakeMultiArity {
                    name: name_constant,
                    count: count as u8,
                },
                Some(span.start),
            );
        }
        let name = self.name_constant(&name, children[1].span)?;
        self.emit(Instruction::DefMacro { name, metadata }, Some(span.start));
        Ok(())
    }

    pub(super) fn require_owned_global(&self, name: &str, span: &Span) -> Result<(), CompileError> {
        if let Ok(registry) = crate::core::namespace_registry() {
            let namespace = registry
                .find(&self.namespace)
                .unwrap_or_else(|| registry.current());
            if let Some(var) = namespace.resolve(&Symbol::parse(name)) {
                let owned_by_compilation_namespace =
                    var.symbol().get_namespace().is_none_or(|owner| {
                        owner == self.namespace.as_str() || owner == "std.foundation"
                    });
                if !owned_by_compilation_namespace {
                    return Err(CompileError::new(
                        CompileErrorKind::UnsupportedForm,
                        format!("Cannot replace referred Var without ns omission: {name}"),
                        Some(span.start),
                    ));
                }
            }
        }
        Ok(())
    }

    /// A name constant's pool index (no instruction emitted). Global
    /// operands are string names resolved at execution time.
    pub(super) fn name_constant(&mut self, name: &str, span: &Span) -> Result<u32, CompileError> {
        self.constant_index_of(crate::core::Value::String(name.to_string()), span)
    }

    /// A global read is bound to the var visible in the compilation
    /// namespace.  Leaving the operand unqualified would resolve it again in
    /// the caller's current namespace when a compiled closure runs, allowing
    /// user refers to redirect Foundation helper calls.
    pub(super) fn global_name_constant(
        &mut self,
        name: &str,
        span: &Span,
    ) -> Result<u32, CompileError> {
        let current_alias_local = name.strip_prefix("-/");
        let qualified = crate::core::namespace_registry()
            .ok()
            .and_then(|registry| {
                let current = registry
                    .find(&self.namespace)
                    .unwrap_or_else(|| registry.current());
                if let Some(local) = current_alias_local {
                    Some(format!("{}/{}", self.namespace, local))
                } else if !name.contains('/') && self.globals.iter().any(|global| global == name) {
                    Some(format!("{}/{}", self.namespace, name))
                } else if let Some((alias, local)) = name.split_once('/') {
                    (if registry.find(alias).is_some() {
                        registry.resolve(&crate::lang::data::Symbol::parse(name))
                    } else {
                        current.resolve(&crate::lang::data::Symbol::parse(name))
                    })
                    .map(|var| var.symbol().as_str().to_owned())
                    .or_else(|| {
                        current
                            .lazy_target(alias)
                            .map(|target| format!("{}/{}", target.as_str(), local))
                    })
                } else {
                    registry
                        .resolve(&crate::lang::data::Symbol::parse(name))
                        .map(|var| var.symbol().as_str().to_owned())
                }
            })
            .unwrap_or_else(|| name.to_owned());
        self.constant_index_of(crate::core::Value::String(qualified), span)
    }

    /// Registers a name as program-declared: visible to global
    /// references compiled from this point on.
    pub(super) fn declare_program_global(&mut self, name: &str) {
        if !self.globals.iter().any(|global| global == name) {
            self.globals.push(name.to_string());
        }
    }

    /// Whether `name` may compile to a global reference: declared by
    /// this program, or resolvable through the compilation namespace
    /// registry (the Runtime path; the free `compile_source` path has
    /// none and only sees program-declared names).
    pub(super) fn visible_global(&self, name: &str) -> bool {
        if self.excluded_foundation_symbol(name) {
            return false;
        }
        if self.allow_unbound_globals
            && !crate::core::syntax_symbol(name)
            && crate::core::IntrinsicOp::from_symbol(name).is_none()
            && !self.visible_namespace(name)
        {
            return true;
        }
        let declared = name
            .strip_prefix("-/")
            .is_some_and(|local| self.globals.iter().any(|global| global == local))
            || self.globals.iter().any(|global| global == name)
            || name
                .strip_prefix(&format!("{}/", self.namespace))
                .is_some_and(|local| self.globals.iter().any(|global| global == local));
        declared
            || crate::core::namespace_registry()
                .map(|registry| {
                    let current = registry
                        .find(&self.namespace)
                        .unwrap_or_else(|| registry.current());
                    let lazy_visible = name
                        .split_once('/')
                        .is_some_and(|(alias, _)| current.lazy_target(alias).is_some());
                    lazy_visible
                        || (name
                            .split_once('/')
                            .and_then(|(namespace, _)| registry.find(namespace))
                            .and_then(|_| registry.resolve(&crate::lang::data::Symbol::parse(name)))
                            .or_else(|| current.resolve(&crate::lang::data::Symbol::parse(name)))
                            .or_else(|| registry.resolve(&crate::lang::data::Symbol::parse(name))))
                        .is_some_and(|var| {
                            crate::core::IntrinsicOp::from_symbol(name).is_none()
                                || var.symbol().get_namespace() != Some("std.foundation")
                        })
                })
                .unwrap_or(false)
    }

    /// Bare namespace aliases are first-class callable values in the
    /// evaluator (`(promise ...)` means calling the namespace's `run` Var).
    /// Keep them distinct from Vars: a namespace has no global Var entry, so
    /// it needs its own validated instruction and runtime lookup.
    pub(super) fn visible_namespace(&self, name: &str) -> bool {
        crate::core::namespace_registry()
            .map(|registry| {
                let current = registry
                    .find(&self.namespace)
                    .unwrap_or_else(|| registry.current());
                let visible = current.lazy_target(name).is_some()
                    || current
                        .aliases()
                        .into_iter()
                        .any(|(alias, _)| alias.as_str() == name)
                    || registry.find(name).is_some();
                visible
            })
            .unwrap_or(false)
    }

    /// Canonical native/protocol callables may be emitted before ordinary
    /// Foundation Vars are available. Their values are still resolved from
    /// the shared namespace registry at execution time.
    pub(super) fn visible_bytecode_callable(&self, name: &str) -> bool {
        if self.excluded_foundation_symbol(name) {
            return false;
        }
        let Some(canonical) = crate::core::canonical_intrinsic_callable_symbol(name) else {
            return false;
        };
        crate::core::namespace_registry()
            .map(|registry| {
                let current = registry
                    .find(&self.namespace)
                    .unwrap_or_else(|| registry.current());
                registry
                    .resolve(&crate::lang::data::Symbol::parse(&canonical))
                    .or_else(|| current.resolve(&crate::lang::data::Symbol::parse(&canonical)))
                    .is_some()
            })
            .unwrap_or(true)
    }

    /// Emits a `GetGlobal` for a visible name.
    pub(super) fn emit_get_global(&mut self, name: &str, span: &Span) -> Result<(), CompileError> {
        let index = self.global_name_constant(name, span)?;
        self.emit(Instruction::GetGlobal(index), Some(span.start));
        Ok(())
    }

    pub(super) fn var_metadata(&mut self, metadata: Option<Rc<Metadata>>) -> Option<u16> {
        metadata.map(|metadata| {
            let index = self.var_metadata.len() as u16;
            self.var_metadata.push(metadata);
            index
        })
    }

    /// `(declare name ...)`: interns a nil var per name without resetting
    /// any existing binding. It supplies forward visibility only; namespace
    /// omission, not declare, controls whether a referred name may be replaced.
    /// Top-level statements only
    /// (stricter than the evaluator, documented); evaluates to nil.
    pub(super) fn compile_declare(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
        top: bool,
    ) -> Result<(), CompileError> {
        if !top {
            return Err(unsupported(
                "declare is only supported as a top-level statement",
                span.start,
            ));
        }
        let names = &children[1..];
        if names.is_empty() {
            self.emit(Instruction::Nil, Some(span.start));
            return Ok(());
        }
        for (index, child) in names.iter().enumerate() {
            let Form::Symbol(name) = child.form else {
                return Err(CompileError::new(
                    CompileErrorKind::Arity,
                    "declare expects name symbols",
                    Some(child.span.start),
                ));
            };
            self.declare_program_global(name);
            let constant = self.name_constant(name, child.span)?;
            self.emit(Instruction::DeclareGlobal(constant), Some(child.span.start));
            if index + 1 != names.len() {
                self.emit(Instruction::Pop, Some(child.span.start));
            }
        }
        Ok(())
    }

    /// Defines one immutable or mutable named-value family, its parallel
    /// constructors, and any inline protocol clauses.
    pub(super) fn compile_named_definition(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
        mutable: bool,
    ) -> Result<(), CompileError> {
        let kind = if mutable { "defmutable" } else { "defstruct" };
        if children.len() < 3 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                format!("{kind} expects a name and field vector"),
                Some(span.start),
            ));
        }
        let (name, _) = binding_symbol(children[1].form, &format!("{kind} name"))
            .map_err(|message| unsupported(message, children[1].span.start))?;
        if name.contains('/') {
            return Err(unsupported(
                format!("{kind} name must be an unqualified symbol"),
                children[1].span.start,
            ));
        }
        let fields: &[Form] = match children[2].form {
            Form::Vector(fields) => fields,
            _ => {
                return Err(CompileError::new(
                    CompileErrorKind::Arity,
                    format!("{kind} expects a field vector"),
                    Some(children[2].span.start),
                ))
            }
        };
        let mut names = Vec::with_capacity(fields.len());
        let mut field_values = Vec::with_capacity(fields.len());
        for field in fields {
            let (field_name, field_value) = match field {
                Form::Symbol(field) if !field.contains('/') => {
                    (field.clone(), crate::core::Value::String(field.clone()))
                }
                Form::Vector(_) => {
                    let named = crate::core::NamedField::from_form(field, kind)
                        .map_err(|message| unsupported(message, children[2].span.start))?;
                    let value = crate::core::form_to_value(field)
                        .map_err(|message| unsupported(message, children[2].span.start))?;
                    (named.name, value)
                }
                _ => {
                    return Err(unsupported(
                        format!("{kind} fields must be symbols or [name schema] vectors"),
                        children[2].span.start,
                    ))
                }
            };
            if names.iter().any(|candidate| candidate == &field_name) {
                return Err(unsupported(
                    format!("Duplicate {kind} field"),
                    children[2].span.start,
                ));
            }
            names.push(field_name);
            field_values.push(field_value);
        }
        let name_index = self.name_constant(&name, children[1].span)?;
        let fields_index = self.constant_index_of(
            crate::core::Value::Vector(field_values.into_iter().collect()),
            span,
        )?;
        self.declare_program_global(&name);
        self.declare_program_global(&format!("->{name}"));
        self.declare_program_global(&format!("map->{name}"));
        self.emit(
            if mutable {
                Instruction::DefMutable {
                    name: name_index,
                    fields: fields_index,
                }
            } else {
                Instruction::DefStruct {
                    name: name_index,
                    fields: fields_index,
                }
            },
            Some(span.start),
        );

        if children.len() == 3 {
            return Ok(());
        }
        self.emit(Instruction::Pop, Some(span.start));
        let mut index = 3;
        while index < children.len() {
            let Form::Symbol(protocol) = children[index].form else {
                return Err(unsupported(
                    format!("{kind} protocol clause expects a protocol symbol"),
                    children[index].span.start,
                ));
            };
            index += 1;
            let start = index;
            while index < children.len() && matches!(children[index].form, Form::List(_)) {
                index += 1;
            }
            if start == index {
                return Err(unsupported(
                    format!("{kind} protocol clause requires method implementations"),
                    children[index - 1].span.start,
                ));
            }
            let extension = Form::List(
                std::iter::once(Form::Symbol("extend-type".into()))
                    .chain(std::iter::once(Form::Symbol(name.to_owned())))
                    .chain(std::iter::once(Form::Symbol(protocol.to_owned())))
                    .chain(
                        children[start..index]
                            .iter()
                            .map(|child| child.form.clone()),
                    )
                    .collect(),
            );
            let value = crate::core::form_to_value(&extension).map_err(|message| {
                CompileError::new(CompileErrorKind::UnsupportedForm, message, Some(span.start))
            })?;
            let constant = self.constant_index_of(value, span)?;
            self.emit(Instruction::ExtendType(constant), Some(span.start));
            self.emit(Instruction::Pop, Some(span.start));
        }
        self.emit(Instruction::Nil, Some(span.start));
        Ok(())
    }

    /// `(field instance :name)`: direct access to one declared mutable field;
    /// the field name is a literal keyword or symbol.
    pub(super) fn compile_field(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if children.len() != 3 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "field expects a mutable value and field name",
                Some(span.start),
            ));
        }
        let field = match children[2].form {
            Form::Keyword(field) | Form::Symbol(field) if !field.contains('/') => field,
            _ => {
                return Err(unsupported(
                    "field name must be an unqualified keyword or symbol",
                    children[2].span.start,
                ))
            }
        };
        let instance = &children[1];
        self.compile_form(instance.form, instance.span, instance.children, false)?;
        if !self.ctx().fallthrough {
            return Ok(());
        }
        let index = self.name_constant(field, children[2].span)?;
        self.emit(Instruction::MutableFieldGet(index), Some(span.start));
        Ok(())
    }

    /// `(instance? type value)`: immutable or mutable named-type membership.
    pub(super) fn compile_instance_of(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if children.len() != 3 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "instance? expects a struct or mutable type and value",
                Some(span.start),
            ));
        }
        for argument in &children[1..] {
            self.compile_form(argument.form, argument.span, argument.children, false)?;
        }
        if !self.ctx().fallthrough {
            return Ok(());
        }
        self.emit(Instruction::InstanceOf, Some(span.start));
        Ok(())
    }

    /// `(defn name ...)` / `(defn- name ...)`: interns a real var holding
    /// the function (single arity) or arity dispatcher (multiple
    /// clauses) and evaluates to the var, matching the evaluator. Definitions
    /// may occur inside a function body as well as in a top-level sequence:
    /// the name is visible before the bodies compile, so self-recursion within
    /// the form resolves through the var (late binding). Referred Vars remain
    /// owned by their defining namespace and must be omitted before a local
    /// definition.
    pub(super) fn compile_defn(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
        _top: bool,
        private: bool,
    ) -> Result<(), CompileError> {
        if children.len() < 4 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "defn expects a name, parameters, and a body",
                Some(span.start),
            ));
        }
        let (name, metadata) = binding_symbol(children[1].form, "defn name")
            .map_err(|message| unsupported(message, children[1].span.start))?;
        self.require_owned_global(&name, children[1].span)?;
        // `definition_metadata` works on the raw forms; the surviving
        // `rest` is a suffix of the elements, so the matching children
        // (with spans) are the same suffix of `children`.
        let raw: Vec<Form> = children.iter().map(|child| child.form.clone()).collect();
        let (metadata, rest) = definition_metadata(metadata, &raw[2..], private, false)
            .map_err(|message| unsupported(format!("{name}: {message}"), children[1].span.start))?;
        if let Some(schema) = schema_var_reference(metadata.as_deref()) {
            let schema_name = schema.as_str();
            if !self.visible_global(schema_name) {
                return Err(CompileError::new(
                    CompileErrorKind::UnboundSymbol,
                    format!("schema Var does not exist: {schema_name}"),
                    Some(children[1].span.start),
                ));
            }
        }
        let offset = children.len() - rest.len();
        let rest_children = &children[offset..];
        if rest_children.is_empty() {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "defn expects a name, parameters, and a body",
                Some(span.start),
            ));
        }
        let async_function = metadata.as_ref().is_some_and(|value| value.flag("async"));
        if let Some(crate::lang::data::MetadataValue::Symbol(target)) = metadata
            .as_ref()
            .and_then(|value| value.get_keyword("inline-target"))
        {
            self.inline_globals
                .insert(name.clone(), target.as_str().to_owned());
        }
        let metadata = self.var_metadata(metadata);
        self.declare_program_global(&name);
        let single_arity = matches!(
            crate::core::form_without_metadata(rest_children[0].form),
            Form::Vector(_)
        );
        if single_arity {
            let suspend_allowed = async_function
                || rest_children[1..]
                    .iter()
                    .any(|child| self.form_may_suspend(child.form));
            self.compile_function(
                Some(&name),
                &rest_children[0],
                &rest_children[1..],
                span,
                async_function,
                suspend_allowed,
            )?;
        } else {
            // Multi-arity: each clause is a list `(params body...)`.
            let mut count = 0usize;
            for clause in rest_children {
                let clause_forms: &[Form] = match clause.form {
                    Form::List(forms) => forms,
                    _ => {
                        return Err(unsupported(
                            "defn multi-arity clauses must be lists",
                            clause.span.start,
                        ))
                    }
                };
                let clause_children =
                    self.list_children(clause_forms, clause.span, clause.children);
                if clause_children.len() < 2 {
                    return Err(CompileError::new(
                        CompileErrorKind::Arity,
                        "defn clause expects parameters and a body",
                        Some(clause.span.start),
                    ));
                }
                let suspend_allowed = async_function
                    || clause_children[1..]
                        .iter()
                        .any(|child| self.form_may_suspend(child.form));
                self.compile_function(
                    None,
                    &clause_children[0],
                    &clause_children[1..],
                    span,
                    async_function,
                    suspend_allowed,
                )?;
                count += 1;
                if count > u8::MAX as usize {
                    return Err(CompileError::new(
                        CompileErrorKind::Limit,
                        "defn supports at most 255 arity clauses",
                        Some(span.start),
                    ));
                }
            }
            let name_constant = self.name_constant(&name, children[1].span)?;
            self.emit(
                Instruction::MakeMultiArity {
                    name: name_constant,
                    count: count as u8,
                },
                Some(span.start),
            );
        }
        if !self.ctx().fallthrough {
            return Ok(());
        }
        let name_index = self.name_constant(&name, children[1].span)?;
        self.emit(
            Instruction::DefGlobal {
                name: name_index,
                metadata,
            },
            Some(span.start),
        );
        self.emit(Instruction::Pop, Some(span.start));
        self.emit(Instruction::VarGlobal(name_index), Some(span.start));
        Ok(())
    }
}

fn macro_params(form: &Form) -> Result<Form, String> {
    let Form::Vector(params) = crate::core::form_without_metadata(form) else {
        return Err("macro parameters must be a vector".into());
    };
    let mut implicit = vec![Form::Symbol("&form".into()), Form::Symbol("&env".into())];
    implicit.extend_from_slice(params);
    Ok(Form::Vector(implicit))
}

fn unsupported(message: impl Into<String>, position: crate::kernel::Position) -> CompileError {
    CompileError::new(
        CompileErrorKind::UnsupportedForm,
        message.into(),
        Some(position),
    )
}
