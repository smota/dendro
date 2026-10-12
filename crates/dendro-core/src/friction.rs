//! G2 #12 rework friction (Rev 5): handoff loops, the pair friction matrix,
//! five attribution bands per seat and advisory alerts. Every number here is
//! a breakdown of canonical R1 values (M2 pairs, M3 chains, seat totals); the
//! optional configuration only changes advisory alert lines.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::privacy::redact;

pub const FRICTION_DEFINITION_VERSION: &str = "g2-rework-friction-1";

pub const BAND_DEFINITION: &str = "Attribution bands (G2 F12-2, Rev 4/5): each linked, timestamped usage event in the period counts in exactly one band of its seat. Its candidate items are the items the seat owns at that instant (K3/K5 historical destination). No candidate item: no item. Unknown history, or candidates in more than one distinct chain: ambiguous. One chain: rework or unknown when the period's M3 classifies that chain so, otherwise other chains (not rework or unknown in this period). Several candidate items in one chain count once. Unlinked and collision sessions belong to no seat; untimestamped records stay outside the period.";

pub const PAIR_TOKENS_NOTE: &str = "full chain, not additive across pairs";

const CATEGORIES: [&str; 4] = ["input_uncached", "output", "cache_read", "cache_write"];
const MAX_CONFIG_BYTES: usize = 64 * 1024;

/// Advisory thresholds (versioned JSON, version 1). Defaults match the frozen
/// G2 acceptance; only alerts read them.
#[derive(Debug, Clone, PartialEq)]
pub struct FrictionConfig {
    pub return_rate_threshold: f64,
    pub min_handoffs: i64,
    pub rework_token_threshold: i64,
    pub rework_token_basis: Vec<String>,
    pub handoff_loop_alert_min_passes: i64,
    /// Where the values came from: defaults, or a redacted path and digest.
    pub source: Value,
}

impl Default for FrictionConfig {
    fn default() -> Self {
        FrictionConfig {
            return_rate_threshold: 0.10,
            min_handoffs: 5,
            rework_token_threshold: 1_000_000,
            rework_token_basis: vec![
                "input_uncached".into(),
                "output".into(),
                "cache_write".into(),
            ],
            handoff_loop_alert_min_passes: 3,
            source: json!({"source": "default"}),
        }
    }
}

impl FrictionConfig {
    pub fn json(&self) -> Value {
        json!({
            "version": 1,
            "return_rate_threshold": self.return_rate_threshold,
            "min_handoffs": self.min_handoffs,
            "rework_token_threshold": self.rework_token_threshold,
            "rework_token_basis": self.rework_token_basis,
            "handoff_loop_alert_min_passes": self.handoff_loop_alert_min_passes,
            "provenance": self.source,
        })
    }

    /// Loads and validates an operator configuration file strictly. Errors
    /// never echo file content, key names or values.
    pub fn load(path: Option<&Path>) -> Result<FrictionConfig> {
        let Some(path) = path else {
            return Ok(FrictionConfig::default());
        };
        let shown = redact(&path.display().to_string());
        let bytes = std::fs::read(path).map_err(|_| anyhow!("config {shown}: cannot be read"))?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(anyhow!(
                "config {shown}: larger than {MAX_CONFIG_BYTES} bytes"
            ));
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| anyhow!("config {shown}: not UTF-8"))?;
        let value = strict_json(text).map_err(|reason| anyhow!("config {shown}: {reason}"))?;
        let mut config = FrictionConfig::default();
        apply(&mut config, &value).map_err(|reason| anyhow!("config {shown}: {reason}"))?;
        config.source = json!({
            "source": "file",
            "path": shown,
            "sha256": hex::encode(Sha256::digest(&bytes)),
        });
        Ok(config)
    }
}

/// JSON parsing that refuses duplicate object keys at any depth.
fn strict_json(text: &str) -> std::result::Result<Value, &'static str> {
    use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};
    struct Strict(Value);
    impl<'de> serde::Deserialize<'de> for Strict {
        fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
            struct V;
            impl<'de> Visitor<'de> for V {
                type Value = Strict;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("JSON")
                }
                fn visit_bool<E>(self, v: bool) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::Bool(v)))
                }
                fn visit_i64<E>(self, v: i64) -> std::result::Result<Strict, E> {
                    Ok(Strict(json!(v)))
                }
                fn visit_u64<E>(self, v: u64) -> std::result::Result<Strict, E> {
                    Ok(Strict(json!(v)))
                }
                fn visit_f64<E>(self, v: f64) -> std::result::Result<Strict, E> {
                    Ok(Strict(json!(v)))
                }
                fn visit_str<E>(self, v: &str) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::String(v.to_string())))
                }
                fn visit_string<E>(self, v: String) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::String(v)))
                }
                fn visit_unit<E>(self) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::Null))
                }
                fn visit_seq<A: SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> std::result::Result<Strict, A::Error> {
                    let mut out = Vec::new();
                    while let Some(Strict(item)) = seq.next_element()? {
                        out.push(item);
                    }
                    Ok(Strict(Value::Array(out)))
                }
                fn visit_map<A: MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> std::result::Result<Strict, A::Error> {
                    let mut out = serde_json::Map::new();
                    while let Some(key) = map.next_key::<String>()? {
                        let Strict(value) = map.next_value()?;
                        if out.insert(key, value).is_some() {
                            return Err(serde::de::Error::custom("duplicate key"));
                        }
                    }
                    Ok(Strict(Value::Object(out)))
                }
            }
            d.deserialize_any(V)
        }
    }
    match serde_json::from_str::<Strict>(text) {
        Ok(Strict(value)) => Ok(value),
        Err(error) if error.to_string().contains("duplicate key") => Err("duplicate key"),
        Err(_) => Err("not valid JSON"),
    }
}

fn apply(config: &mut FrictionConfig, value: &Value) -> std::result::Result<(), &'static str> {
    let top = value.as_object().ok_or("top level must be an object")?;
    if top.keys().any(|key| key != "version" && key != "friction") {
        return Err("unknown top-level key");
    }
    match top.get("version") {
        Some(version) if version.as_i64() == Some(1) && version.is_i64() => {}
        Some(_) => return Err("unsupported version (expected 1)"),
        None => return Err("missing version"),
    }
    let Some(friction) = top.get("friction") else {
        return Ok(());
    };
    let friction = friction.as_object().ok_or("'friction' must be an object")?;
    const KNOWN: [&str; 5] = [
        "return_rate_threshold",
        "min_handoffs",
        "rework_token_threshold",
        "rework_token_basis",
        "handoff_loop_alert_min_passes",
    ];
    if friction.keys().any(|key| !KNOWN.contains(&key.as_str())) {
        return Err("unknown key in 'friction'");
    }
    if let Some(v) = friction.get("return_rate_threshold") {
        let rate = v
            .as_f64()
            .filter(|r| v.is_number() && r.is_finite() && (0.0..=1.0).contains(r));
        config.return_rate_threshold =
            rate.ok_or("return_rate_threshold must be a finite number from 0 to 1")?;
    }
    if let Some(v) = friction.get("min_handoffs") {
        let guard = v.as_i64().filter(|g| *g >= 1);
        config.min_handoffs = guard.ok_or("min_handoffs must be a positive integer")?;
    }
    if let Some(v) = friction.get("rework_token_threshold") {
        let tokens = v.as_i64().filter(|t| *t >= 0);
        config.rework_token_threshold =
            tokens.ok_or("rework_token_threshold must be a non-negative integer")?;
    }
    if let Some(v) = friction.get("rework_token_basis") {
        let list = v.as_array().ok_or("rework_token_basis must be a list")?;
        let mut basis = Vec::new();
        for item in list {
            let name = item
                .as_str()
                .filter(|name| CATEGORIES.contains(name))
                .ok_or("rework_token_basis names an unknown category")?;
            if basis.iter().any(|seen: &String| seen == name) {
                return Err("rework_token_basis repeats a category");
            }
            basis.push(name.to_string());
        }
        if basis.is_empty() {
            return Err("rework_token_basis must not be empty");
        }
        config.rework_token_basis = basis;
    }
    if let Some(v) = friction.get("handoff_loop_alert_min_passes") {
        let passes = v.as_i64().filter(|p| *p >= 3);
        config.handoff_loop_alert_min_passes =
            passes.ok_or("handoff_loop_alert_min_passes must be an integer of at least 3")?;
    }
    Ok(())
}

/// Token categories of one band or chain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input_uncached: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
}

impl Tokens {
    pub fn add(&mut self, other: &Tokens) {
        self.input_uncached += other.input_uncached;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }

    pub fn total(&self) -> i64 {
        self.input_uncached + self.output + self.cache_read + self.cache_write
    }

    pub fn of(&self, category: &str) -> i64 {
        match category {
            "input_uncached" => self.input_uncached,
            "output" => self.output,
            "cache_read" => self.cache_read,
            "cache_write" => self.cache_write,
            _ => 0,
        }
    }

    pub fn json(&self) -> Value {
        json!({
            "input_uncached": self.input_uncached,
            "output": self.output,
            "cache_read": self.cache_read,
            "cache_write": self.cache_write,
            "tokens": self.total(),
        })
    }
}

/// One linked, timestamped, in-period usage event with its owned candidates.
#[derive(Debug, Clone)]
pub struct BandEvent {
    pub source_key: String,
    pub seat: String,
    pub ts: String,
    pub tokens: Tokens,
    /// Items the seat owns at that instant (K3/K5 'owned' relation).
    pub owned_items: Vec<String>,
    /// Some item naming the seat has unprovable history.
    pub unknown_history: bool,
}

pub const BANDS: [&str; 5] = ["rework", "other", "unknown", "ambiguous", "no_item"];

/// The band of one event, with the chain it counts for when there is one.
pub fn band_of(
    event: &BandEvent,
    chain_of_item: &BTreeMap<String, String>,
    rework: &BTreeSet<String>,
    unknown: &BTreeSet<String>,
) -> (&'static str, Option<String>, Vec<String>) {
    let chains: BTreeSet<String> = event
        .owned_items
        .iter()
        .map(|item| {
            chain_of_item
                .get(item)
                .cloned()
                .unwrap_or_else(|| item.clone())
        })
        .collect();
    let list: Vec<String> = chains.iter().cloned().collect();
    if event.unknown_history || chains.len() > 1 {
        return ("ambiguous", None, list);
    }
    let Some(chain) = chains.into_iter().next() else {
        return ("no_item", None, list);
    };
    let band = if rework.contains(&chain) {
        "rework"
    } else if unknown.contains(&chain) {
        "unknown"
    } else {
        "other"
    };
    (band, Some(chain), list)
}

/// A share of a total, or "no data" when the total is 0 (never 0%).
pub fn share(part: i64, total: i64) -> Value {
    if total == 0 {
        json!("no data")
    } else {
        json!(part as f64 / total as f64)
    }
}

/// Everything the friction payload is built from: canonical R1 values for
/// the same build, rig scope and period.
pub struct FrictionInputs<'a> {
    pub edges: &'a [crate::HandoffEdge],
    pub pair_counts: &'a BTreeMap<(String, String), (i64, i64)>,
    pub m3: &'a crate::M3,
    pub events: &'a [BandEvent],
    pub chain_of_item: &'a BTreeMap<String, String>,
    pub seat_totals: &'a BTreeMap<String, i64>,
    pub unlinked: Tokens,
    pub collision: Tokens,
    pub untimestamped_count: usize,
    pub untimestamped_tokens: i64,
    pub config: &'a FrictionConfig,
}

pub fn payload(inputs: &FrictionInputs) -> Value {
    let config = inputs.config;
    let rework: BTreeSet<String> = inputs.m3.rework.iter().map(|r| r.chain.clone()).collect();
    let unknown: BTreeSet<String> = inputs.m3.unknown.iter().map(|u| u.chain.clone()).collect();
    // Bands per seat and per chain, from each event exactly once.
    let mut seat_bands: BTreeMap<String, BTreeMap<&'static str, Tokens>> = BTreeMap::new();
    let mut chain_tokens: BTreeMap<String, Tokens> = BTreeMap::new();
    let mut band_totals: BTreeMap<&'static str, Tokens> = BTreeMap::new();
    let mut event_rows = Vec::new();
    for event in inputs.events {
        let (band, chain, candidates) = band_of(event, inputs.chain_of_item, &rework, &unknown);
        seat_bands
            .entry(event.seat.clone())
            .or_default()
            .entry(band)
            .or_default()
            .add(&event.tokens);
        band_totals.entry(band).or_default().add(&event.tokens);
        if let Some(chain) = &chain {
            chain_tokens
                .entry(chain.clone())
                .or_default()
                .add(&event.tokens);
        }
        event_rows.push(json!({
            "source_key": event.source_key,
            "seat": event.seat,
            "ts": event.ts,
            "categories": event.tokens.json(),
            "owned_items": event.owned_items,
            "unknown_history": event.unknown_history,
            "candidate_chains": candidates,
            "band": band,
            "chain": chain,
        }));
    }
    let seats: Vec<Value> = inputs
        .seat_totals
        .iter()
        .map(|(seat, total)| {
            let bands = seat_bands.get(seat).cloned().unwrap_or_default();
            let sum: i64 = bands.values().map(Tokens::total).sum();
            json!({
                "seat": seat,
                "seat_total": total,
                "bands": BANDS.iter().map(|band| {
                    (band.to_string(), bands.get(band).cloned().unwrap_or_default().json())
                }).collect::<serde_json::Map<_, _>>(),
                "bands_sum": sum,
                "conserved": sum == *total,
            })
        })
        .collect();
    let linked_total: i64 = inputs.seat_totals.values().sum();
    let rework_tokens = band_totals.get("rework").cloned().unwrap_or_default();
    let no_item_tokens = band_totals.get("no_item").cloned().unwrap_or_default();
    // Rev 6/7 pair membership: each (chain, ordered pair) once.
    let mut pair_chains: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for chain in &inputs.m3.rework {
        for (from, to, _) in &chain.qualifying {
            pair_chains
                .entry((from.clone(), to.clone()))
                .or_default()
                .insert(chain.chain.clone());
        }
    }
    let full_chain = |chains: &BTreeSet<String>| -> Tokens {
        let mut sum = Tokens::default();
        for chain in chains {
            if let Some(tokens) = chain_tokens.get(chain) {
                sum.add(tokens);
            }
        }
        sum
    };
    let paired: BTreeSet<String> = pair_chains.values().flatten().cloned().collect();
    let unpaired: BTreeSet<String> = rework.difference(&paired).cloned().collect();
    let unpaired_tokens = full_chain(&unpaired);
    let reconciled: BTreeSet<String> = paired.union(&unpaired).cloned().collect();
    let reconciled_tokens = full_chain(&reconciled);
    // The matrix: every canonical ordered M2 pair with handoffs (self-handoff
    // rows included); rework membership never filters it.
    let matrix: Vec<Value> = inputs
        .pair_counts
        .iter()
        .filter(|(_, (_, handoffs))| *handoffs > 0)
        .map(|((from, to), (returns, handoffs))| {
            json!({
                "from": from,
                "to": to,
                "label": if from == to { Some("self-handoff") } else { None },
                "handoffs": handoffs,
                "returns": returns,
                "return_rate": crate::m2_return_rate(*returns, *handoffs),
                "rework_chains": pair_chains.get(&(from.clone(), to.clone())).cloned().unwrap_or_default(),
                "rework_tokens_full_chain": {
                    "note": PAIR_TOKENS_NOTE,
                    "categories": full_chain(&pair_chains.get(&(from.clone(), to.clone())).cloned().unwrap_or_default()).json(),
                },
            })
        })
        .collect();
    let loops = crate::handoff_loops(inputs.edges);
    // Advisory alerts: text only; they send, write and change nothing.
    let mut alerts = Vec::new();
    for ((from, to), (returns, handoffs)) in inputs.pair_counts.iter().filter(|(_, (_, h))| *h > 0)
    {
        let rate = *returns as f64 / *handoffs as f64;
        let decision = if *handoffs < config.min_handoffs {
            "sample too small"
        } else if rate > config.return_rate_threshold {
            "alert"
        } else {
            "below threshold"
        };
        let text = match decision {
            "alert" => Some(format!(
                "Return rate {from} -> {to}: {returns} of {handoffs} handoffs returned ({:.1}%), above {:.1}%.",
                rate * 100.0,
                config.return_rate_threshold * 100.0
            )),
            "sample too small" => Some(format!(
                "Return rate {from} -> {to}: sample too small ({handoffs} handoffs, fewer than {}).",
                config.min_handoffs
            )),
            _ => None,
        };
        alerts.push(json!({
            "kind": "return_rate",
            "from": from,
            "to": to,
            "handoffs": handoffs,
            "returns": returns,
            "decision": decision,
            "text": text,
        }));
    }
    for ((from, to), chains) in &pair_chains {
        let tokens = full_chain(chains);
        let basis: i64 = config.rework_token_basis.iter().map(|c| tokens.of(c)).sum();
        let decision = if basis > config.rework_token_threshold {
            "alert"
        } else {
            "below threshold"
        };
        alerts.push(json!({
            "kind": "rework_tokens",
            "from": from,
            "to": to,
            "chains": chains,
            "basis": config.rework_token_basis,
            "basis_tokens": basis,
            "cache_read": tokens.cache_read,
            "decision": decision,
            "text": (decision == "alert").then(|| format!(
                "Rework tokens {from} -> {to}: {basis} tokens on the basis {} (cache read {} shown apart), above {}; full chain, not additive across pairs.",
                config.rework_token_basis.join("+"),
                tokens.cache_read,
                config.rework_token_threshold
            )),
        }));
    }
    for found in &loops {
        let passes = found.passes.len() as i64;
        let decision = if passes >= config.handoff_loop_alert_min_passes {
            "alert"
        } else {
            "below threshold"
        };
        alerts.push(json!({
            "kind": "handoff_loop",
            "chain": found.chain,
            "seats": [found.seats.0, found.seats.1],
            "passes": passes,
            "decision": decision,
            "text": (decision == "alert").then(|| format!(
                "Handoff loop on chain {}: {} passes between {} and {}.",
                found.chain, passes, found.seats.0, found.seats.1
            )),
        }));
    }
    json!({
        "friction_definition_version": FRICTION_DEFINITION_VERSION,
        "definitions": {
            "handoff_loop": crate::HANDOFF_LOOP_DEFINITION,
            "bands": BAND_DEFINITION,
            "pair_rework_tokens": PAIR_TOKENS_NOTE,
            "other_band_label": "other chains (not rework or unknown in this period)",
        },
        "config": config.json(),
        "matrix": matrix,
        "handoff_loops": loops,
        "rework_chains_distinct": inputs.m3.rework.len(),
        "rework_tokens_distinct": rework_tokens.json(),
        "unpaired_rework": {
            "line": format!(
                "rework chains with no qualifying pair: {} chains, {} tokens",
                unpaired.len(),
                unpaired_tokens.total()
            ),
            "chains": unpaired,
            "categories": unpaired_tokens.json(),
        },
        "reconciliation": {
            "rule": "distinct union of every pair's rework chains plus the unpaired chains equals the period's M3 rework chains; their tokens, each chain once, equal the distinct rework total; the pair column is never summed",
            "chains_equal_m3": reconciled == rework,
            "tokens_equal_distinct": reconciled_tokens == rework_tokens,
        },
        "unknown_chains": inputs.m3.unknown,
        "seats": seats,
        "linked_total": linked_total,
        "shares": {
            "rework": share(rework_tokens.total(), linked_total),
            "no_item": share(no_item_tokens.total(), linked_total),
        },
        "unlinked": {
            "unlinked": inputs.unlinked.json(),
            "collision": inputs.collision.json(),
            "tokens": inputs.unlinked.total() + inputs.collision.total(),
        },
        "untimestamped": {
            "count": inputs.untimestamped_count,
            "tokens_all_time": inputs.untimestamped_tokens,
            "scope": "outside the period, unchanged from R1",
        },
        "alerts": alerts,
        "evidence": {
            "handoffs": inputs.edges.iter().map(|edge| json!({
                "source_key": edge.source_key,
                "at": edge.ts.to_rfc3339(),
                "from": edge.from_seat,
                "to": edge.to_seat,
                "qitem": edge.qitem_id,
                "chain": edge.chain_key(),
                "is_return": edge.is_return,
            })).collect::<Vec<_>>(),
            "rework_chains": inputs.m3.rework.iter().map(|chain| json!({
                "chain": chain.chain,
                "path": chain.path,
                "checks": chain.checks,
                "qualifying_edges": chain.qualifying.iter().map(|(from, to, key)| json!({
                    "from": from, "to": to, "source_key": key,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "chain_of_item": inputs.chain_of_item,
            "chain_tokens": chain_tokens.iter().map(|(chain, t)| (chain.clone(), t.json())).collect::<serde_json::Map<_, _>>(),
            "events": event_rows,
        },
    })
}
