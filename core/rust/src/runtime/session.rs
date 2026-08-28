/// A process-local kernel that multiplexes isolated evaluator sessions.
///
/// Raw HTA exposes the same lifecycle over its wire targets; this native
/// facade keeps embedding hosts from treating a `Runtime` as the process
/// boundary when several independent sessions can share one kernel.
pub struct SessionKernel {
    session_registry: SessionRegistry,
    development_resources: DevelopmentResourceCatalog,
    bundle_catalog: BundleCatalog,
    mount_registry: MountRegistry,
    sandbox_provider_registry: SandboxProviderRegistry,
    sandbox_registry: SandboxRegistry,
    runtime_factory: Rc<dyn Fn() -> Runtime>,
    test_runner: String,
    execution_backend: String,
    #[cfg(not(target_arch = "wasm32"))]
    source_catalog: Option<crate::project::SourceCatalog>,
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    native_source_cache: Option<SourceBytecodeCache>,
}

#[derive(Default)]
struct SessionRegistry {
    entries: HashMap<String, Session>,
}

#[derive(Default)]
struct DevelopmentResourceCatalog {
    entries: HashMap<String, String>,
}

#[derive(Default)]
struct BundleCatalog {
    entries: HashMap<String, Vec<u8>>,
}

#[derive(Default)]
struct MountRegistry {
    entries: HashMap<u64, FilesystemMount>,
    session_attachments: HashMap<String, u64>,
    sandbox_attachments: HashMap<u64, u64>,
    next_id: u64,
}

#[derive(Default)]
struct SandboxProviderRegistry {
    entries: HashMap<String, Rc<dyn SandboxProvider>>,
}

#[derive(Default)]
struct SandboxRegistry {
    entries: HashMap<u64, Sandbox>,
    next_id: u64,
}

/// An isolated, named execution context owned by a [`SessionKernel`].
pub struct Session {
    spec: SessionSpec,
    runtime: Option<Runtime>,
    state: SessionState,
    filesystem: Option<AttachedFilesystem>,
    authority: SessionAuthorityPolicy,
    last_namespace: String,
    live_sessions: SessionLiveRegistry,
}

struct AttachedFilesystem {
    id: SessionMountId,
    _provider: Rc<dyn core::FileProvider>,
}

impl Session {
    #[cfg(test)]
    fn new(name: &str, runtime: Runtime) -> Self {
        let spec = SessionSpec::zero_authority(name)
            .expect("Session::new requires a validated session name");
        Self::open(spec, runtime)
    }

    fn open(spec: SessionSpec, runtime: Runtime) -> Self {
        let authority = spec.authority;
        let mut session = Self {
            spec,
            runtime: Some(runtime),
            state: SessionState::New,
            filesystem: None,
            authority,
            last_namespace: "user".into(),
            live_sessions: SessionLiveRegistry::default(),
        };
        session.activate();
        session
    }

    pub fn spec(&self) -> &SessionSpec {
        &self.spec
    }

    pub fn id(&self) -> &SessionId {
        &self.spec.id
    }

    pub fn name(&self) -> &str {
        self.id().as_str()
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn filesystem_mount(&self) -> Option<SessionMountId> {
        self.filesystem.as_ref().map(|filesystem| filesystem.id)
    }

    #[cfg(test)]
    pub(crate) fn module_revision(&self, name: &str) -> Result<u64, String> {
        Ok(self.runtime()?.namespace_registry.module_revision(name))
    }

    fn ensure_active(&self) -> Result<(), String> {
        match self.state {
            SessionState::Active => Ok(()),
            SessionState::Closed => Err(format!("SESSION_CLOSED {}", self.name())),
            SessionState::New => Err(format!("SESSION_NOT_ACTIVE {} new", self.name())),
        }
    }

    pub(crate) fn runtime(&self) -> Result<&Runtime, String> {
        let name = self.spec.id.to_string();
        self.runtime
            .as_ref()
            .ok_or_else(|| format!("SESSION_CLOSED {name}"))
    }

    pub(crate) fn runtime_mut(&mut self) -> Result<&mut Runtime, String> {
        let name = self.spec.id.to_string();
        self.runtime
            .as_mut()
            .ok_or_else(|| format!("SESSION_CLOSED {name}"))
    }

    fn activate(&mut self) {
        assert_eq!(
            self.state,
            SessionState::New,
            "session must start exactly once"
        );
        self.state = SessionState::Active;
    }

    fn release(&mut self) -> Option<SessionMountId> {
        if self.state == SessionState::Closed {
            return None;
        }
        self.live_sessions.dispose_all();
        self.last_namespace = self
            .runtime
            .as_ref()
            .map(Runtime::current_namespace)
            .unwrap_or_else(|| self.last_namespace.clone());
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.providers.set_file(None);
        }
        let mount = self.filesystem.take().map(|filesystem| filesystem.id);
        self.runtime.take();
        self.authority = SessionAuthorityPolicy::ZERO;
        self.state = SessionState::Closed;
        mount
    }

    pub fn eval(&mut self, source: &str) -> Result<String, String> {
        self.ensure_active()?;
        self.runtime_mut()?.eval_transfer_text(source)
    }

    pub fn current_namespace(&self) -> String {
        self.runtime
            .as_ref()
            .map(Runtime::current_namespace)
            .unwrap_or_else(|| self.last_namespace.clone())
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    pub fn native_execution_telemetry(
        &self,
    ) -> Result<crate::direct_native::NativeExecutionTelemetry, String> {
        self.ensure_active()?;
        Ok(self.runtime()?.native_execution_telemetry())
    }

    pub fn authority(&self) -> SessionAuthorityPolicy {
        self.authority
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn install_native_socket_provider(&mut self) {
        self.runtime_mut()
            .expect("closed sessions cannot install providers")
            .install_native_socket_provider();
        self.authority.host_network = true;
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn install_native_process_provider(&mut self) {
        self.runtime_mut()
            .expect("closed sessions cannot install providers")
            .install_native_process_provider();
        self.authority.host_process = true;
    }
}

impl crate::lang::protocol::IContext<&str> for Session {
    type Output = Result<String, String>;

    fn call(&mut self, source: &str) -> Self::Output {
        self.eval(source)
    }
}

impl crate::lang::protocol::IComponent for Session {
    type Metadata = SessionMetadata;

    fn props(&self) -> Self::Metadata {
        SessionStatus {
            name: self.id().clone(),
            namespace: self.current_namespace(),
            state: self.state,
            filesystem: self.filesystem_mount(),
            authority: self.authority,
        }
    }

    fn status(&self) -> Self::Metadata {
        self.props()
    }

    fn started(&self) -> bool {
        self.state == SessionState::Active
    }

    fn stopped(&self) -> bool {
        self.state == SessionState::Closed
    }

    fn start(&mut self) {
        self.activate();
    }

    fn stop(&mut self) {
        self.release();
    }
}

impl<'a> crate::lang::protocol::IApplicable<Session, &'a str> for Session {
    type Output = Result<String, String>;

    fn apply_in(&self, runtime: &mut Session, source: &'a str) -> Self::Output {
        self.ensure_active()?;
        crate::lang::protocol::IContext::call(runtime, source)
    }

    fn apply_default(&mut self) -> &mut Session {
        self
    }

    fn transform_in(&self, _runtime: &Session, source: &'a str) -> &'a str {
        source
    }

    fn transform_out(
        &self,
        _runtime: &Session,
        _source: &'a str,
        value: Self::Output,
    ) -> Self::Output {
        value
    }
}

impl<'a> crate::lang::protocol::IInvokeIn<Session, &'a str> for Session {
    type Output = Result<String, String>;

    fn invoke_in(&self, context: &mut Session, source: &'a str) -> Self::Output {
        self.ensure_active()?;
        crate::lang::protocol::IContext::call(context, source)
    }
}

struct FilesystemMount {
    provider: Rc<dyn core::FileProvider>,
    kind: &'static str,
    key: String,
    attachments: usize,
}

impl Default for SessionKernel {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionKernel {
    pub fn new() -> Self {
        Self::with_runtime_factory(Runtime::new(), Rc::new(Runtime::new))
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    /// Creates an isolated-session kernel with a shared direct-native program
    /// cache. The cache is useful for short-lived session owners such as the
    /// Hara test runner; no namespace or mutable value state is shared.
    pub fn new_with_native_engine(engine: crate::direct_native::NativeEngine) -> Self {
        let factory_engine = engine.clone();
        Self::with_runtime_factory(
            Runtime::with_native_engine(engine),
            Rc::new(move || Runtime::with_native_engine(factory_engine.clone())),
        )
    }

    pub(crate) fn with_runtime_factory(
        root_runtime: Runtime,
        runtime_factory: Rc<dyn Fn() -> Runtime>,
    ) -> Self {
        let root_id = SessionId::parse("ROOT").expect("ROOT is a valid session identifier");
        let execution_backend = root_runtime.execution_backend.clone();
        Self {
            session_registry: SessionRegistry {
                entries: HashMap::from([(
                    root_id.to_string(),
                    Session::open(
                        SessionSpec::new(root_id, SessionAuthorityPolicy::ZERO),
                        root_runtime,
                    ),
                )]),
            },
            development_resources: DevelopmentResourceCatalog::default(),
            bundle_catalog: BundleCatalog::default(),
            mount_registry: MountRegistry {
                next_id: 1,
                ..MountRegistry::default()
            },
            sandbox_provider_registry: SandboxProviderRegistry::default(),
            sandbox_registry: SandboxRegistry {
                next_id: 1,
                ..SandboxRegistry::default()
            },
            runtime_factory,
            test_runner: "code.test".into(),
            execution_backend,
            #[cfg(not(target_arch = "wasm32"))]
            source_catalog: None,
            #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
            native_source_cache: None,
        }
    }

    pub fn set_test_runner(&mut self, runner: &str) -> Result<(), String> {
        validate_test_runner(runner)?;
        self.test_runner = runner.into();
        for session in self.session_registry.entries.values_mut() {
            session.runtime_mut()?.configure_test_runner(runner)?;
        }
        Ok(())
    }

    /// Selects the ordinary evaluation backend for every existing and future
    /// session owned by this kernel.
    pub fn set_execution_backend(&mut self, backend: &str) -> Result<(), String> {
        validate_execution_backend(backend)?;
        for session in self.session_registry.entries.values_mut() {
            session.runtime_mut()?.configure_execution_backend(backend)?;
        }
        self.execution_backend = backend.into();
        Ok(())
    }

    /// Mounts a lazy native source catalog in every current session and
    /// carries it into sessions created later by this kernel.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn register_source_catalog(&mut self, catalog: &crate::project::SourceCatalog) {
        self.source_catalog = Some(catalog.clone());
        for session in self.session_registry.entries.values_mut() {
            session
                .runtime_mut()
                .expect("kernel cannot retain a closed session")
                .register_source_catalog(catalog);
        }
    }

    /// Enables the project-local direct-native source-program cache for every
    /// current session and for sessions created later by this kernel.
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    pub fn configure_native_source_cache(
        &mut self,
        root: &std::path::Path,
        source_index_fingerprint: [u8; 32],
    ) {
        let cache = SourceBytecodeCache::new(root, source_index_fingerprint);
        self.native_source_cache = Some(cache.clone());
        for session in self.session_registry.entries.values_mut() {
            session
                .runtime_mut()
                .expect("kernel cannot retain a closed session")
                .set_direct_native_source_cache(cache.clone());
        }
    }

    /// Installs a verified HBX namespace bundle in every current session.
    /// Bundle loading is transactional per session and remains explicit so a
    /// kernel cannot accidentally make an application package available to a
    /// later session without the host opting into it.
    #[cfg(feature = "bytecode-vm")]
    pub fn install_bytecode_bundle(&mut self, bytes: &[u8]) -> Result<(), String> {
        for session in self.session_registry.entries.values_mut() {
            crate::vm::eval_bytecode_bundle(session.runtime_mut()?, bytes)?;
        }
        Ok(())
    }

    pub fn create_session(&mut self, id: SessionId) -> Result<(), String> {
        let spec = SessionSpec::new(id, SessionAuthorityPolicy::ZERO);
        if self.session_registry.entries.contains_key(spec.id.as_str()) {
            return Err(format!("SESSION_EXISTS {}", spec.id));
        }
        let mut runtime = (self.runtime_factory)();
        runtime.configure_test_runner(&self.test_runner)?;
        runtime.configure_execution_backend(&self.execution_backend)?;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(source_catalog) = &self.source_catalog {
            runtime.register_source_catalog(source_catalog);
        }
        #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
        if let Some(source_cache) = &self.native_source_cache {
            runtime.set_direct_native_source_cache(source_cache.clone());
        }
        for (resource, source) in &self.development_resources.entries {
            runtime.register_resource(resource, source);
        }
        self.session_registry
            .entries
            .insert(spec.id.as_str().into(), Session::open(spec, runtime));
        Ok(())
    }

    pub fn session_names(&self) -> Vec<SessionId> {
        let mut names = self
            .session_registry
            .entries
            .values()
            .map(|session| session.id().clone())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    pub fn session(&self, id: &SessionId) -> Result<&Session, String> {
        self.session_registry
            .entries
            .get(id.as_str())
            .ok_or_else(|| format!("NO_SESSION {id}"))
    }

    pub fn session_mut(&mut self, id: &SessionId) -> Result<&mut Session, String> {
        self.session_registry
            .entries
            .get_mut(id.as_str())
            .ok_or_else(|| format!("NO_SESSION {id}"))
    }

    pub fn session_namespace(&self, id: &SessionId) -> Result<String, String> {
        self.session_registry
            .entries
            .get(id.as_str())
            .map(Session::current_namespace)
            .ok_or_else(|| format!("NO_SESSION {id}"))
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    pub fn native_execution_telemetry(
        &self,
        id: &SessionId,
    ) -> Result<crate::direct_native::NativeExecutionTelemetry, String> {
        self.session_registry
            .entries
            .get(id.as_str())
            .ok_or_else(|| format!("NO_SESSION {id}"))?
            .native_execution_telemetry()
    }

    pub fn eval(&mut self, id: &SessionId, source: &str) -> Result<String, String> {
        self.session_registry
            .entries
            .get_mut(id.as_str())
            .ok_or_else(|| format!("NO_SESSION {id}"))?
            .eval(source)
    }

    pub fn register_resource(&mut self, name: &str, source: &str) {
        self.development_resources
            .entries
            .insert(name.into(), source.into());
        for session in self.session_registry.entries.values_mut() {
            session
                .runtime_mut()
                .expect("kernel cannot retain a closed session")
                .register_resource(name, source);
        }
    }

    pub fn remove_resource(&mut self, name: &str) -> bool {
        self.development_resources.entries.remove(name).is_some()
    }

    pub fn resource_names(&self) -> Vec<String> {
        let mut names = self
            .development_resources
            .entries
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    pub fn register_bundle(&mut self, digest: &str, bytes: &[u8]) -> Result<(), String> {
        match self.bundle_catalog.entries.get(digest) {
            Some(current) if current == bytes => Ok(()),
            Some(_) => Err(format!("BUNDLE_DIGEST_CONFLICT {digest}")),
            None => {
                self.bundle_catalog
                    .entries
                    .insert(digest.into(), bytes.into());
                Ok(())
            }
        }
    }

    pub fn bundle(&self, digest: &str) -> Option<&[u8]> {
        self.bundle_catalog.entries.get(digest).map(Vec::as_slice)
    }

    pub fn create_memory_filesystem(&mut self, root: &str) -> SessionMountId {
        self.create_filesystem(Rc::new(core::MemoryFileProvider::new(root)), "memory", root)
    }

    #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
    pub fn create_native_filesystem(&mut self, root: &str) -> SessionMountId {
        self.create_filesystem(Rc::new(core::NativeFileProvider::new(root)), "native", root)
    }

    fn create_filesystem(
        &mut self,
        provider: Rc<dyn core::FileProvider>,
        kind: &'static str,
        key: &str,
    ) -> SessionMountId {
        let id = self.mount_registry.next_id;
        self.mount_registry.next_id = self
            .mount_registry
            .next_id
            .checked_add(1)
            .expect("filesystem mount identifiers exhausted");
        self.mount_registry.entries.insert(
            id,
            FilesystemMount {
                provider,
                kind,
                key: key.into(),
                attachments: 0,
            },
        );
        SessionMountId::new(id)
    }

    pub fn attach_filesystem(
        &mut self,
        session: &SessionId,
        mount_id: SessionMountId,
    ) -> Result<(), String> {
        if !self.session_registry.entries.contains_key(session.as_str()) {
            return Err(format!("NO_SESSION {session}"));
        }
        let provider = self
            .mount_registry
            .entries
            .get(&mount_id.get())
            .ok_or_else(|| format!("NO_FILESYSTEM {mount_id}"))?
            .provider
            .clone();
        if self
            .mount_registry
            .session_attachments
            .get(session.as_str())
            == Some(&mount_id.get())
        {
            return Ok(());
        }
        self.detach_filesystem(session)?;
        self.mount_registry
            .entries
            .get_mut(&mount_id.get())
            .unwrap()
            .attachments += 1;
        self.mount_registry
            .session_attachments
            .insert(session.to_string(), mount_id.get());
        let session = self
            .session_registry
            .entries
            .get_mut(session.as_str())
            .unwrap();
        session
            .runtime_mut()?
            .providers
            .set_file(Some(provider.clone()));
        session.filesystem = Some(AttachedFilesystem {
            id: mount_id,
            _provider: provider,
        });
        Ok(())
    }

    pub fn detach_filesystem(&mut self, session: &SessionId) -> Result<(), String> {
        let session = self
            .session_registry
            .entries
            .get_mut(session.as_str())
            .ok_or_else(|| format!("NO_SESSION {session}"))?;
        session.runtime_mut()?.providers.set_file(None);
        session.filesystem.take();
        if let Some(mount_id) = self
            .mount_registry
            .session_attachments
            .remove(session.id().as_str())
        {
            if let Some(mount) = self.mount_registry.entries.get_mut(&mount_id) {
                mount.attachments = mount.attachments.saturating_sub(1);
            }
        }
        Ok(())
    }

    pub fn filesystem(&self, session: &SessionId) -> Option<SessionMountId> {
        self.session_registry
            .entries
            .get(session.as_str())
            .and_then(Session::filesystem_mount)
    }

    pub fn filesystem_info(&self, mount_id: SessionMountId) -> Result<(&str, &str, usize), String> {
        self.mount_registry
            .entries
            .get(&mount_id.get())
            .map(|mount| (mount.kind, mount.key.as_str(), mount.attachments))
            .ok_or_else(|| format!("NO_FILESYSTEM {mount_id}"))
    }

    pub fn close_filesystem(&mut self, mount_id: SessionMountId) -> Result<(), String> {
        let mount = self
            .mount_registry
            .entries
            .get(&mount_id.get())
            .ok_or_else(|| format!("NO_FILESYSTEM {mount_id}"))?;
        if mount.attachments != 0 {
            return Err(format!("FILESYSTEM_ATTACHED {mount_id}"));
        }
        self.mount_registry.entries.remove(&mount_id.get());
        Ok(())
    }

    pub fn close_session(&mut self, id: &SessionId) -> Result<(), String> {
        if id.as_str() == "ROOT" {
            return Err("ROOT_CANNOT_CLOSE".into());
        }
        if !self.session_registry.entries.contains_key(id.as_str()) {
            return Err(format!("NO_SESSION {id}"));
        }
        self.detach_filesystem(id)?;
        if let Some(mut session) = self.session_registry.entries.remove(id.as_str()) {
            crate::lang::protocol::IComponent::stop(&mut session);
        }
        Ok(())
    }
}

fn validate_test_runner(runner: &str) -> Result<(), String> {
    if matches!(runner, "code.test" | "native") {
        Ok(())
    } else {
        Err("runtime test runner must be code.test or native".into())
    }
}

/// The root Foundation surface deliberately contains only the iterator core.
/// Native iterator mechanics must enter through the `Iter/*` type alias, so
/// reject legacy unqualified call heads before namespace rewriting canonicalizes
/// an alias to its backing method name.
fn reject_legacy_iterator_calls(form: &Form) -> Result<(), String> {
    const LEGACY: &[&str] = &[
        "iter-has?",
        "iter-finite?",
        "iter-materialize",
        "iter-close",
        "iter-map",
        "iter-filter",
        "iter-take-while",
        "iter-drop-while",
        "iter-mapcat",
        "iter-keep",
        "iter-interpose",
        "iter-interleave",
        "iter-every?",
        "iter-any?",
        "iter-take",
        "iter-drop",
        "iter-zip",
        "iter-cycle",
        "iter-partition-pair",
        "iter-partition-all",
        "iter-partition",
        "iter-range",
        "iter-constantly",
        "iter-repeatedly",
        "iter-iterate",
    ];
    match form {
        Form::List(values) => {
            if let Some(Form::Symbol(name)) = values.first() {
                if LEGACY.contains(&name.as_str()) {
                    return Err(format!("unbound symbol: {name}"));
                }
                if name == "quote" {
                    return Ok(());
                }
            }
            for value in values {
                reject_legacy_iterator_calls(value)?;
            }
        }
        Form::Vector(values) | Form::Set(values) => {
            for value in values {
                reject_legacy_iterator_calls(value)?;
            }
        }
        Form::Map(entries) => {
            for (key, value) in entries {
                reject_legacy_iterator_calls(key)?;
                reject_legacy_iterator_calls(value)?;
            }
        }
        Form::Tagged(_, value) | Form::Metadata(_, value) => reject_legacy_iterator_calls(value)?,
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod authority_tests {
    use super::*;

    fn session_id(name: &str) -> SessionId {
        SessionId::parse(name).unwrap()
    }

    #[test]
    fn named_sessions_start_with_zero_host_authority() {
        let mut kernel = SessionKernel::new();
        let root = session_id("ROOT");
        #[cfg(not(target_arch = "wasm32"))]
        {
            kernel
                .session_mut(&root)
                .unwrap()
                .install_native_socket_provider();
            kernel
                .session_mut(&root)
                .unwrap()
                .install_native_process_provider();
            assert_eq!(
                kernel.session(&root).unwrap().authority().profile(),
                "explicit"
            );
        }

        let child_id = session_id("child");
        kernel.create_session(child_id.clone()).unwrap();
        let child = kernel.session(&child_id).unwrap();
        assert_eq!(child.authority(), SessionAuthorityPolicy::ZERO);
        assert_eq!(
            crate::lang::protocol::IComponent::props(child)
                .authority
                .profile(),
            "zero"
        );

        for capability in ["filesystem", "network/socket", "process"] {
            let error = kernel
                .eval(
                    &child_id,
                    &format!("(deref (Host/capability? \"{capability}\"))"),
                )
                .unwrap_err();
            assert!(error.contains("std.native.Host/capability? requires capability :host-call"));
            assert!(error.contains(":native/capability-denied"));
        }
    }

    #[test]
    fn session_status_uses_typed_identity_state_and_mount() {
        use crate::lang::protocol::IComponent;

        let mut kernel = SessionKernel::new();
        let typed = session_id("typed");
        kernel.create_session(typed.clone()).unwrap();
        let initial = kernel.session(&typed).unwrap().props();
        assert_eq!(initial.name.as_str(), "typed");
        assert_eq!(initial.state, SessionState::Active);
        assert_eq!(initial.filesystem, None);

        let mount = kernel.create_memory_filesystem("/");
        kernel.attach_filesystem(&typed, mount).unwrap();
        let mounted = kernel.session(&typed).unwrap().props();
        assert_eq!(mounted.filesystem, Some(mount));
        assert_eq!(kernel.session(&typed).unwrap().spec().id, mounted.name);
    }

    #[test]
    fn scoped_filesystem_mount_does_not_change_host_authority_profile() {
        let mut kernel = SessionKernel::new();
        let mounted = session_id("mounted");
        kernel.create_session(mounted.clone()).unwrap();
        let mount = kernel.create_memory_filesystem("/");
        kernel.attach_filesystem(&mounted, mount).unwrap();
        assert_eq!(
            kernel.session(&mounted).unwrap().authority(),
            SessionAuthorityPolicy::ZERO
        );
        assert_eq!(kernel.filesystem(&mounted), Some(mount));
    }

    #[test]
    fn closing_releases_session_owned_runtime_and_filesystem_once() {
        use crate::lang::protocol::IComponent;

        let mut kernel = SessionKernel::new();
        let child = session_id("owned");
        kernel.create_session(child.clone()).unwrap();
        let mount = kernel.create_memory_filesystem("/");
        assert_eq!(
            Rc::strong_count(&kernel.mount_registry.entries[&mount.get()].provider),
            1
        );

        kernel.attach_filesystem(&child, mount).unwrap();
        assert_eq!(
            Rc::strong_count(&kernel.mount_registry.entries[&mount.get()].provider),
            3
        );

        let mut session = kernel
            .session_registry
            .entries
            .remove(child.as_str())
            .unwrap();
        let released_mount = session.release();
        assert_eq!(released_mount, Some(mount));
        assert_eq!(session.state(), SessionState::Closed);
        assert!(session.runtime.is_none());
        assert!(session.filesystem.is_none());
        assert_eq!(
            Rc::strong_count(&kernel.mount_registry.entries[&mount.get()].provider),
            1
        );

        assert_eq!(session.release(), None);
        session.stop();
        assert_eq!(
            Rc::strong_count(&kernel.mount_registry.entries[&mount.get()].provider),
            1
        );
    }

    #[test]
    fn development_resources_and_sealed_bundles_use_distinct_catalogs() {
        let mut kernel = SessionKernel::new();
        kernel.register_resource("demo/value.hal", "(ns demo.value) (def value 42)");
        assert_eq!(kernel.resource_names(), vec!["demo/value.hal"]);

        kernel.register_bundle("sha256:demo", b"sealed").unwrap();
        kernel.register_bundle("sha256:demo", b"sealed").unwrap();
        assert_eq!(kernel.bundle("sha256:demo"), Some(b"sealed".as_slice()));
        assert_eq!(
            kernel
                .register_bundle("sha256:demo", b"replacement")
                .unwrap_err(),
            "BUNDLE_DIGEST_CONFLICT sha256:demo"
        );

        assert!(kernel.remove_resource("demo/value.hal"));
        assert!(kernel.resource_names().is_empty());
        assert_eq!(kernel.bundle("sha256:demo"), Some(b"sealed".as_slice()));
    }
}
