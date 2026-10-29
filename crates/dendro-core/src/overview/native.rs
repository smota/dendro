//! Persisted production adapter. Every row conversion is fallible; corrupt evidence
//! cannot silently disappear into a complete result.
use super::*;
use crate::metrics::{self, Window};
use crate::{DestinationSnapshot, QueueFact, SeatBound, UsageFact};
use anyhow::Context;
use serde_json::Value;

// Decode typed Arrow batches directly. Native fact keys are primary keys; row
// conversion still fails on missing/invalid required operands instead of hiding
// corrupt evidence with flatten().
use duckdb::arrow::array::{Array, BooleanArray, Float64Array, Int64Array, StringArray};
use duckdb::arrow::record_batch::RecordBatch;
trait ArrowOperand: Sized {
    fn read(array: &dyn Array, at: usize) -> duckdb::Result<Self>;
}
fn invalid_column() -> duckdb::Error {
    duckdb::Error::InvalidParameterName("invalid required overview column".into())
}
impl ArrowOperand for String {
    fn read(array: &dyn Array, at: usize) -> duckdb::Result<Self> {
        if array.is_null(at) {
            return Err(invalid_column());
        }
        Ok(array
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(invalid_column)?
            .value(at)
            .to_owned())
    }
}
impl ArrowOperand for i64 {
    fn read(array: &dyn Array, at: usize) -> duckdb::Result<Self> {
        if array.is_null(at) {
            return Err(invalid_column());
        }
        Ok(array
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(invalid_column)?
            .value(at))
    }
}
impl ArrowOperand for bool {
    fn read(array: &dyn Array, at: usize) -> duckdb::Result<Self> {
        if array.is_null(at) {
            return Err(invalid_column());
        }
        Ok(array
            .as_any()
            .downcast_ref::<BooleanArray>()
            .ok_or_else(invalid_column)?
            .value(at))
    }
}
impl<T: ArrowOperand> ArrowOperand for Option<T> {
    fn read(array: &dyn Array, at: usize) -> duckdb::Result<Self> {
        if array.is_null(at) {
            Ok(None)
        } else {
            T::read(array, at).map(Some)
        }
    }
}
struct ArrowRow<'a> {
    batch: &'a RecordBatch,
    at: usize,
}
impl ArrowRow<'_> {
    fn get<I: TryInto<usize>, T: ArrowOperand>(&self, index: I) -> duckdb::Result<T> {
        let index = index.try_into().map_err(|_| invalid_column())?;
        let col = self.batch.columns().get(index).ok_or_else(invalid_column)?;
        T::read(col.as_ref(), self.at)
    }
}
fn arrow_rows<T>(
    stmt: &mut duckdb::Statement<'_>,
    mut map: impl FnMut(&ArrowRow<'_>) -> duckdb::Result<T>,
) -> duckdb::Result<std::vec::IntoIter<duckdb::Result<T>>> {
    let mut rows = Vec::new();
    for batch in stmt.query_arrow([])? {
        for at in 0..batch.num_rows() {
            rows.push(map(&ArrowRow { batch: &batch, at }));
        }
    }
    Ok(rows.into_iter())
}

#[derive(Default)]
pub(super) struct Support {
    pub bounds: Vec<SeatBound>,
    pub coverage: BTreeMap<String, String>,
    pub safe_evidence: Value,
    pub lens_rows: BTreeMap<String, Vec<Value>>,
    changed_source_keys: BTreeSet<String>,
}

pub struct OverviewStatements<'conn> {
    conn: &'conn duckdb::Connection,
    stmt_native_queue: duckdb::Statement<'conn>,
    stmt_activity: duckdb::Statement<'conn>,
    stmt_bounds: duckdb::Statement<'conn>,
    stmt_destinations: duckdb::Statement<'conn>,
    stmt_changes: Option<duckdb::Statement<'conn>>,
    pub stmt_queue: duckdb::Statement<'conn>,
    pub stmt_usage: duckdb::Statement<'conn>,
    pub stmt_skills: duckdb::Statement<'conn>,
    pub stmt_tools: duckdb::Statement<'conn>,
    pub stmt_pids: duckdb::Statement<'conn>,
    pub stmt_identities: duckdb::Statement<'conn>,
    pub queue_buf: Vec<OverviewQueueFact>,
    pub usage_buf: Vec<OverviewUsageFact>,
    pub skills_buf: Vec<SkillFact>,
    pub tools_buf: Vec<OverviewToolFact>,
    pub pids_buf: Vec<OverviewProcessIdentity>,
    pub identities_buf: Vec<IdentityFact>,
}

impl<'conn> OverviewStatements<'conn> {
    pub fn prepare(conn: &'conn duckdb::Connection) -> Result<Self> {
        let stmt_queue = conn.prepare("SELECT source_key, transition_id, qitem_id, epoch_ms(ts), state, dest_seat, closure_reason, is_recovery, recovery_reason, issue_tags, source_seat, closure_target FROM queue_facts").or_else(|_| conn.prepare("SELECT source_key, COALESCE(TRY_CAST(json_extract_string(to_json(q),'$.transition_id') AS BIGINT),0), qitem_id, epoch_ms(ts), state, dest_seat, json_extract_string(to_json(q),'$.closure_reason'), COALESCE(TRY_CAST(json_extract_string(to_json(q),'$.is_recovery') AS BOOLEAN),false), json_extract_string(to_json(q),'$.recovery_reason'), COALESCE(json_extract_string(to_json(q),'$.issue_tags'),'[]'),source_seat,json_extract_string(to_json(q),'$.closure_target') FROM queue_facts q"))?;
        let stmt_usage = conn.prepare("SELECT source_key, harness, native_session, epoch_ms(ts), (input_uncached + output_tokens + cache_read + cache_write)::BIGINT, currency_total, seq FROM usage_facts ORDER BY source_key").or_else(|_|conn.prepare("SELECT source_key,harness,native_session,epoch_ms(ts),(input_uncached+output_tokens+cache_read+cache_write)::BIGINT,currency_total,0::BIGINT FROM usage_facts ORDER BY source_key"))?;
        let stmt_skills =
            conn.prepare("SELECT source_key, seat, skill, invoked FROM skill_facts")?;
        let stmt_tools = conn.prepare(
            "SELECT source_key, harness, native_session, epoch_ms(ts), tool_name, failed FROM tool_facts",
        )?;
        let stmt_pids = conn.prepare("SELECT harness, session_link, cwd_fingerprint, epoch_ms(first_seen), epoch_ms(last_seen), identity_key, epoch_ms(ended_at) FROM process_identities ORDER BY identity_key").or_else(|_|conn.prepare("SELECT harness,session_link,cwd_fingerprint,epoch_ms(first_seen),epoch_ms(last_seen),identity_key,NULL::BIGINT FROM process_identities ORDER BY identity_key"))?;
        let stmt_identities = conn.prepare("SELECT source_key, harness, native_session, rig, seat, epoch_ms(ts) FROM identity_facts")?;
        let stmt_native_queue =conn.prepare("SELECT source_key, transition_id, qitem_id, epoch_ms(ts), state, source_seat, dest_seat, closure_reason, closure_target, chain_id, parent_qitem_id, is_recovery, recovery_reason, issue_tags FROM queue_facts ORDER BY source_key").or_else(|_|conn.prepare("SELECT source_key,COALESCE(TRY_CAST(json_extract_string(to_json(q),'$.transition_id') AS BIGINT),0),qitem_id,epoch_ms(ts),state,source_seat,dest_seat,json_extract_string(to_json(q),'$.closure_reason'),json_extract_string(to_json(q),'$.closure_target'),json_extract_string(to_json(q),'$.chain_id'),json_extract_string(to_json(q),'$.parent_qitem_id'),COALESCE(TRY_CAST(json_extract_string(to_json(q),'$.is_recovery') AS BOOLEAN),false),json_extract_string(to_json(q),'$.recovery_reason'),COALESCE(json_extract_string(to_json(q),'$.issue_tags'),'[]') FROM queue_facts q ORDER BY source_key"))?;
        let stmt_destinations =conn.prepare("SELECT qitem_id, destination, snapshot_cutoff, reroute_marked, missions FROM destination_snapshots")?;
        let stmt_activity = conn
            .prepare("SELECT source_key, seat, epoch_ms(ts), state, seq FROM activity_facts ORDER BY source_key").or_else(|_|conn.prepare("SELECT source_key,seat,epoch_ms(ts),state,0::BIGINT FROM activity_facts ORDER BY source_key"))?;
        let stmt_bounds = conn
            .prepare("SELECT seat, epoch_ms(created_at), epoch_ms(stopped_at) FROM seat_bounds")?;
        let stmt_changes = conn
            .prepare("SELECT source_key FROM openrig_row_records WHERE row_state='changed'")
            .ok();
        Ok(Self {
            conn,
            stmt_native_queue,
            stmt_activity,
            stmt_bounds,
            stmt_destinations,
            stmt_changes,
            stmt_queue,
            stmt_usage,
            stmt_skills,
            stmt_tools,
            stmt_pids,
            stmt_identities,
            queue_buf: Vec::with_capacity(640),
            usage_buf: Vec::with_capacity(4096),
            skills_buf: Vec::with_capacity(256),
            tools_buf: Vec::with_capacity(512),
            pids_buf: Vec::with_capacity(128),
            identities_buf: Vec::with_capacity(128),
        })
    }

    pub fn evaluate(&mut self, req: &OverviewRequest) -> Result<DirectOverview> {
        let conn = self.conn;
        let tx = if conn.is_autocommit() {
            Some(conn.unchecked_transaction()?)
        } else {
            None
        };
        let result = self.evaluate_in_snapshot(req);
        if result.is_ok() {
            if let Some(tx) = tx {
                tx.commit()?;
            }
        }
        result
    }

    fn evaluate_in_snapshot(&mut self, req: &OverviewRequest) -> Result<DirectOverview> {
        self.queue_buf.clear();
        let rows = arrow_rows(&mut self.stmt_queue, |row| {
            let ts_ms: i64 = row.get(3)?;
            let tags_str: String = row.get(9)?;
            let tags =
                serde_json::from_str(&tags_str).unwrap_or_else(|_| vec!["invalid-tag-json".into()]);
            Ok(OverviewQueueFact {
                source_key: row.get(0)?,
                transition_id: row.get(1)?,
                qitem_id: row.get(2)?,
                ts_ms,
                state: row.get(4)?,
                source_seat: row.get(10)?,
                dest_seat: row.get(5)?,
                closure_reason: row.get(6)?,
                closure_target: row.get(11)?,
                is_recovery: row.get(7)?,
                recovery_reason: row.get(8)?,
                issue_tags: tags,
            })
        })?;
        for fact in rows {
            self.queue_buf.push(fact?);
        }

        self.usage_buf.clear();
        for batch in self.stmt_usage.query_arrow([])? {
            let strings = |i| {
                batch
                    .column(i)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .context("invalid overview string column")
            };
            let ints = |i| {
                batch
                    .column(i)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .context("invalid overview integer column")
            };
            let keys = strings(0)?;
            let harness = strings(1)?;
            let sessions = strings(2)?;
            let times = ints(3)?;
            let tokens = ints(4)?;
            let seq = ints(6)?;
            let money = batch
                .column(5)
                .as_any()
                .downcast_ref::<Float64Array>()
                .context("invalid overview money column")?;
            for i in 0..batch.num_rows() {
                if keys.is_null(i)
                    || harness.is_null(i)
                    || sessions.is_null(i)
                    || tokens.is_null(i)
                    || seq.is_null(i)
                {
                    return Err(anyhow!("missing required overview usage operand"));
                }
                self.usage_buf.push(OverviewUsageFact {
                    source_key: keys.value(i).into(),
                    harness: harness.value(i).into(),
                    native_session: sessions.value(i).into(),
                    ts_ms: (!times.is_null(i)).then(|| times.value(i)),
                    seq: seq.value(i),
                    total_tokens: tokens.value(i),
                    currency_total: (!money.is_null(i)).then(|| money.value(i)),
                });
            }
        }

        self.skills_buf.clear();
        let rows = arrow_rows(&mut self.stmt_skills, |row| {
            Ok(SkillFact {
                source_key: row.get(0)?,
                seat: row.get(1)?,
                skill: row.get(2)?,
                invoked: row.get(3)?,
            })
        })?;
        for fact in rows {
            self.skills_buf.push(fact?);
        }

        self.tools_buf.clear();
        let rows = arrow_rows(&mut self.stmt_tools, |row| {
            let ts_ms: Option<i64> = row.get(3)?;
            Ok(OverviewToolFact {
                source_key: row.get(0)?,
                harness: row.get(1)?,
                native_session: row.get(2)?,
                ts_ms,
                tool_name: row.get(4)?,
                failed: row.get(5)?,
            })
        })?;
        for fact in rows {
            self.tools_buf.push(fact?);
        }

        self.pids_buf.clear();
        let rows = arrow_rows(&mut self.stmt_pids, |row| {
            let f_ms: i64 = row.get(3)?;
            let l_ms: i64 = row.get(4)?;
            Ok(OverviewProcessIdentity {
                identity_key: row.get(5)?,
                harness: row.get(0)?,
                session_link: row.get(1)?,
                cwd_fingerprint: row.get(2)?,
                first_seen_ms: f_ms,
                last_seen_ms: l_ms,
                ended_at_ms: row.get(6)?,
            })
        })?;
        for fact in rows {
            self.pids_buf.push(fact?);
        }

        self.identities_buf.clear();
        let rows = arrow_rows(&mut self.stmt_identities, |row| {
            let ts_ms: Option<i64> = row.get(5)?;
            Ok(IdentityFact {
                source_key: row.get(0)?,
                harness: row.get(1)?,
                native_session: row.get(2)?,
                rig: row.get(3)?,
                seat: row.get(4)?,
                ts: ts_ms.and_then(DateTime::from_timestamp_millis),
            })
        })?;
        for fact in rows {
            self.identities_buf.push(fact?);
        }

        self.identities_buf.sort_by(|a, b| {
            (&a.harness, &a.native_session, a.ts, &a.source_key).cmp(&(
                &b.harness,
                &b.native_session,
                b.ts,
                &b.source_key,
            ))
        });

        let mut activity = arrow_rows(&mut self.stmt_activity, |r| {
            Ok(ActivityFact {
                source_key: r.get(0)?,
                seat: r.get(1)?,
                ts: timestamp(r.get(2)?)?,
                state: r.get(3)?,
                seq: r.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut support = load_support(&mut self.stmt_bounds, self.stmt_changes.as_mut(), req)?;
        self.queue_buf
            .retain(|q| !support.changed_source_keys.contains(&q.source_key));
        self.identities_buf
            .retain(|i| !support.changed_source_keys.contains(&i.source_key));
        self.skills_buf
            .retain(|s| !support.changed_source_keys.contains(&s.source_key));
        activity.retain(|a| !support.changed_source_keys.contains(&a.source_key));

        let mut raw_facts = RawOverviewFacts {
            derived_evidence: vec![],
            queue_facts: &self.queue_buf,
            skill_facts: &self.skills_buf,
            usage_facts: &self.usage_buf,
            tool_facts: &self.tools_buf,
            process_identities: &self.pids_buf,
            identity_facts: &self.identities_buf,
            activity_facts: &activity,
            coverage: None,
            evidence_as_of: None,
        };
        let (incidents, coverage) = derive_incidents(
            &mut self.stmt_native_queue,
            &mut self.stmt_destinations,
            &raw_facts,
            req,
            &mut support,
        )?;
        raw_facts.derived_evidence = incidents;
        raw_facts.coverage = Some(coverage);
        raw_facts.evidence_as_of = raw_facts
            .queue_facts
            .iter()
            .filter(|q| {
                q.ts_ms < cutoff_ms(req.to)
                    && attribution::seat_in_scope(&raw_facts, req, &q.dest_seat, q.ts())
            })
            .map(|q| q.ts())
            .chain(
                raw_facts
                    .usage_facts
                    .iter()
                    .filter(|u| {
                        attribution::session_in_scope(
                            &raw_facts,
                            req,
                            &u.harness,
                            &u.native_session,
                            u.ts(),
                        )
                    })
                    .filter_map(|u| u.ts())
                    .filter(|ts| *ts < req.to),
            )
            .chain(
                raw_facts
                    .tool_facts
                    .iter()
                    .filter(|t| {
                        attribution::session_in_scope(
                            &raw_facts,
                            req,
                            &t.harness,
                            &t.native_session,
                            t.ts(),
                        )
                    })
                    .filter_map(|t| t.ts())
                    .filter(|ts| *ts < req.to),
            )
            .chain(
                activity
                    .iter()
                    .filter(|a| {
                        a.ts < req.to && attribution::seat_in_scope(&raw_facts, req, &a.seat, a.ts)
                    })
                    .map(|a| a.ts),
            )
            .max();
        evaluate_with_support(&raw_facts, req, &support)
    }
}

fn timestamp(ms: i64) -> duckdb::Result<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms)
        .ok_or_else(|| duckdb::Error::InvalidParameterName("invalid evidence timestamp".into()))
}

fn load_support(
    bounds: &mut duckdb::Statement<'_>,
    changes: Option<&mut duckdb::Statement<'_>>,
    req: &OverviewRequest,
) -> Result<Support> {
    let mut result = Support::default();
    // These lifecycle relations predate overview. The API supplies empty typed
    // relations only when opening an older/empty analytics schema.

    result.bounds = arrow_rows(bounds, |r| {
        Ok(SeatBound {
            seat: r.get(0)?,
            created_at: r
                .get::<_, Option<i64>>(1)?
                .and_then(DateTime::from_timestamp_millis),
            stopped_at: r
                .get::<_, Option<i64>>(2)?
                .and_then(DateTime::from_timestamp_millis),
        })
    })?
    .collect::<std::result::Result<Vec<_>, _>>()?;
    result
        .bounds
        .retain(|b| b.created_at.is_none_or(|ts| ts < req.to));
    if let Some(stmt) = changes {
        result.changed_source_keys = stmt
            .query_map([], |r| r.get(0))?
            .collect::<duckdb::Result<_>>()?;
        result
            .coverage
            .insert("mutable_source_provenance".into(), "retained_only".into());
    } else {
        result
            .coverage
            .insert("mutable_source_provenance".into(), "unavailable".into());
    }
    Ok(result)
}

fn derive_incidents(
    native_queue: &mut duckdb::Statement<'_>,
    destinations: &mut duckdb::Statement<'_>,
    facts: &RawOverviewFacts<'_>,
    req: &OverviewRequest,
    support: &mut Support,
) -> Result<(Vec<DerivedIncident>, String)> {
    // Retain history up to cutoff for canonical ownership/lineage; never feed a
    // future reopen or another rig to the canonical evaluators.

    let mut metadata = Vec::new();
    let mut queue = Vec::new();
    let rows = arrow_rows(native_queue, |r| {
        Ok((
            QueueFact {
                source_key: r.get(0)?,
                transition_id: r.get(1)?,
                qitem_id: r.get(2)?,
                ts: timestamp(r.get(3)?)?,
                state: r.get(4)?,
                source_seat: r.get(5)?,
                dest_seat: r.get(6)?,
                closure_reason: r.get(7)?,
                closure_target: r.get(8)?,
                chain_id: r.get(9)?,
                missions: vec![],
            },
            r.get::<_, Option<String>>(10)?,
            r.get::<_, bool>(11)?,
            r.get::<_, Option<String>>(12)?,
            r.get::<_, String>(13)?,
        ))
    })?;
    for row in rows {
        let (q, parent, is_recovery, recovery_reason, tags) = row?;
        if q.ts >= req.to
            || support.changed_source_keys.contains(&q.source_key)
            || !attribution::seat_in_scope(facts, req, &q.dest_seat, q.ts)
        {
            continue;
        }
        metadata.push(crate::vortex::ItemEvidence {
            source_key: q.source_key.clone(),
            qitem_id: q.qitem_id.clone(),
            parent_qitem_id: parent,
            is_recovery,
            recovery_reason,
            issue_tags: serde_json::from_str(&tags).unwrap_or_default(),
        });
        queue.push(q);
    }
    let mut snapshots = BTreeMap::<String, Vec<DestinationSnapshot>>::new();

    let rows = arrow_rows(destinations, |r| {
        Ok((
            r.get::<_, String>(0)?,
            DestinationSnapshot {
                destination: r.get(1)?,
                cutoff: r.get(2)?,
                reroute_marked: r.get(3)?,
                missions: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
            },
        ))
    })?;
    for row in rows {
        let (id, snapshot) = row?;
        if queue.iter().any(|q| q.qitem_id == id)
            && DateTime::parse_from_rfc3339(&snapshot.cutoff)
                .ok()
                .map(|t| t.with_timezone(&Utc))
                .is_some_and(|ts| ts < req.to)
        {
            snapshots.entry(id).or_default().push(snapshot);
        }
    }
    for rows in snapshots.values_mut() {
        rows.sort_by(|a, b| (&a.cutoff, &a.destination).cmp(&(&b.cutoff, &b.destination)));
    }
    let activity: Vec<_> = facts
        .activity_facts
        .iter()
        .filter(|a| a.ts < req.to && attribution::seat_in_scope(facts, req, &a.seat, a.ts))
        .cloned()
        .collect();
    let seats: BTreeSet<_> = activity
        .iter()
        .map(|a| a.seat.clone())
        .chain(queue.iter().map(|q| q.dest_seat.clone()))
        .collect();
    support
        .bounds
        .retain(|b| attribution::seat_in_scope(facts, req, &b.seat, req.to));
    let mut local = Vec::new();
    let mut unresolved = Vec::new();
    let mut add = |key: &str, h: &str, s: &str, ts: Option<DateTime<Utc>>| {
        if !attribution::session_in_scope(facts, req, h, s, ts) || ts.is_some_and(|t| t >= req.to) {
            return;
        }
        let seat = ts
            .and_then(|t| attribution::binding(facts, h, s, t))
            .map(|(_, s)| s.to_owned());
        match (seat, ts) {
            (Some(seat), Some(ts)) => local.push(crate::advisory::ActivityPoint {
                seat,
                ts,
                source_key: key.into(),
            }),
            (seat, _) => unresolved.push(crate::advisory::UnresolvedActivity {
                source_key: key.into(),
                seat,
                reason: "missing_timestamp_or_unproven_session_binding",
            }),
        }
    };
    for u in facts.usage_facts {
        add(&u.source_key, &u.harness, &u.native_session, u.ts());
    }
    for t in facts.tool_facts {
        add(&t.source_key, &t.harness, &t.native_session, t.ts());
    }
    let idle = crate::advisory::evaluate(crate::advisory::Input {
        queue: &queue,
        metadata: &metadata,
        activity: &activity,
        bounds: &support.bounds,
        local_activity: &local,
        unresolved_activity: &unresolved,
        snapshots: &snapshots,
        seats: &seats,
        nudge_metadata: &BTreeMap::new(),
        window: Window {
            from: req.from,
            to: req.to,
        },
        options: &req.advisory_options,
    });
    let edges: Vec<_> = metrics::handoff_edges(&queue)
        .into_iter()
        .filter(|e| e.ts >= req.from && e.ts < req.to)
        .collect();
    let usage: Vec<_> = facts
        .usage_facts
        .iter()
        .filter(|u| {
            attribution::session_in_scope(facts, req, &u.harness, &u.native_session, u.ts())
        })
        .map(|u| UsageFact {
            source_key: u.source_key.clone(),
            harness: u.harness.clone(),
            native_session: u.native_session.clone(),
            message_id: None,
            ts: u.ts(),
            seq: u.seq,
            input_uncached: u.total_tokens,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            currency_total: u.currency_total,
            generation: 0,
            predecessor_source_key: None,
            predecessor_reason: None,
            prior_input_tokens: None,
            prior_cached_input_tokens: None,
            prior_cache_write_input_tokens: None,
            prior_output_tokens: None,
            prior_reasoning_output_tokens: None,
        })
        .collect();
    let identities: Vec<_> = facts
        .identity_facts
        .iter()
        .filter(|i| {
            i.ts.is_some_and(|ts| ts < req.to) && req.rig.as_deref().is_none_or(|r| r == i.rig)
        })
        .cloned()
        .collect();
    let friction = crate::vortex::evaluate(crate::vortex::Input {
        queue: &queue,
        metadata: &metadata,
        edges: &edges,
        usage: &usage,
        identities: &identities,
        activity: &activity,
        snapshots: &snapshots,
        window: Window {
            from: req.from,
            to: req.to,
        },
        thresholds: req.friction_thresholds,
        capture_replay: false,
    })
    .value;
    let mut incidents = Vec::new();
    for finding in idle["findings"].as_array().into_iter().flatten() {
        let qitem = finding["qitem_id"].as_str();
        let resolved = qitem
            .and_then(|id| {
                queue
                    .iter()
                    .filter(|q| q.qitem_id == id)
                    .max_by_key(|q| (q.ts, q.transition_id, &q.source_key))
            })
            .is_some_and(|q| matches!(q.state.as_str(), "done" | "failed" | "denied" | "canceled"));
        incidents.push(DerivedIncident {
            classification: finding["classification"]
                .as_str()
                .unwrap_or_default()
                .into(),
            seat: finding["seat"].as_str().map(str::to_owned),
            qitem_id: qitem.map(str::to_owned),
            duration_secs: finding["duration_secs"].as_f64().unwrap_or_default(),
            resolved: Some(resolved),
            source_keys: serde_json::from_value(finding["source_keys"].clone())?,
            ..Default::default()
        });
    }
    for finding in friction["vortex_loops"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["exceeds_threshold"] == true)
    {
        // A completed handoff at the root does not resolve an open return
        // chain. Resolve only when every retained descendant is terminal.
        let mut related = BTreeSet::new();
        if let Some(root) = finding["root"].as_str() {
            related.insert(root.to_owned());
        }
        loop {
            let before = related.len();
            for m in &metadata {
                if m.parent_qitem_id
                    .as_deref()
                    .is_some_and(|p| related.contains(p))
                {
                    related.insert(m.qitem_id.clone());
                }
            }
            if related.len() == before {
                break;
            }
        }
        let final_states: Vec<_> = related
            .iter()
            .filter_map(|id| {
                queue
                    .iter()
                    .filter(|q| &q.qitem_id == id)
                    .max_by_key(|q| (q.ts, q.transition_id, &q.source_key))
            })
            .collect();
        let resolved = !related.is_empty()
            && final_states.len() == related.len()
            && final_states
                .iter()
                .all(|q| matches!(q.state.as_str(), "done" | "failed" | "denied" | "canceled"));
        let mut source_keys: Vec<String> = serde_json::from_value(finding["source_keys"].clone())?;
        source_keys.extend(final_states.iter().map(|q| q.source_key.clone()));
        source_keys.sort();
        source_keys.dedup();
        incidents.push(DerivedIncident {
            classification: "rework_vortex".into(),
            threshold_breached: Some(true),
            resolved: Some(resolved),
            qitem_id: finding["root"].as_str().map(str::to_owned),
            duration_secs: finding["duration_secs"].as_f64().unwrap_or_default(),
            rework_tokens: finding["rework_tokens"].as_i64(),
            chain_returns: finding["return_count"].as_u64().map(|v| v as usize),
            return_rate: friction["pairs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|pair| {
                    pair["source_keys"].as_array().is_some_and(|keys| {
                        keys.iter().any(|key| {
                            finding["source_keys"]
                                .as_array()
                                .is_some_and(|loop_keys| loop_keys.contains(key))
                        })
                    })
                })
                .filter_map(|pair| pair["return_rate"].as_f64())
                .max_by(f64::total_cmp),
            source_keys,
            ..Default::default()
        });
    }
    let coverage = match idle["coverage"].as_str() {
        Some("observed") => "partial",
        Some("partial") => "partial",
        _ => {
            if incidents.is_empty() {
                "unavailable"
            } else {
                "partial"
            }
        }
    };
    support.coverage.insert(
        "queue".into(),
        if queue.is_empty() {
            "unavailable"
        } else {
            "observed"
        }
        .into(),
    );
    let scoped_usage: Vec<_> = facts
        .usage_facts
        .iter()
        .filter(|u| {
            attribution::session_in_scope(facts, req, &u.harness, &u.native_session, u.ts())
        })
        .collect();
    support.coverage.insert(
        "usage".into(),
        if scoped_usage.iter().any(|u| u.ts_ms.is_none()) {
            "partial"
        } else if scoped_usage
            .iter()
            .any(|u| u.ts().is_some_and(|ts| ts >= req.from && ts < req.to))
        {
            "observed"
        } else {
            "unavailable"
        }
        .into(),
    );
    support.coverage.insert(
        "activity".into(),
        if activity.iter().any(|a| a.ts >= req.from && a.ts < req.to) {
            "observed"
        } else {
            "unavailable"
        }
        .into(),
    );
    support.coverage.insert(
        "advisory".into(),
        idle["coverage"].as_str().unwrap_or("unavailable").into(),
    );
    support.coverage.insert(
        "vortex".into(),
        friction["coverage"]
            .as_str()
            .unwrap_or("unavailable")
            .into(),
    );
    // No admission/hold-file telemetry exists in native analytics. Never infer it
    // from blocked queue states or claim a complete live admission-gate census.
    support
        .coverage
        .insert("admission_holds".into(), "unavailable".into());
    let mut stall_rows = BTreeMap::<String, Value>::new();
    for interval in idle["intervals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| {
            matches!(
                i["classification"].as_str(),
                Some("true_leaf_stall" | "unparked_container_hold")
            )
        })
    {
        let seat = interval["seat"].as_str().unwrap_or_default();
        let row=stall_rows.entry(seat.into()).or_insert_with(||json!({"seat":seat,"leaf_stall_secs":0.0,"unparked_container_secs":0.0,"source_keys":[]}));
        let field = if interval["classification"] == "true_leaf_stall" {
            "leaf_stall_secs"
        } else {
            "unparked_container_secs"
        };
        row[field] = json!(
            row[field].as_f64().unwrap_or_default()
                + interval["duration_secs"].as_f64().unwrap_or_default()
        );
        if let Some(keys) = interval["source_keys"].as_array() {
            row["source_keys"]
                .as_array_mut()
                .unwrap()
                .extend(keys.clone());
        }
    }
    let mut stall_rows: Vec<_> = stall_rows.into_values().collect();
    let duration = |v: &Value| {
        v["leaf_stall_secs"].as_f64().unwrap_or_default()
            + v["unparked_container_secs"].as_f64().unwrap_or_default()
    };
    stall_rows.sort_by(|a, b| {
        duration(b)
            .total_cmp(&duration(a))
            .then_with(|| a["seat"].as_str().cmp(&b["seat"].as_str()))
    });
    stall_rows.truncate(5);
    support
        .lens_rows
        .insert("v_seat_idle_ranking".into(), stall_rows);
    let mut pairs = friction["pairs"].as_array().cloned().unwrap_or_default();
    pairs.sort_by(|a, b| {
        b["return_count"]
            .as_u64()
            .cmp(&a["return_count"].as_u64())
            .then_with(|| a["source_seat"].as_str().cmp(&b["source_seat"].as_str()))
            .then_with(|| a["dest_seat"].as_str().cmp(&b["dest_seat"].as_str()))
    });
    pairs.truncate(5);
    support
        .lens_rows
        .insert("v_handoff_rework_analysis".into(), pairs);
    metadata.sort_by(|a, b| a.source_key.cmp(&b.source_key));
    support.bounds.sort_by(|a, b| {
        (&a.seat, a.created_at, a.stopped_at).cmp(&(&b.seat, b.created_at, b.stopped_at))
    });
    for rows in snapshots.values_mut() {
        rows.sort_by(|a, b| (&a.cutoff, &a.destination).cmp(&(&b.cutoff, &b.destination)));
    }
    support.safe_evidence = json!({"bounds":support.bounds,"metadata":metadata.iter().map(|m|json!([m.source_key,m.qitem_id,m.parent_qitem_id,m.is_recovery,m.recovery_reason.is_some(),m.issue_tags.iter().filter(|s|attribution::positive_issue_id(s)).collect::<Vec<_>>()])).collect::<Vec<_>>(),"snapshots":snapshots,"queue_chains":queue.iter().map(|q|(&q.source_key,&q.chain_id)).collect::<Vec<_>>(),"advisory_omissions":idle["omissions"]});
    Ok((incidents, coverage.into()))
}
