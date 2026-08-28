pub type ProtocolFn = Rc<dyn Fn(&[Value]) -> Result<Value, String>>;
pub type ProtocolSupports = Rc<dyn Fn(&Value) -> bool>;

#[derive(Clone)]
struct ProtocolImplementation {
    supports: ProtocolSupports,
    invoke: ProtocolFn,
}

#[derive(Default, Clone)]
pub struct ProtocolRegistry {
    methods: Rc<RefCell<HashMap<(String, String), Vec<ProtocolImplementation>>>>,
    markers: Rc<RefCell<HashMap<String, Vec<ProtocolSupports>>>>,
    extension_methods: Rc<RefCell<HashMap<(String, String, String, String), ProtocolFn>>>,
    extension_categories: Rc<RefCell<HashSet<(String, String, String)>>>,
    guest_methods: Rc<RefCell<HashMap<(String, String, String), Rc<Function>>>>,
    guest_declarations: Rc<RefCell<HashSet<(String, String)>>>,
}

#[derive(Clone)]
pub(crate) struct ProtocolRegistrySnapshot {
    methods: HashMap<(String, String), Vec<ProtocolImplementation>>,
    markers: HashMap<String, Vec<ProtocolSupports>>,
    extension_methods: HashMap<(String, String, String, String), ProtocolFn>,
    extension_categories: HashSet<(String, String, String)>,
    guest_methods: HashMap<(String, String, String), Rc<Function>>,
    guest_declarations: HashSet<(String, String)>,
}

#[allow(dead_code)]
impl ProtocolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn snapshot(&self) -> ProtocolRegistrySnapshot {
        ProtocolRegistrySnapshot {
            methods: self.methods.borrow().clone(),
            markers: self.markers.borrow().clone(),
            extension_methods: self.extension_methods.borrow().clone(),
            extension_categories: self.extension_categories.borrow().clone(),
            guest_methods: self.guest_methods.borrow().clone(),
            guest_declarations: self.guest_declarations.borrow().clone(),
        }
    }

    pub(crate) fn restore(&self, snapshot: ProtocolRegistrySnapshot) {
        *self.methods.borrow_mut() = snapshot.methods;
        *self.markers.borrow_mut() = snapshot.markers;
        *self.extension_methods.borrow_mut() = snapshot.extension_methods;
        *self.extension_categories.borrow_mut() = snapshot.extension_categories;
        *self.guest_methods.borrow_mut() = snapshot.guest_methods;
        *self.guest_declarations.borrow_mut() = snapshot.guest_declarations;
    }

    pub fn register<F>(
        &mut self,
        protocol: impl Into<String>,
        method: impl Into<String>,
        function: F,
    ) where
        F: Fn(&[Value]) -> Result<Value, String> + 'static,
    {
        let protocol = protocol.into();
        if crate::lang::protocol::find_protocol(&protocol).is_some() {
            self.register_declared(protocol, method, function);
            return;
        }
        let protocol = protocol;
        let supported_protocol = protocol.clone();
        self.register_when(
            protocol,
            method,
            move |value| native_protocol_supports(&supported_protocol, value),
            function,
        );
    }

    pub(crate) fn register_declared<F>(
        &mut self,
        protocol: impl Into<String>,
        method: impl Into<String>,
        function: F,
    ) where
        F: Fn(&[Value]) -> Result<Value, String> + 'static,
    {
        let protocol = protocol.into();
        let method = method.into();
        let declaration = crate::lang::protocol::find_protocol(&protocol)
            .unwrap_or_else(|| panic!("unknown built-in protocol declaration: {protocol}"));
        assert!(
            declaration.method(&method).is_some(),
            "method {method} is not declared by protocol {}",
            declaration.name
        );
        let protocol = declaration.runtime_name();
        let supported_protocol = protocol.clone();
        self.register_when(
            protocol,
            method,
            move |value| native_protocol_supports(&supported_protocol, value),
            function,
        );
    }

    pub fn register_marker<S>(&mut self, protocol: impl Into<String>, supports: S)
    where
        S: Fn(&Value) -> bool + 'static,
    {
        self.markers
            .borrow_mut()
            .entry(protocol.into())
            .or_default()
            .push(Rc::new(supports));
    }

    pub(crate) fn register_marker_declared<S>(&mut self, protocol: impl Into<String>, supports: S)
    where
        S: Fn(&Value) -> bool + 'static,
    {
        let protocol = protocol.into();
        let declaration = crate::lang::protocol::find_protocol(&protocol)
            .unwrap_or_else(|| panic!("unknown built-in protocol declaration: {protocol}"));
        self.register_marker(declaration.runtime_name(), supports);
    }

    pub fn register_when<S, F>(
        &mut self,
        protocol: impl Into<String>,
        method: impl Into<String>,
        supports: S,
        function: F,
    ) where
        S: Fn(&Value) -> bool + 'static,
        F: Fn(&[Value]) -> Result<Value, String> + 'static,
    {
        let protocol = protocol.into();
        let protocol = crate::lang::protocol::find_protocol(&protocol)
            .map(|declaration| declaration.runtime_name())
            .unwrap_or(protocol);
        self.methods
            .borrow_mut()
            .entry((protocol, method.into()))
            .or_default()
            .push(ProtocolImplementation {
                supports: Rc::new(supports),
                invoke: Rc::new(function),
            });
    }

    /// Registers a protocol implementation for one opaque extension type.
    ///
    /// Extension methods are kept separate from the ordinary protocol fallback
    /// chain so collection primitives can dispatch without recursively entering
    /// their own built-in protocol implementation.
    pub fn register_extension<F>(
        &mut self,
        provider: impl Into<String>,
        type_name: impl Into<String>,
        protocol: impl Into<String>,
        method: impl Into<String>,
        function: F,
    ) where
        F: Fn(&[Value]) -> Result<Value, String> + 'static,
    {
        self.extension_methods.borrow_mut().insert(
            (
                provider.into(),
                type_name.into(),
                protocol.into(),
                method.into(),
            ),
            Rc::new(function),
        );
    }

    /// Marks an opaque extension type as a logical collection category such as
    /// `map`. Predicates can then preserve the guest-language collection model.
    pub fn register_extension_category(
        &mut self,
        provider: impl Into<String>,
        type_name: impl Into<String>,
        category: impl Into<String>,
    ) {
        self.extension_categories.borrow_mut().insert((
            provider.into(),
            type_name.into(),
            category.into(),
        ));
    }

    pub fn invoke_extension(
        &self,
        receiver: &ExtensionValue,
        protocol: &str,
        method: &str,
        arguments: &[Value],
    ) -> Result<Value, String> {
        let key = (
            receiver.provider.clone(),
            receiver.type_name.clone(),
            protocol.to_owned(),
            method.to_owned(),
        );
        self.extension_methods
            .borrow()
            .get(&key)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "protocol/unsupported-receiver: extension {}/{} has no {}/{} implementation",
                    receiver.provider, receiver.type_name, protocol, method
                )
            })?(arguments)
    }

    pub fn extension_has_category(&self, receiver: &ExtensionValue, category: &str) -> bool {
        self.extension_categories.borrow().contains(&(
            receiver.provider.clone(),
            receiver.type_name.clone(),
            category.to_owned(),
        ))
    }

    pub fn register_guest(
        &self,
        protocol: impl Into<String>,
        type_name: impl Into<String>,
        method: impl Into<String>,
        function: Rc<Function>,
    ) {
        self.guest_methods.borrow_mut().insert(
            (
                protocol.into(),
                type_name.into(),
                method.into(),
            ),
            function,
        );
    }

    pub fn declare_guest(&self, protocol: impl Into<String>, method: impl Into<String>) {
        self.guest_declarations
            .borrow_mut()
            .insert((protocol.into(), method.into()));
    }

    pub fn replace_guest_protocol(&self, protocol: impl Into<String>) {
        let protocol = protocol.into();
        self.guest_declarations
            .borrow_mut()
            .retain(|(candidate, _)| candidate != &protocol);
        self.guest_methods
            .borrow_mut()
            .retain(|(candidate, _, _), _| candidate != &protocol);
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    pub(crate) fn has_interpreted_guest_functions(&self) -> bool {
        self.guest_methods
            .borrow()
            .values()
            .any(|function| !is_direct_native_function(function))
    }

    pub fn invoke(
        &self,
        protocol: &str,
        method: &str,
        arguments: &[Value],
    ) -> Result<Value, String> {
        let protocol = protocol;
        let known_method = self
            .methods
            .borrow()
            .contains_key(&(protocol.to_owned(), method.to_owned()))
            || self
                .guest_declarations
                .borrow()
                .contains(&(protocol.to_owned(), method.to_owned()))
            || protocol_declarations()
                .iter()
                .any(|declaration| declaration.runtime_name() == protocol);
        if !known_method {
            return Err(format!("missing protocol method: {protocol}/{method}"));
        }
        if let Some(Value::Extension(receiver)) = arguments.first() {
            let extension_method = self.extension_methods.borrow().contains_key(&(
                receiver.provider.clone(),
                receiver.type_name.clone(),
                protocol.to_owned(),
                method.to_owned(),
            ));
            if extension_method {
                return self.invoke_extension(receiver, protocol, method, arguments);
            }
        }
        let named_type = match arguments.first() {
            Some(Value::Struct(receiver)) => Some(receiver.ty.name.as_str()),
            Some(Value::Mutable(receiver)) => Some(receiver.ty.name.as_str()),
            _ => None,
        };
        if let Some(type_name) = named_type {
            let guest_function = self
                .guest_methods
                .borrow()
                .get(&(protocol.to_owned(), type_name.to_owned(), method.to_owned()))
                .cloned();
            if let Some(function) = guest_function {
                return call_function(&function, arguments.to_vec());
            }
        }
        let methods = self.methods.borrow();
        let receiver = arguments.first().ok_or_else(|| {
            format!("protocol/arity: {protocol}/{method} expects at least one argument, received 0")
        })?;
        let last_error = format!(
            "protocol/unsupported-receiver: missing protocol implementation: {protocol}/{method}"
        );
        if let Some(implementations) = methods.get(&(protocol.to_string(), method.to_string())) {
            for implementation in implementations.iter().rev() {
                if (implementation.supports)(receiver) {
                    return (implementation.invoke)(arguments);
                }
            }
        }
        if self
            .guest_declarations
            .borrow()
            .contains(&(protocol.to_owned(), method.to_owned()))
            || protocol_declarations()
                .iter()
                .any(|declaration| declaration.runtime_name() == protocol)
        {
            Err(last_error)
        } else {
            Err(format!("missing protocol method: {protocol}/{method}"))
        }
    }

    pub fn contains(&self, protocol: &str, method: &str) -> bool {
        let methods = self.methods.borrow();
        methods
            .get(&(protocol.to_owned(), method.to_string()))
            .is_some_and(|implementations| !implementations.is_empty())
    }

    pub fn satisfies(&self, protocol: &GuestProtocol, value: &Value) -> bool {
        if let Value::Extension(receiver) = value {
            let protocol_name = protocol
                .name
                .rsplit(|character| character == '/' || character == '.')
                .next()
                .unwrap_or(protocol.name.as_str());
            let category_matches = match protocol_name {
                "IMapType" => self.extension_has_category(receiver, "map"),
                "ISetType" => self.extension_has_category(receiver, "set"),
                "ISequential" => {
                    self.extension_has_category(receiver, "sequential")
                        || self.extension_has_category(receiver, "linear")
                }
                "ILinearType" => self.extension_has_category(receiver, "linear"),
                "IColl" => {
                    self.extension_has_category(receiver, "coll")
                        || ["map", "set", "linear"]
                            .iter()
                            .any(|category| self.extension_has_category(receiver, category))
                }
                _ => false,
            };
            if category_matches {
                return true;
            }
        }
        if !protocol.parents.iter().all(|parent| {
            crate::lang::protocol::find_protocol(parent)
                .is_some_and(|declaration| self.satisfies(&guest_protocol(declaration), value))
        }) {
            return false;
        }
        let protocol_name = protocol.name.clone();
        if protocol.methods.is_empty() {
            if let Some(implementations) = self.markers.borrow().get(&protocol_name) {
                return implementations.iter().rev().any(|supports| supports(value));
            }
            if !protocol.parents.is_empty() {
                return true;
            }
            return false;
        }
        if let Value::Extension(receiver) = value {
            let methods = self.methods.borrow();
            let extensions = self.extension_methods.borrow();
            return protocol.methods.keys().all(|method| {
                extensions.contains_key(&(
                    receiver.provider.clone(),
                    receiver.type_name.clone(),
                    protocol_name.clone(),
                    method.clone(),
                ))
                    || methods
                        .get(&(protocol_name.clone(), method.clone()))
                        .is_some_and(|implementations| {
                            implementations
                                .iter()
                                .rev()
                                .any(|implementation| (implementation.supports)(value))
                        })
            });
        }
        if let Value::Struct(receiver) = value {
            return protocol.methods.keys().all(|method| {
                self.guest_methods.borrow().contains_key(&(
                    protocol_name.clone(),
                    receiver.ty.name.clone(),
                    method.clone(),
                )) || self
                    .methods
                    .borrow()
                    .get(&(protocol_name.clone(), method.clone()))
                    .is_some_and(|implementations| {
                        implementations
                            .iter()
                            .rev()
                            .any(|implementation| (implementation.supports)(value))
                    })
            });
        }
        if let Value::Mutable(receiver) = value {
            return protocol.methods.keys().all(|method| {
                self.guest_methods.borrow().contains_key(&(
                    protocol_name.clone(),
                    receiver.ty.name.clone(),
                    method.clone(),
                )) || self
                    .methods
                    .borrow()
                    .get(&(protocol_name.clone(), method.clone()))
                    .is_some_and(|implementations| {
                        implementations
                            .iter()
                            .rev()
                            .any(|implementation| (implementation.supports)(value))
                    })
            });
        }
        let methods = self.methods.borrow();
        if protocol.methods.keys().all(|method| {
            methods
                .get(&(protocol_name.clone(), method.clone()))
                .is_some_and(|implementations| {
                    implementations
                        .iter()
                        .rev()
                        .any(|implementation| (implementation.supports)(value))
                })
        }) {
            return true;
        }
        false
    }

    /// Returns the built-in collection protocol registry used by evaluator dispatch.
    pub fn core() -> Self {
        let mut registry = Self::new();
        registry.register_marker_declared("IMutable", |value| {
            native_protocol_supports("IMutable", value)
        });
        registry.register_marker_declared("IPersistent", |value| {
            native_protocol_supports("IPersistent", value)
        });
        registry.register_marker_declared("IMapType", |value| {
            native_protocol_supports("IMapType", value)
        });
        registry.register_marker_declared("ISequential", |value| {
            native_protocol_supports("ISequential", value)
        });
        registry.register_marker_declared("ILinearType", |value| {
            native_protocol_supports("ILinearType", value)
        });
        registry.register_marker_declared("ISetType", |value| {
            native_protocol_supports("ISetType", value)
        });
        registry.register_marker_declared("IOFn", |value| matches!(value, Value::Keyword(_)));
        registry.register("std.protocol.icount.ICount", "count", protocol_count);
        registry.register("std.protocol.inth.INth", "nth", protocol_nth);
        registry.register("std.protocol.ilookup.ILookup", "lookup", protocol_lookup);
        registry.register(
            "std.protocol.ipointer.IPointer",
            "ptr-context",
            protocol_pointer_context,
        );
        registry.register("std.protocol.ifind.IFind", "find", protocol_find);
        registry.register("std.protocol.iassoc.IAssoc", "assoc", protocol_assoc);
        registry.register("std.protocol.iconj.IConj", "conj", protocol_conj);
        registry.register("std.protocol.icons.ICons", "cons", protocol_cons);
        registry.register("std.protocol.idissoc.IDissoc", "dissoc", protocol_dissoc);
        registry.register("std.protocol.iempty.IEmpty", "empty", protocol_empty);
        registry.register(
            "std.protocol.iequality.IEquality",
            "equality",
            protocol_equality,
        );
        registry.register(
            "std.protocol.idisplay.IDisplay",
            "display",
            protocol_display,
        );
        registry.register(
            "std.protocol.iencodable.IEncodable",
            "encode-with",
            protocol_encode_with,
        );
        registry.register(
            "std.protocol.iexinfo.IExInfo",
            "data",
            |arguments| match arguments {
                [Value::ExceptionInfo(value)] => Ok((*value.data).clone()),
                [_] => {
                    Err("missing protocol implementation: std.protocol.iexinfo.IExInfo/data".into())
                }
                _ => Err("IExInfo/data expects one argument".into()),
            },
        );
        registry.register("std.protocol.ihash.IHash", "hash", protocol_hash);
        registry.register(
            "std.protocol.ihashcached.IHashCached",
            "hash-current",
            protocol_hash_current,
        );
        registry.register(
            "std.protocol.ihashcached.IHashCached",
            "hash-put",
            protocol_hash_put,
        );
        registry.register_when(
            "std.protocol.ifn.IFn",
            "invoke",
            Value::supports_native_ifn,
            protocol_invoke,
        );
        registry.register("std.protocol.ipair.IPair", "key", protocol_pair_key);
        registry.register("std.protocol.ipair.IPair", "value", protocol_pair_value);
        registry.register(
            "std.protocol.ipeekfirst.IPeekFirst",
            "peek-first",
            protocol_peek_first,
        );
        registry.register(
            "std.protocol.ipeeklast.IPeekLast",
            "peek-last",
            protocol_peek_last,
        );
        registry.register(
            "std.protocol.ipopfirst.IPopFirst",
            "pop-first",
            protocol_pop_first,
        );
        registry.register(
            "std.protocol.ipoplast.IPopLast",
            "pop-last",
            protocol_pop_last,
        );
        registry.register(
            "std.protocol.ipushfirst.IPushFirst",
            "push-first",
            protocol_push_first,
        );
        registry.register(
            "std.protocol.ipushlast.IPushLast",
            "push-last",
            protocol_push_last,
        );
        registry.register("std.protocol.iiter.IIter", "iter", protocol_iter);
        registry.register(
            "std.protocol.iiterator.IIterator",
            "iter-next?",
            |arguments| {
                arguments
                    .first()
                    .ok_or_else(|| "IIterator/iter-next? expects one argument".to_string())
                    .and_then(iterator_has_next)
            },
        );
        registry.register(
            "std.protocol.iiterator.IIterator",
            "iter-next",
            |arguments| {
                arguments
                    .first()
                    .ok_or_else(|| "IIterator/iter-next expects one argument".to_string())
                    .and_then(iterator_next)
            },
        );
        registry.register(
            "std.protocol.iclose.IClose",
            "close",
            |arguments| match arguments {
                [Value::Coroutine(coroutine)] => {
                    coroutine_close(coroutine)?;
                    Ok(Value::Coroutine(coroutine.clone()))
                }
                [Value::Stream(stream)] => {
                    stream_close(stream)?;
                    Ok(Value::Stream(stream.clone()))
                }
                [value] => iterator_close(value),
                _ => Err("IClose/close expects one argument".into()),
            },
        );
        registry.register(
            "std.protocol.inamespaced.INamespaced",
            "name",
            protocol_namespaced_name,
        );
        registry.register(
            "std.protocol.inamespaced.INamespaced",
            "namespace",
            protocol_namespaced_namespace,
        );
        registry.register(
            "std.protocol.istringlike.IStringLike",
            "to-string",
            protocol_string_like_to_string,
        );
        registry.register(
            "std.protocol.istringlike.IStringLike",
            "from-string",
            protocol_string_like_from_string,
        );
        registry.register("std.protocol.iobjtype.IObjType", "meta", protocol_meta);
        registry.register(
            "std.protocol.imetadata.IMetadata",
            "metatype",
            protocol_metatype,
        );
        registry.register(
            "std.protocol.iobjtype.IObjType",
            "with-meta",
            protocol_with_meta,
        );
        registry.register(
            "std.protocol.icoll.IColl",
            "start-string",
            protocol_coll_start,
        );
        registry.register("std.protocol.icoll.IColl", "end-string", protocol_coll_end);
        registry.register("std.protocol.icoll.IColl", "sep-string", protocol_coll_sep);
        registry.register("std.protocol.ideref.IDeref", "deref", protocol_deref);
        registry.register(
            "std.protocol.iapplicable.IApplicable",
            "apply-default",
            protocol_apply_default,
        );
        registry.register(
            "std.protocol.iapplicable.IApplicable",
            "apply-in",
            protocol_apply_in,
        );
        registry.register(
            "std.protocol.iapplicable.IApplicable",
            "transform-in",
            protocol_transform_in,
        );
        registry.register(
            "std.protocol.iapplicable.IApplicable",
            "transform-out",
            protocol_transform_out,
        );
        registry.register(
            "std.protocol.iinvokein.IInvokeIn",
            "invoke-in",
            protocol_invoke_in,
        );
        registry.register(
            "std.protocol.idereftimeout.IDerefTimeout",
            "deref-timeout",
            protocol_deref_timeout,
        );
        registry.register("std.protocol.ireset.IReset", "reset", protocol_reset);
        registry.register("std.protocol.icas.ICas", "cas", protocol_cas);
        registry.register("std.protocol.ireduce.IReduce", "reduce", protocol_reduce);
        registry.register(
            "std.protocol.itomutable.IToMutable",
            "to-mutable",
            protocol_to_mutable,
        );
        registry.register(
            "std.protocol.itopersistent.IToPersistent",
            "to-persistent",
            protocol_to_persistent,
        );
        registry.register(
            "std.protocol.ipromise.IPromise",
            "state",
            protocol_promise_state,
        );
        registry.register(
            "std.protocol.ipromise.IPromise",
            "value",
            protocol_promise_value,
        );
        registry.register("std.protocol.ipromise.IPromise", "then", |arguments| {
            protocol_promise_chain("promise/then", arguments)
        });
        registry.register("std.protocol.ipromise.IPromise", "catch", |arguments| {
            protocol_promise_chain("promise/catch", arguments)
        });
        registry.register("std.protocol.ipromise.IPromise", "finally", |arguments| {
            protocol_promise_chain("promise/finally", arguments)
        });
        registry.register(
            "std.protocol.ipromise.IPromise",
            "cancel",
            protocol_promise_cancel,
        );
        registry.register(
            "std.protocol.icoroutine.ICoroutine",
            "status",
            protocol_coroutine_status,
        );
        registry.register(
            "std.protocol.icoroutine.ICoroutine",
            "resume",
            protocol_coroutine_resume,
        );
        registry.register(
            "std.protocol.istream.IStream",
            "next",
            |arguments| match arguments {
                [Value::Stream(stream)] => Ok(stream_next(stream)),
                [_] => Err("IStream/next expects a stream".into()),
                _ => Err("IStream/next expects one argument".into()),
            },
        );
        registry.register(
            "std.protocol.istreamwrite.IStreamWrite",
            "write",
            |arguments| match arguments {
                [_target, _value] => Err("IStreamWrite/write expects a writable stream".into()),
                _ => Err("IStreamWrite/write expects two arguments".into()),
            },
        );
        registry.register(
            "std.protocol.iabort.IAbort",
            "abort",
            |arguments| match arguments {
                [_target, _error] => Err("IAbort/abort expects an abortable stream".into()),
                _ => Err("IAbort/abort expects two arguments".into()),
            },
        );
        registry.register(
            "std.protocol.iwatch.IWatch",
            "watch-add",
            protocol_watch_add,
        );
        registry.register(
            "std.protocol.iwatch.IWatch",
            "watch-remove",
            protocol_watch_remove,
        );
        registry.register(
            "std.protocol.iwatch.IWatch",
            "watch-list",
            protocol_watch_list,
        );
        registry
    }
}

thread_local! {
    static ACTIVE_PROTOCOLS: RefCell<Option<ProtocolRegistry>> = const { RefCell::new(None) };
    static ACTIVE_NAMESPACES: RefCell<Option<NamespaceRegistry<Value>>> = const { RefCell::new(None) };
    static ACTIVE_DEFINITION_ORIGIN: Cell<VarOrigin> = const { Cell::new(VarOrigin::Source) };
    static ACTIVE_PROMISE_PROVIDER: RefCell<Option<Rc<dyn PromiseProvider>>> = const { RefCell::new(None) };
    static ACTIVE_FILE_PROVIDER: RefCell<Option<Rc<dyn FileProvider>>> = const { RefCell::new(None) };
    static ACTIVE_SOCKET_PROVIDER: RefCell<Option<Rc<dyn SocketProvider>>> = const { RefCell::new(None) };
    static ACTIVE_KERNEL_PROVIDER: RefCell<Option<Rc<KernelProvider>>> = const { RefCell::new(None) };
    static ACTIVE_PACKAGE_CATALOG: RefCell<Option<PackageCatalog>> = const { RefCell::new(None) };
    static ACTIVE_PROCESS_ALLOWED: Cell<bool> = const { Cell::new(false) };
    static ACTIVE_TEST_RUNNER: RefCell<String> = RefCell::new("code.test".into());
    static HOST_CALL_HANDLER: RefCell<Option<Rc<dyn Fn(String, String, Vec<Value>) -> Result<Value, String>>>> = const { RefCell::new(None) };
    static NAMESPACE_SOURCE_PROVIDER: RefCell<Option<Rc<dyn Fn(&str) -> Option<NamespaceResource>>>> = const { RefCell::new(None) };
    static ACTIVE_THROWN_VALUE: RefCell<Option<(String, Value)>> = const { RefCell::new(None) };
    static ACTIVE_MULTIMETHODS: RefCell<HashMap<String, Rc<RefCell<MultiMethod>>>> = RefCell::new(HashMap::new());
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    static ACTIVE_DIRECT_NATIVE_NAMESPACE_LOADER: RefCell<Option<Rc<dyn Fn(&str, NamespaceResource, &mut HashMap<String, Value>) -> Result<(), String>>>> = const { RefCell::new(None) };
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    static ACTIVE_DIRECT_NATIVE_EXECUTION: Cell<bool> = const { Cell::new(false) };
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
pub(crate) type MultiMethodRegistry =
    Rc<RefCell<HashMap<String, Rc<RefCell<MultiMethod>>>>>;

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
#[derive(Clone)]
pub(crate) struct DirectNativeContext {
    pub(crate) namespaces: NamespaceRegistry<Value>,
    /// The namespace in which the frame was compiled. Namespace registries
    /// share their mutable current pointer, so a suspended child must restore
    /// this selection explicitly when it resumes instead of inheriting a
    /// caller which happened to run in the meantime.
    pub(crate) namespace: String,
    pub(crate) protocols: ProtocolRegistry,
    pub(crate) promise_provider: Rc<dyn PromiseProvider>,
    pub(crate) file_provider: Option<Rc<dyn FileProvider>>,
    pub(crate) socket_provider: Option<Rc<dyn SocketProvider>>,
    pub(crate) process_allowed: bool,
    pub(crate) kernel_provider: Option<Rc<KernelProvider>>,
    pub(crate) package_catalog: PackageCatalog,
    pub(crate) macros: Rc<RefCell<HashMap<(String, String), Rc<Function>>>>,
    pub(crate) namespace_source:
        Option<Rc<dyn Fn(&str) -> Option<NamespaceResource>>>,
    pub(crate) host_handler:
        Option<Rc<dyn Fn(String, String, Vec<Value>) -> Result<Value, String>>>,
    pub(crate) test_runner: String,
    pub(crate) definition_origin: VarOrigin,
    pub(crate) multimethods: MultiMethodRegistry,
    pub(crate) native_namespace_loader: Option<
        Rc<dyn Fn(
            &str,
            NamespaceResource,
            &mut HashMap<String, Value>,
        ) -> Result<(), String>>,
    >,
    pub(crate) work_context: Option<crate::work::WorkContext>,
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
impl DirectNativeContext {
    pub(crate) fn capture() -> Self {
        let multimethods = Rc::new(RefCell::new(
            ACTIVE_MULTIMETHODS.with(|active| active.borrow().clone()),
        ));
        Self::capture_with_multimethods(multimethods)
    }

    pub(crate) fn capture_with_multimethods(multimethods: MultiMethodRegistry) -> Self {
        let namespaces = namespace_registry()
            .unwrap_or_else(|_| NamespaceRegistry::new("user"));
        let namespace = namespaces.current().name().as_str().to_owned();
        let protocols = ACTIVE_PROTOCOLS
            .with(|active| active.borrow().clone())
            .unwrap_or_else(ProtocolRegistry::core);
        let promise_provider = ACTIVE_PROMISE_PROVIDER
            .with(|active| active.borrow().clone())
            .unwrap_or_else(|| Rc::new(LocalPromiseProvider));
        let file_provider = ACTIVE_FILE_PROVIDER.with(|active| active.borrow().clone());
        let socket_provider = ACTIVE_SOCKET_PROVIDER.with(|active| active.borrow().clone());
        let process_allowed = ACTIVE_PROCESS_ALLOWED.get();
        let kernel_provider = ACTIVE_KERNEL_PROVIDER.with(|active| active.borrow().clone());
        let package_catalog = ACTIVE_PACKAGE_CATALOG
            .with(|active| active.borrow().clone())
            .unwrap_or_default();
        let macros = ACTIVE_MACROS.with(|active| {
            active
                .borrow()
                .clone()
                .unwrap_or_else(|| Rc::new(RefCell::new(HashMap::new())))
        });
        let namespace_source = NAMESPACE_SOURCE_PROVIDER
            .with(|active| active.borrow().clone());
        let host_handler = HOST_CALL_HANDLER.with(|active| active.borrow().clone());
        let test_runner = ACTIVE_TEST_RUNNER.with(|active| active.borrow().clone());
        let definition_origin = ACTIVE_DEFINITION_ORIGIN.with(Cell::get);
        let native_namespace_loader = ACTIVE_DIRECT_NATIVE_NAMESPACE_LOADER
            .with(|active| active.borrow().clone());
        let work_context = crate::work::current_work_context();
        Self {
            namespaces,
            namespace,
            protocols,
            promise_provider,
            file_provider,
            socket_provider,
            process_allowed,
            kernel_provider,
            package_catalog,
            macros,
            namespace_source,
            host_handler,
            test_runner,
            definition_origin,
            multimethods,
            native_namespace_loader,
            work_context,
        }
    }

    pub(crate) fn with<R>(&self, operation: impl FnOnce() -> R) -> R {
        let namespaces = self.namespaces.clone();
        let namespace = self.namespace.clone();
        let run = || {
            let previous = namespaces.current().name().as_str().to_owned();
            namespaces.set_current(&namespace);
            let result = with_test_runner(&self.test_runner, || {
                with_capability_providers(
                    self.file_provider.clone(),
                    self.socket_provider.clone(),
                    self.process_allowed,
                    self.kernel_provider.clone(),
                    || {
                        with_package_catalog(&self.package_catalog, || {
                            with_promise_provider(self.promise_provider.clone(), || {
                                with_macros(self.macros.clone(), || {
                                    with_namespace_registry(&self.namespaces, || {
                                        with_definition_origin(self.definition_origin, || {
                                            with_protocols(&self.protocols, || {
                                                with_direct_native_context_values(self, operation)
                                            })
                                        })
                                    })
                                })
                            })
                        })
                    },
                )
            });
            namespaces.set_current(&previous);
            result
        };
        if let Some(context) = self.work_context.clone() {
            crate::work::with_current_work_context(context, run)
        } else {
            run()
        }
    }
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
fn with_direct_native_context_values<R>(
    context: &DirectNativeContext,
    operation: impl FnOnce() -> R,
) -> R {
    let run_with_multimethods = || {
        ACTIVE_MULTIMETHODS.with(|active| {
            let previous = std::mem::replace(
                &mut *active.borrow_mut(),
                context.multimethods.borrow().clone(),
            );
            let result = operation();
            *context.multimethods.borrow_mut() = active.borrow().clone();
            *active.borrow_mut() = previous;
            result
        })
    };
    let run_with_loader = || {
        if let Some(loader) = context.native_namespace_loader.clone() {
            with_direct_native_namespace_loader(loader, run_with_multimethods)
        } else {
            run_with_multimethods()
        }
    };
    let run_with_source = || {
        if let Some(provider) = context.namespace_source.clone() {
            with_namespace_source(provider, run_with_loader)
        } else {
            run_with_loader()
        }
    };
    if let Some(handler) = context.host_handler.clone() {
        with_host_calls(handler, run_with_source)
    } else {
        run_with_source()
    }
}

pub(crate) fn with_test_runner<R>(runner: &str, f: impl FnOnce() -> R) -> R {
    ACTIVE_TEST_RUNNER.with(|active| {
        let previous = active.replace(runner.into());
        let result = f();
        active.replace(previous);
        result
    })
}

pub(crate) fn snapshot_multimethods() -> HashMap<String, MultiMethod> {
    ACTIVE_MULTIMETHODS.with(|active| {
        active
            .borrow()
            .iter()
            .map(|(name, state)| (name.clone(), state.borrow().clone()))
            .collect()
    })
}

pub(crate) fn restore_multimethods(snapshot: HashMap<String, MultiMethod>) {
    ACTIVE_MULTIMETHODS.with(|active| {
        *active.borrow_mut() = snapshot
            .into_iter()
            .map(|(name, state)| (name, Rc::new(RefCell::new(state))))
            .collect();
    });
}

pub(crate) fn register_multimethod(name: String, state: Rc<RefCell<MultiMethod>>) {
    ACTIVE_MULTIMETHODS.with(|active| {
        active.borrow_mut().insert(name, state);
    });
}

pub(crate) fn multimethod_state(name: &str) -> Option<Rc<RefCell<MultiMethod>>> {
    ACTIVE_MULTIMETHODS.with(|active| active.borrow().get(name).cloned())
}

pub(crate) fn active_protocol_registry() -> Result<ProtocolRegistry, String> {
    ACTIVE_PROTOCOLS
        .with(|active| active.borrow().clone())
        .ok_or_else(|| "protocol registry is unavailable".into())
}

#[derive(Clone)]
pub enum NamespaceResource {
    Source(String),
    /// A native host source whose contents are read only when the namespace
    /// crosses the require boundary.
    #[cfg(not(target_arch = "wasm32"))]
    SourcePath(std::path::PathBuf),
    #[cfg(feature = "bytecode-vm")]
    Bytecode {
        namespace_form: String,
        artifact: Vec<u8>,
    },
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_source_resource(
    resource: &NamespaceResource,
    namespace: &str,
) -> Result<String, String> {
    match resource {
        NamespaceResource::Source(source) => Ok(source.clone()),
        NamespaceResource::SourcePath(path) => std::fs::read_to_string(path)
            .map_err(|error| format!("{namespace}: cannot read {}: {error}", path.display())),
        #[cfg(feature = "bytecode-vm")]
        NamespaceResource::Bytecode { .. } => {
            Err(format!("{namespace}: bytecode resource is not source text"))
        }
    }
}

pub(crate) fn thrown_error(value: Value) -> String {
    thrown_error_at(value, current_exception_site())
}

pub(crate) fn thrown_error_at(value: Value, site: Option<ExceptionSite>) -> String {
    record_exception_throw(&value, site);
    let error = format!("thrown: {}", value.display());
    ACTIVE_THROWN_VALUE.with(|active| {
        *active.borrow_mut() = Some((error.clone(), value));
    });
    error
}

pub(crate) fn promise_rejection_error(error: PromiseRejection) -> String {
    match error {
        PromiseRejection::Message(message) => message,
        PromiseRejection::Value(value) | PromiseRejection::Cancelled(value) => thrown_error(value),
    }
}

pub(crate) fn caught_error(error: &str) -> Value {
    ACTIVE_THROWN_VALUE.with(|active| {
        let mut active = active.borrow_mut();
        if active
            .as_ref()
            .is_some_and(|(thrown_error, _)| error.starts_with(thrown_error))
        {
            return active.take().unwrap().1;
        }
        Value::String(error.to_owned())
    })
}

pub(crate) fn catch_matches(error: &str, class: &str) -> bool {
    if class == "Exception" || class == "Throwable" {
        return true;
    }
    if let Some(selectors) = class
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        return selectors
            .split(',')
            .any(|selector| catch_matches(error, selector));
    }
    if let Some(selector) = class.strip_prefix(':') {
        return ACTIVE_THROWN_VALUE.with(|active| {
            active.borrow().as_ref().is_some_and(|(message, value)| {
                error.starts_with(message)
                    && matches!(value, Value::ExceptionInfo(info)
                        if map_entries(&info.data).is_some_and(|entries| entries.iter().any(|(key, value)| {
                            matches!(key, Value::Keyword(name) if name.as_str() == "ex/code")
                                && matches!(value, Value::Keyword(code) if code.as_str() == selector)
                        })))
            })
        });
    }
    ACTIVE_THROWN_VALUE.with(|active| {
        active.borrow().as_ref().is_some_and(|(message, value)| {
            error.starts_with(message)
                && match value {
                    Value::Struct(value) => {
                        value.ty.name == class || value.ty.name.ends_with(&format!("/{class}"))
                    }
                    Value::Mutable(value) => {
                        value.ty.name == class || value.ty.name.ends_with(&format!("/{class}"))
                    }
                    _ => false,
                }
        })
    })
}

/// Runs an evaluation with a namespace registry available to namespace builtins.
pub fn with_namespace_registry<R>(
    registry: &NamespaceRegistry<Value>,
    operation: impl FnOnce() -> R,
) -> R {
    ACTIVE_NAMESPACES.with(|active| {
        let previous = active.replace(Some(registry.clone()));
        let result = operation();
        active.replace(previous);
        result
    })
}

pub fn with_definition_origin<R>(origin: VarOrigin, operation: impl FnOnce() -> R) -> R {
    ACTIVE_DEFINITION_ORIGIN.with(|active| {
        let previous = active.replace(origin);
        let result = operation();
        active.set(previous);
        result
    })
}

pub(crate) fn definition_origin() -> VarOrigin {
    ACTIVE_DEFINITION_ORIGIN.with(Cell::get)
}

pub(crate) fn binding_is_local(var: &KernelVar<Value>) -> bool {
    namespace_registry()
        .map(|registry| {
            var.symbol().get_namespace().is_none()
                || var.symbol().get_namespace() == Some(registry.current().name().as_str())
        })
        .unwrap_or(true)
}

/// Names a fresh local var cell: qualified to the current namespace when
/// a registry is active, bare otherwise. Qualifying matters: an
/// unqualified cell fails `binding_is_local`, so redefining the name in
/// the same eval used to shadow with a fresh cell instead of resetting
/// the existing one — the answer then depended on whether the name had
/// survived a previous eval's namespace save-back (which qualifies
/// cells). The JVM runtime always resets the same cell; qualifying at
/// creation makes the tree evaluator agree on both first and later
/// evals (issue #223).
pub(crate) fn local_var_name(name: &str) -> String {
    match namespace_registry() {
        Ok(registry) => format!("{}/{}", registry.current().name().as_str(), name),
        Err(_) => name.to_string(),
    }
}

fn prepare_owned_definition(env: &mut HashMap<String, Value>, name: &str) -> Result<(), String> {
    if let Some(Value::Var(var)) = env.get(name) {
        if !binding_is_local(var) {
            if let Ok(registry) = namespace_registry() {
                registry.current().unmap(&Symbol::parse(name));
            }
            env.remove(name);
        }
    }
    Ok(())
}

/// Defines or updates a global var in the current namespace, mirroring
/// the evaluator's `def` arm (`core.rs` special forms) without the flat
/// env bridge: an existing var local to the current namespace is reused
/// (identity preserved), a referred or missing name gets a fresh cell.
/// Used by the bytecode VM's `DefGlobal` (issue #223).
pub(crate) fn vm_def_global(
    name: &str,
    value: Value,
    metadata: Option<Rc<Metadata>>,
) -> Result<KernelVar<Value>, String> {
    let registry = namespace_registry()?;
    let current = registry.current();
    let local = Symbol::create(None, name);
    if let Some(existing) = current.resolve(&local) {
        if binding_is_local(&existing) {
            existing.reset_value(value);
            if metadata.is_some() {
                existing.set_hara_metadata(metadata);
            }
            existing.set_origin(definition_origin());
            refresh_schema_contract(&existing)?;
            return Ok(existing);
        }
        current.unmap(&local);
    }
    let var = KernelVar::new(format!("{}/{}", current.name().as_str(), name), value);
    var.set_hara_metadata(metadata);
    var.set_origin(definition_origin());
    current.map_var(local, var.clone());
    refresh_schema_contract(&var)?;
    Ok(var)
}

pub(crate) fn vm_def_macro(
    name: &str,
    value: Value,
    metadata: Option<Rc<Metadata>>,
) -> Result<KernelVar<Value>, String> {
    let Value::Function(function) = &value else {
        return Err("defmacro expects a function value".into());
    };
    let function = function.clone();
    let namespace = namespace_registry()?.current().name().as_str().to_owned();
    let var = vm_def_global(name, value, metadata)?;
    register_macro(&namespace, name, function)?;
    Ok(var)
}

/// Declares a global var without assigning it, mirroring the evaluator's
/// `declare` arm: an existing local var is kept (value untouched), a
/// missing name gets a fresh nil cell. Used by the VM (issue #223).
pub(crate) fn vm_declare_global(name: &str) -> Result<KernelVar<Value>, String> {
    let registry = namespace_registry()?;
    let current = registry.current();
    let local = Symbol::create(None, name);
    if let Some(existing) = current.resolve(&local) {
        if binding_is_local(&existing) {
            existing.set_origin(definition_origin());
            return Ok(existing);
        }
        // An explicit `declare` is the source-level ownership boundary.  It
        // authorizes the following definition to replace a referred Var in
        // this namespace; a direct definition still goes through the
        // compiler's ownership check and remains protected.
        current.unmap(&local);
    }
    let var = KernelVar::new(format!("{}/{}", current.name().as_str(), name), Value::Nil);
    var.set_origin(definition_origin());
    current.map_var(local, var.clone());
    Ok(var)
}

/// Resolves a global var by (possibly qualified) name through the
/// registry: current-namespace mappings, aliases, and qualified names.
pub(crate) fn vm_resolve_global(name: &str) -> Result<KernelVar<Value>, String> {
    let registry = namespace_registry()?;
    if let Some(var) = registry.resolve(&Symbol::parse(name)) {
        return Ok(var);
    }
    if let Some((namespace, _)) = name.rsplit_once('/') {
        if NAMESPACE_SOURCE_PROVIDER.with(|active| {
            active
                .borrow()
                .as_ref()
                .is_some_and(|provider| provider(namespace).is_some())
        }) {
            require_namespace(&registry, &mut HashMap::new(), namespace)?;
            if let Some(var) = registry.resolve(&Symbol::parse(name)) {
                return Ok(var);
            }
        }
    }
    Err(format!("unbound symbol: {name}"))
}

/// Resolves a bare namespace symbol as the evaluator does. Namespace aliases
/// are values (and therefore callable through their `run` Var), but they are
/// not Vars themselves and cannot be represented by `vm_resolve_global`.
/// Lazy aliases are materialized at execution time so compiled programs keep
/// the same load boundary as interpreted forms.
pub(crate) fn vm_resolve_namespace_value(name: &str) -> Result<Value, String> {
    let registry = namespace_registry()?;
    if let Some(namespace) = registry
        .current()
        .aliases()
        .into_iter()
        .find_map(|(alias, namespace)| (alias.as_str() == name).then_some(namespace))
    {
        return Ok(Value::Namespace(Rc::new(namespace)));
    }
    if let Some(target) = registry.current().lazy_target(name) {
        require_namespace(&registry, &mut HashMap::new(), target.as_str())?;
        let namespace = registry
            .find(target.as_str())
            .ok_or_else(|| format!("Cannot require missing namespace: {target}"))?;
        registry.current().alias(name, namespace.clone());
        return Ok(Value::Namespace(Rc::new(namespace)));
    }
    registry
        .find(name)
        .map(|namespace| Value::Namespace(Rc::new(namespace)))
        .ok_or_else(|| format!("unbound symbol: {name}"))
}

fn validate_named_definition(kind: &str, name: &str, fields: &[NamedField]) -> Result<(), String> {
    if name.contains('/') {
        return Err(format!("{kind} name must be an unqualified symbol"));
    }
    if fields
        .iter()
        .any(|field| field.name.is_empty() || field.name.contains('/'))
    {
        return Err(format!("{kind} field names must be unqualified symbols"));
    }
    if fields
        .iter()
        .map(|field| &field.name)
        .collect::<HashSet<_>>()
        .len()
        != fields.len()
    {
        return Err(format!("Duplicate {kind} field"));
    }
    Ok(())
}

/// Runs a source declaration as one registry operation.
///
/// Declarations touch the namespace registry, the active protocol dispatch
/// registry, and the flat evaluator environment. Restore all three views when
/// validation or a later inline extension fails.
pub(crate) fn with_declaration_transaction<R>(
    environment: &mut HashMap<String, Value>,
    operation: impl FnOnce(&mut HashMap<String, Value>) -> Result<R, String>,
) -> Result<R, String> {
    let registry = namespace_registry()?;
    let registry_snapshot = registry.snapshot();
    let environment_snapshot = environment.clone();
    let protocol_snapshot = ACTIVE_PROTOCOLS.with(|active| {
        active
            .borrow()
            .as_ref()
            .map(ProtocolRegistry::snapshot)
    });
    let multimethod_snapshot = snapshot_multimethods();

    let result = operation(environment);
    if result.is_err() {
        registry.restore(registry_snapshot);
        *environment = environment_snapshot;
        if let Some(snapshot) = protocol_snapshot {
            ACTIVE_PROTOCOLS.with(|active| {
                if let Some(registry) = active.borrow().as_ref() {
                    registry.restore(snapshot);
                }
            });
        }
        restore_multimethods(multimethod_snapshot);
    }
    result
}

fn prepare_named_binding(namespace: &crate::kernel::Namespace<Value>, name: &str) {
    let symbol = Symbol::parse(name);
    if let Some(existing) = namespace.resolve(&symbol) {
        if existing.symbol().get_namespace() != Some(namespace.name().as_str()) {
            namespace.unmap(&symbol);
        }
    }
}

/// Publishes the type Var and its positional and map constructors for a
/// defstruct or defmutable declaration. The evaluator and bytecode VM both
/// use this path.
pub(crate) fn publish_named_value(
    kind: &str,
    name: &str,
    fields: Vec<NamedField>,
    environment: &mut HashMap<String, Value>,
    metadata: Option<Rc<Metadata>>,
) -> Result<Value, String> {
    validate_named_definition(kind, name, &fields)?;
    let mutable = kind == "defmutable";
    let schema_form = named_value_schema_form(
        &format!("{}/{}", namespace_registry()?.current().name().as_str(), name),
        mutable,
        &fields,
    );
    let metadata = assoc_metadata(metadata, "schema", metadata_value(&schema_form)?)
        .ok_or_else(|| "named value schema metadata could not be created".to_string())?;
    let field_names = fields
        .iter()
        .map(|field| field.name.clone())
        .collect::<Vec<_>>();
    with_declaration_transaction(environment, |environment| {
        let registry = namespace_registry()?;
        let namespace = registry.current();
        let namespace_name = namespace.name().as_str().to_owned();
        let type_name = format!("{}/{}", namespace_name, name);
        let declaration = Rc::new(NamedDeclaration::new(
            type_name.clone(),
            mutable,
            fields.clone(),
            schema_form.clone(),
        ));

        let (type_value, map_constructor) = if mutable {
            let ty = Rc::new(MutableType {
                name: type_name.clone(),
                fields: field_names.clone(),
                declaration: Some(declaration.clone()),
            });
            let map_type = ty.clone();
            let constructor = native_function(&format!("map->{}", name), 1, move |values| {
                let source = values.first().expect("native arity is checked");
                let values = map_type
                    .fields
                    .iter()
                    .map(|field| {
                        map_value(source, &named_field_key(field))
                            .cloned()
                            .unwrap_or(Value::Nil)
                    })
                    .collect();
                Ok(Value::Mutable(Rc::new(MutableValue::from_values(
                    map_type.clone(),
                    values,
                    None,
                )?)))
            });
            (Value::MutableType(ty), constructor)
        } else {
            let ty = Rc::new(StructType {
                name: type_name.clone(),
                fields: field_names,
                declaration: Some(declaration),
            });
            let map_type = ty.clone();
            let constructor = native_function(&format!("map->{}", name), 1, move |values| {
                let source = values.first().expect("native arity is checked");
                let values = map_type
                    .fields
                    .iter()
                    .map(|field| {
                        map_value(source, &named_field_key(field))
                            .cloned()
                            .unwrap_or(Value::Nil)
                    })
                    .collect();
                Ok(Value::Struct(Rc::new(StructValue::from_values(
                    map_type.clone(),
                    values,
                    None,
                )?)))
            });
            (Value::StructType(ty), constructor)
        };

        let bindings = [
            (name.to_owned(), type_value.clone()),
            (format!("->{}", name), type_value),
            (format!("map->{}", name), map_constructor),
        ];
        for (binding, value) in bindings {
            prepare_named_binding(&namespace, &binding);
            let var = namespace.intern(&binding, value);
            var.set_origin(definition_origin());
            if binding == name {
                var.set_hara_metadata(Some(metadata.clone()));
                refresh_schema_contract(&var)?;
            }
            environment.insert(binding.clone(), Value::Var(var.clone()));
            environment.insert(
                format!("{}/{}", namespace_name, binding),
                Value::Var(var),
            );
        }
        Ok(Value::Nil)
    })
}

/// Publishes a guest protocol and all of its method Vars through one
/// namespace/dispatch transaction.
pub(crate) fn publish_guest_protocol(
    name: &str,
    methods: HashMap<String, usize>,
    parents: Vec<String>,
    environment: &mut HashMap<String, Value>,
) -> Result<Value, String> {
    if name.contains('/') || name.is_empty() {
        return Err("defprotocol name must be an unqualified symbol".into());
    }
    if methods.keys().any(|method| method.contains('/')) {
        return Err("protocol method names must be unqualified symbols".into());
    }
    if methods
        .iter()
        .any(|(method, arity)| method.is_empty() || *arity == 0)
    {
        return Err("protocol methods must have a receiver and a non-empty name".into());
    }
    if parents.iter().any(|parent| parent.is_empty()) {
        return Err("protocol parent names must not be empty".into());
    }
    with_declaration_transaction(environment, |environment| {
        let registry = namespace_registry()?;
        let namespace = registry.current();
        let namespace_name = namespace.name().as_str().to_owned();
        let protocol_name = format!("{}.{}", namespace_name, name);
        let previous_protocol = namespace
            .resolve(&Symbol::parse(name))
            .filter(|var| var.symbol().get_namespace() == Some(namespace_name.as_str()))
            .and_then(|var| match var.deref_value() {
                Value::Protocol(protocol) if protocol.name == protocol_name => Some(protocol),
                _ => None,
            });

        for method in methods.keys() {
            for (local, var) in namespace.mappings() {
                if local.as_str() == name
                    || var.symbol().get_namespace() != Some(namespace_name.as_str())
                {
                    continue;
                }
                if let Value::Protocol(other) = var.deref_value() {
                    if other.methods.contains_key(method) {
                        return Err(format!(
                            "Protocol method Var already belongs to {}: {}/{}",
                            local.as_str(),
                            namespace_name,
                            method
                        ));
                    }
                }
            }
            let existing = namespace.resolve(&Symbol::parse(method));
            let same_protocol_reload = previous_protocol
                .as_ref()
                .is_some_and(|previous| previous.methods.contains_key(method));
            if existing
                .as_ref()
                .is_some_and(|var| var.symbol().get_namespace() == Some(namespace_name.as_str()))
                && !same_protocol_reload
            {
                return Err(format!(
                    "Protocol method Var already exists: {}/{}",
                    namespace_name,
                    method
                ));
            }
        }

        if let Some(previous) = &previous_protocol {
            for old_method in previous.methods.keys() {
                if !methods.contains_key(old_method) {
                    let old = Symbol::parse(old_method);
                    if namespace.resolve(&old).is_some_and(|var| {
                        var.symbol().get_namespace() == Some(namespace_name.as_str())
                    }) {
                        namespace.unmap(&old);
                    }
                    environment.remove(old_method);
                    environment.remove(&format!("{}/{}", namespace_name, old_method));
                }
            }
        }

        for method in methods.keys() {
            prepare_named_binding(&namespace, method);
        }
        prepare_named_binding(&namespace, name);

        let protocol = Rc::new(GuestProtocol {
            name: protocol_name.clone(),
            methods,
            parents,
        });
        let protocol_value = Value::Protocol(protocol.clone());
        ACTIVE_PROTOCOLS.with(|active| -> Result<(), String> {
            let registry = active.borrow();
            let registry = registry
                .as_ref()
                .ok_or_else(|| "protocol registry is unavailable".to_string())?;
            registry.replace_guest_protocol(protocol_name.clone());
            for method in protocol.methods.keys() {
                registry.declare_guest(protocol_name.clone(), method.clone());
            }
            Ok(())
        })?;

        let protocol_var = namespace.intern(name, protocol_value.clone());
        protocol_var.set_origin(definition_origin());
        environment.insert(name.to_owned(), Value::Var(protocol_var.clone()));
        environment.insert(
            format!("{}/{}", namespace_name, name),
            Value::Var(protocol_var),
        );
        for method in protocol.methods.keys() {
            let protocol_name = protocol_name.clone();
            let method_name = method.clone();
            let display_name = format!("{}/{}", namespace_name, method);
            let method_value = native_variadic_function(&display_name, move |arguments| {
                protocol_call(&protocol_name, &method_name, &arguments)
            });
            let method_var = namespace.intern(method, method_value);
            method_var.set_origin(definition_origin());
            environment.insert(method.clone(), Value::Var(method_var.clone()));
            environment.insert(
                format!("{}/{}", namespace_name, method),
                Value::Var(method_var),
            );
        }
        Ok(protocol_value)
    })
}

/// `defstruct` against the registry directly, mirroring the evaluator's
/// declaration arm minus inline protocol clauses. Interns `Name`, `->Name`,
/// and `map->Name` into the current namespace and returns nil.
pub(crate) fn vm_defstruct(name: &str, fields: Vec<NamedField>) -> Result<Value, String> {
    let mut environment = HashMap::new();
    publish_named_value("defstruct", name, fields, &mut environment, None)
}

/// `defmutable` against the registry directly. Mutable values use a distinct
/// type descriptor and fixed-field storage while retaining the constructor
/// naming conventions of `defstruct`.
pub(crate) fn vm_defmutable(name: &str, fields: Vec<NamedField>) -> Result<Value, String> {
    let mut environment = HashMap::new();
    publish_named_value("defmutable", name, fields, &mut environment, None)
}

/// Direct field access is reserved for mutable named values. Immutable
/// structs use ordinary associative lookup.
pub(crate) fn mutable_field_value(value: &Value, field: &str) -> Result<Value, String> {
    let Value::Mutable(value) = value else {
        return Err("field expects a mutable value".into());
    };
    value
        .get(field)
        .ok_or_else(|| format!("unknown mutable field: {field}"))
}

/// Replaces one declared mutable field and returns the replacement value.
pub(crate) fn mutable_field_set(
    value: &Value,
    field: &str,
    replacement: Value,
) -> Result<Value, String> {
    let Value::Mutable(value) = value else {
        return Err("field expects a mutable value".into());
    };
    value.set(field, replacement)
}

/// Named-value type identity check shared with the `instance?` special form.
pub(crate) fn named_instance_of(type_value: &Value, value: &Value) -> Result<Value, String> {
    let matches = match type_value {
        Value::StructType(ty) => {
            matches!(value, Value::Struct(value) if Rc::ptr_eq(ty, &value.ty))
        }
        Value::MutableType(ty) => {
            matches!(value, Value::Mutable(value) if Rc::ptr_eq(ty, &value.ty))
        }
        Value::NativeType(native) => native_type_instance(native, value)?,
        _ => return Err("instance? expects a struct or mutable type".into()),
    };
    Ok(Value::Bool(matches))
}

pub(crate) fn namespace_registry() -> Result<NamespaceRegistry<Value>, String> {
    ACTIVE_NAMESPACES
        .with(|active| active.borrow().clone())
        .ok_or_else(|| "namespace runtime is unavailable".into())
}

/// Returns a fresh evaluator environment for the registry's current
/// namespace, including its qualified and aliased bindings.
pub(crate) fn current_namespace_environment() -> Result<HashMap<String, Value>, String> {
    let registry = namespace_registry()?;
    let mut environment = registry
        .current()
        .mappings()
        .into_iter()
        .map(|(name, var)| (name.as_str().to_owned(), Value::Var(var)))
        .collect();
    refresh_namespace_environment(&registry, &mut environment);
    Ok(environment)
}

/// Saves all unqualified evaluator bindings into the registry current namespace.
pub fn save_namespace_environment(
    registry: &NamespaceRegistry<Value>,
    env: &mut HashMap<String, Value>,
) {
    let namespace = registry.current();
    let namespace_name = namespace.name().as_str().to_owned();
    let locals = env
        .iter()
        .filter(|(name, _)| !name.contains('/'))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<_>>();
    for (name, value) in locals {
        let path = format!("{namespace_name}/{name}");
        if matches!(&value, Value::Var(var) if
            (var.symbol().get_namespace().is_some()
                && var.symbol().get_namespace() != Some(namespace_name.as_str()))
                || var.symbol().as_str().starts_with("std.native.")
                || var.symbol().as_str().starts_with("std.protocol.")
        )
        {
            continue;
        }
        let var = match value {
            Value::Var(var) if var.symbol().as_str() == path => var,
            Value::Var(var) => var.requalify(&path),
            value => namespace.intern(&name, value),
        };
        namespace.map_var(crate::lang::data::Symbol::parse(&name), var.clone());
        env.insert(name, Value::Var(var));
    }
}

/// Rebuilds qualified and aliased bindings without changing local bindings.
pub fn refresh_namespace_environment(
    registry: &NamespaceRegistry<Value>,
    env: &mut HashMap<String, Value>,
) {
    env.retain(|name, _| !name.contains('/'));
    for namespace in registry.all() {
        for (_, var) in namespace.mappings() {
            env.insert(var.symbol().as_str().to_owned(), Value::Var(var));
        }
    }
    for (alias, namespace) in registry.current().aliases() {
        for (local, var) in namespace.mappings() {
            env.insert(
                format!("{}/{}", alias.as_str(), local.as_str()),
                Value::Var(var),
            );
        }
    }
}

/// Saves the current namespace, selects name, and loads its bindings.
pub fn select_namespace_environment(
    registry: &NamespaceRegistry<Value>,
    env: &mut HashMap<String, Value>,
    name: &str,
) {
    save_namespace_environment(registry, env);
    let namespace = registry.set_current(name);
    *env = namespace
        .mappings()
        .into_iter()
        .map(|(name, var)| (name.as_str().to_owned(), Value::Var(var)))
        .collect();
    refresh_namespace_environment(registry, env);
}

pub fn apply_global_aliases(registry: &NamespaceRegistry<Value>, namespace: &str) {
    let target = registry.find_or_create(namespace);
    for (alias, library) in registry.global_aliases() {
        if target.name() == &library {
            continue;
        }
        if let Some(source) = registry.find(library.as_str()) {
            target.alias(alias.as_str(), source);
        } else {
            target.lazy_alias(alias.as_str(), library.as_str());
        }
    }
}

pub fn apply_global_imports(registry: &NamespaceRegistry<Value>, namespace: &str) {
    let target = registry.find_or_create(namespace);
    for (local, canonical) in registry.global_imports() {
        if target.resolve(&local).is_none() {
            if let Some(var) = registry.resolve(&canonical) {
                target.map_var(local, var);
            }
        }
    }
}

/// Runs an evaluation with a registry available to protocol dispatch.
pub fn with_protocols<R>(registry: &ProtocolRegistry, operation: impl FnOnce() -> R) -> R {
    ACTIVE_PROTOCOLS.with(|active| {
        let previous = active.replace(Some(registry.clone()));
        let result = operation();
        active.replace(previous);
        result
    })
}

pub fn with_package_catalog<R>(catalog: &PackageCatalog, operation: impl FnOnce() -> R) -> R {
    ACTIVE_PACKAGE_CATALOG.with(|active| {
        let previous = active.replace(Some(catalog.clone()));
        let result = operation();
        active.replace(previous);
        result
    })
}

fn package_catalog() -> PackageCatalog {
    ACTIVE_PACKAGE_CATALOG.with(|active| active.borrow().clone().unwrap_or_default())
}

/// Runs an evaluation through the selected runtime promise provider.
pub fn with_promise_provider<R>(
    provider: Rc<dyn PromiseProvider>,
    operation: impl FnOnce() -> R,
) -> R {
    ACTIVE_PROMISE_PROVIDER.with(|active| {
        let previous = active.replace(Some(provider));
        let result = operation();
        active.replace(previous);
        result
    })
}

fn promise_provider() -> Rc<dyn PromiseProvider> {
    ACTIVE_PROMISE_PROVIDER.with(|active| {
        active
            .borrow()
            .clone()
            .unwrap_or_else(|| Rc::new(LocalPromiseProvider))
    })
}
/// Runs an evaluation through the selected runtime capability providers.
pub fn with_capability_providers<R>(
    file: Option<Rc<dyn FileProvider>>,
    socket: Option<Rc<dyn SocketProvider>>,
    process: bool,
    kernel: Option<Rc<KernelProvider>>,
    operation: impl FnOnce() -> R,
) -> R {
    ACTIVE_FILE_PROVIDER.with(|active_file| {
        ACTIVE_SOCKET_PROVIDER.with(|active_socket| {
            ACTIVE_KERNEL_PROVIDER.with(|active_kernel| {
                ACTIVE_PROCESS_ALLOWED.with(|active_process| {
                    let previous_file = active_file.replace(file);
                    let previous_socket = active_socket.replace(socket);
                    let previous_kernel = active_kernel.replace(kernel);
                    let previous_process = active_process.replace(process);
                    let result = operation();
                    active_file.replace(previous_file);
                    active_socket.replace(previous_socket);
                    active_kernel.replace(previous_kernel);
                    active_process.set(previous_process);
                    result
                })
            })
        })
    })
}

pub type KernelProvider = dyn Fn(String, Vec<Value>) -> Result<Value, String>;

fn kernel_provider(operation: &str) -> Result<Rc<KernelProvider>, String> {
    ACTIVE_KERNEL_PROVIDER.with(|active| {
        active
            .borrow()
            .clone()
            .ok_or_else(|| format!("std.native.Kernel/{operation} requires a kernel provider"))
    })
}

fn file_provider(operation: &str) -> Result<Rc<dyn FileProvider>, String> {
    ACTIVE_FILE_PROVIDER.with(|active| {
        active
            .borrow()
            .clone()
            .ok_or_else(|| format!("{operation} is unsupported or file access is denied"))
    })
}

fn socket_provider(operation: &str) -> Result<Rc<dyn SocketProvider>, String> {
    ACTIVE_SOCKET_PROVIDER.with(|active| {
        active
            .borrow()
            .clone()
            .ok_or_else(|| format!("{operation} is unsupported or network access is denied"))
    })
}

pub(crate) fn native_capability_granted(capability: &str) -> bool {
    match capability {
        "kernel" | "sandbox" => ACTIVE_KERNEL_PROVIDER.with(|active| active.borrow().is_some()),
        "file" => ACTIVE_FILE_PROVIDER.with(|active| active.borrow().is_some()),
        "network" => ACTIVE_SOCKET_PROVIDER.with(|active| active.borrow().is_some()),
        "native-runtime" => ACTIVE_PROCESS_ALLOWED.get(),
        "host-call" => HOST_CALL_HANDLER.with(|active| active.borrow().is_some()),
        _ => false,
    }
}

pub(crate) fn native_capability_error_value(
    native_type: &str,
    method: &str,
    capability: &str,
) -> Value {
    Value::ExceptionInfo(Rc::new(ExceptionInfo {
        message: format!(
            "std.native.{native_type}/{method} requires capability :{capability}"
        ),
        data: Box::new(Value::Map(
            [
                (
                    Value::Keyword("ex/code".into()),
                    Value::Keyword("native/capability-denied".into()),
                ),
                (
                    Value::Keyword("ex/class".into()),
                    Value::Keyword("ex.class/host".into()),
                ),
                (
                    Value::Keyword("native/type".into()),
                    Value::String(format!("std.native.{native_type}")),
                ),
                (
                    Value::Keyword("native/method".into()),
                    Value::String(method.into()),
                ),
                (
                    Value::Keyword("native/capability".into()),
                    Value::Keyword(capability.into()),
                ),
            ]
            .into_iter()
            .collect(),
        )),
        cause: None,
        provenance: Rc::new(RefCell::new(Default::default())),
    }))
}

pub(crate) fn native_capability_denied(
    native_type: &str,
    method: &str,
    capability: &str,
) -> String {
    thrown_error(native_capability_error_value(native_type, method, capability))
}

pub(crate) fn native_capability_denied_promise(
    native_type: &str,
    method: &str,
    capability: &str,
) -> Value {
    let promise = Promise::new();
    promise.reject_value(native_capability_error_value(native_type, method, capability));
    Value::Promise(promise)
}

pub(crate) fn require_native_capability(
    native_type: &str,
    method: &str,
    capability: &str,
) -> Result<(), String> {
    native_capability_granted(capability)
        .then_some(())
        .ok_or_else(|| native_capability_denied(native_type, method, capability))
}

fn require_process_access(operation: &str) -> Result<(), String> {
    ACTIVE_PROCESS_ALLOWED.with(|allowed| {
        allowed
            .get()
            .then_some(())
            .ok_or_else(|| {
                let method = operation
                    .strip_prefix("std.native.Process/")
                    .unwrap_or(operation);
                native_capability_denied("Process", method, "native-runtime")
            })
    })
}
