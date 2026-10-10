#![deny(unsafe_code)]

use std::collections::BTreeMap;

use dendro_core::{
    CanonicalSession, CorrelationResolver, IdentityFact, OrchestrationContext, RuntimeSnapshot,
};

/// R1 does not guess a rig from cwd, pid, or a session id that merely contains a name.
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
        _session: &CanonicalSession,
        _runtime: &RuntimeSnapshot,
    ) -> Option<OrchestrationContext> {
        None
    }
}

/// Standalone naming is not a link. R1 leaves a session unlinked unless an identity event says otherwise.
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
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStatus {
    Linked,
    Unlinked,
    Collision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLink {
    pub harness: String,
    pub native_session: String,
    pub rig: Option<String>,
    pub seat: Option<String>,
    pub evidence: Vec<String>,
    pub status: LinkStatus,
}

/// Link discovered sessions only by exact harness and native session id.
/// Disagreeing identity events are a collision, never a guessed seat.
/// Mission is not stored here; it stays a historical queue-ownership question.
pub fn link_sessions(discovered: &[(&str, &str)], identities: &[IdentityFact]) -> Vec<SessionLink> {
    discovered
        .iter()
        .map(|(harness, native)| {
            let matches: Vec<&IdentityFact> = identities
                .iter()
                .filter(|fact| fact.harness == *harness && fact.native_session == *native)
                .collect();
            if matches.is_empty() {
                return SessionLink {
                    harness: (*harness).to_string(),
                    native_session: (*native).to_string(),
                    rig: None,
                    seat: None,
                    evidence: Vec::new(),
                    status: LinkStatus::Unlinked,
                };
            }
            let mut bindings: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
            for fact in &matches {
                bindings
                    .entry((fact.rig.clone(), fact.seat.clone()))
                    .or_default()
                    .push(fact.source_key.clone());
            }
            if bindings.len() == 1 {
                let ((rig, seat), evidence) = bindings.into_iter().next().unwrap();
                SessionLink {
                    harness: (*harness).to_string(),
                    native_session: (*native).to_string(),
                    rig: Some(rig),
                    seat: Some(seat),
                    evidence,
                    status: LinkStatus::Linked,
                }
            } else {
                let evidence = matches.iter().map(|fact| fact.source_key.clone()).collect();
                SessionLink {
                    harness: (*harness).to_string(),
                    native_session: (*native).to_string(),
                    rig: None,
                    seat: None,
                    evidence,
                    status: LinkStatus::Collision,
                }
            }
        })
        .collect()
}

pub fn link_rate(links: &[SessionLink], harness: &str) -> (usize, usize) {
    let mine: Vec<&SessionLink> = links
        .iter()
        .filter(|link| link.harness == harness)
        .collect();
    let linked = mine
        .iter()
        .filter(|link| link.status == LinkStatus::Linked)
        .count();
    (linked, mine.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(harness: &str, native: &str, rig: &str, seat: &str, key: &str) -> IdentityFact {
        IdentityFact {
            source_key: key.into(),
            harness: harness.into(),
            native_session: native.into(),
            rig: rig.into(),
            seat: seat.into(),
            ts: None,
        }
    }

    #[test]
    fn explicit_identity_links_and_a_name_collision_does_not() {
        let identities = vec![ident(
            "claude",
            "sess-1",
            "agentflow-dev",
            "dev.build",
            "events:1",
        )];
        let links = link_sessions(
            &[
                ("claude", "sess-1"),
                ("claude", "agentflow-dev"),
                ("codex", "sess-1"),
            ],
            &identities,
        );
        assert_eq!(links[0].status, LinkStatus::Linked);
        assert_eq!(links[0].seat.as_deref(), Some("dev.build"));
        assert_eq!(links[0].evidence, vec!["events:1".to_string()]);
        assert_eq!(links[1].status, LinkStatus::Unlinked);
        assert_eq!(links[2].status, LinkStatus::Unlinked);
        assert!(links[1].rig.is_none());
    }

    #[test]
    fn disagreeing_events_are_a_collision() {
        let identities = vec![
            ident("pi", "same", "agentflow-dev", "dev.build", "events:2"),
            ident("pi", "same", "agentflow-dev", "dev.qa", "events:3"),
        ];
        let links = link_sessions(&[("pi", "same")], &identities);
        assert_eq!(links[0].status, LinkStatus::Collision);
        assert!(links[0].seat.is_none());
        assert_eq!(links[0].evidence.len(), 2);
        let (linked, discovered) = link_rate(&links, "pi");
        assert_eq!((linked, discovered), (0, 1));
    }

    #[test]
    fn repeated_same_binding_stays_linked() {
        let identities = vec![
            ident("codex", "s", "agentflow-dev", "orch.arch", "events:4"),
            ident("codex", "s", "agentflow-dev", "orch.arch", "events:5"),
        ];
        let links = link_sessions(&[("codex", "s")], &identities);
        assert_eq!(links[0].status, LinkStatus::Linked);
        assert_eq!(link_rate(&links, "codex"), (1, 1));
    }

    #[test]
    fn heuristic_resolvers_do_not_invent_a_link() {
        let session = CanonicalSession {
            session_id: "agentflow-dev-session".into(),
            harness_name: "claude".into(),
            project_path: "/tmp".into(),
            project_name: "agentflow-dev".into(),
            started_at: chrono::Utc::now(),
            ended_at: None,
            duration_seconds: None,
            user_turns_count: 0,
            assistant_turns_count: 0,
            tool_calls_count: 0,
            input_tokens: 0,
            output_tokens: 0,
            estimated_cost_usd: 0.0,
            status: "open".into(),
            orchestration: None,
        };
        let runtime = RuntimeSnapshot {
            runtime_name: "openrig".into(),
            active_rigs: vec!["agentflow-dev".into()],
        };
        assert!(OpenRigResolver::new().resolve(&session, &runtime).is_none());
        assert!(StandaloneResolver::new()
            .resolve(&session, &runtime)
            .is_none());
    }
}
