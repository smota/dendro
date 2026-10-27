//! I25 model.counterfactual_routing. Pinned fast-tier simulation, not a routing change.
use super::super::contract::{
    finish, refused, Insight, InsightDomain, InsightInput, InsightPopulation, InsightResult,
    InsightStatus, Measure,
};
use super::support::{column_exists, float_measure, table_exists};
use duckdb::params;

pub struct CounterfactualRouting;

pub static INSIGHT: CounterfactualRouting = CounterfactualRouting;

const COVERAGE: &str = "\
counterfactual fast-tier simulation is not an effect and not a routing change \
| assumption: prices and the fast tier come only from the operator-pinned pricing table \
| alternative: the same cache-read series can be explained without a routing change \
| formula: ASOF JOIN of cache-read fraction to the cohort baseline and QUANTILE_CONT inflection at 0.9";

struct Usage {
    harness: String,
    input_uncached: i64,
    cache_write: i64,
    cache_read: i64,
    output: i64,
}

struct Tier {
    profile_pin: String,
    cohort_pin: String,
    input_rate: Option<f64>,
    output_rate: Option<f64>,
    context_limit: Option<i64>,
}

impl Insight for CounterfactualRouting {
    fn id(&self) -> &'static str {
        "I25"
    }
    fn name(&self) -> &'static str {
        "model.counterfactual_routing"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn domain(&self) -> InsightDomain {
        InsightDomain::Model
    }
    fn compute(&self, input: &InsightInput<'_>) -> InsightResult {
        let usage = match load_usage(input) {
            Ok(rows) => rows,
            Err(_) => return refused(self, "store read failed", "usage_facts"),
        };
        if usage.is_empty() {
            return refused(self, "empty population", "usage_facts");
        }
        let observed = observed_ratio(&usage);
        let tier = match load_tier(input) {
            Ok(tier) => tier,
            Err(_) => return refused(self, "store read failed", "_dendro_pricing_tiers"),
        };
        let Some(tier) = tier else {
            return simulation_refused(self, &usage, observed, "unpinned tier profile");
        };
        if tier.profile_pin.is_empty()
            || tier.cohort_pin.is_empty()
            || tier.input_rate.is_none()
            || tier.output_rate.is_none()
        {
            return simulation_refused(self, &usage, observed, "unpinned tier profile");
        }
        let harnesses: std::collections::BTreeSet<&str> =
            usage.iter().map(|row| row.harness.as_str()).collect();
        if harnesses.len() != 1 || !harnesses.contains(tier.cohort_pin.as_str()) {
            return simulation_refused(self, &usage, observed, "noncomparable");
        }
        let inflection = match cache_inflection(input, &tier.cohort_pin) {
            Ok(value) => value,
            Err(_) => return refused(self, "store read failed", "usage_facts"),
        };
        let input_rate = tier.input_rate.unwrap_or(0.0);
        let output_rate = tier.output_rate.unwrap_or(0.0);
        let mut input_side = 0i128;
        let mut output = 0i128;
        let mut peak = 0i128;
        for row in &usage {
            let side = i128::from(row.input_uncached)
                + i128::from(row.cache_write)
                + i128::from(row.cache_read);
            input_side += side;
            output += i128::from(row.output);
            peak = peak.max(side);
        }
        let cost = (input_side as f64) * input_rate / 1_000_000.0
            + (output as f64) * output_rate / 1_000_000.0;
        let (ratio, status) = match tier.context_limit {
            Some(limit) if limit > 0 => (Some(peak as f64 / limit as f64), InsightStatus::Computed),
            _ => (
                None,
                InsightStatus::Partial {
                    reason: "missing context limit".to_string(),
                },
            ),
        };
        let mut measures = vec![
            float_measure(
                "I25.observed_cache_read_ratio",
                "observed_cache_read_ratio",
                observed,
                "ratio",
                "observed sum(cache_read)/sum(input_uncached+cache_write+cache_read); the I02 ratio stays reportable",
            ),
            float_measure(
                "I25.cache_bleed_inflection",
                "cache_bleed_inflection",
                inflection,
                "ratio",
                "QUANTILE_CONT of cache-read fraction at 0.9 after the ASOF alignment",
            ),
            float_measure(
                "I25.simulated_cost",
                "simulated_cost",
                Some(cost),
                "usd",
                "fast-tier input and output rates from the pinned table, per million tokens",
            ),
            float_measure(
                "I25.simulated_context_ratio",
                "simulated_context_ratio",
                ratio,
                "ratio",
                "peak input side over the pinned context limit; unknown when the limit is absent",
            ),
        ];
        measures.sort_by(|left, right| left.id.cmp(&right.id));
        finish(InsightResult {
            id: self.id(),
            name: self.name(),
            version: self.version(),
            domain: self.domain(),
            status,
            population: InsightPopulation {
                what: "usage_facts".to_string(),
                n: usage.len() as u64,
            },
            measures,
            coverage: COVERAGE.to_string(),
            evidence: Vec::new(),
        })
    }
}

fn simulation_refused(
    insight: &CounterfactualRouting,
    usage: &[Usage],
    observed: Option<f64>,
    reason: &str,
) -> InsightResult {
    finish(InsightResult {
        id: insight.id(),
        name: insight.name(),
        version: insight.version(),
        domain: insight.domain(),
        status: InsightStatus::Refused {
            reason: reason.to_string(),
        },
        population: InsightPopulation {
            what: "usage_facts".to_string(),
            n: usage.len() as u64,
        },
        measures: vec![
            float_measure(
                "I25.observed_cache_read_ratio",
                "observed_cache_read_ratio",
                observed,
                "ratio",
                "observed I02 cache-read ratio of sums; the simulation is refused",
            ),
            unknown_cost(),
        ],
        coverage: COVERAGE.to_string(),
        evidence: Vec::new(),
    })
}

fn unknown_cost() -> Measure {
    Measure {
        id: "I25.simulated_cost".to_string(),
        name: "simulated_cost".to_string(),
        value: None,
        unit: "usd".to_string(),
        basis: "unknown; the fast tier is unpinned or the cohort is noncomparable".to_string(),
    }
}

fn observed_ratio(usage: &[Usage]) -> Option<f64> {
    let mut cache = 0i128;
    let mut denom = 0i128;
    for row in usage {
        if row.input_uncached < 0 || row.cache_write < 0 || row.cache_read < 0 {
            return None;
        }
        cache += i128::from(row.cache_read);
        denom += i128::from(row.input_uncached)
            + i128::from(row.cache_write)
            + i128::from(row.cache_read);
    }
    (denom > 0).then_some(cache as f64 / denom as f64)
}

fn load_usage(input: &InsightInput<'_>) -> anyhow::Result<Vec<Usage>> {
    let mut stmt = input.connection().prepare(
        "SELECT harness, input_uncached, cache_write, cache_read, output_tokens
         FROM usage_facts
         WHERE (? IS NULL OR harness = ?)
           AND (? IS NULL OR native_session = ?)
           AND (? IS NULL OR ts >= CAST(? AS TIMESTAMP))
           AND (? IS NULL OR ts < CAST(? AS TIMESTAMP))
         ORDER BY source_key",
    )?;
    let scope = input.scope();
    let harness = scope.harness.as_deref();
    let session = scope.session.as_deref();
    let from = scope.from.as_deref();
    let to = scope.to.as_deref();
    let rows = stmt.query_map(
        params![harness, harness, session, session, from, from, to, to],
        |row| {
            Ok(Usage {
                harness: row.get(0)?,
                input_uncached: row.get(1)?,
                cache_write: row.get(2)?,
                cache_read: row.get(3)?,
                output: row.get(4)?,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_tier(input: &InsightInput<'_>) -> anyhow::Result<Option<Tier>> {
    if !table_exists(input.connection(), "_dendro_pricing_tiers") {
        return Ok(None);
    }
    let has_limit = column_exists(input.connection(), "_dendro_pricing_tiers", "context_limit");
    let sql = if has_limit {
        "SELECT profile_pin, cohort_pin, input_rate, output_rate, context_limit
         FROM _dendro_pricing_tiers
         WHERE tier_id = 'fast'"
    } else {
        "SELECT profile_pin, cohort_pin, input_rate, output_rate, CAST(NULL AS BIGINT)
         FROM _dendro_pricing_tiers
         WHERE tier_id = 'fast'"
    };
    let mut stmt = input.connection().prepare(sql)?;
    let mut rows = stmt.query([])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    Ok(Some(Tier {
        profile_pin: row.get(0)?,
        cohort_pin: row.get(1)?,
        input_rate: row.get(2)?,
        output_rate: row.get(3)?,
        context_limit: row.get(4)?,
    }))
}

fn cache_inflection(input: &InsightInput<'_>, cohort: &str) -> anyhow::Result<Option<f64>> {
    let value: Option<f64> = input.connection().query_row(
        "WITH series AS (
            SELECT ts,
                   CAST(cache_read AS DOUBLE)
                     / NULLIF(input_uncached + cache_write + cache_read, 0) AS frac
            FROM usage_facts
            WHERE harness = ?
              AND ts IS NOT NULL
         ),
         baseline AS (
            SELECT ts, quantile_cont(frac, 0.5) AS baseline_frac
            FROM series
            WHERE frac IS NOT NULL
            GROUP BY ts
         ),
         aligned AS (
            SELECT s.frac
            FROM series s
            ASOF JOIN baseline b ON s.ts >= b.ts
            WHERE s.frac IS NOT NULL
         )
         SELECT quantile_cont(frac, 0.9) FROM aligned",
        params![cohort],
        |row| row.get(0),
    )?;
    Ok(value.filter(|value| value.is_finite()))
}
