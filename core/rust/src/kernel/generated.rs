use std::collections::{HashMap, HashSet};

use super::Form;

const FOUNDATION_LIBRARIES: &[(&str, &str, &str)] = &[
    ("string", "std.foundation.string", "str"),
    ("promise", "std.foundation.promise", "promise"),
    ("bytes", "std.foundation.bytes", "bytes"),
    ("coroutine", "std.foundation.coroutine", "co"),
    ("pretty", "std.foundation.pretty", "pretty"),
];

pub(crate) fn foundation_library_alias(library: &str) -> Option<&'static str> {
    FOUNDATION_LIBRARIES
        .iter()
        .find(|(name, _, _)| *name == library)
        .map(|(_, _, alias)| *alias)
}

#[path = "generated/rewrite.rs"]
mod rewrite;

#[derive(Debug, Clone, Default)]
pub struct GeneratedNamespaceConfig {
    aliases: HashMap<String, String>,
    global_alias: Option<String>,
    global_aliases: HashMap<String, String>,
    declared_global_imports: Vec<String>,
    lazy_aliases: HashMap<String, String>,
    refers: HashMap<String, String>,
    macro_refers: HashMap<String, String>,
    required_namespaces: Vec<String>,
    used_namespaces: Vec<String>,
    used_exclusions: HashMap<String, HashSet<String>>,
    internal_access: HashSet<String>,
    excluded_foundation_libraries: HashSet<String>,
    excluded_foundation: HashSet<String>,
    exposed_foundation: Option<HashSet<String>>,
    native_flavor: Option<String>,
    native_imports: Vec<(String, String)>,
    native_flavor_imports: Vec<(String, String)>,
    role: String,
    blank: bool,
}

impl GeneratedNamespaceConfig {
    pub fn defaults() -> Self {
        Self {
            aliases: HashMap::new(),
            global_alias: None,
            global_aliases: HashMap::new(),
            declared_global_imports: Vec::new(),
            lazy_aliases: HashMap::new(),
            refers: HashMap::new(),
            macro_refers: HashMap::new(),
            required_namespaces: Vec::new(),
            used_namespaces: Vec::new(),
            used_exclusions: HashMap::new(),
            internal_access: HashSet::new(),
            excluded_foundation_libraries: HashSet::new(),
            excluded_foundation: HashSet::new(),
            exposed_foundation: None,
            native_flavor: None,
            native_imports: Vec::new(),
            native_flavor_imports: Vec::new(),
            role: "standard".into(),
            blank: false,
        }
    }

    pub fn configure(clauses: &[Form]) -> Result<Self, String> {
        Self::configure_with(clauses, known_namespace)
    }

    pub fn configure_with(
        clauses: &[Form],
        available: impl Fn(&str) -> bool,
    ) -> Result<Self, String> {
        let mut excluded = HashSet::new();
        let mut overrides = HashMap::new();
        let mut requires = Vec::new();
        let mut uses = Vec::new();
        let mut excluded_foundation = HashSet::new();
        let mut exposed_foundation = None;
        let mut override_seen = false;
        let mut blank = false;
        let mut config_seen = false;
        let mut native_flavor = None;
        let mut native_flavor_imports = Vec::new();
        let mut native_imports = Vec::new();
        let mut role = "standard".to_owned();
        let mut global_alias = None;
        let mut declared_global_imports = Vec::new();

        for clause in clauses {
            let values = list(clause, "ns clauses must be non-empty lists")?;
            let head = values.first().ok_or("ns clauses must be non-empty lists")?;
            let name = keyword(head, "ns clause must start with a keyword")?;
            match name {
                "config" => {
                    if config_seen {
                        return Err("ns accepts only one :config clause".into());
                    }
                    config_seen = true;
                    if values.len() != 2 {
                        return Err(":config expects one map".into());
                    }
                    parse_config(
                        &values[1],
                        &mut blank,
                        &mut excluded_foundation,
                        &mut exposed_foundation,
                        &mut override_seen,
                        &mut excluded,
                        &mut overrides,
                        &mut role,
                        &mut global_alias,
                        &mut declared_global_imports,
                    )?;
                }
                "require" => requires.extend(values[1..].iter().cloned()),
                "use" => uses.extend(values[1..].iter().cloned()),
                "flavor" => {
                    if native_flavor.is_some() {
                        return Err("ns accepts only one :flavor clause".into());
                    }
                    let (flavor, imports) = parse_native_flavor(values)?;
                    native_flavor = Some(flavor);
                    native_flavor_imports = imports;
                }
                "import" => parse_native_imports(&values[1..], &mut native_imports)?,
                other => return Err(format!("Unsupported ns clause: :{other}")),
            }
        }

        if blank && override_seen {
            return Err(":config :blank true cannot be combined with :override".into());
        }
        if blank && exposed_foundation.is_some() {
            return Err(":config :blank true cannot be combined with :only".into());
        }
        if override_seen && exposed_foundation.is_some() {
            return Err(":config :override cannot be combined with :only".into());
        }

        for library in overrides.keys() {
            if excluded.contains(library) {
                return Err(format!(
                    "Foundation library cannot be both excluded and aliased: {library}"
                ));
            }
        }

        let mut config = Self::default();
        config.excluded_foundation_libraries = excluded.clone();
        config.excluded_foundation = excluded_foundation;
        config.exposed_foundation = exposed_foundation;
        config.global_alias = global_alias;
        config.declared_global_imports = declared_global_imports;
        config.native_flavor = native_flavor;
        config.native_imports = native_imports;
        config.native_flavor_imports = native_flavor_imports;
        config.role = role;
        config.blank = blank;
        for (library, namespace, _) in FOUNDATION_LIBRARIES {
            if excluded.contains(*library) {
                continue;
            }
            if let Some(alias) = overrides.get(*library) {
                config.put_alias(alias, namespace)?;
            }
        }
        for require in requires {
            config.apply_require(&require, &available)?;
        }
        for use_form in uses {
            config.apply_use(&use_form, &available)?;
        }
        Ok(config)
    }

    pub fn required_namespaces(&self) -> &[String] {
        &self.required_namespaces
    }

    pub fn lazy_target(&self, alias: &str) -> Option<&str> {
        self.lazy_aliases.get(alias).map(String::as_str)
    }

    pub fn used_namespaces(&self) -> &[String] {
        &self.used_namespaces
    }

    pub fn used_symbol_excluded(&self, namespace: &str, symbol: &str) -> bool {
        self.used_exclusions
            .get(namespace)
            .is_some_and(|excluded| excluded.contains(symbol))
    }

    pub fn internal_access(&self) -> &HashSet<String> {
        &self.internal_access
    }

    pub fn excluded_foundation(&self) -> &HashSet<String> {
        &self.excluded_foundation
    }

    pub fn excluded_foundation_libraries(&self) -> &HashSet<String> {
        &self.excluded_foundation_libraries
    }

    pub fn exposed_foundation(&self) -> Option<&HashSet<String>> {
        self.exposed_foundation.as_ref()
    }

    pub fn blank(&self) -> bool {
        self.blank
    }

    pub fn native_flavor(&self) -> Option<&str> {
        self.native_flavor.as_deref()
    }

    pub fn native_imports(&self) -> &[(String, String)] {
        &self.native_imports
    }

    pub fn native_flavor_imports(&self) -> &[(String, String)] {
        &self.native_flavor_imports
    }

    pub fn role(&self) -> &str {
        &self.role
    }

    pub fn aliases(&self) -> Vec<(String, String)> {
        self.aliases
            .iter()
            .map(|(alias, namespace)| (alias.clone(), namespace.clone()))
            .collect()
    }

    pub fn global_alias(&self) -> Option<&str> {
        self.global_alias.as_deref()
    }

    pub fn declared_global_imports(&self) -> &[String] {
        &self.declared_global_imports
    }

    pub fn set_global_aliases(&mut self, aliases: impl IntoIterator<Item = (String, String)>) {
        self.global_aliases = aliases.into_iter().collect();
    }

    fn put_alias(&mut self, alias: &str, namespace: &str) -> Result<(), String> {
        if alias.is_empty() {
            return Err("Namespace alias cannot be empty".into());
        }
        if alias == "-" {
            return Err("Namespace alias is reserved: -".into());
        }
        if let Some(previous) = self.aliases.get(alias) {
            if previous != namespace {
                return Err(format!(
                    "Namespace alias already refers to {previous}: {alias}"
                ));
            }
            return Ok(());
        }
        self.aliases.insert(alias.into(), namespace.into());
        Ok(())
    }

    pub fn apply_require(
        &mut self,
        form: &Form,
        available: &impl Fn(&str) -> bool,
    ) -> Result<(), String> {
        let (target, options) = match form {
            Form::Vector(items) => {
                let target = match items.first() {
                    Some(Form::Symbol(target)) => target.as_str(),
                    _ => return Err(":require namespace must be a symbol".into()),
                };
                (normalize_namespace(target), &items[1..])
            }
            Form::List(items)
                if items.len() == 2
                    && matches!(&items[0], Form::Symbol(q) if q == "quote")
                    && matches!(&items[1], Form::Symbol(_)) =>
            {
                let target = match &items[1] {
                    Form::Symbol(target) => target.as_str(),
                    _ => unreachable!(),
                };
                (normalize_namespace(target), &[][..])
            }
            _ => return Err(":require expects vectors such as [hara.lib.string :as str]".into()),
        };
        if !known_namespace(target) && !available(target) {
            return Err(format!(
                "Cannot require missing generated namespace: {target}"
            ));
        }
        if options.len() % 2 != 0 {
            return Err(format!("Malformed :require options for {target}"));
        }
        let lazy = options.chunks(2).any(|option| {
            matches!(&option[0], Form::Keyword(name) if name == "lazy")
                && matches!(&option[1], Form::Bool(true))
        });
        let has_alias = options
            .chunks(2)
            .any(|option| matches!(&option[0], Form::Keyword(name) if name == "as"));
        if lazy && !has_alias {
            return Err(":require :lazy requires :as".into());
        }
        if !lazy && !self.required_namespaces.iter().any(|value| value == target) {
            self.required_namespaces.push(target.into());
        }
        for option in options.chunks(2) {
            let name = keyword(&option[0], "Malformed :require options")?;
            match name {
                "as" => {
                    let alias = symbol(&option[1], ":require :as expects an unqualified symbol")?;
                    if alias.contains('/') {
                        return Err(":require :as expects an unqualified symbol".into());
                    }
                    self.put_alias(alias, target)?;
                    if lazy {
                        self.lazy_aliases.insert(alias.into(), target.into());
                    }
                }
                "refer" => {
                    if lazy {
                        return Err(":require :lazy cannot be combined with :refer".into());
                    }
                    if matches!(&option[1], Form::Keyword(name) if name == "all") {
                        if !self.used_namespaces.iter().any(|value| value == target) {
                            self.used_namespaces.push(target.into());
                        }
                        continue;
                    }
                    let names = vector(
                        &option[1],
                        ":require :refer expects a vector of symbols or :all",
                    )?;
                    for value in names {
                        let name = symbol(value, ":require :refer expects unqualified symbols")?;
                        if qualified_symbol(name) {
                            return Err(":require :refer expects unqualified symbols".into());
                        }
                        let canonical = canonical(target, name);
                        if let Some(previous) = self.refers.insert(name.into(), canonical) {
                            return Err(format!(
                                "Referred symbol already exists: {name} ({previous})"
                            ));
                        }
                    }
                }
                "refer-macros" => {
                    if lazy {
                        return Err(":require :lazy cannot be combined with :refer-macros".into());
                    }
                    let names = vector(
                        &option[1],
                        ":require :refer-macros expects a vector of symbols",
                    )?;
                    for value in names {
                        let name =
                            symbol(value, ":require :refer-macros expects unqualified symbols")?;
                        if qualified_symbol(name) {
                            return Err(":require :refer-macros expects unqualified symbols".into());
                        }
                        let canonical = canonical(target, name);
                        if let Some(previous) =
                            self.macro_refers.insert(name.into(), canonical.clone())
                        {
                            if previous != canonical {
                                return Err(format!(
                                    "Referred macro already exists: {name} ({previous})"
                                ));
                            }
                        }
                    }
                }
                "lazy" => {
                    if !matches!(&option[1], Form::Bool(true)) {
                        return Err(":require :lazy expects true".into());
                    }
                }
                "reload" => {
                    if !matches!(&option[1], Form::Bool(true)) {
                        return Err(":require :reload expects true".into());
                    }
                }
                "access" => {
                    if !matches!(&option[1], Form::Bool(true)) {
                        return Err(":require :access expects true".into());
                    }
                    self.internal_access.insert(target.into());
                }
                "exclude" => {
                    let names =
                        vector(&option[1], ":require :exclude expects a vector of symbols")?;
                    for value in names {
                        let name = symbol(value, ":require :exclude expects unqualified symbols")?;
                        if qualified_symbol(name) {
                            return Err(":require :exclude expects unqualified symbols".into());
                        }
                        self.used_exclusions
                            .entry(target.into())
                            .or_default()
                            .insert(name.into());
                        if target == "std.foundation" {
                            self.excluded_foundation.insert(name.into());
                        }
                    }
                }
                other => return Err(format!("Unsupported :require option: :{other}")),
            }
        }
        Ok(())
    }

    pub fn apply_use(
        &mut self,
        form: &Form,
        available: &impl Fn(&str) -> bool,
    ) -> Result<(), String> {
        let target = match form {
            Form::Symbol(target) if !target.contains('/') => normalize_namespace(target),
            _ => return Err(":use expects unqualified namespace symbols".into()),
        };
        if !known_namespace(target) && !available(target) {
            return Err(format!("Cannot use missing generated namespace: {target}"));
        }
        if !self.required_namespaces.iter().any(|value| value == target) {
            self.required_namespaces.push(target.into());
        }
        if !self.used_namespaces.iter().any(|value| value == target) {
            self.used_namespaces.push(target.into());
        }
        Ok(())
    }
}

fn parse_native_flavor(values: &[Form]) -> Result<(String, Vec<(String, String)>), String> {
    let flavor = match values.get(1) {
        Some(Form::Keyword(flavor)) if !flavor.contains('/') && flavor != "wasm" => flavor,
        Some(Form::Keyword(flavor)) if flavor == "wasm" => {
            return Err("native/unsupported-flavor: :wasm (Wasm modules use :import)".into())
        }
        Some(Form::Keyword(flavor)) => return Err(format!("native/invalid-flavor: :{flavor}")),
        _ => return Err(":flavor expects an unqualified host keyword".into()),
    };
    Err(format!(
        "native/unsupported-flavor: :{flavor} (host flavors are only available on JVM/.NET runtimes)"
    ))
}

fn parse_native_imports(
    specifications: &[Form],
    imports: &mut Vec<(String, String)>,
) -> Result<(), String> {
    for specification in specifications {
        match specification {
            Form::Symbol(module) if !module.contains('/') => {
                imports.push((module.clone(), module.clone()));
            }
            Form::Vector(values) if !values.is_empty() => {
                let package = match &values[0] {
                    Form::Symbol(package) if !package.contains('/') => package,
                    _ => return Err(":import package must be a symbol".into()),
                };
                if values.len() == 1 {
                    return Err(":import package vector requires at least one module".into());
                }
                for module in &values[1..] {
                    let module = match module {
                        Form::Symbol(module) if !module.contains('/') && !module.contains('.') => {
                            module
                        }
                        _ => return Err(":import module must be an unqualified symbol".into()),
                    };
                    imports.push((module.clone(), format!("{package}.{module}")));
                }
            }
            _ => return Err(":import expects module symbols or package vectors".into()),
        }
    }
    Ok(())
}

fn parse_config(
    form: &Form,
    blank: &mut bool,
    foundation_overrides: &mut HashSet<String>,
    foundation_exposure: &mut Option<HashSet<String>>,
    override_seen: &mut bool,
    excluded: &mut HashSet<String>,
    overrides: &mut HashMap<String, String>,
    role: &mut String,
    global_alias: &mut Option<String>,
    declared_global_imports: &mut Vec<String>,
) -> Result<(), String> {
    let options = match form {
        Form::Map(options) => options,
        _ => return Err(":config expects one map".into()),
    };
    for (key, value) in options {
        match keyword(key, ":config keys must be unqualified keywords")? {
            "blank" => {
                *blank = match value {
                    Form::Bool(value) => *value,
                    _ => return Err(":config :blank expects a boolean".into()),
                };
            }
            "override" => {
                *override_seen = true;
                for item in vector(
                    value,
                    ":config :override expects a vector of unqualified symbols",
                )? {
                    let name = symbol(
                        item,
                        ":config :override expects a vector of unqualified symbols",
                    )?;
                    if qualified_symbol(name) {
                        return Err(
                            ":config :override expects a vector of unqualified symbols".into()
                        );
                    }
                    if !foundation_overrides.insert(name.into()) {
                        return Err(format!("Duplicate Foundation override: {name}"));
                    }
                }
            }
            "only" => {
                let mut exposed = HashSet::new();
                for item in vector(
                    value,
                    ":config :only expects a vector of unqualified symbols",
                )? {
                    let name = symbol(
                        item,
                        ":config :only expects a vector of unqualified symbols",
                    )?;
                    if qualified_symbol(name) {
                        return Err(
                            ":config :only expects a vector of unqualified symbols".into()
                        );
                    }
                    if !exposed.insert(name.into()) {
                        return Err(format!("Duplicate Foundation selection: {name}"));
                    }
                }
                *foundation_exposure = Some(exposed);
            }
            "rename" => {
                parse_rename(value, excluded, overrides)?;
            }
            "role" => {
                let value = keyword(
                    value,
                    ":config :role expects :default, :internal, or :facade",
                )?;
                if !matches!(value, "default" | "internal" | "facade") {
                    return Err(":config :role expects :default, :internal, or :facade".into());
                }
                *role = if value == "default" {
                    "standard".to_owned()
                } else {
                    value.to_owned()
                };
            }
            "set-global-alias" => {
                let value = symbol(
                    value,
                    ":config :set-global-alias expects an unqualified symbol",
                )?;
                if qualified_symbol(value) {
                    return Err(":config :set-global-alias expects an unqualified symbol".into());
                }
                if value == "-" {
                    return Err(":config :set-global-alias is reserved: -".into());
                }
                *global_alias = Some(value.to_owned());
            }
            "set-global" => {
                for item in vector(
                    value,
                    ":config :set-global expects a vector of qualified Vars",
                )? {
                    let name = symbol(
                        item,
                        ":config :set-global expects a vector of qualified Vars",
                    )?;
                    if !qualified_symbol(name) {
                        return Err(":config :set-global expects qualified Vars".into());
                    }
                    if declared_global_imports.iter().any(|value| value == name) {
                        return Err(format!("Duplicate global import: {name}"));
                    }
                    declared_global_imports.push(name.into());
                }
            }
            other => return Err(format!("Unsupported :config option: :{other}")),
        }
    }
    Ok(())
}

fn parse_rename(
    form: &Form,
    excluded: &mut HashSet<String>,
    overrides: &mut HashMap<String, String>,
) -> Result<(), String> {
    if matches!(form, Form::Keyword(name) if name == "all") {
        return Ok(());
    }
    let options = match form {
        Form::Map(options) => options,
        _ => return Err(":rename expects :all or an options map".into()),
    };
    for (key, value) in options {
        match keyword(key, ":rename option keys must be keywords")? {
            "exclude" => {
                for item in vector(
                    value,
                    ":rename :exclude expects a vector of library symbols",
                )? {
                    let library = library(symbol(
                        item,
                        ":rename :exclude expects unqualified library symbols",
                    )?)?;
                    if !excluded.insert(library.into()) {
                        return Err(format!("Duplicate Foundation library exclusion: {library}"));
                    }
                }
            }
            "alias" => {
                let aliases = match value {
                    Form::Map(aliases) => aliases,
                    _ => return Err(":rename :alias expects a map".into()),
                };
                for (library_form, alias_form) in aliases {
                    let library = library(symbol(
                        library_form,
                        ":rename :alias expects library symbols",
                    )?)?;
                    let alias =
                        symbol(alias_form, "Foundation library aliases must be unqualified symbols")?;
                    if alias.contains('/') {
                        return Err("Foundation library aliases must be unqualified symbols".into());
                    }
                    if overrides.insert(library.into(), alias.into()).is_some() {
                        return Err(format!("Duplicate Foundation library alias: {library}"));
                    }
                }
            }
            other => return Err(format!("Unsupported :config :rename option: :{other}")),
        }
    }
    Ok(())
}

fn list<'a>(form: &'a Form, error: &str) -> Result<&'a [Form], String> {
    match form {
        Form::List(values) => Ok(values),
        _ => Err(error.into()),
    }
}
fn vector<'a>(form: &'a Form, error: &str) -> Result<&'a [Form], String> {
    match form {
        Form::Vector(values) => Ok(values),
        _ => Err(error.into()),
    }
}
fn keyword<'a>(form: &'a Form, error: &str) -> Result<&'a str, String> {
    match form {
        Form::Keyword(value) => Ok(value),
        _ => Err(error.into()),
    }
}
fn symbol<'a>(form: &'a Form, error: &str) -> Result<&'a str, String> {
    match form {
        Form::Symbol(value) => Ok(value),
        _ => Err(error.into()),
    }
}
fn qualified_symbol(value: &str) -> bool {
    value != "/" && value.contains('/')
}
fn library(value: &str) -> Result<&str, String> {
    if value.contains('/') {
        return Err("Foundation library names must be unqualified symbols".into());
    }
    FOUNDATION_LIBRARIES
        .iter()
        .find(|(library, _, _)| *library == value)
        .map(|(library, _, _)| *library)
        .ok_or_else(|| format!("Unknown Foundation library: {value}"))
}
pub(crate) fn normalize_namespace(value: &str) -> &str {
    match value {
        "core" | "hara.lib.core" => "std.foundation",
        "hara.lib.string" => "std.foundation.string",
        "hara.lib.promise" => "std.foundation.promise",
        "hara.lib.bytes" => "std.foundation.bytes",
        "hara.lib.socket" => "std.native.Socket",
        "hara.lib.file" => "std.native.File",
        value => value,
    }
}
pub(crate) fn known_namespace(value: &str) -> bool {
    let value = normalize_namespace(value);
    value == "std.foundation"
        || value == "std.foundation.coroutine"
        || value == "std.native"
        || value.starts_with("std.native.")
        || FOUNDATION_LIBRARIES
            .iter()
            .any(|(_, namespace, _)| *namespace == value)
}
fn canonical(namespace: &str, method: &str) -> String {
    let namespace = normalize_namespace(namespace);
    if namespace.starts_with("std.native.") {
        return format!("{namespace}/{method}");
    }
    if namespace == "std.foundation" {
        return format!("std.foundation/{method}");
    }
    // Coroutine operations are evaluator control forms, not ordinary HAL
    // function calls. Keep their canonical names so `co/yield` and
    // `co/await` remain visible to the fiber evaluator instead of routing
    // through the synchronous Foundation wrapper namespace.
    if namespace == "std.foundation.coroutine" {
        return format!("std.foundation.coroutine/{method}");
    }
    if FOUNDATION_LIBRARIES
        .iter()
        .any(|(_, library_namespace, _)| *library_namespace == namespace)
    {
        return format!("{namespace}/{method}");
    }
    match (namespace, method) {
        ("std.foundation", method) => method.into(),
        ("std.lib.string", method) => format!("str/{method}"),
        ("std.lib.promise", "then") => "promise/then".into(),
        ("std.lib.promise", "catch") => "promise/catch".into(),
        ("std.lib.promise", method) => format!("promise/{method}"),
        ("std.lib.bytes", method) => format!("bytes/{method}"),
        ("std.lib.socket", method) => format!("socket/{method}"),
        ("std.lib.file", method) => format!("file/{method}"),
        (namespace, method) => format!("{namespace}/{method}"),
    }
}

#[cfg(test)]
#[path = "generated/tests.rs"]
mod tests;
