//! Scope, attribution and safe replay operands shared by native and adapted inputs.
use super::*;
use crate::metrics::{self, Window};
use serde_json::Value;

pub(super) fn binding<'a>(
    facts: &'a RawOverviewFacts<'_>,
    h: &str,
    s: &str,
    at: DateTime<Utc>,
) -> Option<(&'a str, &'a str)> {
    let mut result = None;
    let identities = facts.identity_facts;
    let start =
        identities.partition_point(|id| (id.harness.as_str(), id.native_session.as_str()) < (h, s));
    for id in identities[start..]
        .iter()
        .take_while(|i| i.harness == h && i.native_session == s)
        .filter(|i| i.ts.is_some_and(|ts| ts <= at))
    {
        let value = (id.rig.as_str(), id.seat.as_str());
        if result.is_some_and(|prior| prior != value) {
            return None;
        }
        result = Some(value);
    }
    result
}

pub(super) fn session_in_scope(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
    h: &str,
    s: &str,
    at: Option<DateTime<Utc>>,
) -> bool {
    if req.rig.is_none() {
        return true;
    }
    binding(facts, h, s, at.unwrap_or(req.to)).is_some_and(|(r, _)| Some(r) == req.rig.as_deref())
}

pub(super) fn seat_in_scope(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
    seat: &str,
    at: DateTime<Utc>,
) -> bool {
    if req.rig.is_none() {
        return true;
    }
    let mut result = None;
    for id in facts
        .identity_facts
        .iter()
        .filter(|i| i.seat == seat && i.ts.is_some_and(|ts| ts <= at))
    {
        if result.is_some_and(|r| r != id.rig.as_str()) {
            return false;
        }
        result = Some(id.rig.as_str());
    }
    result == req.rig.as_deref()
}

pub(super) fn positive_issue_id(raw: &str) -> bool {
    let id = raw.strip_prefix("issue:").unwrap_or(raw);
    !id.is_empty()
        && id.bytes().all(|b| b.is_ascii_digit())
        && id.parse::<u64>().is_ok_and(|n| n > 0)
}

pub(super) fn completion_reason(reason: Option<&str>) -> bool {
    // Native legacy successful closure sentinel plus explicit completed records.
    // Unknown/open text, handoffs, denials and escalation are never completion.
    matches!(reason, Some("completed" | "complete" | "no-follow-on"))
}

#[derive(Clone, Copy)]
struct SupportingEvent<'a> {
    key: &'a str,
    harness: &'a str,
    session: &'a str,
    ts: Option<DateTime<Utc>>,
}
type SkillSupportIndex<'a> = BTreeMap<&'a str, Vec<SupportingEvent<'a>>>;
fn skill_support_index<'a>(facts: &'a RawOverviewFacts<'_>) -> SkillSupportIndex<'a> {
    fn insert<'a>(index: &mut SkillSupportIndex<'a>, event: SupportingEvent<'a>) {
        index.entry(event.key).or_default().push(event);
        if let Some((parent, suffix)) = event.key.rsplit_once(':') {
            if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
                index.entry(parent).or_default().push(event);
            }
        }
    }
    let mut index = BTreeMap::new();
    for u in facts.usage_facts {
        insert(
            &mut index,
            SupportingEvent {
                key: &u.source_key,
                harness: &u.harness,
                session: &u.native_session,
                ts: u.ts(),
            },
        );
    }
    for t in facts.tool_facts {
        insert(
            &mut index,
            SupportingEvent {
                key: &t.source_key,
                harness: &t.harness,
                session: &t.native_session,
                ts: t.ts(),
            },
        );
    }
    index
}
fn skill_support(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
    skill: &SkillFact,
    index: &SkillSupportIndex<'_>,
) -> Option<(String, DateTime<Utc>, String)> {
    let parent = skill.source_key.rsplit_once(":skill:").map(|(p, _)| p)?;
    let mut support = BTreeSet::new();
    for event in index.get(parent).into_iter().flatten() {
        let Some(ts) = event.ts.filter(|ts| *ts >= req.from && *ts < req.to) else {
            continue;
        };
        if let Some((rig, seat)) = binding(facts, event.harness, event.session, ts) {
            if req.rig.as_deref().is_none_or(|r| r == rig)
                && (skill.seat == seat
                    || skill.seat == format!("{}:{}", event.harness, event.session))
            {
                support.insert((
                    format!("{}\0{}\0{}", event.harness, event.session, seat),
                    ts,
                    event.key.to_owned(),
                ));
            }
        }
    }
    let grains: BTreeSet<_> = support.iter().map(|(grain, ts, _)| (grain, *ts)).collect();
    if grains.len() != 1 {
        return None;
    }
    support.into_iter().next()
}

pub(super) fn context_tax(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
) -> (ContextTaxOverview, Vec<String>) {
    let mut yields: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    let mut invocations = BTreeSet::new();
    let mut invocation_keys = BTreeSet::new();
    let mut excluded = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for t in facts.tool_facts {
        if !seen.insert(t.source_key.as_str()) {
            continue;
        }
        let Some(ts) = t.ts().filter(|ts| *ts >= req.from && *ts < req.to) else {
            continue;
        };
        let bound = binding(facts, &t.harness, &t.native_session, ts);
        if req
            .rig
            .as_deref()
            .is_some_and(|r| bound.is_none_or(|(rig, _)| rig != r))
        {
            continue;
        }
        let counts = yields.entry(t.tool_name.clone()).or_default();
        counts.0 += 1;
        counts.1 += usize::from(t.failed == Some(true));
        counts.2 += usize::from(t.failed.is_none());
        invocation_keys.insert(t.source_key.clone());
        if let Some((_, seat)) = bound {
            invocations.insert((
                format!("{}\0{}\0{}", t.harness, t.native_session, seat),
                t.tool_name.to_ascii_lowercase(),
                ts,
            ));
        }
    }
    let support_index = skill_support_index(facts);
    let mut declarations = BTreeMap::<(String, String), Vec<(DateTime<Utc>, String)>>::new();
    let mut supported_skill_keys = BTreeSet::new();
    for skill in facts.skill_facts {
        if let Some((grain, ts, key)) = skill_support(facts, req, skill, &support_index) {
            supported_skill_keys.insert(key);
            if skill.invoked {
                invocations.insert((grain, skill.skill.to_ascii_lowercase(), ts));
                invocation_keys.insert(skill.source_key.clone());
            } else {
                declarations
                    .entry((grain, skill.skill.to_ascii_lowercase()))
                    .or_default()
                    .push((ts, skill.source_key.clone()));
            }
        } else if seat_in_scope(facts, req, &skill.seat, req.to)
            || facts.identity_facts.iter().any(|id| {
                skill.seat == format!("{}:{}", id.harness, id.native_session)
                    && req.rig.as_deref().is_none_or(|r| r == id.rig)
            })
        {
            excluded.insert(skill.source_key.clone());
        }
    }
    let has_declarations = !declarations.is_empty();
    let mut declaration_keys = BTreeSet::new();
    let mut uncalled = BTreeSet::new();
    for ((grain, name), rows) in declarations {
        let first = rows.iter().map(|(ts, _)| *ts).min().unwrap();
        let reconciled = invocations
            .iter()
            .any(|(g, n, ts)| g == &grain && n == &name && *ts >= first);
        if !reconciled {
            uncalled.insert(name);
            declaration_keys.extend(rows.into_iter().map(|(_, key)| key));
        }
    }
    let mut active_tools: Vec<_> = yields
        .into_iter()
        .map(
            |(tool_name, (invocation_count, error_count, unknown_outcome_count))| ActiveTool {
                tool_name,
                invocation_count,
                error_count,
                unknown_outcome_count,
                // Error rate over observed outcomes; missing outcomes are disclosed separately.
                error_rate: (invocation_count > unknown_outcome_count).then(|| {
                    error_count as f64 / (invocation_count - unknown_outcome_count) as f64
                }),
            },
        )
        .collect();
    active_tools.sort_by(|a, b| {
        b.invocation_count
            .cmp(&a.invocation_count)
            .then_with(|| a.tool_name.cmp(&b.tool_name))
    });
    let active_status = if active_tools.is_empty() {
        "unavailable"
    } else if active_tools.iter().any(|t| t.unknown_outcome_count > 0) {
        "partial"
    } else {
        "measured"
    };
    active_tools.truncate(5);
    let declaration_status = if !has_declarations {
        "unavailable"
    } else if excluded.is_empty() {
        "measured"
    } else {
        "partial"
    };
    let mut all_keys: Vec<_> = declaration_keys
        .iter()
        .chain(invocation_keys.iter())
        .chain(supported_skill_keys.iter())
        .cloned()
        .collect();
    all_keys.sort();
    all_keys.dedup();
    (
        ContextTaxOverview {
            active_tools,
            active_tools_status: active_status.into(),
            excluded_source_keys: excluded.into_iter().collect(),
            uncalled_tools_count: has_declarations.then_some(uncalled.len()),
            declaration_status: declaration_status.into(),
            uncalled_skills: uncalled.into_iter().collect(),
            wasted_schema_tokens: None,
            schema_tokens_status: "unavailable".into(),
            wasted_cost_usd: None,
            declaration_source_keys: declaration_keys.into_iter().collect(),
            invocation_support_source_keys: invocation_keys.into_iter().collect(),
        },
        all_keys,
    )
}

fn project_at(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
    seat: &str,
    ts: DateTime<Utc>,
) -> Option<String> {
    let mut fps = BTreeSet::new();
    let mut missing = false;
    let mut sessions = BTreeSet::new();
    for id in facts.identity_facts {
        if id.seat == seat
            && session_in_scope(facts, req, &id.harness, &id.native_session, Some(ts))
            && binding(facts, &id.harness, &id.native_session, ts).is_some_and(|(_, s)| s == seat)
        {
            sessions.insert((id.harness.as_str(), id.native_session.as_str()));
        }
    }
    for (h, s) in sessions {
        for p in facts.process_identities.iter().filter(|p| {
            p.harness.as_deref() == Some(h)
                && p.session_link.as_deref() == Some(s)
                && p.first_seen_ms <= ts.timestamp_millis()
                && p.last_seen_ms >= ts.timestamp_millis()
                && p.ended_at_ms.is_none_or(|end| ts.timestamp_millis() < end)
        }) {
            if let Some(fp) = &p.cwd_fingerprint {
                fps.insert(fp.clone());
            } else {
                missing = true;
            }
        }
    }
    if fps.len() == 1 && !missing {
        fps.into_iter().next()
    } else {
        None
    }
}

fn project_group<'a>(groups: &'a mut Vec<CwdProjectGroup>, fp: &str) -> &'a mut CwdProjectGroup {
    if let Some(i) = groups.iter().position(|g| g.cwd_fingerprint == fp) {
        return &mut groups[i];
    }
    groups.push(CwdProjectGroup {
        cwd_fingerprint: fp.into(),
        alias: None,
        total_tokens: 0,
        active_hours: 0.0,
        closed_issue_qitems: 0,
        estimated_cost_usd: None,
        usage_source_keys: vec![],
    });
    groups.last_mut().unwrap()
}

pub(super) fn project_activity(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
    support: &native::Support,
    groups: &mut Vec<CwdProjectGroup>,
) -> (f64, f64, Vec<String>) {
    let seats: BTreeSet<_> = facts
        .activity_facts
        .iter()
        .filter(|a| seat_in_scope(facts, req, &a.seat, a.ts))
        .map(|a| a.seat.as_str())
        .collect();
    let mut total = 0.0;
    let mut unassigned = 0.0;
    let mut keys = BTreeSet::new();
    for seat in seats {
        let lives: Vec<_> = support
            .bounds
            .iter()
            .filter(|b| b.seat == seat)
            .cloned()
            .collect();
        // Idle demand cannot alter the active band, so no queue expansion is
        // necessary for this projection of the canonical five-band intervals.
        let intervals = metrics::activity_intervals(
            seat,
            facts.activity_facts,
            &[],
            Window {
                from: req.from,
                to: req.to,
            },
            &lives,
        );
        for interval in intervals.into_iter().filter(|i| i.state == "running") {
            let mut marks = vec![interval.from, interval.to];
            for p in facts.process_identities {
                for t in [Some(p.first_seen_ms), Some(p.last_seen_ms), p.ended_at_ms]
                    .into_iter()
                    .flatten()
                    .filter_map(DateTime::from_timestamp_millis)
                {
                    if t > interval.from && t < interval.to {
                        marks.push(t);
                    }
                }
            }
            marks.extend(
                facts
                    .identity_facts
                    .iter()
                    .filter(|i| i.seat == seat)
                    .filter_map(|i| i.ts)
                    .filter(|t| *t > interval.from && *t < interval.to),
            );
            marks.sort();
            marks.dedup();
            for pair in marks.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if !seat_in_scope(facts, req, seat, a) {
                    continue;
                }
                let secs = (b - interval.observation_start).num_seconds()
                    - (a - interval.observation_start).num_seconds();
                let hours = secs as f64 / 3600.0;
                total += hours;
                if let Some(key) = interval.source_key {
                    keys.insert(key.to_owned());
                }
                if let Some(fp) = project_at(facts, req, seat, a + (b - a) / 2) {
                    project_group(groups, &fp).active_hours += hours;
                } else {
                    unassigned += hours;
                }
            }
        }
    }
    (total, unassigned, keys.into_iter().collect())
}

pub(super) fn project_closures(
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
    tc: &ThroughputAndCostOverview,
    groups: &mut Vec<CwdProjectGroup>,
) -> usize {
    let mut unassigned = 0;
    for key in &tc.closed_issue_source_keys {
        if let Some(q) = facts.queue_facts.iter().find(|q| &q.source_key == key) {
            if let Some(fp) = project_at(facts, req, &q.dest_seat, q.ts()) {
                project_group(groups, &fp).closed_issue_qitems += 1;
            } else {
                unassigned += 1;
            }
        }
    }
    unassigned
}

fn ordered<'a, T: Serialize>(
    rows: impl Iterator<Item = &'a T>,
    key: impl Fn(&T) -> &str,
) -> Vec<&'a T> {
    let mut rows: Vec<_> = rows.collect();
    rows.sort_by(|a, b| {
        key(a).cmp(key(b)).then_with(|| {
            serde_json::to_string(a)
                .unwrap()
                .cmp(&serde_json::to_string(b).unwrap())
        })
    });
    rows
}

pub(super) fn hash_evidence(
    hasher: &mut Sha256,
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
) -> Result<()> {
    use std::cell::RefCell;
    struct Safe<'a, 'r>(&'a str, &'r RefCell<HashMap<&'a str, String>>);
    impl Serialize for Safe<'_, '_> {
        fn serialize<S: serde::Serializer>(
            &self,
            serializer: S,
        ) -> std::result::Result<S::Ok, S::Error> {
            let mut cache = self.1.borrow_mut();
            let value = cache.entry(self.0).or_insert_with(|| crate::redact(self.0));
            serializer.serialize_str(value)
        }
    }
    fn row<T: Serialize>(hasher: &mut Sha256, kind: &str, values: T) -> Result<()> {
        let data = serde_json::to_vec(&(kind, values))?;
        hasher.update((data.len() as u64).to_be_bytes());
        hasher.update(data);
        Ok(())
    }
    let support_index = skill_support_index(facts);
    let cache = RefCell::new(HashMap::new());
    let safe = |s| Safe(s, &cache);
    let from = cutoff_ms(req.from);
    let to = cutoff_ms(req.to);
    for u in ordered(
        facts.usage_facts.iter().filter(|u| {
            session_in_scope(facts, req, &u.harness, &u.native_session, u.ts())
                && u.ts_ms.is_none_or(|t| t >= from && t < to)
        }),
        |u| &u.source_key,
    ) {
        row(
            hasher,
            "usage",
            (
                safe(&u.source_key),
                safe(&u.harness),
                safe(&u.native_session),
                u.ts_ms,
                u.seq,
                u.total_tokens,
                u.currency_total,
            ),
        )?;
    }
    for t in ordered(
        facts.tool_facts.iter().filter(|t| {
            session_in_scope(facts, req, &t.harness, &t.native_session, t.ts())
                && t.ts_ms.is_none_or(|t| t >= from && t < to)
        }),
        |t| &t.source_key,
    ) {
        row(
            hasher,
            "tool",
            (
                safe(&t.source_key),
                safe(&t.harness),
                safe(&t.native_session),
                t.ts_ms,
                safe(&t.tool_name),
                t.failed,
            ),
        )?;
    }
    for s in ordered(facts.skill_facts.iter(), |s| &s.source_key) {
        if let Some((_, ts, key)) = skill_support(facts, req, s, &support_index) {
            row(
                hasher,
                "skill",
                (
                    safe(&s.source_key),
                    safe(&s.seat),
                    safe(&s.skill),
                    s.invoked,
                    ts,
                    crate::redact(&key),
                ),
            )?;
        }
    }
    for i in ordered(
        facts.identity_facts.iter().filter(|i| {
            i.ts.is_some_and(|ts| ts < req.to)
                && req.rig.as_deref().is_none_or(|r| {
                    r == i.rig
                        || facts.identity_facts.iter().any(|other| {
                            other.rig == r
                                && other.harness == i.harness
                                && other.native_session == i.native_session
                        })
                })
        }),
        |i| &i.source_key,
    ) {
        row(
            hasher,
            "identity",
            (
                safe(&i.source_key),
                safe(&i.harness),
                safe(&i.native_session),
                safe(&i.rig),
                safe(&i.seat),
                i.ts,
            ),
        )?;
    }
    for p in ordered(
        facts.process_identities.iter().filter(|p| {
            p.first_seen_ms < to
                && p.last_seen_ms >= from
                && p.harness
                    .as_deref()
                    .zip(p.session_link.as_deref())
                    .is_some_and(|(h, s)| session_in_scope(facts, req, h, s, Some(req.to)))
        }),
        |p| &p.identity_key,
    ) {
        row(
            hasher,
            "process",
            (
                safe(&p.identity_key),
                p.harness.as_deref().map(safe),
                p.session_link.as_deref().map(safe),
                p.cwd_fingerprint.as_deref().map(safe),
                p.first_seen_ms,
                p.last_seen_ms,
                p.ended_at_ms,
            ),
        )?;
    }
    for q in ordered(
        facts
            .queue_facts
            .iter()
            .filter(|q| q.ts_ms < to && seat_in_scope(facts, req, &q.dest_seat, q.ts())),
        |q| &q.source_key,
    ) {
        let reason = q.closure_reason.as_deref().filter(|r| {
            matches!(
                *r,
                "completed"
                    | "complete"
                    | "no-follow-on"
                    | "denied"
                    | "canceled"
                    | "escalation"
                    | "blocked-on"
                    | "handed_off_to"
            )
        });
        row(
            hasher,
            "queue",
            (
                safe(&q.source_key),
                safe(&q.qitem_id),
                q.transition_id,
                q.ts_ms,
                safe(&q.state),
                safe(&q.source_seat),
                safe(&q.dest_seat),
                q.closure_target.as_deref().map(safe),
                completion_reason(q.closure_reason.as_deref()),
                reason,
                q.is_recovery,
                q.recovery_reason.is_some(),
                q.issue_tags
                    .iter()
                    .filter(|s| positive_issue_id(s))
                    .map(|s| safe(s))
                    .collect::<Vec<_>>(),
                q.issue_tags.iter().any(|s| !positive_issue_id(s)),
            ),
        )?;
    }
    for a in ordered(
        facts
            .activity_facts
            .iter()
            .filter(|a| a.ts < req.to && seat_in_scope(facts, req, &a.seat, a.ts)),
        |a| &a.source_key,
    ) {
        row(
            hasher,
            "activity",
            (
                safe(&a.source_key),
                safe(&a.seat),
                a.ts,
                safe(&a.state),
                a.seq,
            ),
        )?;
    }
    for inc in ordered(facts.derived_evidence.iter(), |i| {
        i.source_keys.first().map_or("", String::as_str)
    }) {
        let mut value = serde_json::to_value(inc)?;
        redact_evidence(&mut value);
        row(hasher, "incident", value)?;
    }
    Ok(())
}

// Even safe identifier fields are redacted before hashing if a producer supplied
// a secret-shaped string. Raw prompt/tool text is never part of this projection.
pub(super) fn redact_evidence(value: &mut Value) {
    fn visit(value: &mut Value, cache: &mut HashMap<String, String>) {
        match value {
            Value::String(s) => {
                if let Some(redacted) = cache.get(s) {
                    *s = redacted.clone();
                } else {
                    let redacted = crate::redact(s);
                    cache.insert(s.clone(), redacted.clone());
                    *s = redacted;
                }
            }
            Value::Array(rows) => rows.iter_mut().for_each(|v| visit(v, cache)),
            Value::Object(fields) => fields.values_mut().for_each(|v| visit(v, cache)),
            _ => {}
        }
    }
    visit(value, &mut HashMap::new());
}
