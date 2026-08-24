/// Cloneable private access to the one instrumentation hub owned by a Runtime.
///
/// Product-level controllers retain this handle while their owning Session is
/// active. The wrapper is not a Hara value and is never installed into a
/// namespace or Sandbox authority surface.
#[derive(Clone, Default)]
struct RuntimeInstrumentation {
    hub: Rc<RefCell<instrumentation::InstrumentationHub>>,
}

impl RuntimeInstrumentation {
    fn handle(&self) -> Rc<RefCell<instrumentation::InstrumentationHub>> {
        self.hub.clone()
    }

    #[cfg(test)]
    fn registration_count(&self) -> usize {
        self.hub.borrow().registration_count()
    }

    fn clear(&self) {
        self.hub.borrow_mut().clear();
    }
}

/// Runtime-owned lexical environment and instrumentation state.
///
/// Namespace, provider, package, Session, and Kernel state deliberately stay
/// outside this type. The instrumentation hub follows the Runtime lifecycle
/// here while remaining separate from the lexical environment and execution
/// targets.
#[derive(Default)]
struct RuntimeExecutionState {
    environment: HashMap<String, core::Value>,
    instrumentation: RuntimeInstrumentation,
}

impl RuntimeExecutionState {
    fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    fn environment(&self) -> &HashMap<String, core::Value> {
        &self.environment
    }

    fn environment_mut(&mut self) -> &mut HashMap<String, core::Value> {
        &mut self.environment
    }

    fn snapshot(&self) -> HashMap<String, core::Value> {
        self.environment.clone()
    }

    fn restore(&mut self, environment: HashMap<String, core::Value>) {
        self.environment = environment;
    }

    fn instrumentation_handle(&self) -> Rc<RefCell<instrumentation::InstrumentationHub>> {
        self.instrumentation.handle()
    }

    fn start_fiber(&self, form: Form) -> Result<core::EvalFiber, String> {
        core::EvalFiber::start_forms(vec![form], self.environment.clone())
    }

    fn finish_fiber(&mut self, fiber: &core::EvalFiber) {
        self.environment = fiber.environment();
    }

    fn clear(&mut self) {
        self.environment.clear();
        self.instrumentation.clear();
    }
}
