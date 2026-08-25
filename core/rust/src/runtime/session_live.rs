#[cfg(all(feature = "bytecode-observation", feature = "bytecode-instrumentation"))]
use crate::live_session::InstrumentedHbcLiveSession;
use crate::live_session::{
    InstrumentedInterpreterLiveSession, LiveSession, LiveSessionCapabilities, LiveSessionCommand,
    LiveSessionError, LiveSessionReply, LiveSessionRequest, LiveSessionState, LiveSource,
};

#[derive(Default)]
struct SessionLiveRegistry {
    entries: HashMap<String, Box<dyn LiveSession>>,
}

impl SessionLiveRegistry {
    fn dispose_all(&mut self) {
        for live_session in self.entries.values_mut() {
            let _ = live_session.dispatch_command(LiveSessionCommand::Dispose);
        }
        self.entries.clear();
    }
}

impl Drop for SessionLiveRegistry {
    fn drop(&mut self) {
        self.dispose_all();
    }
}

impl Session {
    fn ensure_live_owner_active(&self) -> Result<(), LiveSessionError> {
        self.ensure_active()
            .map_err(|message| LiveSessionError::new("live-session/owner-closed", message))
    }

    fn ensure_live_session_identity_available(
        &self,
        live_session_id: &str,
    ) -> Result<(), LiveSessionError> {
        if self.live_sessions.entries.contains_key(live_session_id) {
            Err(live_session_already_exists(live_session_id))
        } else {
            Ok(())
        }
    }

    /// Transfers one backend-neutral live session into this Session's private
    /// lifecycle. Live-session identities cannot be reused, including after a
    /// nested session has been cancelled or disposed.
    pub fn register_live_session(
        &mut self,
        mut live_session: Box<dyn LiveSession>,
    ) -> Result<LiveSessionState, LiveSessionError> {
        self.ensure_live_owner_active()?;
        let state = live_session.state();
        if state.session_id.trim().is_empty() {
            let _ = live_session.dispatch_command(LiveSessionCommand::Dispose);
            return Err(LiveSessionError::new(
                "live-session/invalid-identity",
                "live-session id must not be empty",
            ));
        }
        if self.live_sessions.entries.contains_key(&state.session_id) {
            let _ = live_session.dispatch_command(LiveSessionCommand::Dispose);
            return Err(live_session_already_exists(&state.session_id));
        }
        self.live_sessions
            .entries
            .insert(state.session_id.clone(), live_session);
        Ok(state)
    }

    /// Starts the authoritative interpreter target as a built-in controlling
    /// instrument over this Session's Runtime-owned instrumentation hub.
    pub fn start_interpreter_live_session(
        &mut self,
        live_session_id: impl Into<String>,
        source: LiveSource,
    ) -> Result<LiveSessionState, LiveSessionError> {
        self.ensure_live_owner_active()?;
        let live_session_id = live_session_id.into();
        self.ensure_live_session_identity_available(&live_session_id)?;
        let owner_session_id = self.name().to_owned();
        let live_session = InstrumentedInterpreterLiveSession::start(
            self.runtime()
                .map_err(|message| LiveSessionError::new("live-session/owner-closed", message))?,
            owner_session_id,
            live_session_id,
            source,
        )?;
        self.register_live_session(Box::new(live_session))
    }

    /// Starts the authoritative HBC Machine as a built-in controlling
    /// instrument over this Session's Runtime-owned instrumentation hub.
    #[cfg(all(feature = "bytecode-observation", feature = "bytecode-instrumentation"))]
    pub fn start_hbc_live_session(
        &mut self,
        live_session_id: impl Into<String>,
        source: LiveSource,
    ) -> Result<LiveSessionState, LiveSessionError> {
        self.ensure_live_owner_active()?;
        let live_session_id = live_session_id.into();
        self.ensure_live_session_identity_available(&live_session_id)?;
        let owner_session_id = self.name().to_owned();
        let live_session = InstrumentedHbcLiveSession::start(
            self.runtime()
                .map_err(|message| LiveSessionError::new("live-session/owner-closed", message))?,
            owner_session_id,
            live_session_id,
            source,
        )?;
        self.register_live_session(Box::new(live_session))
    }

    /// Starts the authoritative HBC Machine from an already validated HBC0
    /// artifact. Source metadata is retained for revision fencing, while the
    /// live target avoids a second source compilation during startup.
    #[cfg(all(feature = "bytecode-observation", feature = "bytecode-instrumentation"))]
    pub fn start_hbc_live_session_from_artifact(
        &mut self,
        live_session_id: impl Into<String>,
        source: LiveSource,
        artifact: &[u8],
    ) -> Result<LiveSessionState, LiveSessionError> {
        self.ensure_live_owner_active()?;
        let live_session_id = live_session_id.into();
        self.ensure_live_session_identity_available(&live_session_id)?;
        let owner_session_id = self.name().to_owned();
        let live_session = InstrumentedHbcLiveSession::start_from_artifact(
            self.runtime()
                .map_err(|message| LiveSessionError::new("live-session/owner-closed", message))?,
            owner_session_id,
            live_session_id,
            source,
            artifact,
        )?;
        self.register_live_session(Box::new(live_session))
    }

    /// Starts a prepared whole-Wasm session. Whole-Wasm exposes only the
    /// operations its synchronous prepared backend can implement honestly.
    #[cfg(all(feature = "whole-wasm", not(target_arch = "wasm32")))]
    pub fn start_whole_wasm_live_session(
        &mut self,
        live_session_id: impl Into<String>,
        source: LiveSource,
    ) -> Result<LiveSessionState, LiveSessionError> {
        self.ensure_live_owner_active()?;
        let live_session_id = live_session_id.into();
        self.ensure_live_session_identity_available(&live_session_id)?;
        let owner_session_id = self.name().to_owned();
        let runtime = self
            .runtime()
            .map_err(|message| LiveSessionError::new("live-session/owner-closed", message))?;
        let live_session = crate::live_session::WholeWasmLiveSession::start(
            runtime,
            owner_session_id,
            live_session_id,
            source,
        )?;
        self.register_live_session(Box::new(live_session))
    }

    /// Starts a prepared whole-Wasm session from an already compiled HNW0
    /// artifact, avoiding source compilation at session startup.
    #[cfg(all(feature = "whole-wasm", not(target_arch = "wasm32")))]
    pub fn start_whole_wasm_live_session_from_artifact(
        &mut self,
        live_session_id: impl Into<String>,
        source: LiveSource,
        artifact: &[u8],
    ) -> Result<LiveSessionState, LiveSessionError> {
        self.ensure_live_owner_active()?;
        let live_session_id = live_session_id.into();
        self.ensure_live_session_identity_available(&live_session_id)?;
        let owner_session_id = self.name().to_owned();
        let runtime = self
            .runtime()
            .map_err(|message| LiveSessionError::new("live-session/owner-closed", message))?;
        let live_session = crate::live_session::WholeWasmLiveSession::from_artifact(
            runtime,
            owner_session_id,
            live_session_id,
            source,
            artifact.to_vec(),
        )?;
        self.register_live_session(Box::new(live_session))
    }

    /// Compatibility-only feature slice for builds that explicitly enable the
    /// old observation feature without the shared instrumentation probe.
    #[cfg(all(
        feature = "bytecode-observation",
        not(feature = "bytecode-instrumentation")
    ))]
    pub fn start_hbc_live_session(
        &mut self,
        live_session_id: impl Into<String>,
        source: LiveSource,
    ) -> Result<LiveSessionState, LiveSessionError> {
        self.ensure_live_owner_active()?;
        let live_session_id = live_session_id.into();
        self.ensure_live_session_identity_available(&live_session_id)?;
        let live_session =
            crate::live_session::BytecodeLiveSession::compile(live_session_id, source)?;
        self.register_live_session(Box::new(live_session))
    }

    /// Compatibility-only artifact constructor for observation builds that do
    /// not include the shared instrumentation probe.
    #[cfg(all(
        feature = "bytecode-observation",
        not(feature = "bytecode-instrumentation")
    ))]
    pub fn start_hbc_live_session_from_artifact(
        &mut self,
        live_session_id: impl Into<String>,
        source: LiveSource,
        artifact: &[u8],
    ) -> Result<LiveSessionState, LiveSessionError> {
        self.ensure_live_owner_active()?;
        let live_session_id = live_session_id.into();
        self.ensure_live_session_identity_available(&live_session_id)?;
        let live_session = crate::live_session::BytecodeLiveSession::from_artifact(
            live_session_id,
            source.source_id(),
            source.revision(),
            artifact,
        )?;
        self.register_live_session(Box::new(live_session))
    }

    pub fn live_session_ids(&self) -> Vec<String> {
        let mut ids = self
            .live_sessions
            .entries
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    pub fn live_session_count(&self) -> usize {
        self.live_sessions.entries.len()
    }

    pub fn live_session_state(
        &self,
        live_session_id: &str,
    ) -> Result<LiveSessionState, LiveSessionError> {
        self.ensure_live_owner_active()?;
        self.live_sessions
            .entries
            .get(live_session_id)
            .map(|live_session| live_session.state())
            .ok_or_else(|| live_session_not_found(live_session_id))
    }

    pub fn live_session_capabilities(
        &self,
        live_session_id: &str,
    ) -> Result<LiveSessionCapabilities, LiveSessionError> {
        self.ensure_live_owner_active()?;
        self.live_sessions
            .entries
            .get(live_session_id)
            .map(|live_session| live_session.capabilities())
            .ok_or_else(|| live_session_not_found(live_session_id))
    }

    /// Dispatches one fenced command to a live session owned by this Session.
    /// The request is never routed through Sandbox and backend objects never
    /// leave the owning Session.
    pub fn dispatch_live_session(
        &mut self,
        request: LiveSessionRequest,
    ) -> Result<LiveSessionReply, LiveSessionError> {
        self.ensure_live_owner_active()?;
        let live_session_id = request.session_id.clone();
        self.live_sessions
            .entries
            .get_mut(&live_session_id)
            .ok_or_else(|| live_session_not_found(&live_session_id))?
            .dispatch(request)
    }
}

fn live_session_already_exists(live_session_id: &str) -> LiveSessionError {
    LiveSessionError::new(
        "live-session/already-exists",
        format!("live session identity cannot be reused: {live_session_id}"),
    )
}

fn live_session_not_found(live_session_id: &str) -> LiveSessionError {
    LiveSessionError::new(
        "live-session/not-found",
        format!("unknown live session: {live_session_id}"),
    )
}

fn private_sandbox_session(entry_namespace: &str, mut runtime: Runtime) -> Session {
    runtime.use_namespace(entry_namespace);
    let id = SessionId::parse("SANDBOX").expect("SANDBOX is a valid session identifier");
    Session::open(SessionSpec::new(id, SessionAuthorityPolicy::ZERO), runtime)
}

/// Constructs the zero-authority private Session owned by an external sandbox
/// provider. The returned Session may own live sessions, while Sandbox itself
/// retains only its coarse eval/call/cancel/close contract.
#[cfg(not(target_arch = "wasm32"))]
pub fn restricted_sandbox_session(entry_namespace: &str) -> Session {
    private_sandbox_session(entry_namespace, Runtime::sandbox())
}

/// Constructs a zero-authority private sandbox Session with exactly one
/// caller-supplied fully-qualified Host/call authority boundary.
#[cfg(not(target_arch = "wasm32"))]
pub fn restricted_sandbox_session_with_host(
    entry_namespace: &str,
    handler: Rc<dyn Fn(String, String, Vec<core::Value>) -> Result<core::Value, String>>,
) -> Session {
    private_sandbox_session(
        entry_namespace,
        restricted_sandbox_runtime_with_host(handler),
    )
}
