use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::kernel::{Var, VarMetadata, VarOrigin};
use crate::lang::data::Symbol;
use crate::lang::protocol::INamespaced;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamespaceLoadState {
    Unloaded,
    Loading,
    Loaded,
    Failed,
}

impl NamespaceLoadState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unloaded => "unloaded",
            Self::Loading => "loading",
            Self::Loaded => "loaded",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Namespace<V> {
    name: Symbol,
    mappings: Rc<RefCell<HashMap<Symbol, Var<V>>>>,
    aliases: Rc<RefCell<HashMap<Symbol, Namespace<V>>>>,
    lazy_aliases: Rc<RefCell<HashMap<Symbol, Symbol>>>,
    imports: Rc<RefCell<HashMap<Symbol, String>>>,
    native_flavor: Rc<RefCell<Option<String>>>,
    role: Rc<RefCell<String>>,
    foundation_exposed: Rc<RefCell<Option<HashSet<String>>>>,
    foundation_excluded: Rc<RefCell<HashSet<String>>>,
}
impl<V> Namespace<V> {
    pub fn new(name: impl AsRef<str>) -> Self {
        Self {
            name: Symbol::parse(name.as_ref()),
            mappings: Rc::new(RefCell::new(HashMap::new())),
            aliases: Rc::new(RefCell::new(HashMap::new())),
            lazy_aliases: Rc::new(RefCell::new(HashMap::new())),
            imports: Rc::new(RefCell::new(HashMap::new())),
            native_flavor: Rc::new(RefCell::new(None)),
            role: Rc::new(RefCell::new("standard".into())),
            foundation_exposed: Rc::new(RefCell::new(None)),
            foundation_excluded: Rc::new(RefCell::new(HashSet::new())),
        }
    }
    pub fn name(&self) -> &Symbol {
        &self.name
    }
    pub fn same_identity(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.mappings, &other.mappings)
    }
    pub fn identity_address(&self) -> usize {
        Rc::as_ptr(&self.mappings) as usize
    }
    pub fn intern(&self, name: impl AsRef<str>, value: V) -> Var<V>
    where
        V: Clone + 'static,
    {
        let local = Symbol::create(None, name.as_ref());
        if let Some(existing) = self.mappings.borrow().get(&local).cloned() {
            existing.reset_value(value);
            return existing;
        }
        let path = format!("{}/{}", self.name.as_str(), local.as_str());
        let var = Var::new(path, value);
        self.mappings.borrow_mut().insert(local, var.clone());
        var
    }
    pub fn intern_with_metadata(
        &self,
        name: impl AsRef<str>,
        value: V,
        metadata: VarMetadata,
    ) -> Var<V>
    where
        V: Clone + 'static,
    {
        let local = Symbol::create(None, name.as_ref());
        if let Some(existing) = self.mappings.borrow().get(&local).cloned() {
            existing.reset_value(value);
            existing.set_metadata(metadata);
            return existing;
        }
        let path = format!("{}/{}", self.name.as_str(), local.as_str());
        let var = Var::with_metadata(path, value, metadata);
        self.mappings.borrow_mut().insert(local, var.clone());
        var
    }
    pub fn intern_with_origin(&self, name: impl AsRef<str>, value: V, origin: VarOrigin) -> Var<V>
    where
        V: Clone + 'static,
    {
        self.intern_with_metadata(
            name,
            value,
            VarMetadata {
                origin,
                ..VarMetadata::default()
            },
        )
    }
    pub fn map_var(&self, symbol: Symbol, var: Var<V>) {
        self.mappings.borrow_mut().insert(symbol, var);
    }
    pub fn unmap(&self, symbol: &Symbol) -> Option<Var<V>> {
        self.mappings.borrow_mut().remove(symbol)
    }
    pub fn resolve(&self, symbol: &Symbol) -> Option<Var<V>>
    where
        V: Clone,
    {
        if let Some(namespace) = symbol.get_namespace() {
            if namespace == "-" {
                return self
                    .mappings
                    .borrow()
                    .get(&Symbol::create(None, symbol.get_name()))
                    .cloned();
            }
            let alias = Symbol::parse(namespace);
            return self.aliases.borrow().get(&alias).and_then(|ns| {
                ns.mappings
                    .borrow()
                    .get(&Symbol::create(None, symbol.get_name()))
                    .cloned()
            });
        }
        self.mappings.borrow().get(symbol).cloned()
    }
    pub fn unalias(&self, alias: impl AsRef<str>) -> Option<Namespace<V>> {
        let alias = Symbol::parse(alias.as_ref());
        self.lazy_aliases.borrow_mut().remove(&alias);
        self.aliases.borrow_mut().remove(&alias)
    }
    pub fn alias(&self, alias: impl AsRef<str>, namespace: Namespace<V>) {
        let alias = Symbol::parse(alias.as_ref());
        self.lazy_aliases.borrow_mut().remove(&alias);
        self.aliases.borrow_mut().insert(alias, namespace);
    }
    pub fn lazy_alias(&self, alias: impl AsRef<str>, target: impl AsRef<str>) {
        let alias = Symbol::parse(alias.as_ref());
        self.aliases.borrow_mut().remove(&alias);
        self.lazy_aliases
            .borrow_mut()
            .insert(alias, Symbol::parse(target.as_ref()));
    }
    pub fn lazy_target(&self, alias: impl AsRef<str>) -> Option<Symbol> {
        self.lazy_aliases
            .borrow()
            .get(&Symbol::parse(alias.as_ref()))
            .cloned()
    }
    pub fn lazy_aliases(&self) -> Vec<(Symbol, Symbol)> {
        self.lazy_aliases
            .borrow()
            .iter()
            .map(|(alias, target)| (alias.clone(), target.clone()))
            .collect()
    }
    pub fn import(&self, name: impl AsRef<str>, host_type: impl Into<String>) {
        self.imports
            .borrow_mut()
            .insert(Symbol::parse(name.as_ref()), host_type.into());
    }
    pub fn imported(&self, name: &Symbol) -> Option<String> {
        self.imports.borrow().get(name).cloned()
    }
    pub fn set_native_flavor(&self, flavor: Option<String>) {
        *self.native_flavor.borrow_mut() = flavor;
    }
    pub fn native_flavor(&self) -> Option<String> {
        self.native_flavor.borrow().clone()
    }
    pub fn set_role(&self, role: impl Into<String>) {
        *self.role.borrow_mut() = role.into();
    }
    pub fn set_foundation_visibility(
        &self,
        exposed: Option<&HashSet<String>>,
        excluded: &HashSet<String>,
        blank: bool,
    ) {
        *self.foundation_exposed.borrow_mut() = if blank {
            Some(HashSet::new())
        } else {
            exposed.cloned()
        };
        *self.foundation_excluded.borrow_mut() = excluded.clone();
    }
    pub(crate) fn foundation_visible(&self, name: &Symbol) -> bool {
        if self.foundation_excluded.borrow().contains(name.as_str()) {
            return false;
        }
        self.foundation_exposed
            .borrow()
            .as_ref()
            .is_none_or(|exposed| exposed.contains(name.as_str()))
    }
    pub fn role(&self) -> String {
        self.role.borrow().clone()
    }
    pub fn mappings(&self) -> Vec<(Symbol, Var<V>)>
    where
        V: Clone,
    {
        self.mappings
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
    pub fn aliases(&self) -> Vec<(Symbol, Namespace<V>)>
    where
        V: Clone,
    {
        self.aliases
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
    pub fn imports(&self) -> Vec<(Symbol, String)> {
        self.imports
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct NamespaceRegistry<V> {
    namespaces: Rc<RefCell<HashMap<Symbol, Namespace<V>>>>,
    current: Rc<RefCell<Symbol>>,
    loading_states: Rc<RefCell<HashMap<Symbol, NamespaceLoadState>>>,
    load_failures: Rc<RefCell<HashMap<Symbol, String>>>,
    global_aliases: Rc<RefCell<HashMap<Symbol, Symbol>>>,
    global_imports: Rc<RefCell<HashMap<Symbol, Symbol>>>,
    module_revisions: Rc<RefCell<HashMap<Symbol, u64>>>,
    module_dependencies: Rc<RefCell<HashMap<Symbol, Vec<Symbol>>>>,
}

pub struct NamespaceRegistrySnapshot<V> {
    namespaces: HashMap<Symbol, NamespaceSnapshot<V>>,
    current: Symbol,
    loading_states: HashMap<Symbol, NamespaceLoadState>,
    load_failures: HashMap<Symbol, String>,
    global_aliases: HashMap<Symbol, Symbol>,
    global_imports: HashMap<Symbol, Symbol>,
    module_revisions: HashMap<Symbol, u64>,
    module_dependencies: HashMap<Symbol, Vec<Symbol>>,
}

pub struct NamespaceTransactionSnapshot<V> {
    namespace_names: HashSet<Symbol>,
    namespaces: HashMap<Symbol, NamespaceSnapshot<V>>,
    current: Symbol,
    loading_states: HashMap<Symbol, NamespaceLoadState>,
    load_failures: HashMap<Symbol, String>,
    global_aliases: HashMap<Symbol, Symbol>,
    global_imports: HashMap<Symbol, Symbol>,
    module_revisions: HashMap<Symbol, u64>,
    module_dependencies: HashMap<Symbol, Vec<Symbol>>,
}

struct NamespaceSnapshot<V> {
    namespace: Namespace<V>,
    mappings: HashMap<Symbol, (Var<V>, V, VarMetadata)>,
    aliases: HashMap<Symbol, Namespace<V>>,
    lazy_aliases: HashMap<Symbol, Symbol>,
    imports: HashMap<Symbol, String>,
    native_flavor: Option<String>,
    role: String,
    foundation_exposed: Option<HashSet<String>>,
    foundation_excluded: HashSet<String>,
}
impl<V: Clone> Default for NamespaceRegistry<V> {
    fn default() -> Self {
        Self::new("user")
    }
}
impl<V: Clone> NamespaceRegistry<V> {
    fn namespace_snapshot(namespace: &Namespace<V>) -> NamespaceSnapshot<V>
    where
        V: 'static,
    {
        let mappings = namespace
            .mappings
            .borrow()
            .iter()
            .map(|(name, var)| {
                (
                    name.clone(),
                    (var.clone(), var.deref_value(), var.metadata()),
                )
            })
            .collect();
        NamespaceSnapshot {
            namespace: namespace.clone(),
            mappings,
            aliases: namespace.aliases.borrow().clone(),
            lazy_aliases: namespace.lazy_aliases.borrow().clone(),
            imports: namespace.imports.borrow().clone(),
            native_flavor: namespace.native_flavor.borrow().clone(),
            role: namespace.role.borrow().clone(),
            foundation_exposed: namespace.foundation_exposed.borrow().clone(),
            foundation_excluded: namespace.foundation_excluded.borrow().clone(),
        }
    }

    pub fn new(initial: impl AsRef<str>) -> Self {
        let name = Symbol::parse(initial.as_ref());
        let namespace = Namespace::new(name.as_str());
        let mut namespaces = HashMap::new();
        namespaces.insert(name.clone(), namespace);
        let mut loading_states = HashMap::new();
        loading_states.insert(name.clone(), NamespaceLoadState::Loaded);
        Self {
            namespaces: Rc::new(RefCell::new(namespaces)),
            current: Rc::new(RefCell::new(name)),
            loading_states: Rc::new(RefCell::new(loading_states)),
            load_failures: Rc::new(RefCell::new(HashMap::new())),
            global_aliases: Rc::new(RefCell::new(HashMap::new())),
            global_imports: Rc::new(RefCell::new(HashMap::new())),
            module_revisions: Rc::new(RefCell::new(HashMap::new())),
            module_dependencies: Rc::new(RefCell::new(HashMap::new())),
        }
    }
    pub fn current(&self) -> Namespace<V> {
        self.namespaces
            .borrow()
            .get(&*self.current.borrow())
            .cloned()
            .expect("current namespace exists")
    }
    pub fn find(&self, name: impl AsRef<str>) -> Option<Namespace<V>> {
        self.namespaces
            .borrow()
            .get(&Symbol::parse(name.as_ref()))
            .cloned()
    }
    pub fn find_or_create(&self, name: impl AsRef<str>) -> Namespace<V> {
        let symbol = Symbol::parse(name.as_ref());
        if let Some(namespace) = self.namespaces.borrow().get(&symbol).cloned() {
            return namespace;
        }
        let namespace = Namespace::new(symbol.as_str());
        self.namespaces
            .borrow_mut()
            .insert(symbol.clone(), namespace.clone());
        self.loading_states
            .borrow_mut()
            .entry(symbol)
            .or_insert(NamespaceLoadState::Loaded);
        namespace
    }
    pub fn set_current(&self, name: impl AsRef<str>) -> Namespace<V> {
        let namespace = self.find_or_create(name);
        *self.current.borrow_mut() = namespace.name().clone();
        namespace
    }
    pub fn all(&self) -> Vec<Namespace<V>> {
        self.namespaces.borrow().values().cloned().collect()
    }
    /// Returns every namespace known either as a materialized namespace or as
    /// a discoverable module with load state.  Catalog-backed modules can
    /// therefore be inspected before a Namespace value exists.
    pub fn known_names(&self) -> Vec<Symbol> {
        let mut names = self
            .namespaces
            .borrow()
            .keys()
            .chain(self.loading_states.borrow().keys())
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        names.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        names
    }
    pub fn load_state(&self, name: impl AsRef<str>) -> Option<NamespaceLoadState> {
        self.loading_states
            .borrow()
            .get(&Symbol::parse(name.as_ref()))
            .copied()
    }
    pub fn set_load_state(&self, name: impl AsRef<str>, state: NamespaceLoadState) {
        self.loading_states
            .borrow_mut()
            .insert(Symbol::parse(name.as_ref()), state);
    }
    pub fn clear_load_state(&self, name: impl AsRef<str>) {
        self.loading_states
            .borrow_mut()
            .remove(&Symbol::parse(name.as_ref()));
    }
    pub fn load_failure(&self, name: impl AsRef<str>) -> Option<String> {
        self.load_failures
            .borrow()
            .get(&Symbol::parse(name.as_ref()))
            .cloned()
    }
    pub fn set_load_failure(&self, name: impl AsRef<str>, detail: impl Into<String>) {
        self.load_failures
            .borrow_mut()
            .insert(Symbol::parse(name.as_ref()), detail.into());
    }
    pub fn clear_load_failure(&self, name: impl AsRef<str>) {
        self.load_failures
            .borrow_mut()
            .remove(&Symbol::parse(name.as_ref()));
    }
    pub fn register_global_alias(
        &self,
        alias: impl AsRef<str>,
        namespace: impl AsRef<str>,
    ) -> Result<(), String> {
        let alias = Symbol::parse(alias.as_ref());
        let namespace = Symbol::parse(namespace.as_ref());
        if alias.get_namespace().is_some() || alias.as_str() == "-" {
            return Err(format!("Invalid global namespace alias: {alias}"));
        }
        if let Some(previous) = self.global_aliases.borrow().get(&alias) {
            if previous != &namespace {
                return Err(format!(
                    "Global namespace alias already refers to {previous}: {alias}"
                ));
            }
            return Ok(());
        }
        self.global_aliases.borrow_mut().insert(alias, namespace);
        Ok(())
    }
    pub fn global_aliases(&self) -> Vec<(Symbol, Symbol)> {
        self.global_aliases
            .borrow()
            .iter()
            .map(|(alias, namespace)| (alias.clone(), namespace.clone()))
            .collect()
    }
    pub fn register_global_import(
        &self,
        shorthand: impl AsRef<str>,
        canonical: impl AsRef<str>,
    ) -> Result<(), String> {
        let shorthand = Symbol::parse(shorthand.as_ref());
        let canonical = Symbol::parse(canonical.as_ref());
        if shorthand.get_namespace().is_none() {
            return Err(format!("Invalid global import Var: {shorthand}"));
        }
        let local = Symbol::create(None, shorthand.get_name());
        if let Some(previous) = self.global_imports.borrow().get(&local) {
            if previous != &canonical {
                return Err(format!(
                    "Global import already refers to {previous}: {}",
                    local
                ));
            }
            return Ok(());
        }
        self.global_imports
            .borrow_mut()
            .insert(local, canonical);
        Ok(())
    }
    pub fn global_imports(&self) -> Vec<(Symbol, Symbol)> {
        self.global_imports
            .borrow()
            .iter()
            .map(|(shorthand, canonical)| (shorthand.clone(), canonical.clone()))
            .collect()
    }
    pub fn module_revision(&self, name: impl AsRef<str>) -> u64 {
        self.module_revisions
            .borrow()
            .get(&Symbol::parse(name.as_ref()))
            .copied()
            .unwrap_or(0)
    }
    pub fn commit_module_revision(&self, name: impl AsRef<str>) -> u64 {
        let name = Symbol::parse(name.as_ref());
        let next = self.module_revision(name.as_str()) + 1;
        self.module_revisions.borrow_mut().insert(name, next);
        next
    }
    pub fn module_dependencies(&self, name: impl AsRef<str>) -> Vec<Symbol> {
        self.module_dependencies
            .borrow()
            .get(&Symbol::parse(name.as_ref()))
            .cloned()
            .unwrap_or_default()
    }
    pub fn clear_module_dependencies(&self, name: impl AsRef<str>) {
        self.module_dependencies
            .borrow_mut()
            .insert(Symbol::parse(name.as_ref()), Vec::new());
    }
    pub fn record_module_dependency(&self, module: impl AsRef<str>, dependency: impl AsRef<str>) {
        let module = Symbol::parse(module.as_ref());
        let dependency = Symbol::parse(dependency.as_ref());
        let mut dependencies = self.module_dependencies.borrow_mut();
        let values = dependencies.entry(module).or_default();
        if !values.contains(&dependency) {
            values.push(dependency);
        }
    }
    pub fn snapshot(&self) -> NamespaceRegistrySnapshot<V>
    where
        V: 'static,
    {
        let namespaces = self
            .namespaces
            .borrow()
            .iter()
            .map(|(name, namespace)| (name.clone(), Self::namespace_snapshot(namespace)))
            .collect();
        NamespaceRegistrySnapshot {
            namespaces,
            current: self.current.borrow().clone(),
            loading_states: self.loading_states.borrow().clone(),
            load_failures: self.load_failures.borrow().clone(),
            global_aliases: self.global_aliases.borrow().clone(),
            global_imports: self.global_imports.borrow().clone(),
            module_revisions: self.module_revisions.borrow().clone(),
            module_dependencies: self.module_dependencies.borrow().clone(),
        }
    }

    pub fn transaction_snapshot<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> NamespaceTransactionSnapshot<V>
    where
        V: 'static,
    {
        let selected = names.into_iter().map(Symbol::parse).collect::<HashSet<_>>();
        let namespaces = self.namespaces.borrow();
        NamespaceTransactionSnapshot {
            namespace_names: namespaces.keys().cloned().collect(),
            namespaces: namespaces
                .iter()
                .filter(|(name, _)| selected.contains(*name))
                .map(|(name, namespace)| (name.clone(), Self::namespace_snapshot(namespace)))
                .collect(),
            current: self.current.borrow().clone(),
            loading_states: self.loading_states.borrow().clone(),
            load_failures: self.load_failures.borrow().clone(),
            global_aliases: self.global_aliases.borrow().clone(),
            global_imports: self.global_imports.borrow().clone(),
            module_revisions: self.module_revisions.borrow().clone(),
            module_dependencies: self.module_dependencies.borrow().clone(),
        }
    }
    pub fn restore(&self, snapshot: NamespaceRegistrySnapshot<V>)
    where
        V: 'static,
    {
        let mut namespaces = HashMap::new();
        for (name, saved) in snapshot.namespaces {
            let namespace = saved.namespace;
            let mut mappings = HashMap::new();
            for (local, (var, value, metadata)) in saved.mappings {
                var.reset_value(value);
                var.set_metadata(metadata);
                mappings.insert(local, var);
            }
            *namespace.mappings.borrow_mut() = mappings;
            *namespace.aliases.borrow_mut() = saved.aliases;
            *namespace.lazy_aliases.borrow_mut() = saved.lazy_aliases;
            *namespace.imports.borrow_mut() = saved.imports;
            *namespace.native_flavor.borrow_mut() = saved.native_flavor;
            *namespace.role.borrow_mut() = saved.role;
            *namespace.foundation_exposed.borrow_mut() = saved.foundation_exposed;
            *namespace.foundation_excluded.borrow_mut() = saved.foundation_excluded;
            namespaces.insert(name, namespace);
        }
        *self.namespaces.borrow_mut() = namespaces;
        *self.current.borrow_mut() = snapshot.current;
        *self.loading_states.borrow_mut() = snapshot.loading_states;
        *self.load_failures.borrow_mut() = snapshot.load_failures;
        *self.global_aliases.borrow_mut() = snapshot.global_aliases;
        *self.global_imports.borrow_mut() = snapshot.global_imports;
        *self.module_revisions.borrow_mut() = snapshot.module_revisions;
        *self.module_dependencies.borrow_mut() = snapshot.module_dependencies;
    }

    pub fn restore_transaction(&self, snapshot: NamespaceTransactionSnapshot<V>)
    where
        V: 'static,
    {
        let mut namespaces = self.namespaces.borrow_mut();
        namespaces.retain(|name, _| snapshot.namespace_names.contains(name));
        for (name, saved) in snapshot.namespaces {
            let namespace = saved.namespace;
            let mut mappings = HashMap::new();
            for (local, (var, value, metadata)) in saved.mappings {
                var.reset_value(value);
                var.set_metadata(metadata);
                mappings.insert(local, var);
            }
            *namespace.mappings.borrow_mut() = mappings;
            *namespace.aliases.borrow_mut() = saved.aliases;
            *namespace.lazy_aliases.borrow_mut() = saved.lazy_aliases;
            *namespace.imports.borrow_mut() = saved.imports;
            *namespace.native_flavor.borrow_mut() = saved.native_flavor;
            *namespace.role.borrow_mut() = saved.role;
            *namespace.foundation_exposed.borrow_mut() = saved.foundation_exposed;
            *namespace.foundation_excluded.borrow_mut() = saved.foundation_excluded;
            namespaces.insert(name, namespace);
        }
        drop(namespaces);
        *self.current.borrow_mut() = snapshot.current;
        *self.loading_states.borrow_mut() = snapshot.loading_states;
        *self.load_failures.borrow_mut() = snapshot.load_failures;
        *self.global_aliases.borrow_mut() = snapshot.global_aliases;
        *self.global_imports.borrow_mut() = snapshot.global_imports;
        *self.module_revisions.borrow_mut() = snapshot.module_revisions;
        *self.module_dependencies.borrow_mut() = snapshot.module_dependencies;
    }
    pub fn remove(&self, name: impl AsRef<str>) -> Option<Namespace<V>> {
        let symbol = Symbol::parse(name.as_ref());
        if symbol == *self.current.borrow() {
            return None;
        }
        self.loading_states.borrow_mut().remove(&symbol);
        self.load_failures.borrow_mut().remove(&symbol);
        self.module_revisions.borrow_mut().remove(&symbol);
        self.module_dependencies.borrow_mut().remove(&symbol);
        self.global_aliases
            .borrow_mut()
            .retain(|_, namespace| namespace != &symbol);
        self.namespaces.borrow_mut().remove(&symbol)
    }
    pub fn resolve(&self, symbol: &Symbol) -> Option<Var<V>>
    where
        V: Clone,
    {
        if let Some(namespace_name) = symbol.get_namespace() {
            let local = Symbol::create(None, symbol.get_name());
            if namespace_name == "-" {
                return self.current().mappings.borrow().get(&local).cloned();
            }
            if let Some(namespace) = self.find(namespace_name) {
                return namespace.mappings.borrow().get(&local).cloned();
            }
            if let Some(namespace_name) = self
                .global_aliases
                .borrow()
                .get(&Symbol::parse(namespace_name))
            {
                if let Some(namespace) = self.find(namespace_name.as_str()) {
                    return namespace.mappings.borrow().get(&local).cloned();
                }
            }
            return self
                .current()
                .aliases
                .borrow()
                .get(&Symbol::parse(namespace_name))
                .and_then(|namespace| namespace.mappings.borrow().get(&local).cloned());
        }
        let current = self.current();
        current
            .resolve(symbol)
            .or_else(|| {
                self.global_imports
                    .borrow()
                    .get(symbol)
                    .cloned()
                    .and_then(|canonical| self.resolve(&canonical))
            })
            .or_else(|| {
                self.find("std.foundation")
                    .filter(|_| current.foundation_visible(symbol))
                    .and_then(|foundation| foundation.resolve(symbol))
            })
            .or_else(|| {
                let name = symbol.as_str();
                (name.starts_with("std.native.") || name.starts_with("std.protocol.")).then(
                    || {
                        self.namespaces.borrow().values().find_map(|namespace| {
                            namespace
                                .mappings
                                .borrow()
                                .values()
                                .find(|var| var.symbol().as_str() == name)
                                .cloned()
                        })
                    },
                )?
            })
    }
    pub fn set_var(&self, symbol: Symbol, var: Var<V>) -> Result<Var<V>, String>
    where
        V: Clone,
    {
        let namespace = match symbol.get_namespace() {
            Some(name) => self
                .find(name)
                .ok_or_else(|| format!("Namespace not found: {name}"))?,
            None => self.current(),
        };
        namespace.map_var(Symbol::create(None, symbol.get_name()), var.clone());
        Ok(var)
    }
    pub fn visible_symbol_names(&self) -> Vec<String> {
        let current = self.current();
        let mut names = current
            .mappings
            .borrow()
            .iter()
            .map(|(name, var)| {
                (
                    name.as_str().to_owned(),
                    var.hara_metadata()
                        .is_some_and(|metadata| metadata.flag("public")),
                )
            })
            .collect::<Vec<_>>();
        if current.name().as_str() != "std.foundation" {
            if let Some(foundation) = self.find("std.foundation") {
                names.extend(
                    foundation
                        .mappings()
                        .into_iter()
                        .filter(|(name, _)| {
                            current.foundation_visible(name) && current.resolve(name).is_none()
                        })
                        .map(|(name, var)| {
                            (
                                name.as_str().to_owned(),
                                var.hara_metadata()
                                    .is_some_and(|metadata| metadata.flag("public")),
                            )
                        }),
                );
            }
        }
        for (alias, namespace) in current.aliases.borrow().iter() {
            names.extend(namespace.mappings.borrow().iter().map(|(name, var)| {
                (
                    format!("{}/{}", alias.as_str(), name.as_str()),
                    var.hara_metadata()
                        .is_some_and(|metadata| metadata.flag("public")),
                )
            }));
        }
        names.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
        names.dedup_by(|left, right| left.0 == right.0);
        names.into_iter().map(|(name, _)| name).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Namespace, NamespaceLoadState, VarMetadata};
    use crate::lang::data::Symbol;
    use crate::lang::protocol::IDeref;
    #[test]
    fn resolves_local_and_aliased_vars() {
        let source = Namespace::new("source");
        source.intern("answer", 42);
        let original = source.resolve(&Symbol::parse("answer")).unwrap();
        let reinterned = source.intern("answer", 43);
        assert!(original.same_identity(&reinterned));
        assert_eq!(original.deref(), 43);
        let target = Namespace::new("target");
        target.alias("s", source.clone());
        assert_eq!(
            source.resolve(&Symbol::parse("answer")).unwrap().deref(),
            43
        );
        assert_eq!(
            target.resolve(&Symbol::parse("s/answer")).unwrap().deref(),
            43
        );
        assert_eq!(
            source.resolve(&Symbol::parse("-/answer")).unwrap().deref(),
            43
        );
    }
    #[test]
    fn registry_manages_lifecycle_resolution_and_visibility() {
        let registry = super::NamespaceRegistry::new("user");
        registry.current().intern("local", 1);
        let library = registry.find_or_create("example.lib");
        library.intern("answer", 42);
        library.intern("IExample/method", 43);
        registry.current().alias("lib", library);
        assert_eq!(
            registry
                .resolve(&Symbol::parse("example.lib/answer"))
                .unwrap()
                .deref(),
            42
        );
        assert_eq!(
            registry
                .resolve(&Symbol::parse("lib/answer"))
                .unwrap()
                .deref(),
            42
        );
        assert_eq!(
            registry.resolve(&Symbol::parse("-/local")).unwrap().deref(),
            1
        );
        assert_eq!(
            registry
                .resolve(&Symbol::parse("example.lib/IExample/method"))
                .unwrap()
                .deref(),
            43
        );
        assert_eq!(
            registry.visible_symbol_names(),
            vec!["lib/IExample/method", "lib/answer", "local"]
        );
        assert!(registry.remove("user").is_none());
        assert!(registry.remove("example.lib").is_some());
    }

    #[test]
    fn registry_tracks_session_local_namespace_loading_state() {
        let first = super::NamespaceRegistry::<i32>::new("user");
        let second = super::NamespaceRegistry::<i32>::new("user");

        assert_eq!(first.load_state("user"), Some(NamespaceLoadState::Loaded));
        assert_eq!(first.load_state("example.lazy"), None);

        first.set_load_state("example.lazy", NamespaceLoadState::Unloaded);
        first.set_load_state("example.lazy", NamespaceLoadState::Loading);
        first.set_load_state("example.lazy", NamespaceLoadState::Failed);

        assert_eq!(
            first.load_state("example.lazy"),
            Some(NamespaceLoadState::Failed)
        );
        assert_eq!(second.load_state("example.lazy"), None);
    }

    #[test]
    fn visible_symbols_rank_public_vars_before_helpers() {
        let registry = super::NamespaceRegistry::new("user");
        registry.current().intern("zebra-helper", 1);
        registry.current().intern("alpha-helper", 2);
        registry.current().intern_with_metadata(
            "recommended-api",
            3,
            VarMetadata {
                hara: Some(crate::lang::data::Metadata::new(vec![(
                    crate::lang::data::MetadataValue::Keyword(crate::lang::data::Keyword::from(
                        "public",
                    )),
                    crate::lang::data::MetadataValue::Boolean(true),
                )])),
                ..VarMetadata::default()
            },
        );
        registry.current().intern_with_metadata(
            "advertised-api",
            4,
            VarMetadata {
                hara: Some(crate::lang::data::Metadata::new(vec![(
                    crate::lang::data::MetadataValue::Keyword(crate::lang::data::Keyword::from(
                        "public",
                    )),
                    crate::lang::data::MetadataValue::Boolean(true),
                )])),
                ..VarMetadata::default()
            },
        );
        assert_eq!(
            registry.visible_symbol_names(),
            vec![
                "advertised-api",
                "recommended-api",
                "alpha-helper",
                "zebra-helper"
            ]
        );
    }
}
