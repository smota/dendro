//! Population totals over unique usage observations.
//!
//! Observed currency and estimated currency are never added together.
//! A missing cost is unknown, not zero. Inclusive per-root totals walk
//! parent→child links and are not additive across roots. Shared cost with
//! no explicit weights stays unallocated.
use crate::domain_v1::{
    AttributionScope, CounterSemantics, Inclusion, TokenComponent, UsageObservation, Validate,
};
use std::collections::{BTreeMap, BTreeSet};

pub use crate::domain_v1::{Decimal, Money, Ratio};

/// Catalog rates in USD per million tokens.
///
/// Empty on purpose: repo docs and config cite no provider price list.
/// An operator-supplied spotlight price is not a row in this table.
/// Reasoning is inside output and is not priced again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TokenRates {
    pub model: &'static str,
    pub input_uncached_per_million: f64,
    pub output_per_million: f64,
    pub cache_read_per_million: f64,
    pub cache_write_per_million: f64,
}

pub const MODEL_RATES: &[TokenRates] = &[];

/// Unpriced models are `None`, never `0`.
pub fn estimate_usd(
    model: Option<&str>,
    input_uncached: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
) -> Option<f64> {
    let name = model?;
    let rate = MODEL_RATES.iter().find(|row| row.model == name)?;
    if [input_uncached, output, cache_read, cache_write]
        .into_iter()
        .any(|n| n < 0)
    {
        return None;
    }
    let usd = (input_uncached as f64) * rate.input_uncached_per_million / 1_000_000.0
        + (output as f64) * rate.output_per_million / 1_000_000.0
        + (cache_read as f64) * rate.cache_read_per_million / 1_000_000.0
        + (cache_write as f64) * rate.cache_write_per_million / 1_000_000.0;
    if !usd.is_finite() || usd < 0.0 {
        return None;
    }
    Some(usd)
}

/// Label on every inclusive root total. Summing these with child exclusive
/// totals counts descendants twice.
pub const INCLUSIVE_LABEL: &str = "inclusive (with sub-agents)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountingRefusal {
    NoObservations,
    AccountingPolicyNotAdmitted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostStatus {
    Complete,
    Partial,
    Unknown,
}

/// `known_usd` is the sum of finite observations only.
/// [`CostRollup::total_usd`] is `None` unless every row in the set was finite,
/// so a partial set is not reported as the smaller known sum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CostRollup {
    pub status: CostStatus,
    pub known_usd: f64,
    pub known_rows: u64,
    pub unknown_rows: u64,
}

impl CostRollup {
    pub fn total_usd(self) -> Option<f64> {
        match self.status {
            CostStatus::Complete => Some(self.known_usd),
            CostStatus::Partial | CostStatus::Unknown => None,
        }
    }

    pub fn empty() -> Self {
        Self {
            status: CostStatus::Unknown,
            known_usd: 0.0,
            known_rows: 0,
            unknown_rows: 0,
        }
    }
}

pub fn rollup_costs<I>(costs: I) -> CostRollup
where
    I: IntoIterator<Item = Option<f64>>,
{
    let mut known_usd = 0.0;
    let mut known_rows = 0u64;
    let mut unknown_rows = 0u64;
    let mut saw = false;
    for cost in costs {
        saw = true;
        match cost.filter(|n| n.is_finite()) {
            Some(n) => {
                known_usd += n;
                known_rows += 1;
            }
            None => unknown_rows += 1,
        }
    }
    let status = if !saw || known_rows == 0 {
        CostStatus::Unknown
    } else if unknown_rows == 0 {
        CostStatus::Complete
    } else {
        CostStatus::Partial
    };
    CostRollup {
        status,
        known_usd,
        known_rows,
        unknown_rows,
    }
}

/// Merge exclusive session rollups. Empty parts add no rows.
pub fn merge_rollups(parts: &[CostRollup]) -> CostRollup {
    let known_rows = parts.iter().map(|part| part.known_rows).sum();
    let unknown_rows = parts.iter().map(|part| part.unknown_rows).sum();
    let known_usd = parts.iter().map(|part| part.known_usd).sum();
    let status = if known_rows == 0 {
        CostStatus::Unknown
    } else if unknown_rows == 0 {
        CostStatus::Complete
    } else {
        CostStatus::Partial
    };
    CostRollup {
        status,
        known_usd,
        known_rows,
        unknown_rows,
    }
}

/// Explicit weights that sum to 1 allocate the shared amount.
/// Missing weights, empty weights, and weights that do not sum to 1 stay
/// unallocated. This does not invent equal shares.
pub fn allocate_shared(amount: f64, weights: Option<&[f64]>) -> Option<Vec<f64>> {
    let weights = weights?;
    if weights.is_empty() || !amount.is_finite() {
        return None;
    }
    if weights
        .iter()
        .any(|weight| !weight.is_finite() || *weight < 0.0)
    {
        return None;
    }
    let sum: f64 = weights.iter().copied().sum();
    if (sum - 1.0).abs() > 1e-9 {
        return None;
    }
    Some(weights.iter().map(|weight| amount * weight).collect())
}

#[derive(Clone, Debug, PartialEq)]
pub struct UsageSlice {
    pub source_key: String,
    pub harness: String,
    pub session: String,
    pub input_uncached: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    pub observed_usd: Option<f64>,
    pub estimated_usd: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenSum {
    pub input_uncached: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
}

impl TokenSum {
    fn add(&mut self, row: &UsageSlice) {
        self.input_uncached += row.input_uncached;
        self.output += row.output;
        self.cache_read += row.cache_read;
        self.cache_write += row.cache_write;
        self.reasoning += row.reasoning;
    }

    pub fn tokens(self) -> i64 {
        self.input_uncached + self.output + self.cache_read + self.cache_write
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Population {
    pub tokens: TokenSum,
    pub observed: CostRollup,
    pub estimated: CostRollup,
}

impl Population {
    fn from_rows<'a>(rows: impl IntoIterator<Item = &'a UsageSlice>) -> Self {
        let mut tokens = TokenSum::default();
        let mut observed = Vec::new();
        let mut estimated = Vec::new();
        for row in rows {
            tokens.add(row);
            observed.push(row.observed_usd.filter(|n| n.is_finite()));
            estimated.push(row.estimated_usd.filter(|n| n.is_finite()));
        }
        Self {
            tokens,
            observed: rollup_costs(observed),
            estimated: rollup_costs(estimated),
        }
    }

    pub fn empty() -> Self {
        Self {
            tokens: TokenSum::default(),
            observed: CostRollup::empty(),
            estimated: CostRollup::empty(),
        }
    }
}

/// One row per `source_key`. A repeated key is the same observation.
pub fn fleet(rows: &[UsageSlice]) -> Population {
    Population::from_rows(unique(rows))
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionTotal {
    pub harness: String,
    pub session: String,
    pub population: Population,
}

/// Per-session totals. Their token sums and cost row counts add up to [`fleet`].
pub fn exclusive_sessions(rows: &[UsageSlice]) -> Vec<SessionTotal> {
    let mut grouped: BTreeMap<(String, String), Vec<&UsageSlice>> = BTreeMap::new();
    for row in unique(rows) {
        grouped
            .entry((row.harness.clone(), row.session.clone()))
            .or_default()
            .push(row);
    }
    grouped
        .into_iter()
        .map(|((harness, session), group)| SessionTotal {
            harness,
            session,
            population: Population::from_rows(group),
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionLink {
    pub harness: String,
    pub parent_session: String,
    pub child_session: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InclusiveTotal {
    pub harness: String,
    pub session: String,
    pub population: Population,
    pub label: &'static str,
    pub non_additive: bool,
}

/// Roots are sessions that are not a child on `edges`.
/// Each total includes the root and its descendants once.
/// `non_additive` is always set: adding a root total to a child's exclusive
/// total counts the child twice.
pub fn inclusive_roots(rows: &[UsageSlice], edges: &[SessionLink]) -> Vec<InclusiveTotal> {
    let unique_rows = unique(rows);
    let mut children: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    let mut child_ids: BTreeSet<(String, String)> = BTreeSet::new();
    for edge in edges {
        if edge.parent_session.is_empty()
            || edge.child_session.is_empty()
            || edge.parent_session == edge.child_session
        {
            continue;
        }
        children
            .entry((edge.harness.clone(), edge.parent_session.clone()))
            .or_default()
            .insert(edge.child_session.clone());
        child_ids.insert((edge.harness.clone(), edge.child_session.clone()));
    }
    let mut nodes: BTreeSet<(String, String)> = BTreeSet::new();
    for row in &unique_rows {
        nodes.insert((row.harness.clone(), row.session.clone()));
    }
    for parent in children.keys() {
        nodes.insert(parent.clone());
    }
    let mut by_session: BTreeMap<(String, String), Vec<&UsageSlice>> = BTreeMap::new();
    for row in &unique_rows {
        by_session
            .entry((row.harness.clone(), row.session.clone()))
            .or_default()
            .push(*row);
    }
    nodes
        .into_iter()
        .filter(|node| !child_ids.contains(node))
        .map(|(harness, session)| {
            let mut members = Vec::new();
            let mut stack = vec![session.clone()];
            let mut seen = BTreeSet::new();
            while let Some(current) = stack.pop() {
                if !seen.insert(current.clone()) {
                    continue;
                }
                if let Some(group) = by_session.get(&(harness.clone(), current.clone())) {
                    members.extend(group.iter().copied());
                }
                if let Some(next) = children.get(&(harness.clone(), current)) {
                    stack.extend(next.iter().cloned());
                }
            }
            InclusiveTotal {
                harness,
                session,
                population: Population::from_rows(members),
                label: INCLUSIVE_LABEL,
                non_additive: true,
            }
        })
        .collect()
}

fn unique(rows: &[UsageSlice]) -> Vec<&UsageSlice> {
    let mut seen = BTreeMap::<&str, &UsageSlice>::new();
    for row in rows {
        seen.entry(row.source_key.as_str()).or_insert(row);
    }
    seen.into_values().collect()
}

/// Admitted observations can be totaled without double counting or an invented split.
/// Still refused: an empty set, duplicate ids, cumulative/gauge/unknown counters,
/// shared or legacy attribution (no weights on the observation), unknown inclusion,
/// a source-reported total beside the components, and equivalence evidence
/// (no dedupe rule). `IncludedIn` components are not added on top of their parent.
pub fn check_total_admission(observations: &[UsageObservation]) -> Result<(), AccountingRefusal> {
    total_observed_money(observations).map(|_| ())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservedMoney {
    Complete(Money),
    Partial { unknown_rows: u64, known_rows: u64 },
    Unknown { rows: u64 },
}

pub fn total_observed_money(
    observations: &[UsageObservation],
) -> Result<ObservedMoney, AccountingRefusal> {
    if observations.is_empty() {
        return Err(AccountingRefusal::NoObservations);
    }
    let mut seen = BTreeSet::new();
    for observation in observations {
        if !seen.insert(&observation.id) || !observation_admitted(observation) {
            return Err(AccountingRefusal::AccountingPolicyNotAdmitted);
        }
    }
    let mut known: Option<Money> = None;
    let mut known_rows = 0u64;
    let mut unknown_rows = 0u64;
    for observation in observations {
        match &observation.observed_currency {
            None => unknown_rows += 1,
            Some(money) => {
                known_rows += 1;
                known = Some(match known {
                    None => money.clone(),
                    Some(acc) => acc
                        .checked_add(money)
                        .map_err(|_| AccountingRefusal::AccountingPolicyNotAdmitted)?,
                });
            }
        }
    }
    if unknown_rows == 0 {
        Ok(ObservedMoney::Complete(
            known.expect("at least one currency"),
        ))
    } else if known_rows == 0 {
        Ok(ObservedMoney::Unknown { rows: unknown_rows })
    } else {
        // The known sum is intentionally absent. Reporting it would be the smaller number.
        Ok(ObservedMoney::Partial {
            unknown_rows,
            known_rows,
        })
    }
}

fn observation_admitted(observation: &UsageObservation) -> bool {
    if observation.counter.semantics != CounterSemantics::Delta {
        return false;
    }
    if !matches!(
        observation.attribution,
        AttributionScope::Invocation
            | AttributionScope::Attempt
            | AttributionScope::Assignment
            | AttributionScope::Unattributed
    ) {
        return false;
    }
    if !observation.equivalence_evidence.is_empty() {
        return false;
    }
    if let Some(money) = &observation.observed_currency {
        if money.validate().is_err() {
            return false;
        }
    }
    for component in &observation.components {
        if component.component == TokenComponent::SourceReportedTotal {
            return false;
        }
        match component.inclusion {
            Inclusion::Unknown => return false,
            Inclusion::IncludedIn(other) if other == component.component => return false,
            Inclusion::Exclusive | Inclusion::IncludedIn(_) => {}
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain_v1::*;

    fn slice(
        key: &str,
        harness: &str,
        session: &str,
        tokens: i64,
        observed: Option<f64>,
        estimated: Option<f64>,
    ) -> UsageSlice {
        UsageSlice {
            source_key: key.into(),
            harness: harness.into(),
            session: session.into(),
            input_uncached: tokens,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            observed_usd: observed,
            estimated_usd: estimated,
        }
    }

    #[test]
    fn observed_plus_unknown_is_partial_not_the_smaller_sum() {
        let rollup = rollup_costs([Some(1.0), Some(2.0), None]);
        assert_eq!(rollup.status, CostStatus::Partial);
        assert_eq!(rollup.unknown_rows, 1);
        assert_eq!(rollup.known_rows, 2);
        assert_eq!(rollup.known_usd, 3.0);
        assert_eq!(rollup.total_usd(), None);
        assert_ne!(rollup.total_usd(), Some(3.0));
        assert_ne!(rollup.total_usd(), Some(1.0));
        let all_missing = rollup_costs([None, None]);
        assert_eq!(all_missing.status, CostStatus::Unknown);
        assert_eq!(all_missing.total_usd(), None);
        assert_ne!(all_missing.total_usd(), Some(0.0));
    }

    #[test]
    fn exclusive_sessions_sum_to_the_fleet_and_duplicate_keys_count_once() {
        let rows = vec![
            slice("a", "grok", "parent", 10, Some(1.0), None),
            slice("a", "grok", "parent", 99, Some(50.0), None),
            slice("b", "grok", "child", 7, None, None),
            slice("c", "pi", "solo", 3, Some(0.5), None),
        ];
        let fleet = fleet(&rows);
        assert_eq!(fleet.tokens.tokens(), 20);
        let exclusive = exclusive_sessions(&rows);
        let exclusive_tokens: i64 = exclusive
            .iter()
            .map(|session| session.population.tokens.tokens())
            .sum();
        assert_eq!(exclusive_tokens, fleet.tokens.tokens());
        let merged = merge_rollups(
            &exclusive
                .iter()
                .map(|session| session.population.observed)
                .collect::<Vec<_>>(),
        );
        assert_eq!(merged.known_rows, fleet.observed.known_rows);
        assert_eq!(merged.unknown_rows, fleet.observed.unknown_rows);
        assert_eq!(merged.status, fleet.observed.status);
        assert_eq!(merged.total_usd(), fleet.observed.total_usd());
        assert_eq!(fleet.observed.status, CostStatus::Partial);
        assert_eq!(fleet.estimated.status, CostStatus::Unknown);
    }

    #[test]
    fn inclusive_root_is_at_least_exclusive_and_labeled_non_additive() {
        let rows = vec![
            slice("p", "grok", "parent", 10, Some(1.0), None),
            slice("c", "grok", "child", 7, Some(0.25), None),
            slice("o", "pi", "other", 4, Some(0.25), None),
        ];
        let edges = vec![SessionLink {
            harness: "grok".into(),
            parent_session: "parent".into(),
            child_session: "child".into(),
        }];
        let fleet = fleet(&rows);
        let exclusive = exclusive_sessions(&rows);
        let parent = exclusive
            .iter()
            .find(|session| session.session == "parent")
            .unwrap();
        let child = exclusive
            .iter()
            .find(|session| session.session == "child")
            .unwrap();
        let inclusive = inclusive_roots(&rows, &edges);
        let root = inclusive
            .iter()
            .find(|session| session.session == "parent")
            .unwrap();
        assert!(root.population.tokens.tokens() >= parent.population.tokens.tokens());
        assert_eq!(root.population.tokens.tokens(), 17);
        assert_eq!(root.label, INCLUSIVE_LABEL);
        assert!(root.non_additive);
        assert!(
            root.population.tokens.tokens() + child.population.tokens.tokens()
                > fleet.tokens.tokens()
        );
        let summed_inclusive: i64 = inclusive
            .iter()
            .map(|session| session.population.tokens.tokens())
            .sum();
        assert_eq!(summed_inclusive, fleet.tokens.tokens());
        assert!(inclusive.iter().all(|session| session.non_additive));
        assert!(inclusive.iter().all(|session| session.session != "child"));
    }

    #[test]
    fn claude_sidechain_usage_stays_on_the_parent_session() {
        // Claude stores sidechain usage on `sessionId` (the parent). `agentId`
        // is only the child on the lineage edge and has no usage row. The
        // inclusive parent total must not add a second copy of those tokens.
        let rows = vec![slice(
            "parent-line",
            "claude",
            "parent-session",
            19,
            None,
            None,
        )];
        let edges = vec![SessionLink {
            harness: "claude".into(),
            parent_session: "parent-session".into(),
            child_session: "child-agent".into(),
        }];
        let fleet = fleet(&rows);
        let exclusive = exclusive_sessions(&rows);
        assert_eq!(exclusive.len(), 1);
        assert_eq!(exclusive[0].population.tokens.tokens(), 19);
        let child = exclusive
            .iter()
            .find(|session| session.session == "child-agent");
        assert!(child.is_none());
        let inclusive = inclusive_roots(&rows, &edges);
        let parent = inclusive
            .iter()
            .find(|session| session.session == "parent-session")
            .unwrap();
        assert_eq!(parent.population.tokens.tokens(), fleet.tokens.tokens());
        assert_eq!(parent.population.tokens.tokens(), 19);
        assert_eq!(parent.label, INCLUSIVE_LABEL);
        assert!(parent.non_additive);
        assert_eq!(parent.population.observed.status, CostStatus::Unknown);
    }

    #[test]
    fn shared_cost_without_weights_is_unallocated() {
        assert!(allocate_shared(10.0, None).is_none());
        assert!(allocate_shared(10.0, Some(&[])).is_none());
        assert!(allocate_shared(10.0, Some(&[0.5])).is_none());
        let split = allocate_shared(10.0, Some(&[0.25, 0.75])).unwrap();
        assert_eq!(split, vec![2.5, 7.5]);
    }

    #[test]
    fn unpriced_model_is_none_not_zero() {
        assert!(MODEL_RATES.is_empty());
        assert_eq!(
            estimate_usd(Some("grok-4.7-build-fast"), 1_000, 1_000, 0, 0),
            None
        );
        assert_eq!(estimate_usd(None, 0, 0, 0, 0), None);
    }

    fn factory() -> IdentityFactory {
        IdentityFactory::new(&[7; 32], Identifier::new("acct-test").unwrap()).unwrap()
    }

    fn observation(name: &str, currency: Option<Money>) -> UsageObservation {
        UsageObservation {
            id: UsageId::derive(&factory(), &[name.as_bytes()]),
            invocation: None,
            attempt: None,
            components: vec![UsageComponent {
                component: TokenComponent::InputUncached,
                quantity: Count(1),
                inclusion: Inclusion::Exclusive,
            }],
            observed_currency: currency,
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

    fn usd(coefficient: i128) -> Money {
        Money::from_source(
            coefficient,
            1,
            CurrencyCode::try_from("USD".to_string()).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn admission_accepts_delta_rows_and_refuses_undefined_totals() {
        assert_eq!(
            check_total_admission(&[]),
            Err(AccountingRefusal::NoObservations)
        );
        let priced = observation("a", Some(usd(2)));
        let missing = observation("b", None);
        assert_eq!(check_total_admission(std::slice::from_ref(&priced)), Ok(()));
        match total_observed_money(&[priced, missing]).unwrap() {
            ObservedMoney::Partial {
                unknown_rows,
                known_rows,
            } => {
                assert_eq!((unknown_rows, known_rows), (1, 1));
            }
            other => panic!("partial total dropped the unknown row: {other:?}"),
        }
        let mut shared = observation("shared", Some(usd(1)));
        shared.attribution = AttributionScope::Shared;
        assert_eq!(
            check_total_admission(&[shared]),
            Err(AccountingRefusal::AccountingPolicyNotAdmitted)
        );
        let mut cumulative = observation("cumulative", None);
        cumulative.counter.semantics = CounterSemantics::Cumulative;
        cumulative.counter.identity = Some(CounterId::derive(&factory(), &[b"counter"]));
        assert_eq!(
            check_total_admission(&[cumulative]),
            Err(AccountingRefusal::AccountingPolicyNotAdmitted)
        );
        let duplicate = observation("a", None);
        assert_eq!(
            check_total_admission(&[duplicate.clone(), duplicate]),
            Err(AccountingRefusal::AccountingPolicyNotAdmitted)
        );
    }
}
