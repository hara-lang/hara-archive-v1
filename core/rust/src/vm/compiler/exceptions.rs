//! Exception compilation: `try`/`catch`/`finally` handler-table
//! entries and guest `throw` (issue #203). Split from `compiler.rs` to
//! stay under the repository's per-file line cap.

use crate::kernel::{Form, Position, Span};

use super::{Child, Compiler};
use crate::vm::error::{CompileError, CompileErrorKind};
use crate::vm::opcode::Instruction;
use crate::vm::program::{CatchEntry, TryEntry};

/// A `try` region currently being compiled. Used to reject `recur`
/// crossing a `finally` boundary: a recur targeting a loop opened before
/// the try must leave the region, which would skip the finally.
pub(super) struct TryContext {
    pub(super) has_finally: bool,
    /// `loops.len()` when the try began.
    pub(super) loop_depth: usize,
}

impl Compiler {
    /// Compiles `try`/`catch`/`finally` into a static handler-table entry
    /// plus inline clause and finally regions (notes §18.4). Catch
    /// dispatch happens in the machine through the shared
    /// `core::catch_matches`/`core::caught_error` boundary, so clause
    /// code only runs after the machine pre-stores the binding slot.
    pub(super) fn compile_try(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
        tail: bool,
    ) -> Result<(), CompileError> {
        // Partition body forms from catch/finally clauses; once a clause
        // appears, later non-clause forms are an error (fiber spelling).
        let mut body_end = 1;
        let mut clauses_started = false;
        for (index, child) in children.iter().enumerate().skip(1) {
            let is_clause = matches!(
                child.form,
                Form::List(elements)
                    if matches!(elements.first(), Some(Form::Symbol(name))
                        if name == "catch" || name == "finally")
            );
            if is_clause {
                clauses_started = true;
            } else if clauses_started {
                return Err(CompileError::new(
                    CompileErrorKind::Arity,
                    "try clauses must follow body",
                    Some(child.span.start),
                ));
            } else {
                body_end = index + 1;
            }
        }
        let body = &children[1..body_end.max(1)];
        // Parse clauses. Shapes mirror the fiber: (catch name body) is the
        // implicit Exception clause; (catch Class name body...) dispatches
        // on a class symbol. Finally bodies concatenate.
        let mut catches: Vec<(String, String, Position, Vec<Child>)> = Vec::new();
        let mut finally_body: Vec<Child> = Vec::new();
        for clause in &children[body_end.max(1)..] {
            let Form::List(elements) = clause.form else {
                unreachable!("partition keeps only clause lists")
            };
            let clause_children = self.list_children(elements, clause.span, clause.children);
            let Some(Form::Symbol(keyword)) = elements.first() else {
                unreachable!("partition keeps only catch/finally")
            };
            if keyword == "finally" {
                if clause_children.len() < 2 {
                    return Err(CompileError::new(
                        CompileErrorKind::Arity,
                        "finally expects a body",
                        Some(clause.span.start),
                    ));
                }
                finally_body.extend_from_slice(&clause_children[1..]);
                continue;
            }
            if clause_children.len() < 3 {
                return Err(CompileError::new(
                    CompileErrorKind::Arity,
                    "catch expects class, name, and body",
                    Some(clause.span.start),
                ));
            }
            let implicit_catch = match clause_children.get(1).map(|child| &child.form) {
                Some(Form::Symbol(name)) if !is_exception_class(name) => {
                    clause_children.len() == 3
                        || !matches!(
                            clause_children.get(2).map(|child| &child.form),
                            Some(Form::Symbol(_))
                        )
                }
                _ => false,
            };
            if implicit_catch {
                let Form::Symbol(name) = clause_children[1].form else {
                    return Err(CompileError::new(
                        CompileErrorKind::Arity,
                        "catch name must be symbol",
                        Some(clause.span.start),
                    ));
                };
                catches.push((
                    "Exception".to_string(),
                    name.clone(),
                    clause_children[1].span.start,
                    clause_children[2..].to_vec(),
                ));
            } else if clause_children.len() >= 4 {
                let selector = match clause_children[1].form {
                    Form::Symbol(class) => class.clone(),
                    Form::Keyword(code) if code.contains('/') => format!(":{code}"),
                    Form::Vector(codes)
                        if !codes.is_empty()
                            && codes.iter().all(
                                |code| matches!(code, Form::Keyword(name) if name.contains('/')),
                            ) =>
                    {
                        let selectors = codes
                            .iter()
                            .map(|code| match code {
                                Form::Keyword(name) => format!(":{name}"),
                                _ => unreachable!(),
                            })
                            .collect::<Vec<_>>()
                            .join(",");
                        format!("[{selectors}]")
                    }
                    _ => {
                        return Err(CompileError::new(
                            CompileErrorKind::Arity,
                            if matches!(clause_children[1].form, Form::Symbol(_)) {
                                "catch selector must be a namespaced keyword, a non-empty vector of namespaced keywords, or omitted"
                            } else {
                                "catch class must be symbol"
                            },
                            Some(clause_children[1].span.start),
                        ))
                    }
                };
                let Form::Symbol(name) = clause_children[2].form else {
                    return Err(CompileError::new(
                        CompileErrorKind::Arity,
                        "catch name must be symbol",
                        Some(clause_children[2].span.start),
                    ));
                };
                catches.push((
                    selector,
                    name.clone(),
                    clause_children[2].span.start,
                    clause_children[3..].to_vec(),
                ));
            } else {
                return Err(CompileError::new(
                    CompileErrorKind::Arity,
                    "catch expects class, name, and body",
                    Some(clause.span.start),
                ));
            }
        }

        let has_finally = !finally_body.is_empty();
        // Hidden pending slots: never name-resolvable (notes §18.4).
        let pending_value = if has_finally {
            Some(self.ctx_mut().scopes.declare_hidden()?)
        } else {
            None
        };
        let pending_error = if has_finally {
            Some(self.ctx_mut().scopes.declare_hidden()?)
        } else {
            None
        };
        if let Some(flag) = pending_error {
            self.emit(Instruction::False, Some(span.start));
            self.emit(Instruction::StoreLocal(flag), Some(span.start));
        }
        let entry_index = self.ctx().handlers.len();
        let start = self.ctx().code.len() as u32;
        self.ctx_mut().handlers.push(TryEntry {
            start,
            end: 0,
            depth: 0,
            catches: Vec::new(),
            finally: None,
            pending_value,
            pending_error,
        });
        let loop_depth = self.ctx().loops.len();
        self.ctx_mut().tries.push(TryContext {
            has_finally,
            loop_depth,
        });
        // Tail position propagates into catch-only regions: a recur there
        // leaves the protected range through an ordinary jump, which the
        // table makes free. With a finally the result must route through
        // the pending slots, and recur across the boundary is rejected.
        let region_tail = tail && !has_finally;
        self.compile_sequence(body, region_tail)?;
        let body_fell = self.ctx().fallthrough;
        let mut finally_jumps = Vec::new();
        let mut after_jumps = Vec::new();
        if body_fell {
            if let Some(pending) = pending_value {
                self.emit(Instruction::StoreLocal(pending), None);
                finally_jumps.push(self.emit(Instruction::Jump(0), None));
            } else {
                after_jumps.push(self.emit(Instruction::Jump(0), None));
            }
        }
        self.ctx_mut().handlers[entry_index].end = self.ctx().code.len() as u32;
        let mut clause_fell = false;
        for (class, name, name_position, clause_body) in catches {
            let target = self.ctx().code.len() as u32;
            self.ctx_mut().scopes.push_scope();
            let binding = self.ctx_mut().scopes.declare(&name).map_err(|error| {
                CompileError::new(error.kind(), error.message(), Some(name_position))
            })?;
            self.ctx_mut().handlers[entry_index]
                .catches
                .push(CatchEntry {
                    class,
                    binding,
                    target,
                });
            // The clause is a fresh entry point reached by unwinding; the
            // body's terminal state does not carry over.
            self.ctx_mut().fallthrough = true;
            self.compile_sequence(&clause_body, region_tail)?;
            if self.ctx().fallthrough {
                clause_fell = true;
                if let Some(pending) = pending_value {
                    self.emit(Instruction::StoreLocal(pending), None);
                    finally_jumps.push(self.emit(Instruction::Jump(0), None));
                } else {
                    after_jumps.push(self.emit(Instruction::Jump(0), None));
                }
            }
            self.ctx_mut().scopes.pop_scope();
        }
        if let Some(flag) = pending_error {
            let pending = pending_value.expect("pending value with finally");
            let finally_target = self.ctx().code.len();
            self.ctx_mut().handlers[entry_index].finally = Some(finally_target as u32);
            for jump in finally_jumps {
                self.patch_jump(jump, finally_target);
            }
            // The finally region is reached on every path, including pure
            // unwind paths when neither the body nor any clause falls
            // through.
            self.ctx_mut().fallthrough = true;
            self.compile_sequence(&finally_body, false)?;
            let finally_fell = self.ctx().fallthrough;
            if finally_fell {
                self.emit(Instruction::Pop, None);
                self.emit(Instruction::LoadLocal(flag), None);
                let jump_normal = self.emit(Instruction::JumpIfFalse(0), None);
                self.emit(Instruction::LoadLocal(pending), None);
                self.emit(Instruction::Rethrow, Some(span.start));
                let normal = self.ctx().code.len();
                self.patch_jump(jump_normal, normal);
                self.emit(Instruction::LoadLocal(pending), None);
            }
            self.ctx_mut().fallthrough = finally_fell;
        } else {
            let after = self.ctx().code.len();
            for jump in after_jumps {
                self.patch_jump(jump, after);
            }
            self.ctx_mut().fallthrough = body_fell || clause_fell;
        }
        self.ctx_mut().tries.pop();
        Ok(())
    }

    /// Compiles guest `(throw value)`: raises through the shared
    /// `core::thrown_error` boundary. Terminal, like `recur`.
    pub(super) fn compile_throw(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if children.len() != 2 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "throw expects one value",
                Some(span.start),
            ));
        }
        let argument = &children[1];
        self.compile_form(argument.form, argument.span, argument.children, false)?;
        if !self.ctx().fallthrough {
            return Ok(());
        }
        self.emit(Instruction::Throw, Some(span.start));
        self.ctx_mut().fallthrough = false;
        Ok(())
    }
}

fn is_exception_class(name: &str) -> bool {
    matches!(
        name,
        "Exception" | "Throwable" | "std.native.Exception" | "std.native.Throwable"
    )
}
