//! Builder conformance checks, not an independent acceptance oracle.
use dendro_core::domain_v1::*;
use std::collections::BTreeSet;

fn factory() -> IdentityFactory {
    IdentityFactory::new(&[42; 32], ident("test-key")).unwrap()
}
fn ident(s: &str) -> Identifier {
    Identifier::new(s).unwrap()
}
macro_rules! id {
    ($kind:ident,$s:expr) => {
        $kind::derive(&factory(), &[$s.as_bytes()])
    };
}
fn evidence() -> Vec<EvidenceId> {
    vec![id!(EvidenceId, "fixture")]
}
fn source_origin(first_sequence: u64) -> Option<SequenceOrigin> {
    Some(SequenceOrigin {
        conversation: id!(ConversationId, "c"),
        source: id!(SourceInstanceId, "source"),
        generation: 0,
        first_sequence,
        evidence: evidence(),
    })
}
fn clock() -> ClockId {
    id!(ClockId, "clock")
}
fn interval(a: i64, b: i64) -> Interval {
    Interval::new(UtcMicros(a), UtcMicros(b), clock()).unwrap()
}
fn boundary(t: i64) -> Boundary {
    Boundary {
        time: Some(UtcMicros(t)),
        clock: Some(clock()),
        error_us: Some(0),
        order: None,
        evidence: evidence(),
    }
}
fn provenance(key: &str, revision: u64, ingested: i64) -> Provenance {
    Provenance {
        fact_id: FactId::derive(&factory(), &[key.as_bytes(), &revision.to_be_bytes()]),
        source_instance_id: id!(SourceInstanceId, "source"),
        source_generation: SourceGeneration {
            number: 0,
            predecessor: None,
            predecessor_evidence: vec![],
        },
        source_key: id!(SourceKey, key),
        source_schema: SchemaIdentity {
            id: ident("fixture"),
            version: None,
        },
        parser: SchemaIdentity {
            id: ident("test"),
            version: Some(ident("1.0.0")),
        },
        event_time: Some(UtcMicros(2)),
        event_sequence: Some(1),
        clock_id: None,
        clock_error_us: None,
        observed_at: UtcMicros(ingested),
        ingested_at: UtcMicros(ingested),
        granularity: Grain::Event,
        evidence_ids: evidence(),
        revision,
        supersedes: None,
        retracts: None,
    }
}
fn usage(name: &str) -> UsageObservation {
    UsageObservation {
        id: id!(UsageId, name),
        invocation: None,
        attempt: None,
        components: vec![UsageComponent {
            component: TokenComponent::InputUncached,
            quantity: Count(1),
            inclusion: Inclusion::Exclusive,
        }],
        observed_currency: None,
        attribution: AttributionScope::Unattributed,
        assignment: None,
        attribution_evidence: vec![],
        attribution_event: None,
        interval: None,
        counter: CounterEvidence {
            semantics: CounterSemantics::Delta,
            identity: None,
            initialized_since_zero: None,
            reset: None,
        },
        equivalence_evidence: vec![],
    }
}
fn attempt(name: &str) -> ExecutionAttempt {
    ExecutionAttempt {
        id: id!(AttemptId, name),
        participant: id!(ParticipantId, name),
        participant_incarnation: id!(IncarnationId, name),
        conversation: None,
        work: Some(id!(WorkId, "work")),
        start: Some(boundary(0)),
        end: None,
        result: AttemptResult::Unknown,
        termination_evidence: vec![],
        activity: vec![ActivitySegment {
            id: id!(SegmentId, name),
            interval: interval(0, 100),
            evidence: evidence(),
        }],
    }
}
fn assignment(name: &str) -> WorkAssignment {
    WorkAssignment {
        id: id!(AssignmentId, name),
        work: id!(WorkId, "work"),
        participant: id!(ParticipantId, name),
        role: AssignmentRole::Executor,
        attempt: Some(id!(AttemptId, name)),
        validity: Some(interval(0, 100)),
        activity: vec![ActivitySegment {
            id: id!(SegmentId, name),
            interval: interval(0, 100),
            evidence: evidence(),
        }],
        source_authority: ident("fixture"),
        native_assignment: id!(NativeKey, name),
        evidence: evidence(),
    }
}
fn work() -> Vec<WorkItem> {
    vec![WorkItem {
        id: id!(WorkId, "work"),
        native_key: id!(NativeKey, "work"),
        acceptance_rule: None,
        dependencies: vec![],
        outcomes: vec![],
    }]
}
fn edge(name: &str, relation: WorkRelation) -> WorkEdge {
    WorkEdge {
        id: id!(EdgeId, name),
        source_authority: ident("fixture"),
        source_instance: id!(SourceInstanceId, "source"),
        source_generation: 0,
        native_relationship: id!(NativeKey, name),
        source_event: id!(SourceKey, name),
        status: EdgeStatus::Evidenced,
        boundary: boundary(0),
        observed_at: UtcMicros(1),
        evidence: evidence(),
        relation,
    }
}
fn spawn(a: &str, b: &str) -> WorkRelation {
    WorkRelation::Spawn {
        creator: id!(AssignmentId, a),
        child: id!(AssignmentId, b),
        child_work_link: vec![],
    }
}
fn delegate(a: &str, b: &str) -> WorkRelation {
    WorkRelation::DelegateExisting {
        caller: id!(AssignmentId, a),
        executor: id!(AssignmentId, b),
        work_relation: evidence(),
    }
}
fn bound_usage(name: &str) -> UsageObservation {
    let mut u = usage(name);
    u.attempt = Some(id!(AttemptId, name));
    u.assignment = Some(id!(AssignmentId, name));
    u.attribution = AttributionScope::Assignment;
    u.attribution_evidence = evidence();
    u.interval = Some(interval(0, 10));
    u
}

#[test]
fn identity_domains_and_tuple_boundaries_are_distinct() {
    let f = factory();
    assert_ne!(
        ParticipantId::derive(&f, &[b"x"]).identity(),
        AttemptId::derive(&f, &[b"x"]).identity()
    );
    assert_ne!(
        AttemptId::derive(&f, &[b"ab", b"c"]),
        AttemptId::derive(&f, &[b"a", b"bc"])
    );
    assert_ne!(
        AttemptId::derive(&f, &[b"a"]),
        AttemptId::derive(&f, &[b"a", b""])
    );
    assert!(IdentityFactory::new(&[], ident("key")).is_err());
    assert_eq!(f_key(&f), IdentityAlgorithm::HmacSha256);
}
fn f_key(f: &IdentityFactory) -> IdentityAlgorithm {
    AttemptId::derive(f, &[b"x"]).identity().algorithm.clone()
}

#[test]
fn namespace_incarnation_and_generation_do_not_collapse() {
    let f = factory();
    let a = SourceInstanceId::derive(&f, &[b"authority", b"incarnation1"]);
    let b = SourceInstanceId::derive(&f, &[b"authority", b"incarnation2"]);
    assert_ne!(a, b);
    let mut ledger = RevisionLedger::default();
    let a = Record::new(provenance("a", 0, 1), usage("a")).unwrap();
    let mut b = a.clone();
    b.provenance.source_generation = SourceGeneration {
        number: 1,
        predecessor: Some(0),
        predecessor_evidence: evidence(),
    };
    b.provenance.fact_id = id!(FactId, "generation1");
    ledger.append(a).unwrap();
    ledger.append(b).unwrap();
    assert_eq!(
        ledger
            .at_cutoff(UtcMicros(1), RevisionPolicy::CorrectionsAtCutoffV1)
            .unwrap()
            .valid
            .len(),
        2
    );
    assert!(SourceGeneration {
        number: 1,
        predecessor: Some(0),
        predecessor_evidence: vec![]
    }
    .validate()
    .is_err());
}

#[test]
fn decimals_normalize_before_equality_serialization_and_revision_hash() {
    let a = Decimal::new(10, 100).unwrap();
    let b = Decimal::new(1, 10).unwrap();
    assert_eq!(a, b);
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
    assert_eq!(Decimal::new(0, 10000).unwrap().denominator(), 1);
    assert_eq!(
        Decimal::new(-200, 1000).unwrap(),
        Decimal::new(-2, 10).unwrap()
    );
    assert!(Decimal::new(1, 3).is_err());
    assert!(Decimal::new(1, 0).is_err());
    let json = r#"{"coefficient":10,"denominator":100}"#;
    assert_eq!(serde_json::from_str::<Decimal>(json).unwrap(), b);
    let mut a = Record::new(provenance("cost", 0, 1), usage("a")).unwrap();
    a.value.observed_currency = Some(Money {
        amount: Decimal::new(10, 100).unwrap(),
        currency: "USD".to_owned().try_into().unwrap(),
        source_scale: None,
    });
    let mut b = a.clone();
    b.value.observed_currency.as_mut().unwrap().amount = Decimal::new(1, 10).unwrap();
    assert_eq!(revision_digest(&a).unwrap(), revision_digest(&b).unwrap());
    let mut ledger = RevisionLedger::default();
    assert_eq!(ledger.append(a).unwrap(), AppendResult::Inserted);
    assert_eq!(ledger.append(b).unwrap(), AppendResult::Idempotent);
}

#[test]
fn quantities_refuse_overflow_invalid_units_and_mixed_currency() {
    assert_eq!(
        Count(u64::MAX).checked_add(Count(1)),
        Err(Refusal::Overflow)
    );
    assert_eq!(
        Decimal::new(i128::MAX, 1)
            .unwrap()
            .checked_add(Decimal::new(1, 1).unwrap()),
        Err(Refusal::Overflow)
    );
    assert!(Quantity {
        value: Count(1),
        unit: Unit::Unknown
    }
    .validate()
    .is_err());
    let money = |c: &str| Money {
        amount: Decimal::new(1, 1).unwrap(),
        currency: c.to_owned().try_into().unwrap(),
        source_scale: None,
    };
    assert!(money("USD").checked_add(&money("EUR")).is_err());
    assert!(CurrencyCode::try_from("usd".to_owned()).is_err());
    let mut r = Record::new(provenance("big", 0, 1), usage("big")).unwrap();
    r.value.observed_currency = Some(Money {
        amount: Decimal::new(i128::MAX, 1).unwrap(),
        currency: "USD".to_owned().try_into().unwrap(),
        source_scale: None,
    });
    assert!(revision_digest(&r).is_ok());
}

#[test]
fn allocation_is_reduced_exact_and_residual_is_explicit() {
    let mut a = Allocation {
        id: id!(AllocationId, "allocation"),
        observations: BTreeSet::from([id!(UsageId, "shared")]),
        beneficiaries: vec![
            Beneficiary {
                assignment: id!(AssignmentId, "a"),
                weight: Ratio::new(6, 10).unwrap(),
            },
            Beneficiary {
                assignment: id!(AssignmentId, "b"),
                weight: Ratio::new(3, 10).unwrap(),
            },
        ],
        method: RuleRef {
            id: ident("weights"),
            version: ident("1.0.0"),
        },
        authority: ident("fixture"),
        validity: None,
        evidence: evidence(),
    };
    assert_eq!(a.residual().unwrap(), Ratio::new(1, 10).unwrap());
    a.validate().unwrap();
    assert_eq!(Ratio::new(3, 6).unwrap(), Ratio::new(1, 2).unwrap());
    a.beneficiaries[0].weight = Ratio::new(7, 10).unwrap();
    a.beneficiaries[1].weight = Ratio::new(4, 10).unwrap();
    assert!(a.validate().is_err());
    assert!(Ratio::new(1, 0).is_err());
    assert!(Ratio::new(u128::MAX, 1)
        .unwrap()
        .checked_add(Ratio::ONE)
        .is_err());
}

#[test]
fn half_open_intervals_keep_signed_time_and_extreme_duration() {
    let i = interval(-2, 2);
    assert!(i.contains(UtcMicros(-2)));
    assert!(!i.contains(UtcMicros(2)));
    assert_eq!(i.duration_us(), 4);
    assert!(Interval::new(UtcMicros(2), UtcMicros(1), clock()).is_err());
    assert_eq!(interval(i64::MIN, i64::MAX).duration_us(), u64::MAX);
    let mut b = boundary(1);
    b.clock = Some(id!(ClockId, "other"));
    assert_eq!(boundary_no_later(&boundary(2), &b).unwrap(), None);
    assert_eq!(
        boundary_no_later(&boundary(2), &boundary(1)).unwrap(),
        Some(false)
    );
    b = boundary(1);
    b.error_us = Some(2);
    assert_eq!(boundary_no_later(&boundary(1), &b).unwrap(), None);
}

#[test]
fn corrections_and_retractions_respect_ingestion_cutoff() {
    let mut ledger = RevisionLedger::default();
    let a = Record::new(provenance("a", 0, 1), usage("a")).unwrap();
    let mut b = Record::new(provenance("a", 1, 3), usage("a")).unwrap();
    b.provenance.supersedes = Some(a.provenance.fact_id.clone());
    b.value.components[0].quantity = Count(2);
    let mut c = Record::new(provenance("a", 2, 5), usage("a")).unwrap();
    c.provenance.retracts = Some(b.provenance.fact_id.clone());
    ledger.append(a).unwrap();
    ledger.append(b).unwrap();
    ledger.append(c).unwrap();
    assert_eq!(
        ledger
            .at_cutoff(UtcMicros(2), RevisionPolicy::CorrectionsAtCutoffV1)
            .unwrap()
            .valid[0]
            .value
            .components[0]
            .quantity,
        Count(1)
    );
    assert_eq!(
        ledger
            .at_cutoff(UtcMicros(4), RevisionPolicy::CorrectionsAtCutoffV1)
            .unwrap()
            .valid[0]
            .value
            .components[0]
            .quantity,
        Count(2)
    );
    assert!(ledger
        .at_cutoff(UtcMicros(5), RevisionPolicy::CorrectionsAtCutoffV1)
        .unwrap()
        .valid
        .is_empty());
    assert_eq!(ledger.records().len(), 3);
}

#[test]
fn duplicate_receipts_are_idempotent_and_conflict_is_cutoff_visible() {
    let mut ledger = RevisionLedger::default();
    let a = Record::new(provenance("a", 0, 1), usage("a")).unwrap();
    ledger.append(a.clone()).unwrap();
    let mut b = a.clone();
    b.provenance.ingested_at = UtcMicros(9);
    assert_eq!(ledger.append(b.clone()).unwrap(), AppendResult::Idempotent);
    b.value.components[0].quantity = Count(2);
    assert_eq!(ledger.append(b).unwrap(), AppendResult::Conflict);
    assert_eq!(
        ledger
            .at_cutoff(UtcMicros(2), RevisionPolicy::CorrectionsAtCutoffV1)
            .unwrap()
            .valid
            .len(),
        1
    );
    let snapshot = ledger
        .at_cutoff(UtcMicros(9), RevisionPolicy::CorrectionsAtCutoffV1)
        .unwrap();
    assert!(snapshot.valid.is_empty());
    assert_eq!(snapshot.conflicts.values().next().unwrap().len(), 2);
    assert_eq!(ledger.conflicts().count(), 1);
}

#[test]
fn corrections_must_reference_exact_prior_fact_in_same_namespace() {
    let mut ledger = RevisionLedger::default();
    let a = Record::new(provenance("a", 0, 1), usage("a")).unwrap();
    ledger.append(a.clone()).unwrap();
    let b = Record::new(provenance("a", 1, 2), usage("a")).unwrap();
    assert!(ledger.append(b.clone()).is_err());
    let mut c = b;
    c.provenance.supersedes = Some(a.provenance.fact_id.clone());
    c.provenance.source_instance_id = id!(SourceInstanceId, "foreign");
    assert!(ledger.append(c).is_err());
    let mut c = a;
    c.provenance.retracts = Some(c.provenance.fact_id.clone());
    assert!(c.validate().is_err());
}

#[test]
fn usage_only_is_valid_and_does_not_create_an_invocation_or_attempt() {
    let u = usage("usage-only");
    u.validate().unwrap();
    assert!(u.attempt.is_none());
    assert!(u.invocation.is_none());
    let i = ModelInvocation {
        id: id!(InvocationId, "i"),
        provider: ident("provider"),
        native_call: id!(NativeKey, "call"),
        attempt: None,
        model_binding: None,
        start: None,
        end: None,
        status: AttemptResult::Unknown,
        call_evidence: vec![],
    };
    assert!(i.validate().is_err());
}

fn message(n: u64, role: MessageRole) -> Record<MessageObservation> {
    Record::new(
        provenance(&format!("m{n}"), 0, 1),
        MessageObservation {
            id: MessageId::derive(&factory(), &[&n.to_be_bytes()]),
            conversation: id!(ConversationId, "c"),
            invocation: None,
            attempt: None,
            direction: Direction::Inbound,
            role,
            modalities: BTreeSet::from([Modality::Text]),
            source_order: Some(n),
            boundary: boundary(n as i64),
            counts: vec![Quantity {
                value: Count(1),
                unit: Unit::Messages,
            }],
            grain: Grain::Message,
        },
    )
    .unwrap()
}
#[test]
fn first_user_requires_complete_ordered_prefix_and_no_bodies() {
    let messages = vec![
        message(0, MessageRole::System),
        message(1, MessageRole::User),
        message(2, MessageRole::Assistant),
    ];
    let mut p = SequencePrefix {
        conversation: id!(ConversationId, "c"),
        source: id!(SourceInstanceId, "source"),
        generation: 0,
        first_sequence: 0,
        through_sequence: 2,
        complete: false,
        origin: source_origin(0),
        evidence: evidence(),
    };
    assert!(first_user_message(&messages, &p).is_err());
    p.complete = true;
    assert_eq!(
        first_user_message(&messages, &p).unwrap().unwrap().value.id,
        messages[1].value.id
    );
    assert!(first_user_message(&messages[1..], &p).is_err());
    let wire = serde_json::to_string(&messages[1]).unwrap();
    let injected = wire.replacen("\"value\":{", "\"value\":{\"body\":\"raw\",", 1);
    assert!(serde_json::from_str::<Record<MessageObservation>>(&injected).is_err());
}

#[test]
fn unknown_wire_enum_and_invalid_envelope_are_rejected() {
    assert!(serde_json::from_str::<AttemptResult>("\"totally-new\"").is_err());
    assert!(Identifier::new("raw\ntext").is_err());
    assert!(Identifier::new("C:/private/path").is_err());
    assert!(Identifier::new("sk-secret").is_err());
    let mut r = Record::new(provenance("a", 0, 1), usage("a")).unwrap();
    r.provenance.evidence_ids.clear();
    assert!(
        serde_json::from_str::<Record<UsageObservation>>(&serde_json::to_string(&r).unwrap())
            .is_err()
    );
}

#[test]
fn capability_stages_require_independent_structural_evidence() {
    let mut c = CapabilityState {
        id: id!(CapabilityStateId, "state"),
        definition: id!(CapabilityId, "definition"),
        harness_binding: None,
        context_binding: None,
        stage: CapabilityStage::Available,
        scope: ProductionScope::Unknown,
        validity: None,
        structural_evidence: evidence(),
    };
    c.validate().unwrap();
    let serialized = serde_json::to_string(&c).unwrap();
    assert!(serialized.contains("available"));
    assert!(!serialized.contains("loaded"));
    c.stage = CapabilityStage::Loaded;
    c.structural_evidence.clear();
    assert!(c.validate().is_err());
}

#[test]
fn diamond_unions_shared_assignment_once_with_one_spawn_origin() {
    let assignments: Vec<_> = ["p", "a", "b", "d"].map(assignment).into();
    let attempts: Vec<_> = ["p", "a", "b", "d"].map(attempt).into();
    let work = work();
    let edges = vec![
        edge("pa", spawn("p", "a")),
        edge("pb", spawn("p", "b")),
        edge("ad", delegate("a", "d")),
        edge("bd", delegate("b", "d")),
    ];
    let graph = WorkGraph::new(&assignments, &attempts, &work, &edges).unwrap();
    assert!(graph.issues().is_empty());
    let usage: Vec<_> = ["p", "a", "b", "d"].map(bound_usage).into();
    assert_eq!(
        graph
            .inclusive_observations(&id!(AssignmentId, "p"), &usage)
            .unwrap()
            .valid
            .len(),
        4
    );
    assert_eq!(
        graph
            .inclusive_observations(&id!(AssignmentId, "a"), &usage)
            .unwrap()
            .valid
            .len(),
        2
    );
}

#[test]
fn competing_spawn_creators_quarantine_closure_preserving_direct_usage() {
    let assignments: Vec<_> = ["p", "a", "d"].map(assignment).into();
    let attempts: Vec<_> = ["p", "a", "d"].map(attempt).into();
    let work = work();
    let edges = vec![edge("pd", spawn("p", "d")), edge("ad", spawn("a", "d"))];
    let graph = WorkGraph::new(&assignments, &attempts, &work, &edges).unwrap();
    assert_eq!(graph.issues().len(), 2);
    assert!(graph
        .inclusive_assignments(&id!(AssignmentId, "p"))
        .is_err());
    assert_eq!(
        graph
            .direct_observations(&id!(AssignmentId, "d"), &[bound_usage("d")])
            .unwrap()
            .valid
            .len(),
        1
    );
}

#[test]
fn eligible_cycle_refuses_fold_but_interaction_cycle_does_not() {
    let assignments: Vec<_> = ["a", "b"].map(assignment).into();
    let attempts: Vec<_> = ["a", "b"].map(attempt).into();
    let work = work();
    let cyclic = vec![
        edge("ab", delegate("a", "b")),
        edge("ba", delegate("b", "a")),
    ];
    let graph = WorkGraph::new(&assignments, &attempts, &work, &cyclic).unwrap();
    assert_eq!(
        graph.inclusive_assignments(&id!(AssignmentId, "a")),
        Err(Refusal::Cycle)
    );
    assert_eq!(
        graph
            .direct_observations(&id!(AssignmentId, "a"), &[bound_usage("a")])
            .unwrap()
            .valid
            .len(),
        1
    );
    let mut legal = vec![
        edge("ab", delegate("a", "b")),
        edge(
            "ba",
            WorkRelation::Return {
                executor: id!(AssignmentId, "b"),
                caller: id!(AssignmentId, "a"),
                correlation: id!(EdgeId, "ab"),
                result_boundary: boundary(10),
            },
        ),
    ];
    legal[1].boundary = boundary(10);
    let graph = WorkGraph::new(&assignments, &attempts, &work, &legal).unwrap();
    assert!(graph.issues().is_empty());
    assert_eq!(
        graph
            .inclusive_assignments(&id!(AssignmentId, "b"))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn missing_endpoints_and_ineligible_edges_never_invent_roots() {
    let assignments = vec![assignment("a")];
    let attempts = vec![attempt("a")];
    let work = work();
    let mut edges = vec![edge("missing", delegate("a", "missing"))];
    let graph = WorkGraph::new(&assignments, &attempts, &work, &edges).unwrap();
    assert!(graph.issues().contains_key(&edges[0].id));
    assert!(graph
        .inclusive_assignments(&id!(AssignmentId, "a"))
        .is_err());
    edges[0].status = EdgeStatus::Truncated;
    let graph = WorkGraph::new(&assignments, &attempts, &work, &edges).unwrap();
    assert!(graph
        .inclusive_assignments(&id!(AssignmentId, "a"))
        .is_err());
}

#[test]
fn assignment_membership_does_not_expand_entire_persistent_worker() {
    let a = vec![assignment("a")];
    let t = vec![attempt("a")];
    let w = work();
    let graph = WorkGraph::new(&a, &t, &w, &[]).unwrap();
    let mut u = bound_usage("a");
    u.interval = Some(interval(90, 110));
    let ambiguous_id = u.id.clone();
    let selected = graph
        .direct_observations(&id!(AssignmentId, "a"), &[u])
        .unwrap();
    assert!(selected.valid.is_empty());
    assert_eq!(
        selected.ambiguous.get(&ambiguous_id),
        Some(&Refusal::AmbiguousMembership)
    );
    let mut unassigned = usage("other-session");
    unassigned.attempt = Some(id!(AttemptId, "a"));
    assert!(graph
        .direct_observations(&id!(AssignmentId, "a"), &[unassigned])
        .unwrap()
        .valid
        .is_empty());
}

#[test]
fn spawn_time_and_delegation_caller_interval_are_checked() {
    let a = vec![assignment("a"), assignment("b")];
    let t = vec![attempt("a"), attempt("b")];
    let w = work();
    let mut e = edge("ab", spawn("a", "b"));
    e.boundary = boundary(10);
    let graph = WorkGraph::new(&a, &t, &w, std::slice::from_ref(&e)).unwrap();
    assert_eq!(graph.issues().get(&e.id), Some(&Refusal::InvalidInterval));
    e.relation = delegate("a", "b");
    e.boundary = boundary(100);
    let graph = WorkGraph::new(&a, &t, &w, std::slice::from_ref(&e)).unwrap();
    assert_eq!(graph.issues().get(&e.id), Some(&Refusal::InvalidInterval));
}

#[test]
fn same_participant_can_hold_multiple_work_assignments() {
    let mut a = vec![assignment("a"), assignment("b")];
    a[1].participant = a[0].participant.clone();
    a[1].attempt = None;
    a[1].work = id!(WorkId, "other");
    let mut w = work();
    let mut other = w[0].clone();
    other.id = id!(WorkId, "other");
    w.push(other);
    let t = vec![attempt("a")];
    let graph = WorkGraph::new(&a, &t, &w, &[]).unwrap();
    assert_eq!(graph.inclusive_assignments(&a[1].id).unwrap().len(), 1);
}

#[test]
fn retry_and_resume_keep_attempt_identity_and_segment_order() {
    let a = vec![assignment("a"), assignment("b")];
    let mut t = vec![attempt("a"), attempt("b")];
    t[1].start = Some(boundary(11));
    let w = work();
    let e = vec![edge(
        "retry",
        WorkRelation::Retry {
            prior: t[0].id.clone(),
            later: t[1].id.clone(),
        },
    )];
    assert!(WorkGraph::new(&a, &t, &w, &e).unwrap().issues().is_empty());
    let segment = ActivitySegment {
        id: id!(SegmentId, "later"),
        interval: interval(100, 110),
        evidence: evidence(),
    };
    t[0].activity.push(segment.clone());
    let e = vec![edge(
        "resume",
        WorkRelation::Resume {
            attempt: t[0].id.clone(),
            prior: t[0].activity[0].id.clone(),
            later: segment.id,
            continuation_evidence: evidence(),
        },
    )];
    assert!(WorkGraph::new(&a, &t, &w, &e).unwrap().issues().is_empty());
}

#[test]
fn policy_absence_is_not_assessed_and_migration_is_not_claimed() {
    assert_eq!(
        dendro_core::security_v1::assess_authority(&[]).unwrap(),
        PolicyAssessment::NotAssessed
    );
    assert!(dendro_core::migration_v4::check_migration_admission(3).is_err());
}

#[test]
fn coverage_preserves_missing_denominator_and_checked_counts() {
    let mut c = Coverage {
        id: id!(CoverageId, "coverage"),
        source: id!(SourceInstanceId, "source"),
        schema: SchemaIdentity {
            id: ident("fixture"),
            version: None,
        },
        record_kind: ident("usage"),
        window: None,
        discovery: DiscoveryStatus::Present,
        read: ReadStatus::Complete,
        projection: ProjectionStatus::Supported,
        accepted: Count(0),
        rejected: Count(0),
        recognized_unprojected: Count(0),
        expected_population: None,
        denominator_basis: DenominatorBasis::Unknown,
        gaps: BTreeSet::new(),
    };
    c.validate().unwrap();
    assert_eq!(c.encountered().unwrap(), Count(0));
    c.denominator_basis = DenominatorBasis::ExpectedPopulation;
    assert!(c.validate().is_err());
    c.denominator_basis = DenominatorBasis::EncounteredRecords;
    c.accepted = Count(u64::MAX);
    c.rejected = Count(1);
    assert!(c.validate().is_err());
}

#[test]
fn namespace_identity_validation_rejects_cross_source_records() {
    let f = factory();
    let authority = ident("authority");
    let product = ident("product");
    let incarnation = id!(IncarnationId, "incarnation");
    let source = SourceInstance {
        id: SourceInstanceId::scoped(&f, &authority, &product, &incarnation),
        authority,
        product,
        producer_version: None,
        producer_incarnation: incarnation,
        capture_mode: CaptureMode::Export,
        schema: SchemaIdentity {
            id: ident("fixture"),
            version: None,
        },
        configured_root: None,
        trust: Trust::Unverified,
    };
    source.validate_identity(&f).unwrap();
    let mut r = Record::new(provenance("a", 0, 1), usage("a")).unwrap();
    assert_eq!(
        r.validate_source(&source, &f),
        Err(Refusal::InvalidReference)
    );
    r.provenance.source_instance_id = source.id.clone();
    r.validate_source(&source, &f).unwrap();
    let mut changed = source.clone();
    changed.producer_incarnation = id!(IncarnationId, "new-life");
    assert!(changed.validate_identity(&f).is_err());
    assert_ne!(
        SourceKey::scoped(&f, &source.id, 0, b"row"),
        SourceKey::scoped(&f, &source.id, 1, b"row")
    );
}

#[test]
fn source_order_is_local_and_conflicting_clock_order_is_not_accepted() {
    let order = |n| SourceOrder {
        source_instance: id!(SourceInstanceId, "source"),
        generation: 0,
        sequence: n,
    };
    let mut a = boundary(10);
    a.clock = None;
    a.error_us = None;
    a.order = Some(order(1));
    let mut b = a.clone();
    b.order = Some(order(2));
    assert_eq!(boundary_no_later(&a, &b).unwrap(), Some(true));
    b.order.as_mut().unwrap().generation = 1;
    assert_eq!(boundary_no_later(&a, &b).unwrap(), None);
    a = boundary(20);
    a.order = Some(order(1));
    b = boundary(10);
    b.order = Some(order(2));
    assert_eq!(boundary_no_later(&a, &b), Err(Refusal::Conflict));
}

#[test]
fn handoff_and_join_do_not_expand_closure_and_transfer_conflicts_remain_visible() {
    let a = vec![assignment("a"), assignment("b")];
    let t = vec![attempt("a"), attempt("b")];
    let w = work();
    let mut e = vec![
        edge(
            "handoff",
            WorkRelation::Handoff {
                source: a[0].id.clone(),
                successor: a[1].id.clone(),
                transfer_evidence: evidence(),
            },
        ),
        edge(
            "join",
            WorkRelation::Join {
                waiting: a[0].id.clone(),
                children: BTreeSet::from([a[1].id.clone()]),
                receipt: evidence(),
            },
        ),
    ];
    let g = WorkGraph::new(&a, &t, &w, &e).unwrap();
    assert!(g.issues().is_empty());
    assert_eq!(g.inclusive_assignments(&a[0].id).unwrap().len(), 1);
    let mut conflict = e[0].clone();
    conflict.id = id!(EdgeId, "handoff2");
    conflict.relation = WorkRelation::Handoff {
        source: a[1].id.clone(),
        successor: a[0].id.clone(),
        transfer_evidence: evidence(),
    };
    e.push(conflict);
    let g = WorkGraph::new(&a, &t, &w, &e).unwrap();
    assert_eq!(g.effective_status(&e[0].id).unwrap(), EdgeStatus::Conflict);
    assert_eq!(e[0].status, EdgeStatus::Evidenced);
}

#[test]
fn dependency_cycles_do_not_destroy_independent_assignment_direct_facts() {
    let a = vec![assignment("a")];
    let t = vec![attempt("a")];
    let mut w = work();
    let mut other = w[0].clone();
    other.id = id!(WorkId, "other");
    w.push(other);
    let e = vec![
        edge(
            "forward",
            WorkRelation::Dependency {
                prerequisite: w[0].id.clone(),
                dependent: w[1].id.clone(),
                validity: Some(interval(0, 10)),
                satisfaction: None,
            },
        ),
        edge(
            "backward",
            WorkRelation::Dependency {
                prerequisite: w[1].id.clone(),
                dependent: w[0].id.clone(),
                validity: Some(interval(0, 10)),
                satisfaction: None,
            },
        ),
    ];
    let g = WorkGraph::new(&a, &t, &w, &e).unwrap();
    assert_eq!(g.issues().len(), 2);
    assert!(g.issues().values().all(|r| *r == Refusal::Cycle));
    assert_eq!(
        g.direct_observations(&a[0].id, &[bound_usage("a")])
            .unwrap()
            .valid
            .len(),
        1
    );
}

#[test]
fn fragment_cannot_become_first_user_message() {
    let mut m = message(0, MessageRole::User);
    m.value.grain = Grain::Fragment;
    let p = SequencePrefix {
        conversation: m.value.conversation.clone(),
        source: m.provenance.source_instance_id.clone(),
        generation: 0,
        first_sequence: 0,
        through_sequence: 0,
        complete: true,
        origin: source_origin(0),
        evidence: evidence(),
    };
    assert!(first_user_message(&[m], &p).is_err());
}

#[test]
fn conflicting_usage_identity_is_not_silently_set_deduplicated() {
    let a = vec![assignment("a")];
    let t = vec![attempt("a")];
    let w = work();
    let g = WorkGraph::new(&a, &t, &w, &[]).unwrap();
    let u = bound_usage("a");
    let mut conflict = u.clone();
    conflict.components[0].quantity = Count(2);
    assert_eq!(
        g.direct_observations(&a[0].id, &[u, conflict]),
        Err(Refusal::Conflict)
    );
}

#[test]
fn platform_units_and_effect_assertions_keep_distinct_meanings() {
    let mut e = EnvironmentObservation {
        id: id!(EnvironmentObservationId, "sample"),
        placement: None,
        process: Some(ProcessIdentity {
            environment: id!(EnvironmentId, "host"),
            pid: 123,
            creation_marker: CreationMarker::LinuxStartTicks(77),
        }),
        kind: EnvironmentObservationKind::Resource,
        quantity: Some(Quantity {
            value: Count(10),
            unit: Unit::LinuxRssBytes,
        }),
        interval: None,
        operation: None,
        resource: None,
        result: ActionStatus::Unknown,
        capture_method: CaptureMethod::Procfs,
        visibility_limits: BTreeSet::new(),
    };
    e.validate().unwrap();
    e.quantity.as_mut().unwrap().unit = Unit::WindowsWorkingSetBytes;
    assert!(e.validate().is_err());
    e.quantity = None;
    e.kind = EnvironmentObservationKind::Syscall;
    e.operation = Some(Operation::Write);
    e.resource = Some(ResourceClass::File);
    e.capture_method = CaptureMethod::ProducerAssertion;
    assert!(e.validate().is_err());
    e.capture_method = CaptureMethod::Strace;
    e.validate().unwrap();
}

#[test]
fn unknown_versions_and_invalid_semver_do_not_become_defaults() {
    assert!(SemanticVersion::try_from("1.2.3-alpha.1+build.2".to_owned()).is_ok());
    for s in ["1.2", "01.2.3", "1.2.3-alpha.01", "1.2.3+", "1.2.3\n"] {
        assert!(SemanticVersion::try_from(s.to_owned()).is_err());
    }
    let state = CapabilityState {
        id: id!(CapabilityStateId, "state"),
        definition: id!(CapabilityId, "def"),
        harness_binding: None,
        context_binding: None,
        stage: CapabilityStage::Defined,
        scope: ProductionScope::Unknown,
        validity: None,
        structural_evidence: evidence(),
    };
    assert_eq!(state.scope, ProductionScope::Unknown);
}

#[test]
fn review_f1_concurrent_creator_assignments_on_one_attempt_conflict() {
    let mut assignments = vec![assignment("p1"), assignment("p2"), assignment("c")];
    let mut shared = attempt("p1");
    shared.work = None;
    assignments[1].participant = assignments[0].participant.clone();
    assignments[1].attempt = assignments[0].attempt.clone();
    assignments[1].work = id!(WorkId, "w2");
    let attempts = vec![shared, attempt("c")];
    let mut works = work();
    let mut w2 = works[0].clone();
    w2.id = id!(WorkId, "w2");
    works.push(w2);
    let mut edges = vec![
        edge("p1c", spawn("p1", "c")),
        edge("p1c-again", spawn("p1", "c")),
    ];
    let repeated = WorkGraph::new(&assignments, &attempts, &works, &edges).unwrap();
    assert!(repeated.issues().is_empty());
    edges.push(edge(
        "p2c",
        WorkRelation::Spawn {
            creator: assignments[1].id.clone(),
            child: assignments[2].id.clone(),
            child_work_link: evidence(),
        },
    ));
    let graph = WorkGraph::new(&assignments, &attempts, &works, &edges).unwrap();
    assert_eq!(graph.issues().len(), 3);
    for e in &edges {
        assert_eq!(graph.effective_status(&e.id).unwrap(), EdgeStatus::Conflict);
    }
    for a in &assignments[..2] {
        assert_eq!(graph.inclusive_assignments(&a.id), Err(Refusal::Conflict));
    }
    assert_eq!(
        graph
            .direct_observations(&assignments[2].id, &[bound_usage("c")])
            .unwrap()
            .valid
            .len(),
        1
    );
}

#[test]
fn review_f2_assignment_presence_must_agree_with_declared_scope() {
    for scope in [
        AttributionScope::Shared,
        AttributionScope::Unattributed,
        AttributionScope::Invocation,
        AttributionScope::Attempt,
        AttributionScope::Legacy,
    ] {
        let mut u = bound_usage("a");
        u.invocation = Some(id!(InvocationId, "call"));
        u.attribution = scope;
        assert_eq!(u.validate(), Err(Refusal::InvalidReference), "{scope:?}");
    }
    let mut u = bound_usage("a");
    u.assignment = None;
    assert_eq!(u.validate(), Err(Refusal::InvalidReference));
    u.attribution = AttributionScope::Shared;
    u.validate().unwrap();
}

#[test]
fn review_f3_conflicts_are_chain_scoped_and_both_contents_remain_visible() {
    let mut ledger = RevisionLedger::default();
    let ka = Record::new(provenance("ka", 0, 1), usage("ka")).unwrap();
    let kb = Record::new(provenance("kb", 0, 1), usage("kb")).unwrap();
    let mut divergent = kb.clone();
    divergent.provenance.ingested_at = UtcMicros(5);
    divergent.value.components[0].quantity = Count(2);
    ledger.append(ka.clone()).unwrap();
    ledger.append(kb.clone()).unwrap();
    assert_eq!(ledger.append(divergent).unwrap(), AppendResult::Conflict);
    let before = ledger
        .at_cutoff(UtcMicros(4), RevisionPolicy::CorrectionsAtCutoffV1)
        .unwrap();
    assert_eq!(before.valid.len(), 2);
    assert!(before.conflicts.is_empty());
    let after = ledger
        .at_cutoff(UtcMicros(10), RevisionPolicy::CorrectionsAtCutoffV1)
        .unwrap();
    assert_eq!(after.valid.len(), 1);
    assert_eq!(after.valid[0].provenance.fact_id, ka.provenance.fact_id);
    let (chain, contents) = after.conflicts.iter().next().unwrap();
    assert_eq!(chain.source_key, kb.provenance.source_key);
    assert_eq!(contents.len(), 2);
    let values: BTreeSet<_> = contents
        .iter()
        .map(|r| r.value.components[0].quantity.0)
        .collect();
    assert_eq!(values, BTreeSet::from([1, 2]));
    // No source-wide quarantine: a later correction on the independent key works.
    let mut correction = ka.clone();
    correction.provenance = provenance("ka", 1, 11);
    correction.provenance.supersedes = Some(ka.provenance.fact_id);
    correction.value.components[0].quantity = Count(7);
    ledger.append(correction).unwrap();
    let later = ledger
        .at_cutoff(UtcMicros(12), RevisionPolicy::CorrectionsAtCutoffV1)
        .unwrap();
    assert_eq!(later.valid[0].value.components[0].quantity, Count(7));
    assert_eq!(later.conflicts.len(), 1);
}

#[test]
fn review_f4_harmless_identifier_substrings_are_not_secret_prefixes() {
    for s in [
        "task-1",
        "disk-0",
        "risk-high",
        "desk-app",
        "disk-key:task-1",
    ] {
        assert!(Identifier::new(s).is_ok(), "{s}");
    }
    for s in [
        "sk-secret",
        "x:sk-secret",
        "x@ghp_secret",
        "x.gho_secret",
        "x-xai-secret",
    ] {
        assert_eq!(Identifier::new(s), Err(Refusal::InvalidIdentifier), "{s}");
    }
}

#[test]
fn review_f5_first_user_requires_explicit_origin_including_nonzero_origins() {
    let messages = vec![message(5, MessageRole::User)];
    let mut p = SequencePrefix {
        conversation: id!(ConversationId, "c"),
        source: id!(SourceInstanceId, "source"),
        generation: 0,
        first_sequence: 5,
        through_sequence: 5,
        complete: true,
        origin: None,
        evidence: evidence(),
    };
    assert!(first_user_message(&messages, &p).is_err());
    p.origin = source_origin(0);
    assert_eq!(
        first_user_message(&messages, &p),
        Err(Refusal::InvalidReference)
    );
    p.origin = source_origin(5);
    assert_eq!(
        first_user_message(&messages, &p).unwrap().unwrap().value.id,
        messages[0].value.id
    );
    p.origin.as_mut().unwrap().evidence.clear();
    assert_eq!(
        first_user_message(&messages, &p),
        Err(Refusal::MissingEvidence)
    );
    p.origin = source_origin(5);
    p.origin.as_mut().unwrap().source = id!(SourceInstanceId, "other");
    assert_eq!(
        first_user_message(&messages, &p),
        Err(Refusal::InvalidReference)
    );
}

#[test]
fn review_f6_unordered_evidence_and_components_canonicalize_without_mutation() {
    let e1 = id!(EvidenceId, "one");
    let e2 = id!(EvidenceId, "two");
    let mut a = Record::new(provenance("canon", 0, 1), usage("canon")).unwrap();
    a.provenance.evidence_ids = vec![e1.clone(), e2.clone()];
    a.provenance.source_generation = SourceGeneration {
        number: 1,
        predecessor: Some(0),
        predecessor_evidence: vec![e1.clone(), e2.clone()],
    };
    a.value.components.push(UsageComponent {
        component: TokenComponent::Output,
        quantity: Count(2),
        inclusion: Inclusion::Exclusive,
    });
    a.value.equivalence_evidence = vec![e1.clone(), e2.clone()];
    let original = serde_json::to_vec(&a).unwrap();
    let mut b = a.clone();
    b.provenance.evidence_ids = vec![e2.clone(), e1.clone(), e2.clone()];
    b.provenance
        .source_generation
        .predecessor_evidence
        .reverse();
    b.value.components.reverse();
    b.value.equivalence_evidence.reverse();
    assert_eq!(revision_digest(&a).unwrap(), revision_digest(&b).unwrap());
    assert_eq!(serde_json::to_vec(&a).unwrap(), original);
    let mut ledger = RevisionLedger::default();
    ledger.append(a).unwrap();
    assert_eq!(ledger.append(b).unwrap(), AppendResult::Idempotent);
    let mut msg = message(0, MessageRole::User);
    msg.value.counts = vec![
        Quantity {
            value: Count(1),
            unit: Unit::Messages,
        },
        Quantity {
            value: Count(2),
            unit: Unit::Bytes,
        },
    ];
    let mut reordered = msg.clone();
    reordered.value.counts.reverse();
    assert_eq!(
        revision_digest(&msg).unwrap(),
        revision_digest(&reordered).unwrap()
    );
    reordered
        .value
        .counts
        .push(reordered.value.counts[0].clone());
    assert_ne!(
        revision_digest(&msg).unwrap(),
        revision_digest(&reordered).unwrap()
    );
}

#[test]
fn review_f6_edge_receipts_are_excluded_but_event_order_and_activity_order_remain() {
    let mut a = Record::new(provenance("edge", 0, 1), edge("edge", delegate("a", "b"))).unwrap();
    let mut b = a.clone();
    b.value.observed_at = UtcMicros(90);
    b.provenance.observed_at = UtcMicros(90);
    b.provenance.ingested_at = UtcMicros(90);
    b.value.evidence.extend(evidence());
    assert_eq!(revision_digest(&a).unwrap(), revision_digest(&b).unwrap());
    let mut ledger = RevisionLedger::default();
    ledger.append(a.clone()).unwrap();
    assert_eq!(ledger.append(b).unwrap(), AppendResult::Idempotent);
    a.value.boundary.order = Some(SourceOrder {
        source_instance: id!(SourceInstanceId, "source"),
        generation: 0,
        sequence: 2,
    });
    assert_ne!(
        revision_digest(&a).unwrap(),
        revision_digest(&ledger.records()[0]).unwrap()
    );
    let mut ordered = Record::new(provenance("attempt", 0, 1), attempt("a")).unwrap();
    ordered.value.activity.push(ActivitySegment {
        id: id!(SegmentId, "second"),
        interval: interval(100, 110),
        evidence: evidence(),
    });
    let mut reversed = ordered.clone();
    reversed.value.activity.reverse();
    assert_ne!(
        revision_digest(&ordered).unwrap(),
        revision_digest(&reversed).unwrap()
    );
}

#[test]
fn review_f7_partial_membership_retains_valid_and_ambiguous_rows_in_ancestors() {
    let a = vec![assignment("p"), assignment("a")];
    let t = vec![attempt("p"), attempt("a")];
    let w = work();
    let e = vec![edge("pa", delegate("p", "a"))];
    let g = WorkGraph::new(&a, &t, &w, &e).unwrap();
    let valid = bound_usage("a");
    let mut straddle = valid.clone();
    straddle.id = id!(UsageId, "straddle");
    straddle.interval = Some(interval(90, 110));
    let mut missing = valid.clone();
    missing.id = id!(UsageId, "missing-attempt");
    missing.attempt = None;
    let mut contradictory = valid.clone();
    contradictory.id = id!(UsageId, "shared-with-assignment");
    contradictory.attribution = AttributionScope::Shared;
    let rows = vec![
        valid.clone(),
        straddle.clone(),
        missing.clone(),
        contradictory.clone(),
    ];
    let direct = g.direct_observations(&a[1].id, &rows).unwrap();
    assert_eq!(direct.valid, BTreeSet::from([valid.id]));
    assert_eq!(direct.ambiguous.len(), 3);
    assert_eq!(
        direct.ambiguous.get(&straddle.id),
        Some(&Refusal::AmbiguousMembership)
    );
    assert_eq!(
        direct.ambiguous.get(&missing.id),
        Some(&Refusal::AmbiguousMembership)
    );
    assert_eq!(
        direct.ambiguous.get(&contradictory.id),
        Some(&Refusal::InvalidReference)
    );
    assert_eq!(g.inclusive_observations(&a[0].id, &rows).unwrap(), direct);
    assert_eq!(rows[1].interval.as_ref().unwrap().end(), UtcMicros(110));
    assert_eq!(rows[1].assignment, straddle.assignment);
}

#[test]
fn review_f8_source_scale_survives_serialization_but_not_canonical_identity() {
    let currency = || CurrencyCode::try_from("USD".to_owned()).unwrap();
    for (coefficient, denominator, canonical, scale) in [
        (10, 100, Decimal::new(1, 10).unwrap(), 2),
        (0, 100, Decimal::new(0, 1).unwrap(), 2),
        (-250, 100, Decimal::new(-25, 10).unwrap(), 2),
    ] {
        let money = Money::from_source(coefficient, denominator, currency()).unwrap();
        assert_eq!(money.amount, canonical);
        assert_eq!(money.source_scale.unwrap().decimal_places(), scale);
        let decoded: Money = serde_json::from_str(&serde_json::to_string(&money).unwrap()).unwrap();
        assert_eq!(decoded.source_scale, money.source_scale);
    }
    let a = Money::from_source(10, 100, currency()).unwrap();
    let b = Money::from_source(1, 10, currency()).unwrap();
    assert_eq!(a, b);
    assert_ne!(a.source_scale, b.source_scale);
    let mut ra = Record::new(provenance("precision", 0, 1), usage("precision")).unwrap();
    ra.value.observed_currency = Some(a.clone());
    let mut rb = ra.clone();
    rb.value.observed_currency = Some(b.clone());
    assert_eq!(revision_digest(&ra).unwrap(), revision_digest(&rb).unwrap());
    let mut ledger = RevisionLedger::default();
    ledger.append(ra).unwrap();
    assert_eq!(ledger.append(rb).unwrap(), AppendResult::Idempotent);
    assert_eq!(
        ledger.records()[0]
            .value
            .observed_currency
            .as_ref()
            .unwrap()
            .source_scale,
        a.source_scale
    );
    assert!(a.checked_add(&b).unwrap().source_scale.is_none());
    assert!(SourceScale::new(39).is_err());
    let mut invalid = b;
    invalid.source_scale = Some(SourceScale::new(0).unwrap());
    assert_eq!(invalid.validate(), Err(Refusal::InvalidQuantity));
}

#[test]
fn review_f9_reingestion_never_redates_the_first_admitted_receipt() {
    let mut ledger = RevisionLedger::default();
    let a = Record::new(provenance("first-admitted", 0, 9), usage("first-admitted")).unwrap();
    let mut backdated = a.clone();
    backdated.provenance.observed_at = UtcMicros(1);
    backdated.provenance.ingested_at = UtcMicros(1);
    ledger.append(a).unwrap();
    assert_eq!(ledger.append(backdated).unwrap(), AppendResult::Idempotent);
    assert!(ledger
        .at_cutoff(UtcMicros(5), RevisionPolicy::CorrectionsAtCutoffV1)
        .unwrap()
        .valid
        .is_empty());
    assert_eq!(
        ledger
            .at_cutoff(UtcMicros(9), RevisionPolicy::CorrectionsAtCutoffV1)
            .unwrap()
            .valid[0]
            .provenance
            .ingested_at,
        UtcMicros(9)
    );
}

#[test]
fn review_f7_partial_partitions_remain_disjoint_for_invalid_precision_metadata() {
    let assignments = vec![assignment("a")];
    let attempts = vec![attempt("a")];
    let works = work();
    let graph = WorkGraph::new(&assignments, &attempts, &works, &[]).unwrap();
    let mut original = bound_usage("a");
    original.observed_currency =
        Some(Money::from_source(1, 10, "USD".to_owned().try_into().unwrap()).unwrap());
    let mut malformed = original.clone();
    malformed.observed_currency.as_mut().unwrap().source_scale = Some(SourceScale::new(0).unwrap());
    let mut independent = bound_usage("a");
    independent.id = id!(UsageId, "independent");
    let selected = graph
        .direct_observations(
            &assignments[0].id,
            &[original.clone(), malformed, independent.clone()],
        )
        .unwrap();
    assert_eq!(selected.valid, BTreeSet::from([independent.id]));
    assert_eq!(
        selected.ambiguous.get(&original.id),
        Some(&Refusal::InvalidQuantity)
    );
    assert!(selected
        .valid
        .iter()
        .all(|id| !selected.ambiguous.contains_key(id)));
}

#[test]
fn review_g1_unrelated_usage_conflicts_preserve_direct_and_inclusive_results() {
    let assignments = vec![assignment("p"), assignment("a"), assignment("b")];
    let attempts = vec![attempt("p"), attempt("a"), attempt("b")];
    let works = work();
    let edges = vec![edge("pa", delegate("p", "a"))];
    let graph = WorkGraph::new(&assignments, &attempts, &works, &edges).unwrap();
    let local = bound_usage("a");
    let unrelated = usage("unattributed");
    let mut conflicting = unrelated.clone();
    conflicting.components[0].quantity = Count(9);
    let other = bound_usage("b");
    let mut other_conflict = other.clone();
    other_conflict.components[0].quantity = Count(9);
    let mut rows = vec![
        local.clone(),
        unrelated,
        conflicting,
        other,
        other_conflict,
        local.clone(),
    ];
    for _ in 0..2 {
        let direct = graph
            .direct_observations(&assignments[1].id, &rows)
            .unwrap();
        assert_eq!(direct.valid, BTreeSet::from([local.id.clone()]));
        assert!(direct.ambiguous.is_empty());
        assert_eq!(
            graph
                .inclusive_observations(&assignments[0].id, &rows)
                .unwrap(),
            direct
        );
        assert_eq!(
            graph.direct_observations(&assignments[2].id, &rows),
            Err(Refusal::Conflict)
        );
        rows.reverse();
    }
}

#[test]
fn review_g1_relevant_ids_compare_all_copies_in_both_orders() {
    let assignments = vec![assignment("a"), assignment("b")];
    let attempts = vec![attempt("a"), attempt("b")];
    let works = work();
    let graph = WorkGraph::new(&assignments, &attempts, &works, &[]).unwrap();
    let local = bound_usage("a");
    let mut unattributed = local.clone();
    unattributed.assignment = None;
    unattributed.attribution = AttributionScope::Unattributed;
    let mut other = bound_usage("b");
    other.id = local.id.clone();
    for copy in [unattributed, other] {
        for rows in [vec![local.clone(), copy.clone()], vec![copy, local.clone()]] {
            assert_eq!(
                graph.direct_observations(&assignments[0].id, &rows),
                Err(Refusal::Conflict)
            );
        }
    }
}

#[test]
fn review_g2_alternative_idempotence_and_cutoffs_hold_in_both_orders() {
    let record = |amount, time| {
        let mut value = usage("g2");
        value.components[0].quantity = Count(amount);
        Record::new(provenance("g2", 0, time), value).unwrap()
    };
    for (original_amount, alternative_amount) in [(1, 2), (2, 1)] {
        for (original_time, alternative_time) in [(9, 3), (3, 9)] {
            let mut ledger = RevisionLedger::default();
            assert_eq!(
                ledger
                    .append(record(original_amount, original_time))
                    .unwrap(),
                AppendResult::Inserted
            );
            assert_eq!(
                ledger
                    .append(record(alternative_amount, alternative_time))
                    .unwrap(),
                AppendResult::Conflict
            );
            // Earlier and later duplicate receipts must neither duplicate nor redate
            // either content, independent of which content was admitted first.
            for time in [1, 40] {
                assert_eq!(
                    ledger.append(record(alternative_amount, time)).unwrap(),
                    AppendResult::Idempotent
                );
                assert_eq!(
                    ledger.append(record(original_amount, time)).unwrap(),
                    AppendResult::Idempotent
                );
            }
            assert_eq!(ledger.records().len(), 1);
            assert_eq!(ledger.conflicts().count(), 1);
            assert_eq!(
                ledger.conflicts().next().unwrap().provenance.ingested_at,
                UtcMicros(alternative_time)
            );
            for cutoff in [-1, 2, 3, 5, 8, 9, 10] {
                let snapshot = ledger
                    .at_cutoff(UtcMicros(cutoff), RevisionPolicy::CorrectionsAtCutoffV1)
                    .unwrap();
                if cutoff < original_time.max(alternative_time) {
                    assert!(snapshot.conflicts.is_empty());
                    assert_eq!(snapshot.valid.len(), usize::from(cutoff >= original_time));
                    if let Some(r) = snapshot.valid.first() {
                        assert_eq!(r.value.components[0].quantity, Count(original_amount));
                    }
                } else {
                    assert!(snapshot.valid.is_empty());
                    assert_eq!(snapshot.conflicts.len(), 1);
                    let contents = snapshot.conflicts.values().next().unwrap();
                    assert_eq!(contents.len(), 2);
                    assert_eq!(
                        contents
                            .iter()
                            .map(|r| r.value.components[0].quantity.0)
                            .collect::<BTreeSet<_>>(),
                        BTreeSet::from([1, 2])
                    );
                }
            }
            assert_eq!(
                ledger.append(record(3, 12)).unwrap(),
                AppendResult::Conflict
            );
            assert_eq!(
                ledger.append(record(3, 1)).unwrap(),
                AppendResult::Idempotent
            );
            assert_eq!(
                ledger
                    .at_cutoff(UtcMicros(10), RevisionPolicy::CorrectionsAtCutoffV1)
                    .unwrap()
                    .conflicts
                    .values()
                    .next()
                    .unwrap()
                    .len(),
                2
            );
            let at12 = ledger
                .at_cutoff(UtcMicros(12), RevisionPolicy::CorrectionsAtCutoffV1)
                .unwrap();
            assert_eq!(
                at12.conflicts
                    .values()
                    .next()
                    .unwrap()
                    .iter()
                    .map(|r| r.value.components[0].quantity.0)
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from([1, 2, 3])
            );
            assert_eq!(ledger.conflicts().count(), 2);
        }
    }
}
