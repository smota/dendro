use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentRole {
    Caller,
    Executor,
    Reviewer,
    Observer,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkAssignment {
    pub id: AssignmentId,
    pub work: WorkId,
    pub participant: ParticipantId,
    pub role: AssignmentRole,
    pub attempt: Option<AttemptId>,
    pub validity: Option<Interval>,
    pub activity: Vec<ActivitySegment>,
    pub source_authority: Identifier,
    pub native_assignment: NativeKey,
    pub evidence: Vec<EvidenceId>,
}
impl Validate for WorkAssignment {
    fn validate(&self) -> Result<(), Refusal> {
        super::records::require_evidence(&self.evidence)?;
        let mut seen = BTreeSet::new();
        for s in &self.activity {
            s.validate()?;
            if !seen.insert(&s.id) {
                return Err(Refusal::Conflict);
            }
            if let Some(v) = &self.validity {
                if !v.contains_interval(&s.interval) {
                    return Err(Refusal::InvalidInterval);
                }
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeStatus {
    Evidenced,
    Conflict,
    Absent,
    Truncated,
}

/// Closed shapes make endpoint cardinalities explicit. Missing references remain
/// typed unresolved IDs at graph validation; they never create synthetic roots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkRelation {
    Spawn {
        creator: AssignmentId,
        child: AssignmentId,
        child_work_link: Vec<EvidenceId>,
    },
    DelegateExisting {
        caller: AssignmentId,
        executor: AssignmentId,
        work_relation: Vec<EvidenceId>,
    },
    Handoff {
        source: AssignmentId,
        successor: AssignmentId,
        transfer_evidence: Vec<EvidenceId>,
    },
    Return {
        executor: AssignmentId,
        caller: AssignmentId,
        correlation: EdgeId,
        result_boundary: Boundary,
    },
    Join {
        waiting: AssignmentId,
        children: BTreeSet<AssignmentId>,
        receipt: Vec<EvidenceId>,
    },
    Retry {
        prior: AttemptId,
        later: AttemptId,
    },
    Resume {
        attempt: AttemptId,
        prior: SegmentId,
        later: SegmentId,
        continuation_evidence: Vec<EvidenceId>,
    },
    Dependency {
        prerequisite: WorkId,
        dependent: WorkId,
        validity: Option<Interval>,
        satisfaction: Option<OutcomeId>,
    },
}

impl WorkRelation {
    /// Serde `kind` (`rename_all = "snake_case"`). `session_edges.relation` stores these names.
    pub fn wire_kind(&self) -> &'static str {
        match self {
            Self::Spawn { .. } => "spawn",
            Self::DelegateExisting { .. } => "delegate_existing",
            Self::Handoff { .. } => "handoff",
            Self::Return { .. } => "return",
            Self::Join { .. } => "join",
            Self::Retry { .. } => "retry",
            Self::Resume { .. } => "resume",
            Self::Dependency { .. } => "dependency",
        }
    }

    pub fn is_wire_kind(name: &str) -> bool {
        matches!(
            name,
            "spawn"
                | "delegate_existing"
                | "handoff"
                | "return"
                | "join"
                | "retry"
                | "resume"
                | "dependency"
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkEdge {
    pub id: EdgeId,
    pub source_authority: Identifier,
    pub source_instance: SourceInstanceId,
    pub source_generation: u64,
    pub native_relationship: NativeKey,
    /// Identity of the source event, used to detect conflicting handoff transitions.
    pub source_event: SourceKey,
    pub status: EdgeStatus,
    pub boundary: Boundary,
    pub observed_at: UtcMicros,
    pub evidence: Vec<EvidenceId>,
    pub relation: WorkRelation,
}
impl Validate for WorkEdge {
    fn validate(&self) -> Result<(), Refusal> {
        super::records::require_evidence(&self.evidence)?;
        self.boundary.validate()?;
        match &self.relation {
            WorkRelation::Join {
                children, receipt, ..
            } => {
                if children.is_empty() {
                    return Err(Refusal::InvalidReference);
                }
                super::records::require_evidence(receipt)?;
            }
            WorkRelation::DelegateExisting { work_relation, .. } => {
                super::records::require_evidence(work_relation)?
            }
            WorkRelation::Handoff {
                transfer_evidence, ..
            } => super::records::require_evidence(transfer_evidence)?,
            WorkRelation::Return {
                result_boundary, ..
            } => result_boundary.validate()?,
            WorkRelation::Retry { prior, later } if prior == later => {
                return Err(Refusal::InvalidReference)
            }
            WorkRelation::Resume {
                prior,
                later,
                continuation_evidence,
                ..
            } => {
                if prior == later {
                    return Err(Refusal::InvalidReference);
                }
                super::records::require_evidence(continuation_evidence)?;
            }
            WorkRelation::Dependency {
                prerequisite,
                dependent,
                ..
            } if prerequisite == dependent => return Err(Refusal::Cycle),
            _ => {}
        }
        Ok(())
    }
}

/// Validation never mutates or deletes direct facts or asserted relationships.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationSelection {
    pub valid: BTreeSet<UsageId>,
    /// Asserted assignment membership not proved for these observations. Keep
    /// original facts and route them to shared/unattributed scope with reasons.
    pub ambiguous: BTreeMap<UsageId, Refusal>,
}
pub struct WorkGraph<'a> {
    assignments: BTreeMap<&'a AssignmentId, &'a WorkAssignment>,
    attempts: BTreeMap<&'a AttemptId, &'a ExecutionAttempt>,
    work: BTreeSet<&'a WorkId>,
    edges: &'a [WorkEdge],
    issues: BTreeMap<EdgeId, Refusal>,
    eligible: BTreeMap<AssignmentId, BTreeSet<AssignmentId>>,
    barriers: BTreeMap<AssignmentId, Refusal>,
}
fn index_unique<'a, K: Ord + 'a, V: 'a>(
    values: &'a [V],
    key: impl Fn(&'a V) -> &'a K,
) -> Result<BTreeMap<&'a K, &'a V>, Refusal> {
    let mut map = BTreeMap::new();
    for v in values {
        if map.insert(key(v), v).is_some() {
            return Err(Refusal::Conflict);
        }
    }
    Ok(map)
}
impl<'a> WorkGraph<'a> {
    pub fn new(
        assignments: &'a [WorkAssignment],
        attempts: &'a [ExecutionAttempt],
        work: &'a [WorkItem],
        edges: &'a [WorkEdge],
    ) -> Result<Self, Refusal> {
        for a in assignments {
            a.validate()?;
        }
        for a in attempts {
            a.validate()?;
        }
        let mut graph = Self {
            assignments: index_unique(assignments, |v| &v.id)?,
            attempts: index_unique(attempts, |v| &v.id)?,
            work: index_unique(work, |v| &v.id)?.into_keys().collect(),
            edges,
            issues: BTreeMap::new(),
            eligible: BTreeMap::new(),
            barriers: BTreeMap::new(),
        };
        let indexed = index_unique(edges, |v| &v.id)?;
        let mut creators: BTreeMap<&AttemptId, Vec<&WorkEdge>> = BTreeMap::new();
        let mut handoffs: BTreeMap<(&SourceInstanceId, u64, &SourceKey), Vec<&WorkEdge>> =
            BTreeMap::new();
        for edge in edges {
            if let Err(reason) = edge.validate() {
                graph.issues.insert(edge.id.clone(), reason);
                continue;
            }
            if edge.status != EdgeStatus::Evidenced {
                continue;
            }
            let checked = graph.validate_edge(edge, &indexed);
            if let Err(reason) = checked {
                graph.issues.insert(edge.id.clone(), reason);
                continue;
            }
            if let WorkRelation::Spawn { child, .. } = &edge.relation {
                if let Some(id) = graph.assignment(child)?.attempt.as_ref() {
                    creators.entry(id).or_default().push(edge);
                }
            }
            if matches!(edge.relation, WorkRelation::Handoff { .. }) {
                handoffs
                    .entry((
                        &edge.source_instance,
                        edge.source_generation,
                        &edge.source_event,
                    ))
                    .or_default()
                    .push(edge);
            }
        }
        for group in creators.values() {
            let mut origin_assignments = BTreeSet::new();
            for e in group {
                if let WorkRelation::Spawn { creator, .. } = &e.relation {
                    origin_assignments.insert(creator);
                }
            }
            if origin_assignments.len() > 1 {
                for e in group {
                    graph.issues.insert(e.id.clone(), Refusal::Conflict);
                }
            }
        }
        for group in handoffs.values() {
            // Multiple assertions of one typed creator/transition do not create
            // additional creators. Different endpoint claims remain conflicts.
            let endpoints: BTreeSet<_> = group
                .iter()
                .filter_map(|e| match &e.relation {
                    WorkRelation::Handoff {
                        source, successor, ..
                    } => Some((source, successor)),
                    _ => None,
                })
                .collect();
            if endpoints.len() > 1 {
                for e in group {
                    graph.issues.insert(e.id.clone(), Refusal::Conflict);
                }
            }
        }
        // Validate the independent DAGs without interpreting return/handoff/join
        // history as lineage. Mark edges in cycles; ancestors refuse their fold.
        let mut attempt_arcs = Vec::new();
        let mut dependency_arcs = Vec::new();
        let mut segment_arcs = Vec::new();
        for e in edges {
            if e.status != EdgeStatus::Evidenced || graph.issues.contains_key(&e.id) {
                continue;
            }
            match &e.relation {
                WorkRelation::Spawn { creator, child, .. } => {
                    let a = graph.assignment(creator)?;
                    let b = graph.assignment(child)?;
                    if let (Some(a), Some(b)) = (&a.attempt, &b.attempt) {
                        attempt_arcs.push((a.clone(), b.clone(), e.id.clone()));
                    }
                }
                WorkRelation::Retry { prior, later } => {
                    attempt_arcs.push((prior.clone(), later.clone(), e.id.clone()))
                }
                WorkRelation::Dependency {
                    prerequisite,
                    dependent,
                    ..
                } => dependency_arcs.push((prerequisite.clone(), dependent.clone(), e.id.clone())),
                WorkRelation::Resume { prior, later, .. } => {
                    segment_arcs.push((prior.clone(), later.clone(), e.id.clone()))
                }
                _ => {}
            }
        }
        for id in cycle_edges(&attempt_arcs)
            .into_iter()
            .chain(cycle_edges(&dependency_arcs))
            .chain(cycle_edges(&segment_arcs))
        {
            graph.issues.insert(id, Refusal::Cycle);
        }
        // Keep invalid closure edges as barriers, not silently dropped children.
        for e in edges {
            if e.status != EdgeStatus::Evidenced {
                continue;
            }
            if let Some((a, b)) = closure_pair(&e.relation) {
                graph
                    .eligible
                    .entry(a.clone())
                    .or_default()
                    .insert(b.clone());
            }
        }
        let closure_arcs: Vec<_> = edges
            .iter()
            .filter(|e| e.status == EdgeStatus::Evidenced && !graph.issues.contains_key(&e.id))
            .filter_map(|e| {
                closure_pair(&e.relation).map(|(a, b)| (a.clone(), b.clone(), e.id.clone()))
            })
            .collect();
        for id in cycle_edges(&closure_arcs) {
            graph.issues.insert(id, Refusal::Cycle);
        }
        for edge in edges {
            if let WorkRelation::Return { correlation, .. } = &edge.relation {
                if graph.issues.contains_key(correlation) {
                    graph
                        .issues
                        .insert(edge.id.clone(), Refusal::InvalidReference);
                }
            }
        }
        for e in edges {
            if let Some((source, _)) = closure_pair(&e.relation) {
                let reason = if e.status != EdgeStatus::Evidenced {
                    Some(Refusal::MissingEvidence)
                } else {
                    graph.issues.get(&e.id).copied()
                };
                if let Some(reason) = reason {
                    graph.barriers.entry(source.clone()).or_insert(reason);
                }
            }
        }
        Ok(graph)
    }
    fn assignment(&self, id: &AssignmentId) -> Result<&'a WorkAssignment, Refusal> {
        self.assignments
            .get(id)
            .copied()
            .ok_or(Refusal::InvalidReference)
    }
    /// Asserted status remains unchanged; failed validation exposes conflict.
    pub fn issues(&self) -> &BTreeMap<EdgeId, Refusal> {
        &self.issues
    }
    pub fn effective_status(&self, id: &EdgeId) -> Result<EdgeStatus, Refusal> {
        let edge = self
            .edges
            .iter()
            .find(|e| &e.id == id)
            .ok_or(Refusal::InvalidReference)?;
        Ok(if self.issues.contains_key(id) {
            EdgeStatus::Conflict
        } else {
            edge.status
        })
    }
    fn attempt(&self, id: &AttemptId) -> Result<&'a ExecutionAttempt, Refusal> {
        self.attempts
            .get(id)
            .copied()
            .ok_or(Refusal::InvalidReference)
    }
    fn valid_assignment(&self, id: &AssignmentId) -> Result<&'a WorkAssignment, Refusal> {
        let a = self.assignment(id)?;
        if !self.work.contains(&a.work) {
            return Err(Refusal::InvalidReference);
        }
        if let Some(id) = &a.attempt {
            let t = self.attempt(id)?;
            if t.participant != a.participant || t.work.as_ref().is_some_and(|w| w != &a.work) {
                return Err(Refusal::InvalidReference);
            }
        }
        Ok(a)
    }
    fn validate_edge(
        &self,
        edge: &WorkEdge,
        indexed: &BTreeMap<&EdgeId, &WorkEdge>,
    ) -> Result<(), Refusal> {
        match &edge.relation {
            WorkRelation::Spawn {
                creator,
                child,
                child_work_link,
            } => {
                let a = self.valid_assignment(creator)?;
                let b = self.valid_assignment(child)?;
                if b.role != AssignmentRole::Executor || a.id == b.id {
                    return Err(Refusal::InvalidReference);
                }
                if a.work != b.work {
                    super::records::require_evidence(child_work_link)?;
                }
                let prior = self.attempt(a.attempt.as_ref().ok_or(Refusal::MissingEvidence)?)?;
                let target = self.attempt(b.attempt.as_ref().ok_or(Refusal::MissingEvidence)?)?;
                if prior.id == target.id {
                    return Err(Refusal::Cycle);
                }
                let creation = target.start.as_ref().ok_or(Refusal::MissingEvidence)?;
                if let Some(start) = &prior.start {
                    if boundary_no_later(start, &edge.boundary)? == Some(false) {
                        return Err(Refusal::InvalidInterval);
                    }
                }
                if boundary_no_later(&edge.boundary, creation)? == Some(false) {
                    return Err(Refusal::InvalidInterval);
                }
            }
            WorkRelation::DelegateExisting {
                caller, executor, ..
            } => {
                let a = self.valid_assignment(caller)?;
                let b = self.valid_assignment(executor)?;
                if a.id == b.id || b.role != AssignmentRole::Executor {
                    return Err(Refusal::InvalidReference);
                }
                boundary_within(
                    &edge.boundary,
                    a.validity.as_ref().ok_or(Refusal::MissingEvidence)?,
                )?;
                // Existing means the executor has creation/start evidence. Unknown
                // cross-clock ordering remains unknown, not asserted causality.
                let start = b.validity.as_ref().ok_or(Refusal::MissingEvidence)?;
                if edge.boundary.clock.as_ref() == Some(start.clock())
                    && edge.boundary.time.is_some_and(|t| !start.contains(t))
                {
                    return Err(Refusal::InvalidInterval);
                }
            }
            WorkRelation::Handoff {
                source, successor, ..
            } => {
                let a = self.valid_assignment(source)?;
                let b = self.valid_assignment(successor)?;
                if a.id == b.id || a.work != b.work {
                    return Err(Refusal::InvalidReference);
                }
                if edge.boundary.time.is_none() {
                    return Err(Refusal::MissingEvidence);
                }
            }
            WorkRelation::Return {
                executor,
                caller,
                correlation,
                result_boundary,
            } => {
                self.valid_assignment(executor)?;
                self.valid_assignment(caller)?;
                let linked = indexed.get(correlation).ok_or(Refusal::InvalidReference)?;
                if linked.status != EdgeStatus::Evidenced
                    || closure_pair(&linked.relation) != Some((caller, executor))
                {
                    return Err(Refusal::InvalidReference);
                }
                if result_boundary.time.is_none() && result_boundary.order.is_none() {
                    return Err(Refusal::MissingEvidence);
                }
                if boundary_no_later(&linked.boundary, result_boundary)? == Some(false)
                    || boundary_no_later(result_boundary, &edge.boundary)? == Some(false)
                {
                    return Err(Refusal::InvalidInterval);
                }
            }
            WorkRelation::Join {
                waiting, children, ..
            } => {
                self.valid_assignment(waiting)?;
                for child in children {
                    self.valid_assignment(child)?;
                    if child == waiting {
                        return Err(Refusal::InvalidReference);
                    }
                }
                if edge.boundary.time.is_none() && edge.boundary.order.is_none() {
                    return Err(Refusal::MissingEvidence);
                }
            }
            WorkRelation::Retry { prior, later } => {
                let a = self.attempt(prior)?;
                let b = self.attempt(later)?;
                if a.work.is_none() || a.work != b.work {
                    return Err(Refusal::InvalidReference);
                }
                // Retry is an ordered attempt succession, not proof the prior
                // process terminated. A timed-out attempt may still be running.
                if let (Some(a), Some(b)) = (&a.start, &b.start) {
                    if boundary_no_later(a, b)? == Some(false) {
                        return Err(Refusal::InvalidInterval);
                    }
                }
            }
            WorkRelation::Resume {
                attempt,
                prior,
                later,
                ..
            } => {
                let t = self.attempt(attempt)?;
                let a = t
                    .activity
                    .iter()
                    .find(|s| &s.id == prior)
                    .ok_or(Refusal::InvalidReference)?;
                let b = t
                    .activity
                    .iter()
                    .find(|s| &s.id == later)
                    .ok_or(Refusal::InvalidReference)?;
                if a.interval.clock() != b.interval.clock() {
                    return Err(Refusal::UnknownClock);
                }
                if a.interval.end() > b.interval.start() {
                    return Err(Refusal::InvalidInterval);
                }
            }
            WorkRelation::Dependency {
                prerequisite,
                dependent,
                validity,
                ..
            } => {
                if !self.work.contains(prerequisite) || !self.work.contains(dependent) {
                    return Err(Refusal::InvalidReference);
                }
                if let Some(v) = validity {
                    boundary_within(&edge.boundary, v)?;
                }
            }
        }
        Ok(())
    }
    /// Assignment closure only. Direct facts remain available even on refusal.
    pub fn inclusive_assignments(
        &self,
        root: &AssignmentId,
    ) -> Result<BTreeSet<AssignmentId>, Refusal> {
        self.valid_assignment(root)?;
        let mut seen = BTreeSet::new();
        let mut pending = vec![root.clone()];
        while let Some(id) = pending.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(reason) = self.barriers.get(&id) {
                return Err(*reason);
            }
            if let Some(children) = self.eligible.get(&id) {
                pending.extend(children.iter().cloned());
            }
        }
        Ok(seen)
    }
    /// Membership uses evidenced assignment, attempt and whole activity interval.
    /// A timestamp/overlap alone never prorates a straddling observation.
    pub fn direct_observations(
        &self,
        assignment: &AssignmentId,
        usage: &[UsageObservation],
    ) -> Result<ObservationSelection, Refusal> {
        let a = self.valid_assignment(assignment)?;
        let mut selected = ObservationSelection::default();
        let asserted: BTreeSet<_> = usage
            .iter()
            .filter(|u| u.assignment.as_ref() == Some(assignment))
            .map(|u| &u.id)
            .collect();
        let mut unique = BTreeMap::new();
        // Compare every copy of relevant IDs, including copies assigned elsewhere
        // or unattributed. Conflicts on wholly unrelated IDs cannot taint this fold.
        for u in usage.iter().filter(|u| asserted.contains(&u.id)) {
            let digest = canonical_value_digest(u)?;
            if unique
                .insert(&u.id, digest)
                .is_some_and(|old| old != digest)
            {
                return Err(Refusal::Conflict);
            }
        }
        for u in usage
            .iter()
            .filter(|u| u.assignment.as_ref() == Some(assignment))
        {
            if let Err(reason) = u.validate() {
                selected.ambiguous.insert(u.id.clone(), reason);
                continue;
            }
            let contained = u.interval.as_ref().is_some_and(|interval| {
                a.activity
                    .iter()
                    .any(|s| s.interval.contains_interval(interval))
            });
            if u.attempt.is_none()
                || u.attempt != a.attempt
                || u.attribution_evidence.is_empty()
                || !contained
            {
                selected
                    .ambiguous
                    .insert(u.id.clone(), Refusal::AmbiguousMembership);
                continue;
            }
            selected.valid.insert(u.id.clone());
        }
        // Equivalent canonical contents may carry different retained metadata.
        // If any candidate for an ID is invalid/ambiguous, keep that ID solely
        // in the explicit ambiguous partition, without losing other valid IDs.
        selected
            .valid
            .retain(|id| !selected.ambiguous.contains_key(id));
        Ok(selected)
    }
    pub fn inclusive_observations(
        &self,
        root: &AssignmentId,
        usage: &[UsageObservation],
    ) -> Result<ObservationSelection, Refusal> {
        let mut selected = ObservationSelection::default();
        for a in self.inclusive_assignments(root)? {
            let direct = self.direct_observations(&a, usage)?;
            selected.valid.extend(direct.valid);
            selected.ambiguous.extend(direct.ambiguous);
        }
        Ok(selected)
    }
}
fn closure_pair(relation: &WorkRelation) -> Option<(&AssignmentId, &AssignmentId)> {
    match relation {
        WorkRelation::Spawn { creator, child, .. } => Some((creator, child)),
        WorkRelation::DelegateExisting {
            caller, executor, ..
        } => Some((caller, executor)),
        _ => None,
    }
}
fn boundary_within(boundary: &Boundary, interval: &Interval) -> Result<(), Refusal> {
    let time = boundary.time.ok_or(Refusal::MissingEvidence)?;
    if boundary.clock.as_ref() != Some(interval.clock()) {
        return Err(Refusal::UnknownClock);
    }
    let error = boundary.error_us.ok_or(Refusal::UnknownClock)?;
    if i128::from(time.0) - i128::from(error) < i128::from(interval.start().0)
        || i128::from(time.0) + i128::from(error) >= i128::from(interval.end().0)
    {
        return Err(Refusal::InvalidInterval);
    }
    Ok(())
}
/// Iterative strongly connected components avoid recursive stack exhaustion and
/// repeated per-edge traversals. Ordered indexes add O(log V) lookup overhead.
fn cycle_edges<K: Ord + Clone>(arcs: &[(K, K, EdgeId)]) -> BTreeSet<EdgeId> {
    let mut adjacency: BTreeMap<&K, Vec<&K>> = BTreeMap::new();
    let mut reverse: BTreeMap<&K, Vec<&K>> = BTreeMap::new();
    let mut nodes = BTreeSet::new();
    for (a, b, _) in arcs {
        adjacency.entry(a).or_default().push(b);
        reverse.entry(b).or_default().push(a);
        nodes.insert(a);
        nodes.insert(b);
    }
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    for root in nodes {
        let mut pending = vec![(root, false)];
        while let Some((node, finished)) = pending.pop() {
            if finished {
                order.push(node);
                continue;
            }
            if !seen.insert(node) {
                continue;
            }
            pending.push((node, true));
            if let Some(next) = adjacency.get(node) {
                pending.extend(next.iter().rev().map(|n| (*n, false)));
            }
        }
    }
    let mut component = BTreeMap::new();
    let mut number = 0usize;
    for root in order.into_iter().rev() {
        if component.contains_key(root) {
            continue;
        }
        let mut pending = vec![root];
        while let Some(node) = pending.pop() {
            if component.contains_key(node) {
                continue;
            }
            component.insert(node, number);
            if let Some(next) = reverse.get(node) {
                pending.extend(next.iter().copied());
            }
        }
        number += 1;
    }
    let mut cyclic = BTreeSet::new();
    for (a, b, id) in arcs {
        if component.get(a) == component.get(b) {
            cyclic.insert(id.clone());
        }
    }
    cyclic
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkItem {
    pub id: WorkId,
    pub native_key: NativeKey,
    pub acceptance_rule: Option<RuleRef>,
    pub dependencies: Vec<EdgeId>,
    pub outcomes: Vec<OutcomeId>,
}
impl Validate for WorkItem {
    fn validate(&self) -> Result<(), Refusal> {
        Ok(())
    }
}
