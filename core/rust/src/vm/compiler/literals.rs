//! Quoted, syntax-quoted, collection, and primitive emission.

use super::*;

impl Compiler {
    pub(super) fn compile_quote(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if children.len() != 2 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "quote expects one argument",
                Some(span.start),
            ));
        }
        let value = crate::core::form_to_value(children[1].form).map_err(|message| {
            CompileError::new(CompileErrorKind::UnsupportedForm, message, Some(span.start))
        })?;
        self.constant(value, span)
    }

    pub(super) fn compile_syntax_quote(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if children.len() != 2 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "syntax-quote expects one argument",
                Some(span.start),
            ));
        }
        self.compile_syntax_value(children[1].form, span, false)
    }

    pub(super) fn compile_syntax_value(
        &mut self,
        form: &Form,
        span: &Span,
        nested: bool,
    ) -> Result<(), CompileError> {
        if let Some(argument) = unquote_argument(form, "unquote") {
            let argument = argument.map_err(|message| {
                CompileError::new(CompileErrorKind::Arity, message, Some(span.start))
            })?;
            return self.compile_form(&argument, span, None, false);
        }
        if unquote_argument(form, "unquote-splicing").is_some() {
            return Err(CompileError::new(
                CompileErrorKind::UnsupportedForm,
                if nested {
                    "unquote-splicing is only valid as a collection element"
                } else {
                    "unquote-splicing is not valid at the root of syntax-quote"
                },
                Some(span.start),
            ));
        }
        match crate::core::form_without_metadata(form) {
            Form::List(values) | Form::Vector(values) => {
                let vector = matches!(crate::core::form_without_metadata(form), Form::Vector(_));
                let spliced = values
                    .iter()
                    .any(|value| unquote_argument(value, "unquote-splicing").is_some());
                for value in values {
                    if let Some(argument) = unquote_argument(value, "unquote-splicing") {
                        let argument = argument.map_err(|message| {
                            CompileError::new(CompileErrorKind::Arity, message, Some(span.start))
                        })?;
                        self.compile_form(&argument, span, None, false)?;
                    } else {
                        self.compile_syntax_value(value, span, true)?;
                        if spliced {
                            self.emit(Instruction::BuildList(1), Some(span.start));
                        }
                    }
                }
                let count = self.collection_count(values.len(), span)?;
                if spliced {
                    self.emit(Instruction::ConcatList(count), Some(span.start));
                    if vector {
                        self.emit(Instruction::ToVector, Some(span.start));
                    }
                } else if vector {
                    self.emit(Instruction::BuildVector(count), Some(span.start));
                } else {
                    self.emit(Instruction::BuildList(count), Some(span.start));
                }
                Ok(())
            }
            Form::Map(entries) => {
                for (key, value) in entries {
                    self.compile_syntax_value(key, span, true)?;
                    self.compile_syntax_value(value, span, true)?;
                }
                self.emit(
                    Instruction::BuildMap(entries.len() as u16),
                    Some(span.start),
                );
                Ok(())
            }
            Form::Set(values) => {
                for value in values {
                    self.compile_syntax_value(value, span, true)?;
                }
                self.emit(Instruction::BuildSet(values.len() as u16), Some(span.start));
                Ok(())
            }
            _ => {
                let value = crate::core::form_to_value(form).map_err(|message| {
                    CompileError::new(CompileErrorKind::UnsupportedForm, message, Some(span.start))
                })?;
                self.constant(value, span)
            }
        }
    }

    pub(super) fn compile_primitive(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
        op: IntrinsicOp,
    ) -> Result<(), CompileError> {
        let argc = children.len() - 1;
        if argc > MAX_PRIMITIVE_ARGUMENTS {
            return Err(CompileError::new(
                CompileErrorKind::Limit,
                format!("primitive calls support at most {MAX_PRIMITIVE_ARGUMENTS} arguments"),
                Some(span.start),
            ));
        }
        if children[1..]
            .iter()
            .all(|argument| constant_form(argument.form))
        {
            let arguments = children[1..]
                .iter()
                .map(|argument| crate::core::form_to_value(argument.form))
                .collect::<Result<Vec<_>, _>>();
            if let Ok(arguments) = arguments {
                if let Ok(value) = crate::core::apply_intrinsic(op, &arguments) {
                    return self.constant(value, span);
                }
            }
        }
        for argument in &children[1..] {
            self.compile_form(argument.form, argument.span, argument.children, false)?;
        }
        if !self.ctx().fallthrough {
            return Ok(());
        }
        let target = self.name_constant(op.operator(), span)?;
        self.emit(
            Instruction::IntrinsicCall {
                target,
                argc: argc as u8,
            },
            Some(span.start),
        );
        Ok(())
    }
}
