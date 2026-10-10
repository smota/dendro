use dendro_core::{CanonicalSession, CorrelationResolver, OrchestrationContext, RuntimeSnapshot};

pub struct OpenRigResolver;

impl OpenRigResolver {
    pub fn new() -> Self {
        Self
    }
}

impl Default for OpenRigResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl CorrelationResolver for OpenRigResolver {
    fn name(&self) -> &'static str {
        "openrig-resolver"
    }

    fn resolve(
        &self,
        session: &CanonicalSession,
        runtime: &RuntimeSnapshot,
    ) -> Option<OrchestrationContext> {
        if runtime.runtime_name != "openrig" {
            return None;
        }

        // Resolves seat/rig matching heuristics (e.g. matching cwd, session name or bindings)
        for rig in &runtime.active_rigs {
            if session.project_name.contains(rig) || session.session_id.contains(rig) {
                return Some(OrchestrationContext {
                    rig_name: rig.clone(),
                    seat_role: "builder".to_string(),
                    mission_id: None,
                    handoff_from: None,
                });
            }
        }

        None
    }
}

pub struct StandaloneResolver;

impl StandaloneResolver {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StandaloneResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl CorrelationResolver for StandaloneResolver {
    fn name(&self) -> &'static str {
        "standalone-resolver"
    }

    fn resolve(
        &self,
        _session: &CanonicalSession,
        _runtime: &RuntimeSnapshot,
    ) -> Option<OrchestrationContext> {
        Some(OrchestrationContext {
            rig_name: "standalone".to_string(),
            seat_role: "direct-user".to_string(),
            mission_id: None,
            handoff_from: None,
        })
    }
}
