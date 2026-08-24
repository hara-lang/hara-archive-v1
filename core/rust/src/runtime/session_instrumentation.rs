impl SessionKernel {
    /// Returns the embedding-only native instrumentation service owned by one
    /// active Session Runtime.
    ///
    /// This host method does not install a Hara Var, grant guest authority, or
    /// add an ambient Runtime lookup. The returned service holds only a weak
    /// Runtime identity and becomes unusable when the Session closes.
    pub fn instrumentation(
        &self,
        id: &SessionId,
    ) -> Result<instrumentation::NativeInstrumentation, instrumentation::NativeInstrumentationError>
    {
        let session = self
            .session_registry
            .entries
            .get(id.as_str())
            .ok_or_else(|| {
                instrumentation::NativeInstrumentationError::UnknownSession(id.to_string())
            })?;
        if session.state() != SessionState::Active {
            return Err(instrumentation::NativeInstrumentationError::SessionClosed(
                id.to_string(),
            ));
        }
        let runtime = session.runtime().map_err(|_| {
            instrumentation::NativeInstrumentationError::SessionClosed(id.to_string())
        })?;
        Ok(instrumentation::NativeInstrumentation::new(
            id.to_string(),
            runtime.execution.instrumentation_handle(),
        ))
    }
}
