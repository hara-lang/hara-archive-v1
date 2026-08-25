use super::super::Form;
use super::{canonical, known_namespace, GeneratedNamespaceConfig};

impl GeneratedNamespaceConfig {
    pub fn rewrite(&self, form: Form) -> Form {
        self.rewrite_form(form, false)
    }

    /// Rewrites a form before macro expansion. In addition to ordinary
    /// namespace aliases and `:refer` entries, explicit `:refer-macros`
    /// declarations canonicalize list operators to the defining macro Var.
    pub fn rewrite_for_macroexpand(&self, form: Form) -> Form {
        self.rewrite_form(form, true)
    }

    fn rewrite_form(&self, form: Form, macro_head: bool) -> Form {
        match form {
            Form::Symbol(name) => Form::Symbol(self.resolve_symbol(&name)),
            Form::List(values) => {
                if matches!(
                    values.first(),
                    Some(Form::Symbol(name))
                        if name == "quote"
                            || name == "require"
                            || (macro_head && name == "syntax-quote")
                ) {
                    return Form::List(values);
                }
                let mut values = values.into_iter();
                let Some(head) = values.next() else {
                    return Form::List(Vec::new());
                };
                let head = match head {
                    Form::Symbol(name) if macro_head => {
                        Form::Symbol(self.resolve_macro_symbol(&name))
                    }
                    value => self.rewrite_form(value, macro_head),
                };
                Form::List(
                    std::iter::once(head)
                        .chain(values.map(|value| self.rewrite_form(value, macro_head)))
                        .collect(),
                )
            }
            Form::Vector(values) => Form::Vector(
                values
                    .into_iter()
                    .map(|value| self.rewrite_form(value, macro_head))
                    .collect(),
            ),
            Form::Set(values) => Form::Set(
                values
                    .into_iter()
                    .map(|value| self.rewrite_form(value, macro_head))
                    .collect(),
            ),
            Form::Map(values) => Form::Map(
                values
                    .into_iter()
                    .map(|(key, value)| {
                        (
                            self.rewrite_form(key, macro_head),
                            self.rewrite_form(value, macro_head),
                        )
                    })
                    .collect(),
            ),
            Form::Tagged(tag, value) => {
                Form::Tagged(tag, Box::new(self.rewrite_form(*value, macro_head)))
            }
            Form::Metadata(meta, value) => {
                Form::Metadata(meta, Box::new(self.rewrite_form(*value, macro_head)))
            }
            value => value,
        }
    }

    fn resolve_macro_symbol(&self, symbol: &str) -> String {
        self.macro_refers
            .get(symbol)
            .cloned()
            .unwrap_or_else(|| self.resolve_symbol(symbol))
    }

    fn resolve_symbol(&self, symbol: &str) -> String {
        if let Some(canonical) = crate::core::canonical_native_symbol(symbol) {
            return canonical;
        }
        if let Some(canonical) = self.refers.get(symbol) {
            return canonical.clone();
        }
        if symbol.contains('/') {
            if let Ok(registry) = crate::core::namespace_registry() {
                if let Some(variable) = registry.resolve(&crate::lang::data::Symbol::parse(symbol)) {
                    return variable.symbol().as_str().to_owned();
                }
            }
        }
        let Some((alias, method)) = symbol.split_once('/') else {
            return symbol.into();
        };
        if self.lazy_aliases.contains_key(alias) {
            return symbol.into();
        }
        if let Some(namespace) = self.global_aliases.get(alias) {
            return canonical(namespace, method);
        }
        if let Some(namespace) = self.aliases.get(alias) {
            return canonical(namespace, method);
        }
        if known_namespace(alias) {
            return canonical(alias, method);
        }
        symbol.into()
    }
}
