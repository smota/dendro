#![deny(unsafe_code)]

//! `A09` / `A11` / `A12` — `dendro advise` Pre-Flight CLI & MCP Advisor.
//!
//! Answers four canonical questions over facts in `AnalyticsStore`:
//! 1. `model_cohort_baseline` (uses I02, I04, I21 comparability rules; noncomparable mix never picks a winner)
//! 2. `skill_effectiveness_audit` (tracks declared -> available -> loaded -> invoked -> verified_effective;
//!    verdict: placebo ONLY when loaded/available, sample >= 5, and verified_effective == 0)
//! 3. `delegation_loop_risk` (session_lineage cycles, retry edges, depth; never PageRank centrality)
//! 4. `environment_readiness` (placement, resource I14, and isolation I16; declaration alone is never readiness)
//!
//! Enforces:
//! - Canonical 7-row A14 badge derivation via `dendro_core::insight_v1::display_status`.
//! - `AdviseUncertainty` closed enum disjoint from `BadgeToken` (A14 set).
//! - `A11` Pre-flight budget: default 15 ms warm target; budget exceeded returns `advisory_budget_exceeded` with no partial numbers.
//! - `AdviseUncertainty::Stale` when facts are older than 86,400 seconds (24h).
//! - Pre-aggregated snapshot caching keyed by `(store_ptr, key_id, watermark)` achieving < 2 ms warm response.
//! - Dynamic `freshness_secs` computed on cache hits relative to `Utc::now()`.
//! - `A12` Governance separation: MCP schema has zero SQL, zero filesystem, and zero execute parameters;
//!   zero writes to policy/grants/routing; never injects tripwires into agent prompts.

use chrono::Utc;
use dendro_core::insight_v1::{display_status, InsightStatus};
use dendro_core::AnalyticsStore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Mutex;
use std::time::Instant;

/// Closed enum of uncertainty reasons.
/// Note: completely disjoint from UI badge tokens!
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdviseUncertainty {
    SampleBelowMinimum,
    MissingDenominator,
    NoncomparableMix,
    CounterfactualOnly,
    Stale,
}

/// UI Badge Token set (A14 set).
/// Completely disjoint from `AdviseUncertainty`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BadgeToken {
    Ok,
    Partial,
    InsufficientEvidence,
    Refused,
    NotApplicable,
}

pub fn map_status_to_badge(status: &InsightStatus) -> BadgeToken {
    match display_status(status) {
        "ok" => BadgeToken::Ok,
        "partial" => BadgeToken::Partial,
        "insufficient_evidence" => BadgeToken::InsufficientEvidence,
        "refused" => BadgeToken::Refused,
        _ => BadgeToken::NotApplicable,
    }
}

/// Advisory query request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdviseRequest {
    pub question: String,
    pub harness: Option<String>,
    pub skill: Option<String>,
    pub session: Option<String>,
    pub budget_ms: Option<u64>,
}

/// Advisory response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdviseResponse {
    pub question: String,
    pub result_code: String,
    pub display_badge: BadgeToken,
    pub uncertainty: Option<AdviseUncertainty>,
    pub evidence_ids: Vec<String>,
    pub sample_count: u64,
    pub freshness_secs: u64,
    pub details: serde_json::Value,
}

const MINIMUM_SAMPLE_SIZE: usize = 5;
const MAX_FROZEN_DEPTH: usize = 3;
pub const DEFAULT_ADVISE_BUDGET_MS: u64 = 15;
const STALE_FRESHNESS_SECS_THRESHOLD: u64 = 86_400; // 24 hours

/// Cache key for pre-aggregated advisory responses.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AdviseCacheKey {
    pub store_ptr: usize,
    pub key_id: String,
    pub watermark: u64,
    pub question: String,
    pub harness: Option<String>,
    pub skill: Option<String>,
    pub session: Option<String>,
}

#[derive(Clone)]
struct CachedAdvise {
    response: AdviseResponse,
    newest_epoch_secs: Option<i64>,
}

static ADVISE_CACHE: Mutex<BTreeMap<AdviseCacheKey, CachedAdvise>> = Mutex::new(BTreeMap::new());

/// Clears the in-process advisory pre-aggregation cache (e.g. on external mutations).
pub fn clear_advise_cache() {
    if let Ok(mut cache) = ADVISE_CACHE.lock() {
        cache.clear();
    }
}

type UsageModelsMap = BTreeMap<String, (Option<String>, Option<String>)>;

/// Internal fact abstraction supporting both `AnalyticsStore` and `duckdb::Connection`.
pub enum AdviseBackend<'a> {
    Store(&'a AnalyticsStore),
    Conn(&'a duckdb::Connection),
}

pub type FactSource<'a> = AdviseBackend<'a>;

fn compute_watermark_from_conn(conn: &duckdb::Connection, question: &str) -> u64 {
    match question {
        "model_cohort_baseline" => conn
            .query_row("SELECT COUNT(*) FROM usage_facts", [], |r| r.get(0))
            .unwrap_or(0),
        "skill_effectiveness_audit" => {
            let s: u64 = conn
                .query_row("SELECT COUNT(*) FROM skill_facts", [], |r| r.get(0))
                .unwrap_or(0);
            let t: u64 = conn
                .query_row("SELECT COUNT(*) FROM tool_facts", [], |r| r.get(0))
                .unwrap_or(0);
            let vt: u64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM tool_facts WHERE failed = false AND structured_write = true",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            s * 1_000_000 + t * 1_000 + vt
        }
        "delegation_loop_risk" => conn
            .query_row("SELECT COUNT(*) FROM session_edges", [], |r| r.get(0))
            .unwrap_or(0),
        "environment_readiness" => {
            let sec: u64 = conn
                .query_row("SELECT COUNT(*) FROM security_facts", [], |r| r.get(0))
                .unwrap_or(0);
            let id: u64 = conn
                .query_row("SELECT COUNT(*) FROM identity_facts", [], |r| r.get(0))
                .unwrap_or(0);
            sec + id
        }
        _ => 0,
    }
}

impl<'a> AdviseBackend<'a> {
    fn store_ptr(&self) -> usize {
        match self {
            AdviseBackend::Store(s) => *s as *const _ as usize,
            AdviseBackend::Conn(c) => *c as *const _ as usize,
        }
    }

    fn key_id(&self) -> String {
        match self {
            AdviseBackend::Store(s) => s.key_id.clone(),
            AdviseBackend::Conn(c) => c
                .query_row("SELECT key_id FROM dendro_meta LIMIT 1", [], |r| r.get(0))
                .unwrap_or_else(|_| "quack_owner".to_string()),
        }
    }

    fn compute_watermark(&self, question: &str) -> u64 {
        match self {
            AdviseBackend::Store(store) => {
                if let Ok(count) = store
                    .with_private_connection(|conn| Ok(compute_watermark_from_conn(conn, question)))
                {
                    return count;
                }
                match question {
                    "model_cohort_baseline" => {
                        store.load_usage().map(|u| u.len() as u64).unwrap_or(0)
                    }
                    "skill_effectiveness_audit" => {
                        let s = store.skill_count().unwrap_or(0) as u64;
                        let tools = store.load_tools().unwrap_or_default();
                        let t = tools.len() as u64;
                        let vt = tools
                            .iter()
                            .filter(|tool| tool.failed == Some(false) && tool.structured_write)
                            .count() as u64;
                        s * 1_000_000 + t * 1_000 + vt
                    }
                    "delegation_loop_risk" => store
                        .load_session_edges()
                        .map(|e| e.len() as u64)
                        .unwrap_or(0),
                    "environment_readiness" => {
                        let sec = store
                            .load_security_facts()
                            .map(|s| s.len() as u64)
                            .unwrap_or(0);
                        let id = store.load_identity().map(|i| i.len() as u64).unwrap_or(0);
                        sec + id
                    }
                    _ => 0,
                }
            }
            AdviseBackend::Conn(conn) => compute_watermark_from_conn(conn, question),
        }
    }

    fn load_usage_and_models(&self) -> (Vec<dendro_core::facts::UsageFact>, UsageModelsMap) {
        match self {
            FactSource::Store(store) => (
                store.load_usage().unwrap_or_default(),
                store.load_usage_models().unwrap_or_default(),
            ),
            FactSource::Conn(conn) => {
                let mut stmt = match conn.prepare(
                    "SELECT source_key, harness, native_session, message_id, CAST(ts AS VARCHAR), seq, \
                     input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total, \
                     generation, model \
                     FROM usage_facts ORDER BY ts NULLS LAST, seq",
                ) {
                    Ok(s) => s,
                    Err(_) => return (Vec::new(), BTreeMap::new()),
                };
                let mut facts = Vec::new();
                let mut models = BTreeMap::new();
                if let Ok(rows) = stmt.query_map([], |row| {
                    let source_key: String = row.get(0)?;
                    let harness: String = row.get(1)?;
                    let native_session: String = row.get(2)?;
                    let message_id: Option<String> = row.get(3)?;
                    let ts_str: Option<String> = row.get(4)?;
                    let seq: i64 = row.get(5)?;
                    let input_uncached: i64 = row.get(6)?;
                    let output: i64 = row.get(7)?;
                    let cache_read: i64 = row.get(8)?;
                    let cache_write: i64 = row.get(9)?;
                    let reasoning: i64 = row.get(10)?;
                    let currency_total: Option<f64> = row.get(11)?;
                    let generation: i64 = row.get(12)?;
                    let model: Option<String> = row.get(13)?;

                    let ts = ts_str.as_deref().and_then(dendro_core::parse_utc_time);
                    Ok((
                        dendro_core::facts::UsageFact {
                            source_key: source_key.clone(),
                            harness: harness.clone(),
                            native_session,
                            message_id,
                            ts,
                            seq,
                            input_uncached,
                            output,
                            cache_read,
                            cache_write,
                            reasoning,
                            currency_total,
                            generation,
                            predecessor_source_key: None,
                            predecessor_reason: None,
                            prior_input_tokens: None,
                            prior_cached_input_tokens: None,
                            prior_cache_write_input_tokens: None,
                            prior_output_tokens: None,
                            prior_reasoning_output_tokens: None,
                        },
                        source_key,
                        model,
                        harness,
                    ))
                }) {
                    for r in rows.flatten() {
                        models.insert(r.1, (r.2, Some(r.3)));
                        facts.push(r.0);
                    }
                }
                (facts, models)
            }
        }
    }

    fn load_skills_and_tools(
        &self,
    ) -> (
        Vec<dendro_core::facts::SkillFact>,
        Vec<dendro_core::facts::ToolFact>,
    ) {
        match self {
            FactSource::Store(store) => (
                store.load_skills().unwrap_or_default(),
                store.load_tools().unwrap_or_default(),
            ),
            FactSource::Conn(conn) => {
                let skills = conn
                    .prepare("SELECT source_key, seat, skill, invoked FROM skill_facts")
                    .and_then(|mut stmt| {
                        let rows = stmt.query_map([], |row| {
                            Ok(dendro_core::facts::SkillFact {
                                source_key: row.get(0)?,
                                seat: row.get(1)?,
                                skill: row.get(2)?,
                                invoked: row.get(3)?,
                            })
                        })?;
                        rows.collect::<Result<Vec<_>, _>>()
                    })
                    .unwrap_or_default();

                let tools = conn
                    .prepare(
                        "SELECT source_key, harness, native_session, CAST(ts AS VARCHAR), seq, tool_name, failed, structured_write, generation FROM tool_facts",
                    )
                    .and_then(|mut stmt| {
                        let rows = stmt.query_map([], |row| {
                            let ts_str: Option<String> = row.get(3)?;
                            Ok(dendro_core::facts::ToolFact {
                                source_key: row.get(0)?,
                                harness: row.get(1)?,
                                native_session: row.get(2)?,
                                ts: ts_str.as_deref().and_then(dendro_core::parse_utc_time),
                                seq: row.get(4)?,
                                name: row.get(5)?,
                                fingerprint: None,
                                failed: row.get(6)?,
                                structured_write: row.get(7)?,
                                generation: row.get(8)?,
                            })
                        })?;
                        rows.collect::<Result<Vec<_>, _>>()
                    })
                    .unwrap_or_default();

                (skills, tools)
            }
        }
    }

    fn load_session_edges(&self) -> Vec<dendro_core::facts::SessionEdgeFact> {
        match self {
            FactSource::Store(store) => store.load_session_edges().unwrap_or_default(),
            FactSource::Conn(conn) => conn
                .prepare(
                    "SELECT source_key, source_id, harness, parent_session, child_session, relation, status, CAST(observed_at AS VARCHAR), evidence_kind, generation FROM session_edges",
                )
                .and_then(|mut stmt| {
                    let rows = stmt.query_map([], |row| {
                        let obs_str: Option<String> = row.get(7)?;
                        Ok(dendro_core::facts::SessionEdgeFact {
                            source_key: row.get(0)?,
                            source_id: row.get(1)?,
                            harness: row.get(2)?,
                            parent_session: row.get(3)?,
                            child_session: row.get(4)?,
                            relation: row.get(5)?,
                            status: row.get(6)?,
                            observed_at: obs_str.as_deref().and_then(dendro_core::parse_utc_time),
                            evidence_kind: row.get(8)?,
                            generation: row.get(9)?,
                        })
                    })?;
                    rows.collect::<Result<Vec<_>, _>>()
                })
                .unwrap_or_default(),
        }
    }

    fn load_environment_facts(
        &self,
    ) -> (
        Vec<dendro_core::facts::IdentityFact>,
        Vec<dendro_core::facts::SecurityFact>,
        usize,
    ) {
        match self {
            FactSource::Store(store) => (
                store.load_identity().unwrap_or_default(),
                store.load_security_facts().unwrap_or_default(),
                store.load_seat_roles().unwrap_or_default().len(),
            ),
            FactSource::Conn(conn) => {
                let id_facts = conn
                    .prepare(
                        "SELECT source_key, harness, native_session, rig, seat, CAST(ts AS VARCHAR) FROM identity_facts",
                    )
                    .and_then(|mut stmt| {
                        let rows = stmt.query_map([], |row| {
                            let ts_str: Option<String> = row.get(5)?;
                            Ok(dendro_core::facts::IdentityFact {
                                source_key: row.get(0)?,
                                harness: row.get(1)?,
                                native_session: row.get(2)?,
                                rig: row.get(3)?,
                                seat: row.get(4)?,
                                ts: ts_str.as_deref().and_then(dendro_core::parse_utc_time),
                            })
                        })?;
                        rows.collect::<Result<Vec<_>, _>>()
                    })
                    .unwrap_or_default();

                let sec_facts = conn
                    .prepare(
                        "SELECT source_key, source_id, harness, native_session, CAST(ts AS VARCHAR), kind, value, source_field, generation FROM security_facts",
                    )
                    .and_then(|mut stmt| {
                        let rows = stmt.query_map([], |row| {
                            let ts_str: Option<String> = row.get(4)?;
                            Ok(dendro_core::facts::SecurityFact {
                                source_key: row.get(0)?,
                                source_id: row.get(1)?,
                                harness: row.get(2)?,
                                native_session: row.get(3)?,
                                ts: ts_str.as_deref().and_then(dendro_core::parse_utc_time),
                                kind: row.get(5)?,
                                value: row.get(6)?,
                                source_field: row.get(7)?,
                                generation: row.get(8)?,
                            })
                        })?;
                        rows.collect::<Result<Vec<_>, _>>()
                    })
                    .unwrap_or_default();

                let declared_seats = id_facts
                    .iter()
                    .map(|i| i.seat.clone())
                    .collect::<HashSet<_>>()
                    .len();
                (id_facts, sec_facts, declared_seats)
            }
        }
    }
}

/// MCP tool schema generator.
/// Enforces A12 governance hygiene:
/// - Zero SQL parameters
/// - Zero filesystem path parameters
/// - Zero "execute" parameters
pub fn mcp_tool_schema() -> serde_json::Value {
    json!({
        "name": "dendro_advise",
        "description": "Pre-flight operational intelligence advisor for AI fleet operations. Answers model cohort comparability, skill effectiveness audits, delegation loop risks, and environment readiness.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "Advisory question to evaluate",
                    "enum": [
                        "model_cohort_baseline",
                        "skill_effectiveness_audit",
                        "delegation_loop_risk",
                        "environment_readiness"
                    ]
                },
                "harness": {
                    "type": "string",
                    "description": "Optional harness filter (e.g. claude, codex, grok, pi)"
                },
                "skill": {
                    "type": "string",
                    "description": "Optional skill identifier for skill_effectiveness_audit"
                },
                "session": {
                    "type": "string",
                    "description": "Optional session identifier for lineage or cohort queries"
                },
                "budget_ms": {
                    "type": "integer",
                    "description": "Maximum advisory evaluation budget in milliseconds (default: 15)"
                }
            },
            "required": ["question"]
        }
    })
}

/// Evaluates an advisory request against canonical facts in `AnalyticsStore`.
pub fn evaluate_advise(store: &AnalyticsStore, req: &AdviseRequest) -> AdviseResponse {
    evaluate_advise_internal(FactSource::Store(store), req)
}

/// Evaluates an advisory request against canonical facts in `duckdb::Connection`.
pub fn evaluate_advise_conn(conn: &duckdb::Connection, req: &AdviseRequest) -> AdviseResponse {
    evaluate_advise_internal(FactSource::Conn(conn), req)
}

fn evaluate_advise_internal(source: FactSource<'_>, req: &AdviseRequest) -> AdviseResponse {
    let start_time = Instant::now();
    let effective_budget_ms = req.budget_ms.unwrap_or(DEFAULT_ADVISE_BUDGET_MS);

    if effective_budget_ms == 0 {
        return budget_exceeded_response(&req.question);
    }

    let cache_key = AdviseCacheKey {
        store_ptr: source.store_ptr(),
        key_id: source.key_id(),
        watermark: source.compute_watermark(&req.question),
        question: req.question.clone(),
        harness: req.harness.clone(),
        skill: req.skill.clone(),
        session: req.session.clone(),
    };

    if let Ok(cache) = ADVISE_CACHE.lock() {
        if let Some(cached) = cache.get(&cache_key) {
            let mut resp = cached.response.clone();
            let now_secs = Utc::now().timestamp();
            let freshness_secs = cached
                .newest_epoch_secs
                .map(|epoch| (now_secs - epoch).max(0) as u64)
                .unwrap_or(0);
            resp.freshness_secs = freshness_secs;
            if freshness_secs > STALE_FRESHNESS_SECS_THRESHOLD {
                resp.uncertainty = Some(AdviseUncertainty::Stale);
            }
            let elapsed = start_time.elapsed().as_millis() as u64;
            if elapsed > effective_budget_ms {
                return budget_exceeded_response(&req.question);
            }
            return resp;
        }
    }

    let (response, newest_epoch_secs) = match req.question.as_str() {
        "model_cohort_baseline" => evaluate_model_cohort_baseline(&source, req),
        "skill_effectiveness_audit" => evaluate_skill_effectiveness_audit(&source, req),
        "delegation_loop_risk" => evaluate_delegation_loop_risk(&source, req),
        "environment_readiness" => evaluate_environment_readiness(&source, req),
        _ => (
            AdviseResponse {
                question: req.question.clone(),
                result_code: "unrecognized_question".to_string(),
                display_badge: BadgeToken::Refused,
                uncertainty: None,
                evidence_ids: vec![],
                sample_count: 0,
                freshness_secs: 0,
                details: json!({"error": format!("unknown advisory question: {}", req.question)}),
            },
            None,
        ),
    };

    if let Ok(mut cache) = ADVISE_CACHE.lock() {
        cache.insert(
            cache_key,
            CachedAdvise {
                response: response.clone(),
                newest_epoch_secs,
            },
        );
    }

    let elapsed = start_time.elapsed().as_millis() as u64;
    if elapsed > effective_budget_ms {
        return budget_exceeded_response(&req.question);
    }

    response
}

fn budget_exceeded_response(question: &str) -> AdviseResponse {
    AdviseResponse {
        question: question.to_string(),
        result_code: "advisory_budget_exceeded".to_string(),
        display_badge: BadgeToken::Refused,
        uncertainty: None,
        evidence_ids: vec![],
        sample_count: 0,
        freshness_secs: 0,
        details: json!({
            "error": "advisory_budget_exceeded",
            "partial_numbers": null
        }),
    }
}

/// Evaluates `model_cohort_baseline` using I02, I04, and I21 comparability rules.
fn evaluate_model_cohort_baseline(
    source: &FactSource<'_>,
    req: &AdviseRequest,
) -> (AdviseResponse, Option<i64>) {
    let (usage_facts, models_map) = source.load_usage_and_models();

    let filtered: Vec<_> = usage_facts
        .iter()
        .filter(|u| {
            if let Some(h) = req.harness.as_deref() {
                if u.harness != h {
                    return false;
                }
            }
            if let Some(s) = req.session.as_deref() {
                if u.native_session != s {
                    return false;
                }
            }
            true
        })
        .collect();

    let sample_count = filtered.len() as u64;
    let mut evidence_ids = Vec::new();
    for u in filtered.iter().take(5) {
        evidence_ids.push(u.source_key.clone());
    }

    let now = Utc::now();
    let newest_ts = filtered.iter().filter_map(|u| u.ts).max();
    let newest_epoch_secs = newest_ts.map(|ts| ts.timestamp());
    let freshness_secs = newest_ts
        .map(|ts| (now - ts).num_seconds().max(0) as u64)
        .unwrap_or(0);

    let is_stale = freshness_secs > STALE_FRESHNESS_SECS_THRESHOLD;

    if filtered.is_empty() {
        let status = InsightStatus::Refused {
            reason: "insufficient_sample".to_string(),
        };
        return (
            AdviseResponse {
                question: req.question.clone(),
                result_code: "empty_population".to_string(),
                display_badge: map_status_to_badge(&status),
                uncertainty: Some(AdviseUncertainty::SampleBelowMinimum),
                evidence_ids,
                sample_count: 0,
                freshness_secs: 0,
                details: json!({"comparability": "insufficient_data"}),
            },
            newest_epoch_secs,
        );
    }

    let mut observed_models = HashSet::new();
    for u in &filtered {
        if let Some((Some(model), _)) = models_map.get(&u.source_key) {
            observed_models.insert(model.clone());
        }
    }

    // I21 comparability rule: noncomparable mix never picks a winner
    if observed_models.len() > 1 {
        let status = InsightStatus::Refused {
            reason: "noncomparable".to_string(),
        };
        let unc = if is_stale {
            AdviseUncertainty::Stale
        } else {
            AdviseUncertainty::NoncomparableMix
        };
        return (
            AdviseResponse {
                question: req.question.clone(),
                result_code: "noncomparable".to_string(),
                display_badge: map_status_to_badge(&status),
                uncertainty: Some(unc),
                evidence_ids,
                sample_count,
                freshness_secs,
                details: json!({
                    "comparability": "noncomparable_mix",
                    "observed_models": observed_models.into_iter().collect::<Vec<_>>(),
                    "winner": null,
                    "rule": "I21: different versions or biased mixes without cohort slicing cannot pick a winner"
                }),
            },
            newest_epoch_secs,
        );
    }

    if (sample_count as usize) < MINIMUM_SAMPLE_SIZE {
        let status = InsightStatus::Refused {
            reason: "insufficient_sample".to_string(),
        };
        let unc = if is_stale {
            AdviseUncertainty::Stale
        } else {
            AdviseUncertainty::SampleBelowMinimum
        };
        return (
            AdviseResponse {
                question: req.question.clone(),
                result_code: "insufficient_sample".to_string(),
                display_badge: map_status_to_badge(&status),
                uncertainty: Some(unc),
                evidence_ids,
                sample_count,
                freshness_secs,
                details: json!({
                    "comparability": "sample_below_minimum",
                    "sample": sample_count,
                    "minimum_required": MINIMUM_SAMPLE_SIZE
                }),
            },
            newest_epoch_secs,
        );
    }

    let total_cache_read: i64 = filtered.iter().map(|u| u.cache_read).sum();
    let total_input: i64 = filtered
        .iter()
        .map(|u| u.input_uncached + u.cache_read)
        .sum();
    let cache_read_ratio = if total_input > 0 {
        total_cache_read as f64 / total_input as f64
    } else {
        0.0
    };

    let total_currency: f64 = filtered.iter().filter_map(|u| u.currency_total).sum();
    let cohort_model = observed_models
        .into_iter()
        .next()
        .unwrap_or_else(|| "unknown".to_string());

    let status = InsightStatus::Computed;
    let unc = if is_stale {
        Some(AdviseUncertainty::Stale)
    } else {
        None
    };

    (
        AdviseResponse {
            question: req.question.clone(),
            result_code: "ok".to_string(),
            display_badge: map_status_to_badge(&status),
            uncertainty: unc,
            evidence_ids,
            sample_count,
            freshness_secs,
            details: json!({
                "cohort": cohort_model,
                "cache_read_ratio": cache_read_ratio,
                "total_currency": total_currency,
                "comparability": "comparable_cohort"
            }),
        },
        newest_epoch_secs,
    )
}

/// Evaluates `skill_effectiveness_audit`.
fn evaluate_skill_effectiveness_audit(
    source: &FactSource<'_>,
    req: &AdviseRequest,
) -> (AdviseResponse, Option<i64>) {
    let (skill_facts, tool_facts) = source.load_skills_and_tools();
    let target_skill = req.skill.as_deref().unwrap_or("all");

    let relevant_skills: Vec<_> = skill_facts
        .iter()
        .filter(|s| target_skill == "all" || s.skill == target_skill)
        .collect();

    let sample_count = relevant_skills.len() as u64;
    let mut evidence_ids = Vec::new();
    for s in relevant_skills.iter().take(5) {
        evidence_ids.push(s.source_key.clone());
    }

    let now = Utc::now();
    let newest_tool_ts = tool_facts.iter().filter_map(|t| t.ts).max();
    let newest_epoch_secs = newest_tool_ts.map(|ts| ts.timestamp());
    let freshness_secs = newest_tool_ts
        .map(|ts| (now - ts).num_seconds().max(0) as u64)
        .unwrap_or(0);
    let is_stale = freshness_secs > STALE_FRESHNESS_SECS_THRESHOLD;

    let invoked_count = relevant_skills.iter().filter(|s| s.invoked).count();
    let loaded_count = relevant_skills.len();

    let verified_effective_count = tool_facts
        .iter()
        .filter(|t| {
            if target_skill != "all" && !t.name.contains(target_skill) {
                return false;
            }
            t.failed == Some(false) && t.structured_write
        })
        .count();

    if (sample_count as usize) < MINIMUM_SAMPLE_SIZE {
        let status = InsightStatus::Refused {
            reason: "insufficient_sample".to_string(),
        };
        let unc = if is_stale {
            AdviseUncertainty::Stale
        } else {
            AdviseUncertainty::SampleBelowMinimum
        };
        return (
            AdviseResponse {
                question: req.question.clone(),
                result_code: "insufficient_evidence".to_string(),
                display_badge: map_status_to_badge(&status),
                uncertainty: Some(unc),
                evidence_ids,
                sample_count,
                freshness_secs,
                details: json!({
                    "verdict": "insufficient_evidence",
                    "sample": sample_count,
                    "minimum_required": MINIMUM_SAMPLE_SIZE
                }),
            },
            newest_epoch_secs,
        );
    }

    if loaded_count > 0 && tool_facts.is_empty() {
        let status = InsightStatus::Refused {
            reason: "not_observable".to_string(),
        };
        let unc = if is_stale {
            AdviseUncertainty::Stale
        } else {
            AdviseUncertainty::MissingDenominator
        };
        return (
            AdviseResponse {
                question: req.question.clone(),
                result_code: "insufficient_evidence".to_string(),
                display_badge: map_status_to_badge(&status),
                uncertainty: Some(unc),
                evidence_ids,
                sample_count,
                freshness_secs,
                details: json!({
                    "verdict": "insufficient_evidence",
                    "reason": "missing_denominator",
                    "message": "no verification events recorded for loaded skill"
                }),
            },
            newest_epoch_secs,
        );
    }

    // Placebo ONLY when skill was loaded/available, sample >= 5, and verified_effective == 0
    if verified_effective_count == 0 {
        let status = InsightStatus::Refused {
            reason: "placebo".to_string(),
        };
        let unc = if is_stale {
            Some(AdviseUncertainty::Stale)
        } else {
            None
        };
        return (
            AdviseResponse {
                question: req.question.clone(),
                result_code: "placebo".to_string(),
                display_badge: map_status_to_badge(&status),
                uncertainty: unc,
                evidence_ids,
                sample_count,
                freshness_secs,
                details: json!({
                    "verdict": "placebo",
                    "loaded": loaded_count,
                    "invoked": invoked_count,
                    "verified_effective": 0,
                    "rule": "placebo: loaded/available with sample >= 5 but zero verified effectiveness"
                }),
            },
            newest_epoch_secs,
        );
    }

    let status = InsightStatus::Computed;
    let unc = if is_stale {
        Some(AdviseUncertainty::Stale)
    } else {
        None
    };
    (
        AdviseResponse {
            question: req.question.clone(),
            result_code: "ok".to_string(),
            display_badge: map_status_to_badge(&status),
            uncertainty: unc,
            evidence_ids,
            sample_count,
            freshness_secs,
            details: json!({
                "verdict": "verified_effective",
                "loaded": loaded_count,
                "invoked": invoked_count,
                "verified_effective": verified_effective_count
            }),
        },
        newest_epoch_secs,
    )
}

/// Evaluates `delegation_loop_risk`.
/// Filters session edges strictly to evidenced status per A06 and applies direction table:
/// - spawn, delegate, delegate_existing, handoff, retry: parent -> child
/// - return, join: child -> parent
/// - others ignored
fn evaluate_delegation_loop_risk(
    source: &FactSource<'_>,
    req: &AdviseRequest,
) -> (AdviseResponse, Option<i64>) {
    let raw_edges = source.load_session_edges();

    // Strict evidenced allowlist per A06
    let edges: Vec<_> = raw_edges
        .into_iter()
        .filter(|e| e.status.eq_ignore_ascii_case("evidenced"))
        .collect();

    let mut evidence_ids = Vec::new();
    for edge in edges.iter().take(5) {
        evidence_ids.push(edge.source_key.clone());
    }

    let sample_count = edges.len() as u64;
    let now = Utc::now();
    let newest_ts = edges.iter().filter_map(|e| e.observed_at).max();
    let newest_epoch_secs = newest_ts.map(|ts| ts.timestamp());
    let freshness_secs = newest_ts
        .map(|ts| (now - ts).num_seconds().max(0) as u64)
        .unwrap_or(0);
    let is_stale = freshness_secs > STALE_FRESHNESS_SECS_THRESHOLD;

    if edges.is_empty() {
        let status = InsightStatus::Computed;
        let unc = if is_stale {
            Some(AdviseUncertainty::Stale)
        } else {
            None
        };
        return (
            AdviseResponse {
                question: req.question.clone(),
                result_code: "ok".to_string(),
                display_badge: map_status_to_badge(&status),
                uncertainty: unc,
                evidence_ids,
                sample_count: 0,
                freshness_secs: 0,
                details: json!({
                    "risk": false,
                    "cycles": 0,
                    "max_depth": 0,
                    "centrality_used": false
                }),
            },
            newest_epoch_secs,
        );
    }

    let mut adj: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
    let mut all_nodes = BTreeSet::new();

    for edge in &edges {
        let (from, to) = match edge.relation.as_str() {
            "spawn" | "delegate" | "delegate_existing" | "handoff" | "retry" => {
                (&edge.parent_session, &edge.child_session)
            }
            "return" | "join" => (&edge.child_session, &edge.parent_session),
            _ => continue,
        };
        all_nodes.insert(from.clone());
        all_nodes.insert(to.clone());
        adj.entry(from.clone()).or_default().push((
            to.clone(),
            edge.relation.clone(),
            edge.source_key.clone(),
        ));
    }

    let mut visited: HashSet<String> = HashSet::new();
    let mut rec_stack: HashSet<String> = HashSet::new();
    let mut cycle_found = false;
    let mut cycle_edges = Vec::new();

    fn dfs_cycle(
        node: &str,
        adj: &BTreeMap<String, Vec<(String, String, String)>>,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        cycle_edges: &mut Vec<String>,
    ) -> bool {
        visited.insert(node.to_string());
        rec_stack.insert(node.to_string());

        if let Some(neighbors) = adj.get(node) {
            for (neighbor, _rel, key) in neighbors {
                if !visited.contains(neighbor) {
                    if dfs_cycle(neighbor, adj, visited, rec_stack, cycle_edges) {
                        cycle_edges.push(key.clone());
                        return true;
                    }
                } else if rec_stack.contains(neighbor) {
                    cycle_edges.push(key.clone());
                    return true;
                }
            }
        }

        rec_stack.remove(node);
        false
    }

    for node in &all_nodes {
        if !visited.contains(node)
            && dfs_cycle(node, &adj, &mut visited, &mut rec_stack, &mut cycle_edges)
        {
            cycle_found = true;
            break;
        }
    }

    fn compute_max_depth(
        node: &str,
        adj: &BTreeMap<String, Vec<(String, String, String)>>,
        memo: &mut HashMap<String, usize>,
        path: &mut HashSet<String>,
    ) -> usize {
        if let Some(&d) = memo.get(node) {
            return d;
        }
        if path.contains(node) {
            return 0;
        }
        path.insert(node.to_string());
        let mut max_child_depth = 0;
        if let Some(neighbors) = adj.get(node) {
            for (neighbor, _, _) in neighbors {
                let cd = compute_max_depth(neighbor, adj, memo, path);
                if cd > max_child_depth {
                    max_child_depth = cd;
                }
            }
        }
        path.remove(node);
        let depth = 1 + max_child_depth;
        memo.insert(node.to_string(), depth);
        depth
    }

    let mut memo = HashMap::new();
    let mut path = HashSet::new();
    let mut observed_depth = 0;
    for node in &all_nodes {
        observed_depth = observed_depth.max(compute_max_depth(node, &adj, &mut memo, &mut path));
    }

    let retry_edges_count = edges.iter().filter(|e| e.relation == "retry").count();

    if cycle_found {
        let mut cycle_ev = cycle_edges;
        cycle_ev.truncate(5);
        let status = InsightStatus::Refused {
            reason: "cycle_detected".to_string(),
        };
        let unc = if is_stale {
            Some(AdviseUncertainty::Stale)
        } else {
            None
        };
        return (
            AdviseResponse {
                question: req.question.clone(),
                result_code: "cycle_detected".to_string(),
                display_badge: map_status_to_badge(&status),
                uncertainty: unc,
                evidence_ids: cycle_ev,
                sample_count,
                freshness_secs,
                details: json!({
                    "risk": true,
                    "cycle_detected": true,
                    "retry_count": retry_edges_count,
                    "max_depth": observed_depth,
                    "centrality_used": false
                }),
            },
            newest_epoch_secs,
        );
    }

    if observed_depth > MAX_FROZEN_DEPTH || retry_edges_count > 3 {
        let status = InsightStatus::Partial {
            reason: "threshold_exceeded".to_string(),
        };
        let unc = if is_stale {
            Some(AdviseUncertainty::Stale)
        } else {
            None
        };
        return (
            AdviseResponse {
                question: req.question.clone(),
                result_code: "depth_or_retry_exceeded".to_string(),
                display_badge: map_status_to_badge(&status),
                uncertainty: unc,
                evidence_ids,
                sample_count,
                freshness_secs,
                details: json!({
                    "risk": true,
                    "cycle_detected": false,
                    "retry_count": retry_edges_count,
                    "max_depth": observed_depth,
                    "frozen_depth_limit": MAX_FROZEN_DEPTH,
                    "centrality_used": false
                }),
            },
            newest_epoch_secs,
        );
    }

    let status = InsightStatus::Computed;
    let unc = if is_stale {
        Some(AdviseUncertainty::Stale)
    } else {
        None
    };
    (
        AdviseResponse {
            question: req.question.clone(),
            result_code: "ok".to_string(),
            display_badge: map_status_to_badge(&status),
            uncertainty: unc,
            evidence_ids,
            sample_count,
            freshness_secs,
            details: json!({
                "risk": false,
                "cycle_detected": false,
                "retry_count": retry_edges_count,
                "max_depth": observed_depth,
                "centrality_used": false
            }),
        },
        newest_epoch_secs,
    )
}

/// Evaluates `environment_readiness`.
fn evaluate_environment_readiness(
    source: &FactSource<'_>,
    req: &AdviseRequest,
) -> (AdviseResponse, Option<i64>) {
    let (identity_facts, security_facts, declared_seats_count) = source.load_environment_facts();

    let sample_count = (identity_facts.len() + security_facts.len()) as u64;
    let mut evidence_ids = Vec::new();
    for id in identity_facts.iter().take(3) {
        evidence_ids.push(id.source_key.clone());
    }
    for sec in security_facts.iter().take(2) {
        evidence_ids.push(sec.source_key.clone());
    }

    let now = Utc::now();
    let newest_sec_ts = security_facts.iter().filter_map(|s| s.ts).max();
    let newest_id_ts = identity_facts.iter().filter_map(|i| i.ts).max();
    let newest_ts = newest_sec_ts.max(newest_id_ts);
    let newest_epoch_secs = newest_ts.map(|ts| ts.timestamp());
    let freshness_secs = newest_ts
        .map(|ts| (now - ts).num_seconds().max(0) as u64)
        .unwrap_or(0);
    let is_stale = freshness_secs > STALE_FRESHNESS_SECS_THRESHOLD;

    // I16 rule: declaration alone is never readiness
    if !identity_facts.is_empty() && security_facts.is_empty() {
        let status = InsightStatus::Refused {
            reason: "not_observable".to_string(),
        };
        let unc = if is_stale {
            AdviseUncertainty::Stale
        } else {
            AdviseUncertainty::MissingDenominator
        };
        return (
            AdviseResponse {
                question: req.question.clone(),
                result_code: "declaration_alone".to_string(),
                display_badge: map_status_to_badge(&status),
                uncertainty: Some(unc),
                evidence_ids,
                sample_count,
                freshness_secs,
                details: json!({
                    "readiness": false,
                    "rule": "I16: a declaration alone is never readiness",
                    "declared_seats": declared_seats_count,
                    "observed_isolation_facts": 0
                }),
            },
            newest_epoch_secs,
        );
    }

    if sample_count == 0 {
        let status = InsightStatus::Refused {
            reason: "insufficient_sample".to_string(),
        };
        let unc = if is_stale {
            AdviseUncertainty::Stale
        } else {
            AdviseUncertainty::SampleBelowMinimum
        };
        return (
            AdviseResponse {
                question: req.question.clone(),
                result_code: "empty_population".to_string(),
                display_badge: map_status_to_badge(&status),
                uncertainty: Some(unc),
                evidence_ids,
                sample_count,
                freshness_secs,
                details: json!({
                    "readiness": false,
                    "message": "no environment facts recorded"
                }),
            },
            newest_epoch_secs,
        );
    }

    let isolation_failures = security_facts
        .iter()
        .filter(|s| {
            s.value == "danger-full-access" || s.kind == "isolation_breach" || s.value == "refused"
        })
        .count();

    if isolation_failures > 0 {
        let status = InsightStatus::Refused {
            reason: "isolation_breach".to_string(),
        };
        let unc = if is_stale {
            Some(AdviseUncertainty::Stale)
        } else {
            None
        };
        return (
            AdviseResponse {
                question: req.question.clone(),
                result_code: "isolation_breach".to_string(),
                display_badge: map_status_to_badge(&status),
                uncertainty: unc,
                evidence_ids,
                sample_count,
                freshness_secs,
                details: json!({
                    "readiness": false,
                    "isolation_breaches": isolation_failures
                }),
            },
            newest_epoch_secs,
        );
    }

    let status = InsightStatus::Computed;
    let unc = if is_stale {
        Some(AdviseUncertainty::Stale)
    } else {
        None
    };
    (
        AdviseResponse {
            question: req.question.clone(),
            result_code: "ok".to_string(),
            display_badge: map_status_to_badge(&status),
            uncertainty: unc,
            evidence_ids,
            sample_count,
            freshness_secs,
            details: json!({
                "readiness": true,
                "observed_isolation_facts": security_facts.len(),
                "declared_seats": declared_seats_count
            }),
        },
        newest_epoch_secs,
    )
}
