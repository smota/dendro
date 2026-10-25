//! Compiled insight contract. Callers select a scope. Insights run fixed queries.
use crate::AnalyticsStore;
use duckdb::params;
use serde::Serialize;
use std::fmt;

/// Where an insight looks. `model`, `harness`, `coordination`, `environment`, or `security`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightDomain {
    Model,
    Harness,
    Coordination,
    Environment,
    Security,
}

/// Read-only scope. Absent fields are unbounded. The window is half-open `[from, to)`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scope {
    pub harness: Option<String>,
    pub session: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

impl Scope {
    pub(crate) fn refusal(&self) -> Option<&'static str> {
        if self.harness.as_ref().is_some_and(|value| value.is_empty())
            || self.session.as_ref().is_some_and(|value| value.is_empty())
        {
            return Some("undefined input");
        }
        match (&self.from, &self.to) {
            (Some(from), Some(to)) if from.as_str() >= to.as_str() => Some("undefined input"),
            _ => None,
        }
    }
}

/// Store plus scope. Insights borrow this. They do not accept SQL from callers.
pub struct InsightInput<'a> {
    store: &'a AnalyticsStore,
    scope: &'a Scope,
}

impl<'a> InsightInput<'a> {
    pub(crate) fn new(store: &'a AnalyticsStore, scope: &'a Scope) -> Self {
        Self { store, scope }
    }

    pub(crate) fn store(&self) -> &'a AnalyticsStore {
        self.store
    }

    pub(crate) fn scope(&self) -> &'a Scope {
        self.scope
    }

    /// DuckDB connection for a fixed `prepare` and `params!` query inside this crate.
    pub(crate) fn connection(&self) -> &'a duckdb::Connection {
        self.store.connection()
    }

    pub(crate) fn usage_rows(&self) -> anyhow::Result<Vec<UsageRow>> {
        let mut stmt = self.connection().prepare(
            "SELECT source_key, model, input_uncached, output_tokens, cache_read, cache_write, reasoning, currency_total
             FROM usage_facts
             WHERE (? IS NULL OR harness = ?)
               AND (? IS NULL OR native_session = ?)
               AND (? IS NULL OR ts >= CAST(? AS TIMESTAMP))
               AND (? IS NULL OR ts < CAST(? AS TIMESTAMP))
             ORDER BY source_key",
        )?;
        let harness = self.scope.harness.as_deref();
        let session = self.scope.session.as_deref();
        let from = self.scope.from.as_deref();
        let to = self.scope.to.as_deref();
        let rows = stmt.query_map(
            params![harness, harness, session, session, from, from, to, to],
            |row| {
                Ok(UsageRow {
                    source_key: row.get(0)?,
                    model: row.get(1)?,
                    input_uncached: row.get(2)?,
                    output: row.get(3)?,
                    cache_read: row.get(4)?,
                    cache_write: row.get(5)?,
                    reasoning: row.get(6)?,
                    currency_total: row.get(7)?,
                })
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(crate) fn warning_rows(&self) -> anyhow::Result<Vec<WarningRow>> {
        let mut stmt = self
            .connection()
            .prepare("SELECT path, reason FROM warnings ORDER BY path, reason, line_no")?;
        let rows = stmt.query_map([], |row| {
            Ok(WarningRow {
                path: row.get(0)?,
                reason: row.get(1)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UsageRow {
    pub source_key: String,
    pub model: Option<String>,
    pub input_uncached: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    pub currency_total: Option<f64>,
}

#[derive(Debug, Clone)]
pub(crate) struct WarningRow {
    pub path: String,
    pub reason: String,
}

pub trait Insight: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn domain(&self) -> InsightDomain;
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult;
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InsightStatus {
    Computed,
    Partial { reason: String },
    Refused { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InsightPopulation {
    pub what: String,
    pub n: u64,
}

/// A counted amount. `None` means unknown. Unknown is never `Some(0)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MeasureValue {
    Int(i64),
    Float(f64),
}

impl fmt::Display for MeasureValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(value) => write!(f, "{value}"),
            Self::Float(value) => write!(f, "{value}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Measure {
    pub id: String,
    pub name: String,
    pub value: Option<MeasureValue>,
    pub unit: String,
    pub basis: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InsightResult {
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub domain: InsightDomain,
    pub status: InsightStatus,
    pub population: InsightPopulation,
    pub measures: Vec<Measure>,
    pub coverage: String,
    pub evidence: Vec<String>,
}

pub(crate) fn refused(insight: &dyn Insight, reason: &str, what: &str) -> InsightResult {
    InsightResult {
        id: insight.id(),
        name: insight.name(),
        version: insight.version(),
        domain: insight.domain(),
        status: InsightStatus::Refused {
            reason: reason.to_string(),
        },
        population: InsightPopulation {
            what: what.to_string(),
            n: 0,
        },
        measures: Vec::new(),
        coverage: reason.to_string(),
        evidence: Vec::new(),
    }
}

pub(crate) fn finish(mut result: InsightResult) -> InsightResult {
    result
        .measures
        .sort_by(|left, right| left.id.cmp(&right.id));
    result.evidence.sort();
    result.evidence.dedup();
    result.evidence.truncate(5);
    result
}

pub(crate) fn label_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "absent".to_string()
    } else {
        out
    }
}

pub(crate) fn model_key(model: &Option<String>) -> String {
    match model {
        Some(name) if !name.is_empty() => label_id(name),
        _ => "absent".to_string(),
    }
}

pub fn format_text(results: &[InsightResult]) -> String {
    let mut lines = Vec::new();
    for result in results {
        let status = match &result.status {
            InsightStatus::Computed => "computed".to_string(),
            InsightStatus::Partial { reason } => format!("partial ({reason})"),
            InsightStatus::Refused { reason } => format!("refused ({reason})"),
        };
        lines.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{} n={}",
            result.id,
            result.name,
            result.version,
            domain_name(result.domain),
            status,
            result.population.what,
            result.population.n
        ));
        lines.push(format!("  coverage: {}", result.coverage));
        for measure in &result.measures {
            let value = match &measure.value {
                Some(value) => value.to_string(),
                None => "unknown".to_string(),
            };
            lines.push(format!(
                "  {}\t{value}\t{}\t{}",
                measure.id, measure.unit, measure.basis
            ));
        }
        if result.evidence.is_empty() {
            lines.push("  evidence:".to_string());
        } else {
            lines.push(format!("  evidence: {}", result.evidence.join(" ")));
        }
    }
    lines.join("\n")
}

fn domain_name(domain: InsightDomain) -> &'static str {
    match domain {
        InsightDomain::Model => "model",
        InsightDomain::Harness => "harness",
        InsightDomain::Coordination => "coordination",
        InsightDomain::Environment => "environment",
        InsightDomain::Security => "security",
    }
}
