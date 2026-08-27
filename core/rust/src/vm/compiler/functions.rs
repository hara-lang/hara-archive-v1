//! Function and call compilation: `fn` closures with by-value captures
//! (including variadic parameters) and direct calls. Global forms live
//! in `globals.rs` (issue #223). Split from `compiler.rs` to stay under
//! the repository's per-file line cap.

use crate::core::IntrinsicOp;
use crate::kernel::{Form, Position, Span};
use crate::vm::error::{CompileError, CompileErrorKind};
use crate::vm::opcode::Instruction;
use crate::vm::program::{FunctionPrototype, MAX_CAPTURES};
use crate::vm::source_map::SourceMap;

use super::scope::ScopeStack;
use super::{placeholder, Child, Compiler, FnContext};

impl Compiler {
    /// A call whose operator is itself an expression, e.g. `((fn [x] x) 1)`.
    pub(super) fn compile_expression_call(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if self.compile_immediate_fn_call(children, span)? {
            return Ok(());
        }
        let callee = &children[0];
        self.compile_form(callee.form, callee.span, callee.children, false)?;
        if !self.ctx().fallthrough {
            return Ok(());
        }
        // A capture-free function literal that is called immediately never
        // needs to become a heap closure. Its body is already a prototype,
        // so replace the just-emitted Closure with a direct VM call.
        let argc = (children.len() - 1) as u8;
        let direct = match self.ctx().code.last() {
            Some(Instruction::Closure {
                prototype,
                captures: 0,
            }) => {
                let proto = &self.functions[usize::from(*prototype)];
                let accepts = (!proto.variadic && proto.arity == u16::from(argc))
                    || (proto.variadic && u16::from(argc) >= proto.arity);
                accepts.then_some(*prototype)
            }
            _ => None,
        };
        if direct.is_some() {
            self.ctx_mut().code.pop();
            self.ctx_mut().source_map.pop();
        }
        self.compile_call_arguments(children, span)?;
        if !self.ctx().fallthrough {
            return Ok(());
        }
        match direct {
            Some(prototype) => self.emit(
                Instruction::CallStatic { prototype, argc },
                Some(span.start),
            ),
            None => self.emit(Instruction::Call { argc }, Some(span.start)),
        };
        Ok(())
    }

    pub(super) fn compile_fn_form(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if children.len() < 3 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "fn expects parameters and a body",
                Some(span.start),
            ));
        }
        if matches!(
            crate::core::form_without_metadata(children[1].form),
            Form::Vector(_)
        ) {
            let async_function = metadata_flag(children[1].form, "async")?;
            let suspend_allowed = async_function
                || children[2..]
                    .iter()
                    .any(|child| self.form_may_suspend(child.form));
            return self.compile_function(
                None,
                &children[1],
                &children[2..],
                span,
                async_function,
                suspend_allowed,
            );
        }
        if !matches!(
            crate::core::form_without_metadata(children[1].form),
            Form::List(_)
        ) {
            return Err(CompileError::new(
                CompileErrorKind::UnsupportedForm,
                "function parameters must be a vector",
                Some(children[1].span.start),
            ));
        }
        let mut count = 0usize;
        for clause in &children[1..] {
            let clause_forms = match crate::core::form_without_metadata(clause.form) {
                Form::List(forms) => forms,
                _ => {
                    return Err(CompileError::new(
                        CompileErrorKind::UnsupportedForm,
                        "fn multi-arity clauses must be lists",
                        Some(clause.span.start),
                    ))
                }
            };
            let clause_children = self.list_children(clause_forms, clause.span, clause.children);
            if clause_children.len() < 2 {
                return Err(CompileError::new(
                    CompileErrorKind::Arity,
                    "fn clause expects parameters and a body",
                    Some(clause.span.start),
                ));
            }
            let async_function = metadata_flag(clause_children[0].form, "async")?;
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
                    "fn supports at most 255 arity clauses",
                    Some(clause.span.start),
                ));
            }
        }
        let name = self.name_constant("<anonymous>", span)?;
        self.emit(
            Instruction::MakeMultiArity {
                name,
                count: count as u8,
            },
            Some(span.start),
        );
        Ok(())
    }

    /// Detects an await in one function body. This enables OpenResty-style
    /// synchronous entry: an ordinary function remains synchronous when it
    /// completes immediately, but its existing VM call stack may suspend when
    /// execution actually reaches an await.
    pub(super) fn form_may_suspend(&self, form: &Form) -> bool {
        match crate::core::form_without_metadata(form) {
            Form::List(values) => {
                if matches!(values.first(), Some(Form::Symbol(name)) if self.is_coroutine_var(name, "await") || self.is_coroutine_var(name, "yield"))
                {
                    return true;
                }
                values.iter().any(|value| self.form_may_suspend(value))
            }
            Form::Vector(values) | Form::Set(values) => {
                values.iter().any(|value| self.form_may_suspend(value))
            }
            Form::Map(entries) => entries
                .iter()
                .any(|(key, value)| self.form_may_suspend(key) || self.form_may_suspend(value)),
            Form::Tagged(_, value) => self.form_may_suspend(value),
            _ => false,
        }
    }

    /// Compiles a `fn`/`defn` body into a new function context and emits
    /// the closure creation (capture loads + `Closure`) into the
    /// enclosing context. `name` names the function value (display and
    /// errors); self-references resolve as globals, not captures
    /// (issue #223).
    pub(super) fn compile_function(
        &mut self,
        name: Option<&str>,
        params: &Child<'_>,
        body: &[Child<'_>],
        span: &Span,
        async_function: bool,
        suspend_allowed: bool,
    ) -> Result<(), CompileError> {
        let elements: &[Form] = match crate::core::form_without_metadata(params.form) {
            Form::Vector(elements) => elements,
            Form::List(_) => {
                return Err(CompileError::new(
                    CompileErrorKind::UnsupportedForm,
                    "function parameters must be a vector",
                    Some(params.span.start),
                ))
            }
            _ => {
                return Err(CompileError::new(
                    CompileErrorKind::Arity,
                    "function parameters must be a vector",
                    Some(params.span.start),
                ))
            }
        };
        let param_children = self.list_children(elements, params.span, params.children);
        let mut names: Vec<String> = Vec::with_capacity(elements.len());
        let mut rest_at: Option<usize> = None;
        for param in &param_children {
            match param.form {
                Form::Symbol(name) if name == "&" => {
                    if rest_at.is_some() {
                        return Err(CompileError::new(
                            CompileErrorKind::Arity,
                            "function parameters support a single & rest parameter",
                            Some(param.span.start),
                        ));
                    }
                    rest_at = Some(names.len());
                }
                Form::Symbol(name) => names.push(name.clone()),
                _ => {
                    return Err(CompileError::new(
                        CompileErrorKind::UnsupportedForm,
                        "fn destructuring is not supported",
                        Some(param.span.start),
                    ))
                }
            }
        }
        // With `& rest`, the fixed arity is the count before `&` and the
        // rest parameter occupies the slot directly above the fixed
        // params (captures sit above it), matching `call_function`.
        let (arity, variadic) = match rest_at {
            Some(fixed) => {
                if names.len() != fixed + 1 {
                    return Err(CompileError::new(
                        CompileErrorKind::Arity,
                        "the & rest parameter must be the last parameter",
                        Some(params.span.start),
                    ));
                }
                (fixed as u16, true)
            }
            None => (names.len() as u16, false),
        };
        // Free variables become capture slots directly above the params.
        let mut free: Vec<(String, Option<Position>)> = Vec::new();
        {
            let mut bound = names.clone();
            for child in body {
                self.collect_free(child, &mut bound, &mut free);
            }
        }
        if free.len() > MAX_CAPTURES {
            return Err(CompileError::new(
                CompileErrorKind::Limit,
                format!("closures support at most {MAX_CAPTURES} captures"),
                Some(span.start),
            ));
        }
        let proto_id = self.functions.len();
        self.functions.push(placeholder(
            name.map(str::to_string),
            arity,
            free.len() as u16,
            variadic,
            async_function,
        ));
        let mut scopes = ScopeStack::new();
        scopes.push_scope();
        // `names` includes the rest parameter; the param-child zip only
        // covers the fixed params plus `&`, so positions come from the
        // name symbols where possible.
        for (name, param) in names.iter().zip(&param_children) {
            scopes.declare(name).map_err(|error| {
                CompileError::new(error.kind(), error.message(), Some(param.span.start))
            })?;
        }
        for (name, position) in &free {
            scopes
                .declare(name)
                .map_err(|error| CompileError::new(error.kind(), error.message(), *position))?;
        }
        self.contexts.push(FnContext {
            proto_id,
            name: name.map(str::to_string),
            params: arity,
            variadic,
            suspend_allowed,
            async_function,
            captures: free,
            code: Vec::new(),
            source_map: SourceMap::default(),
            scopes,
            loops: Vec::new(),
            tries: Vec::new(),
            handlers: Vec::new(),
            fallthrough: true,
        });
        let compiled = self.compile_sequence(body, true);
        if compiled.is_ok() && self.ctx().fallthrough {
            self.emit(Instruction::Return, None);
        }
        let context = self.close_context();
        compiled?;
        // The closure is created in the enclosing context: load each
        // capture (pre-declared as a binding in every intermediate
        // function, so one resolution step suffices), then the function
        // value itself.
        for (name, position) in &context.captures {
            let Some(slot) = self.ctx().scopes.resolve(name) else {
                return Err(CompileError::new(
                    CompileErrorKind::UnboundSymbol,
                    format!("unbound symbol: {name}"),
                    *position,
                ));
            };
            self.emit(Instruction::LoadLocal(slot), *position);
        }
        self.emit(
            Instruction::Closure {
                prototype: proto_id as u16,
                captures: context.captures.len() as u8,
            },
            Some(span.start),
        );
        Ok(())
    }

    /// Pops the current function context and fills in its reserved
    /// prototype. Returns the context for its capture list.
    pub(super) fn close_context(&mut self) -> FnContext {
        let context = self.contexts.pop().expect("balanced context stack");
        self.functions[context.proto_id] = FunctionPrototype {
            name: context.name.clone(),
            async_function: context.async_function,
            arity: context.params,
            variadic: context.variadic,
            capture_count: context.captures.len() as u16,
            local_count: context.scopes.high_water(),
            max_stack: 0,
            code: context.code.clone(),
            source_map: context.source_map.clone(),
            handlers: context.handlers.clone(),
        };
        context
    }

    /// Free-variable pre-pass: collects symbol references inside `child`
    /// that are not bound within the current function (params, let's,
    /// loops, and nested fn params are bound; special-form and primitive
    /// operators are not references). First-occurrence order, deduped.
    fn collect_free(
        &self,
        child: &Child<'_>,
        bound: &mut Vec<String>,
        free: &mut Vec<(String, Option<Position>)>,
    ) {
        match child.form {
            Form::Symbol(name) => {
                if bound.iter().any(|b| b == name) || free.iter().any(|(f, _)| f == name) {
                    // Bound in this function or already collected.
                } else if self.ctx().scopes.resolve(name).is_some()
                    || (!self.visible_global(name)
                        && IntrinsicOp::from_symbol(name).is_none()
                        && !self.visible_bytecode_callable(name)
                        && !self.visible_namespace(name))
                {
                    // An enclosing lexical binding always wins over a Var or
                    // builtin with the same name. Everything else that is not
                    // globally resolvable is a prospective capture; closure
                    // emission reports it if no enclosing slot exists.
                    free.push((name.clone(), Some(child.span.start)));
                } else {
                    // Globals compile to GetGlobal (late binding through the
                    // shared Var cell, issue #223), never captures.
                }
            }
            Form::List(elements) if !elements.is_empty() => {
                let protected = matches!(
                    elements.first(),
                    Some(Form::Symbol(name)) if name == "quote" || name == "syntax-quote"
                );
                if !protected {
                    if let Ok(expanded) = crate::core::vm_macroexpand(child.form) {
                        if expanded != *child.form {
                            let expanded = Child {
                                form: &expanded,
                                span: child.span,
                                children: None,
                            };
                            self.collect_free(&expanded, bound, free);
                            return;
                        }
                    }
                }
                let children = self.list_children(elements, child.span, child.children);
                match &elements[0] {
                    Form::Symbol(head) if self.is_coroutine_var(head, "await") => {
                        for c in &children[1..] {
                            self.collect_free(c, bound, free);
                        }
                    }
                    Form::Symbol(head) if self.is_coroutine_var(head, "yield") => {
                        for c in &children[1..] {
                            self.collect_free(c, bound, free);
                        }
                    }
                    Form::Symbol(head) if self.is_host_call_var(head) => {
                        for c in &children[1..] {
                            self.collect_free(c, bound, free);
                        }
                    }
                    Form::Symbol(head) => match head.as_str() {
                        "if" | "and" | "or" | "cond" | "do" | "recur" | "try" | "throw"
                        | "finally" => {
                            for c in &children[1..] {
                                self.collect_free(c, bound, free);
                            }
                        }
                        "quote" => {}
                        "." => {
                            if let Some(receiver) = children.get(1) {
                                self.collect_free(receiver, bound, free);
                            }
                            if let Some(method) = children.get(2) {
                                if let Form::List(arguments) = method.form {
                                    for argument in &arguments[1..] {
                                        let argument = Child {
                                            form: argument,
                                            span: method.span,
                                            children: None,
                                        };
                                        self.collect_free(&argument, bound, free);
                                    }
                                }
                            }
                        }
                        "syntax-quote" => {
                            if let Some(template) = children.get(1) {
                                self.collect_syntax_free(template, bound, free);
                            }
                        }
                        // Catch clauses bind their name symbol over the
                        // clause body; the class symbol is not a
                        // reference. Shapes mirror the compiler:
                        // (catch name body) or (catch Class name body...).
                        "catch" => {
                            let marked = bound.len();
                            if matches!(children.get(1).map(|child| child.form), Some(Form::Symbol(name)) if !matches!(name.as_str(), "Exception" | "Throwable" | "std.native.Exception" | "std.native.Throwable"))
                            {
                                if let Form::Symbol(name) = children[1].form {
                                    bound.push(name.clone());
                                }
                                for c in &children[2..] {
                                    self.collect_free(c, bound, free);
                                }
                            } else if children.len() >= 4 {
                                if let Form::Symbol(name) = children[2].form {
                                    bound.push(name.clone());
                                }
                                for c in &children[3..] {
                                    self.collect_free(c, bound, free);
                                }
                            } else {
                                // Malformed: rejected by the compiler
                                // later; walk conservatively.
                                for c in &children[1..] {
                                    self.collect_free(c, bound, free);
                                }
                            }
                            bound.truncate(marked);
                        }
                        "let" | "loop" => {
                            if children.len() >= 2 {
                                let bindings = &children[1];
                                match bindings.form {
                                    Form::Vector(pair_forms) | Form::List(pair_forms) => {
                                        let pairs = self.list_children(
                                            pair_forms,
                                            bindings.span,
                                            bindings.children,
                                        );
                                        // Initializers see only earlier
                                        // bindings (sequential `let`);
                                        // each name binds right after
                                        // its initializer.
                                        let marked = bound.len();
                                        for pair in pairs.chunks(2) {
                                            if let [name, initializer] = pair {
                                                self.collect_free(initializer, bound, free);
                                                collect_pattern_names(name.form, bound);
                                            }
                                        }
                                        for c in &children[2..] {
                                            self.collect_free(c, bound, free);
                                        }
                                        bound.truncate(marked);
                                    }
                                    _ => {
                                        for c in &children[1..] {
                                            self.collect_free(c, bound, free);
                                        }
                                    }
                                }
                            }
                        }
                        "binding" => {
                            if let Some(bindings) = children.get(1) {
                                match bindings.form {
                                    Form::Vector(pairs) | Form::List(pairs) => {
                                        let pairs = self.list_children(
                                            pairs,
                                            bindings.span,
                                            bindings.children,
                                        );
                                        for pair in pairs.chunks(2) {
                                            if let [_, value] = pair {
                                                self.collect_free(value, bound, free);
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            for body in &children[2..] {
                                self.collect_free(body, bound, free);
                            }
                        }
                        "field" => {
                            if let Some(receiver) = children.get(1) {
                                self.collect_free(receiver, bound, free);
                            }
                        }
                        "set!" => {
                            if let Some(place) = children.get(1) {
                                match place.form {
                                    Form::List(parts)
                                        if matches!(
                                            parts.first(),
                                            Some(Form::Symbol(operation)) if operation == "field"
                                        ) =>
                                    {
                                        let place_children =
                                            self.list_children(parts, place.span, place.children);
                                        if let Some(receiver) = place_children.get(1) {
                                            self.collect_free(receiver, bound, free);
                                        }
                                    }
                                    Form::Symbol(_) => {}
                                    _ => self.collect_free(place, bound, free),
                                }
                            }
                            for replacement in &children[2..] {
                                self.collect_free(replacement, bound, free);
                            }
                        }
                        "letfn" => {
                            let marked = bound.len();
                            let definitions =
                                children.get(1).and_then(|bindings| match bindings.form {
                                    Form::Vector(definitions) => Some((bindings, definitions)),
                                    _ => None,
                                });
                            if let Some((bindings, definitions)) = definitions {
                                let definitions = self.list_children(
                                    definitions,
                                    bindings.span,
                                    bindings.children,
                                );
                                for definition in &definitions {
                                    if let Form::List(parts) = definition.form {
                                        if let Some(Form::Symbol(name)) = parts.first() {
                                            bound.push(name.clone());
                                        }
                                    }
                                }
                                for definition in &definitions {
                                    let Form::List(parts) = definition.form else {
                                        continue;
                                    };
                                    if parts.len() < 3 {
                                        continue;
                                    }
                                    let local_marked = bound.len();
                                    if let Form::Vector(params) = &parts[1] {
                                        for param in params {
                                            collect_pattern_names(param, bound);
                                        }
                                    }
                                    for form in &parts[2..] {
                                        let child = Child {
                                            form,
                                            span: definition.span,
                                            children: None,
                                        };
                                        self.collect_free(&child, bound, free);
                                    }
                                    bound.truncate(local_marked);
                                }
                            }
                            for body in &children[2..] {
                                self.collect_free(body, bound, free);
                            }
                            bound.truncate(marked);
                        }
                        "fn" => {
                            let marked = bound.len();
                            match children
                                .get(1)
                                .map(|c| crate::core::form_without_metadata(c.form))
                            {
                                Some(Form::Vector(params)) => {
                                    for param in params {
                                        collect_pattern_names(param, bound);
                                    }
                                    for c in &children[2..] {
                                        self.collect_free(c, bound, free);
                                    }
                                }
                                _ => {
                                    for clause in &children[1..] {
                                        let clause_marked = bound.len();
                                        match crate::core::form_without_metadata(clause.form) {
                                            Form::List(parts) if !parts.is_empty() => {
                                                let parts = self.list_children(
                                                    parts,
                                                    clause.span,
                                                    clause.children,
                                                );
                                                if let Some(params) = parts.first() {
                                                    if let Form::Vector(params) =
                                                        crate::core::form_without_metadata(
                                                            params.form,
                                                        )
                                                    {
                                                        for param in params {
                                                            collect_pattern_names(param, bound);
                                                        }
                                                    }
                                                }
                                                for body in &parts[1..] {
                                                    self.collect_free(body, bound, free);
                                                }
                                            }
                                            _ => self.collect_free(clause, bound, free),
                                        }
                                        bound.truncate(clause_marked);
                                    }
                                }
                            }
                            bound.truncate(marked);
                        }
                        // Namespace management is structural data consumed by
                        // NamespaceOperation; its symbols are not lexical
                        // references in the surrounding function.
                        "ns" | "ns+" | "require" => {}
                        // Rejected by the compiler later; nothing to collect.
                        "defn" | "var" => {}
                        _ if IntrinsicOp::from_symbol(head).is_some()
                            && !self.visible_global(head) =>
                        {
                            for c in &children[1..] {
                                self.collect_free(c, bound, free);
                            }
                        }
                        _ => {
                            // A function call: the operator is a
                            // reference unless it is a visible global.
                            if !bound.iter().any(|b| b == head)
                                && !free.iter().any(|(f, _)| f == head)
                                && (self.ctx().scopes.resolve(head).is_some()
                                    || (!self.visible_global(head)
                                        && !self.visible_bytecode_callable(head)
                                        && !self.visible_namespace(head)))
                            {
                                free.push((head.clone(), Some(children[0].span.start)));
                            }
                            for c in &children[1..] {
                                self.collect_free(c, bound, free);
                            }
                        }
                    },
                    _ => {
                        for c in &children {
                            self.collect_free(c, bound, free);
                        }
                    }
                }
            }
            Form::Metadata(_, value) => {
                let wrapped = Child {
                    form: value,
                    span: child.span,
                    children: None,
                };
                self.collect_free(&wrapped, bound, free);
            }
            Form::Vector(values) | Form::Set(values) => {
                for form in values {
                    let nested = Child {
                        form,
                        span: child.span,
                        children: None,
                    };
                    self.collect_free(&nested, bound, free);
                }
            }
            Form::Map(entries) => {
                for (key, value) in entries {
                    for form in [key, value] {
                        let nested = Child {
                            form,
                            span: child.span,
                            children: None,
                        };
                        self.collect_free(&nested, bound, free);
                    }
                }
            }
            _ => {}
        }
    }

    fn collect_syntax_free(
        &self,
        child: &Child<'_>,
        bound: &mut Vec<String>,
        free: &mut Vec<(String, Option<Position>)>,
    ) {
        match crate::core::form_without_metadata(child.form) {
            Form::List(values) if matches!(values.first(), Some(Form::Symbol(name)) if name == "unquote" || name == "unquote-splicing") => {
                if let Some(value) = values.get(1) {
                    let nested = Child {
                        form: value,
                        span: child.span,
                        children: None,
                    };
                    self.collect_free(&nested, bound, free);
                }
            }
            Form::List(values) | Form::Vector(values) | Form::Set(values) => {
                for value in values {
                    let nested = Child {
                        form: value,
                        span: child.span,
                        children: None,
                    };
                    self.collect_syntax_free(&nested, bound, free);
                }
            }
            Form::Map(entries) => {
                for (key, value) in entries {
                    for form in [key, value] {
                        let nested = Child {
                            form,
                            span: child.span,
                            children: None,
                        };
                        self.collect_syntax_free(&nested, bound, free);
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_pattern_names(pattern: &Form, output: &mut Vec<String>) {
    match pattern {
        Form::Symbol(name) if name != "&" => output.push(name.clone()),
        Form::Vector(items) => {
            let mut skip_marker = false;
            for item in items {
                if matches!(item, Form::Symbol(name) if name == "&")
                    || matches!(item, Form::Keyword(name) if name == "as")
                {
                    skip_marker = true;
                    continue;
                }
                collect_pattern_names(item, output);
                skip_marker = false;
            }
            let _ = skip_marker;
        }
        Form::Map(entries) => {
            for (binding, value) in entries {
                match binding {
                    Form::Keyword(name) if matches!(name.as_str(), "keys" | "strs" | "syms") => {
                        if let Form::Vector(names) = value {
                            for name in names {
                                collect_pattern_names(name, output);
                            }
                        }
                    }
                    Form::Keyword(name) if name == "as" => collect_pattern_names(value, output),
                    Form::Keyword(name) if name == "or" => {}
                    _ => collect_pattern_names(binding, output),
                }
            }
        }
        _ => {}
    }
}

fn metadata_flag(form: &Form, key: &str) -> Result<bool, CompileError> {
    let Form::Metadata(metadata, _) = form else {
        return Ok(false);
    };
    crate::core::metadata_from_form(metadata)
        .map(|metadata| metadata.flag(key))
        .map_err(|message| CompileError::new(CompileErrorKind::UnsupportedForm, message, None))
}
