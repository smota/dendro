//! Trace-only file provenance captured in the analytics snapshot. Observation is
//! deliberately a second phase, after the read connection has been dropped.
use crate::file_metadata::{
    lexical_reason, observe_path, safe_identity, FileMetadata, PathObservation,
};
use crate::metrics::{binding_of, SessionBinding};
use crate::{AnalyticsStore, SourceCoverage, UsageFact};
use anyhow::Result;
use duckdb::{params, Connection};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

type SourceRow = (String, String, String, i64, Option<String>);
const BASIS: &str = "file-metadata-v1";

fn file_harness(h: &str) -> bool {
    matches!(h, "claude" | "codex" | "pi")
}

#[derive(Clone, Debug)]
struct Baseline {
    status: &'static str,
    attempt: Option<String>,
    observed: Option<String>,
    metadata: Option<FileMetadata>,
    stale: bool,
}

impl Baseline {
    fn unavailable(status: &'static str) -> Self {
        Self {
            status,
            attempt: None,
            observed: None,
            metadata: None,
            stale: false,
        }
    }
}

fn utc(s: &str) -> bool {
    let Some(body) = s.strip_suffix('Z').or_else(|| s.strip_suffix("+00:00")) else {
        return false;
    };
    let b = body.as_bytes();
    if b.len() < 19 || &b[..4] == b"0000" {
        return false;
    }
    for (i, &byte) in b[..19].iter().enumerate() {
        let valid = match i {
            4 | 7 => byte == b'-',
            10 => byte == b'T',
            13 | 16 => byte == b':',
            _ => byte.is_ascii_digit(),
        };
        if !valid {
            return false;
        }
    }
    if b[17] > b'5'
        || (b.len() > 19
            && (b[19] != b'.'
                || !(1..=9).contains(&(b.len() - 20))
                || !b[20..].iter().all(u8::is_ascii_digit)))
    {
        return false;
    }
    chrono::DateTime::parse_from_rfc3339(s).is_ok_and(|t| t.offset().local_minus_utc() == 0)
}

fn unsigned(s: &str) -> bool {
    s.parse::<u64>().is_ok_and(|n| n.to_string() == s)
}

fn signed(s: &str) -> bool {
    s.parse::<i128>().is_ok_and(|n| n.to_string() == s)
}

#[derive(Default)]
struct Baselines {
    unsupported: bool,
    rows: HashMap<(String, i64), Baseline>,
}

impl Baselines {
    fn load(conn: &Connection) -> Result<Self> {
        // Catalog validation precedes every optional-table SELECT. Absence and
        // unsupported shape do not trigger DDL or mask unrelated query errors.
        let mut stmt = conn.prepare("SELECT column_name, data_type FROM information_schema.columns WHERE table_catalog = current_database() AND table_schema = ? AND table_name = ? ORDER BY ordinal_position")?;
        let columns = stmt
            .query_map(params!["main", "file_source_baselines"], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if columns.is_empty() {
            return Ok(Self::default());
        }
        let expected = [
            ("source_id", "VARCHAR"),
            ("generation", "BIGINT"),
            ("metadata_version", "INTEGER"),
            ("latest_association", "VARCHAR"),
            ("latest_attempt_at", "VARCHAR"),
            ("last_stable_observed_at", "VARCHAR"),
            ("last_stable_type", "VARCHAR"),
            ("last_stable_device", "VARCHAR"),
            ("last_stable_inode", "VARCHAR"),
            ("last_stable_size", "VARCHAR"),
            ("last_stable_mtime_ns", "VARCHAR"),
            ("association_reason", "VARCHAR"),
        ];
        if columns.len() != expected.len()
            || columns
                .iter()
                .zip(expected)
                .any(|((n, t), (en, et))| n != en || t != et)
        {
            return Ok(Self {
                unsupported: true,
                ..Self::default()
            });
        }
        let mut stmt = conn.prepare("SELECT source_id,generation,metadata_version,latest_association,latest_attempt_at,last_stable_observed_at,last_stable_type,last_stable_device,last_stable_inode,last_stable_size,last_stable_mtime_ns,association_reason FROM file_source_baselines")?;
        let mut rows = stmt.query([])?;
        let mut result = Self::default();
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            let generation: i64 = r.get(1)?;
            let version: i32 = r.get(2)?;
            let association: String = r.get(3)?;
            let attempt: String = r.get(4)?;
            let stable: Vec<Option<String>> = (5..11)
                .map(|i| r.get(i))
                .collect::<std::result::Result<_, _>>()?;
            let reason: String = r.get(11)?;
            let count = stable.iter().flatten().count();
            let metadata = if count == 6 {
                Some(FileMetadata {
                    r#type: stable[1].clone().unwrap(),
                    device: stable[2].clone().unwrap(),
                    inode: stable[3].clone().unwrap(),
                    size: stable[4].clone().unwrap(),
                    mtime_ns: stable[5].clone().unwrap(),
                })
            } else {
                None
            };
            let valid_meta = metadata.as_ref().is_none_or(|m| {
                m.r#type == "regular"
                    && unsigned(&m.device)
                    && unsigned(&m.inode)
                    && unsigned(&m.size)
                    && signed(&m.mtime_ns)
            });
            let baseline = if version != 1 {
                Baseline::unavailable("unsupported")
            } else if generation < 0
                || !utc(&attempt)
                || (count != 0 && count != 6)
                || !valid_meta
                || stable[0].as_deref().is_some_and(|s| !utc(s))
                || !matches!(
                    (association.as_str(), reason.as_str()),
                    ("verified", "verified") | ("unavailable", "association_unverified")
                )
                || (association == "verified"
                    && (count != 6 || stable[0].as_deref() != Some(&attempt)))
            {
                Baseline::unavailable("invalid")
            } else {
                Baseline {
                    status: if association == "verified" {
                        "available"
                    } else {
                        "association_unverified"
                    },
                    attempt: Some(attempt),
                    observed: stable[0].clone(),
                    stale: association == "unavailable" && count == 6,
                    metadata,
                }
            };
            result.rows.insert((id, generation), baseline);
        }
        Ok(result)
    }

    fn get(&self, id: Option<&str>, generation: Option<i64>) -> Baseline {
        if self.unsupported {
            return Baseline::unavailable("unsupported");
        }
        id.zip(generation)
            .and_then(|(id, g)| self.rows.get(&(id.to_owned(), g)))
            .cloned()
            .unwrap_or_else(|| Baseline::unavailable("absent"))
    }
}

#[derive(Debug)]
struct Entry {
    source_id: Option<String>,
    harness: Option<String>,
    generation: Option<i64>,
    path: Option<String>,
    latest: Option<i64>,
    context: Option<String>,
    eligible: bool,
    baseline: Baseline,
}

#[derive(Default)]
pub(crate) struct Captured {
    entries: Vec<Entry>,
    tuples: HashMap<(String, i64), usize>,
    contexts: HashMap<String, usize>,
    row_refs: Vec<Option<usize>>,
    loop_refs: Vec<Option<usize>>,
}

struct CaptureContext<'a> {
    sources: HashMap<&'a str, &'a SourceRow>,
    harnesses: HashMap<String, BTreeSet<String>>,
    baselines: Baselines,
}

impl Captured {
    fn add(
        &mut self,
        ctx: &CaptureContext<'_>,
        id: Option<&str>,
        harness: Option<&str>,
        generation: Option<i64>,
        origin: String,
        eligible: bool,
    ) -> usize {
        let id = id.filter(|s| safe_identity(s));
        let conflicting = id
            .and_then(|id| ctx.harnesses.get(id))
            .is_some_and(|h| h.len() != 1 || harness.is_some_and(|v| !h.contains(v)));
        let harness = harness.filter(|h| file_harness(h) && !conflicting);
        let known = id.is_some() && harness.is_some() && generation.is_some();
        let existing = if known {
            self.tuples
                .get(&(id.unwrap().to_owned(), generation.unwrap()))
        } else {
            self.contexts.get(&origin)
        };
        if let Some(&i) = existing {
            self.entries[i].eligible |= eligible;
            return i;
        }
        let source = id.and_then(|id| ctx.sources.get(id).copied());
        let i = self.entries.len();
        let context = (!known).then_some(origin);
        if known {
            self.tuples
                .insert((id.unwrap().to_owned(), generation.unwrap()), i);
        } else {
            self.contexts.insert(context.clone().unwrap(), i);
        }
        self.entries.push(Entry {
            source_id: id.map(str::to_owned),
            harness: harness.map(str::to_owned),
            generation,
            path: source.filter(|_| !conflicting).map(|s| s.2.clone()),
            latest: source.map(|s| s.3),
            context,
            eligible,
            baseline: if known {
                ctx.baselines.get(id, generation)
            } else {
                Baseline::unavailable("absent")
            },
        });
        i
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture(
        store: &AnalyticsStore,
        usage: &[UsageFact],
        untimed: &[&UsageFact],
        sources: &[SourceRow],
        coverage: &[SourceCoverage],
        bindings: &BTreeMap<(String, String), SessionBinding>,
        rig: Option<&str>,
        loops: &Value,
    ) -> Result<Self> {
        let conn = store.connection();
        let mut stmt = conn.prepare("SELECT source_key,source_id FROM usage_facts")?;
        let ids: HashMap<String, Option<String>> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut stmt = conn.prepare("SELECT source_id,harness FROM usage_facts UNION SELECT source_id,harness FROM tool_facts UNION SELECT source_id,harness FROM source_files")?;
        let pairs = stmt.query_map([], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut harnesses: HashMap<String, BTreeSet<String>> = HashMap::new();
        for pair in pairs {
            let (id, h) = pair?;
            if let Some(id) = id {
                harnesses.entry(id).or_default().insert(h);
            }
        }
        let ctx = CaptureContext {
            sources: sources.iter().map(|s| (s.0.as_str(), s)).collect(),
            harnesses,
            baselines: Baselines::load(conn)?,
        };
        let mut result = Self::default();
        for (i, u) in usage.iter().enumerate() {
            let entry = file_harness(&u.harness).then(|| {
                result.add(
                    &ctx,
                    ids.get(&u.source_key).and_then(|id| id.as_deref()),
                    Some(&u.harness),
                    Some(u.generation),
                    format!("usage:{i}"),
                    true,
                )
            });
            result.row_refs.push(entry);
        }
        let observations = loops["observations"]
            .as_array()
            .expect("captured loop observations");
        let mut selected = HashMap::new();
        let mut represented_coverage = HashSet::new();
        for o in observations {
            if let (Some(reference), Some(harness)) =
                (o["source_ref"].as_str(), o["harness"].as_str())
            {
                selected.entry(reference).or_insert(harness);
            }
        }
        for s in loops["sources"].as_array().expect("captured loop sources") {
            let id = s["source_id"].as_str();
            let reference = s["source_ref"].as_str().unwrap();
            let h = s["coverage"]["harness"]
                .as_str()
                .or_else(|| selected.get(reference).copied())
                .or_else(|| {
                    id.and_then(|id| ctx.harnesses.get(id))
                        .and_then(|h| h.iter().find(|h| file_harness(h)).map(String::as_str))
                });
            let eligible = selected.contains_key(reference)
                || s["coverage"]["in_scope"].as_bool() == Some(true);
            let entry = h.filter(|h| file_harness(h)).map(|h| {
                result.add(
                    &ctx,
                    id,
                    Some(h),
                    s["generation"].as_i64(),
                    format!("loop:{reference}"),
                    eligible,
                )
            });
            if entry.is_some() {
                if let (Some(id), Some(generation), Some(harness)) = (
                    id,
                    s["generation"].as_i64(),
                    s["coverage"]["harness"].as_str(),
                ) {
                    // This is the exact current row already enumerated through
                    // coverage, even if its public identity is unavailable. Do
                    // not merge distinct unavailable fact origins by this key.
                    represented_coverage.insert((id, generation, harness));
                }
            }
            result.loop_refs.push(entry);
        }
        for o in observations.iter().filter(|o| {
            o["source_ref"].is_null() && o["harness"].as_str().is_some_and(file_harness)
        }) {
            result.add(
                &ctx,
                None,
                o["harness"].as_str(),
                o["generation"].as_i64(),
                format!("loop-observation:{}", o["observation_id"].as_str().unwrap()),
                true,
            );
        }
        for (i, u) in untimed.iter().enumerate() {
            if file_harness(&u.harness) {
                result.add(
                    &ctx,
                    ids.get(&u.source_key).and_then(|id| id.as_deref()),
                    Some(&u.harness),
                    Some(u.generation),
                    format!("untimed:{i}"),
                    true,
                );
            }
        }
        for (i, s) in sources.iter().enumerate() {
            let in_scope = rig.is_none()
                || s.4
                    .as_deref()
                    .is_some_and(|native| binding_of(bindings, &s.1, native).rig() == rig);
            // Existing excluded coverage refs were captured above. Do not expose
            // unrelated global sources to answer scope counts.
            if file_harness(&s.1)
                && in_scope
                && !represented_coverage.contains(&(s.0.as_str(), s.3, s.1.as_str()))
            {
                let eligible = coverage.iter().any(|c| c.source_id == s.0);
                result.add(
                    &ctx,
                    Some(&s.0),
                    Some(&s.1),
                    Some(s.3),
                    format!("source:{i}"),
                    eligible,
                );
            }
        }
        Ok(result)
    }

    pub(crate) fn annotate(self, rows: &mut Value, loops: &mut Value) -> Value {
        self.annotate_with(rows, loops, observe_path)
    }

    fn annotate_with(
        self,
        rows: &mut Value,
        loops: &mut Value,
        mut observe: impl FnMut(&Path) -> PathObservation,
    ) -> Value {
        let mut cache: HashMap<String, PathObservation> = HashMap::new();
        let mut entries = Vec::with_capacity(self.entries.len());
        for (i, e) in self.entries.iter().enumerate() {
            let reason = lexical_reason(e.path.as_deref(), e.eligible, true, e.context.is_none());
            // Authorization is evaluated per tuple BEFORE the pathname cache.
            let observed = match reason {
                Some(reason) => PathObservation {
                    path_state: "not_checked",
                    reason,
                    checked_at: None,
                    metadata: None,
                },
                None => cache
                    .entry(e.path.clone().unwrap())
                    .or_insert_with(|| observe(Path::new(e.path.as_deref().unwrap())))
                    .clone(),
            };
            let display_path = e
                .path
                .as_deref()
                .filter(|p| lexical_reason(Some(p), true, true, true).is_none());
            let inconsistent = e.generation.is_some_and(|g| g < 0)
                || e.latest.is_some_and(|g| g < 0)
                || e.generation.zip(e.latest).is_some_and(|(g, l)| l < g);
            let generation_state = match (e.generation, e.latest) {
                (Some(g), Some(l)) if e.context.is_none() && g >= 0 && l >= g => {
                    if l == g {
                        "latest_recorded"
                    } else {
                        "superseded"
                    }
                }
                _ => "unknown",
            };
            let mut limits = BTreeSet::from([
                "cannot_verify_content",
                "filesystem_race",
                "inode_reuse",
                "point_in_time",
                "timestamp_granularity",
            ]);
            if inconsistent {
                limits.insert("generation_inconsistent");
            }
            if generation_state == "unknown" {
                limits.insert("generation_unavailable");
            }
            if e.latest.is_none() {
                limits.insert("latest_metadata_unavailable");
            }
            if e.context.is_some() {
                limits.insert("identity_unavailable");
            }
            if observed.reason == "unsupported_platform" {
                limits.insert("unsupported_platform");
            }
            let b = &e.baseline;
            match b.status {
                "absent" => {
                    limits.insert("baseline_absent");
                }
                "unsupported" => {
                    limits.insert("baseline_unsupported");
                }
                "invalid" => {
                    limits.insert("baseline_invalid");
                }
                "association_unverified" => {
                    limits.insert("baseline_association_unverified");
                }
                _ => {}
            }
            if b.stale {
                limits.insert("stale_baseline_retained");
            }
            let baseline = (b.status == "available").then(|| json!({"source_id":e.source_id,"generation":e.generation,"observed_at":b.observed,"metadata":b.metadata,"basis":BASIS,"association":"latest_stable_ingest_observation_of_generation"}));
            let comparison = if observed.path_state != "present" {
                "not_compared"
            } else if b.status != "available" {
                "baseline_unavailable"
            } else if observed.metadata == b.metadata {
                "matches_recorded"
            } else {
                "differs_recorded"
            };
            let state = if generation_state == "superseded" {
                "superseded"
            } else if observed.path_state != "present" {
                observed.path_state
            } else if comparison == "differs_recorded" {
                "changed"
            } else {
                "present"
            };
            entries.push(json!({"file_state_ref":format!("f{i}"),"source_id":e.source_id,"harness":e.harness,"generation":e.generation,"display_path":display_path,"latest_recorded_generation":e.latest,"generation_state":generation_state,"path_state":observed.path_state,"metadata_comparison":comparison,"state":state,"observation":{"method":"metadata_only","checked_at":observed.checked_at,"reason":observed.reason,"metadata":observed.metadata,"comparison_basis":BASIS,"limitations":limits,"scope":if e.eligible {"in_scope"}else{"out_of_scope"},"baseline_status":b.status,"latest_ingest_attempt_at":b.attempt},"baseline":baseline,"unavailable_context":e.context}));
        }
        for (row, entry) in rows
            .as_array_mut()
            .expect("trace rows")
            .iter_mut()
            .zip(self.row_refs)
        {
            row["file_state_ref"] = json!(entry.map(|i| format!("f{i}")));
        }
        for (source, entry) in loops["sources"]
            .as_array_mut()
            .expect("loop sources")
            .iter_mut()
            .zip(self.loop_refs)
        {
            source["file_state_ref"] = json!(entry.map(|i| format!("f{i}")));
            if let Some(i) = entry {
                source["current_state"] = entries[i]["state"].clone();
                if !entries[i]["observation"]["checked_at"].is_null() {
                    source["limitations"]
                        .as_array_mut()
                        .unwrap()
                        .retain(|l| l["code"] != "current_source_not_checked");
                }
            }
        }
        json!(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(size: &str) -> FileMetadata {
        FileMetadata {
            r#type: "regular".into(),
            device: "18446744073709551615".into(),
            inode: "11".into(),
            size: size.into(),
            mtime_ns: "-1".into(),
        }
    }

    fn seed(conn: &Connection) {
        conn.execute(
            "INSERT INTO file_source_baselines VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                "S",
                1,
                1,
                "verified",
                "2026-10-15T00:00:00Z",
                "2026-10-15T00:00:00Z",
                "regular",
                "18446744073709551615",
                "11",
                "100",
                "-1",
                "verified"
            ],
        )
        .unwrap();
    }

    #[test]
    fn timestamps_match_the_public_utc_shape() {
        for valid in [
            "2026-10-15T00:00:00Z",
            "2026-10-15T00:00:00.123456789+00:00",
            "0001-01-01T00:00:00Z",
        ] {
            assert!(utc(valid), "{valid}");
        }
        for invalid in [
            "0000-01-01T00:00:00Z",
            "2026-10-15 00:00:00Z",
            "2026-10-15t00:00:00Z",
            "2026-10-15T00:00:60Z",
            "2026-10-15T00:00:00.1234567890Z",
            "2026-10-15T00:00:00.Z",
            "2026-10-15T00:00:00+01:00",
            "2026-10-15T00:00:00.éZ",
        ] {
            assert!(!utc(invalid), "{invalid}");
        }
    }

    #[test]
    fn optional_catalog_validation_and_lossless_values() {
        for (edit,expected) in [
            ("", "available"),
            ("DROP TABLE file_source_baselines", "absent"),
            ("ALTER TABLE file_source_baselines DROP COLUMN association_reason", "unsupported"),
            ("ALTER TABLE file_source_baselines ALTER COLUMN metadata_version TYPE BIGINT", "unsupported"),
            ("UPDATE file_source_baselines SET metadata_version=99", "unsupported"),
            ("UPDATE file_source_baselines SET last_stable_observed_at=NULL", "invalid"),
            ("UPDATE file_source_baselines SET last_stable_inode='01'", "invalid"),
            ("UPDATE file_source_baselines SET last_stable_mtime_ns='-0'", "invalid"),
            ("UPDATE file_source_baselines SET last_stable_device='18446744073709551616'", "invalid"),
            ("UPDATE file_source_baselines SET latest_association='unavailable',association_reason='association_unverified'", "association_unverified"),
        ] {
            let store = AnalyticsStore::open_in_memory("kid").unwrap();
            let conn = store.connection();
            seed(conn);
            conn.execute_batch(edit).unwrap();
            let loaded = Baselines::load(conn).unwrap().get(Some("S"),Some(1));
            assert_eq!(loaded.status,expected,"{edit}");
            if expected == "available" { assert_eq!(loaded.metadata,Some(metadata("100"))); }
            if expected == "association_unverified" { assert!(loaded.stale); }
            if expected == "absent" {
                let n:i64=conn.query_row("SELECT count(*) FROM information_schema.tables WHERE table_name = 'file_source_baselines'",[],|r|r.get(0)).unwrap();
                assert_eq!(n,0,"read did not create the missing optional table");
            }
        }
    }

    #[test]
    fn scope_and_lexical_policy_precede_cache_with_per_generation_comparison() {
        let mut entries = Vec::new();
        for (generation, eligible, path) in [
            (1, true, "/synthetic/file"),
            (2, false, "/synthetic/file"),
            (3, true, "/synthetic/file"),
            (4, true, "/synthetic/auth.json"),
        ] {
            entries.push(Entry {
                source_id: Some("S".into()),
                harness: Some("claude".into()),
                generation: Some(generation),
                path: Some(path.into()),
                latest: Some(3),
                context: None,
                eligible,
                baseline: Baseline {
                    status: "available",
                    attempt: Some("2026-10-15T00:00:00Z".into()),
                    observed: Some("2026-10-15T00:00:00Z".into()),
                    metadata: Some(metadata(if generation == 1 { "99" } else { "100" })),
                    stale: false,
                },
            });
        }
        let capture = Captured {
            entries,
            row_refs: vec![Some(0)],
            loop_refs: vec![Some(0), Some(1)],
            ..Captured::default()
        };
        let mut rows = json!([{}]);
        let mut loops = json!({"sources":[{"limitations":[{"code":"producing_revision_not_recorded"},{"code":"current_source_not_checked"}]},{"limitations":[{"code":"producing_revision_not_recorded"},{"code":"current_source_not_checked"}]}]});
        let mut calls = 0;
        let states = capture.annotate_with(&mut rows, &mut loops, |path| {
            calls += 1;
            assert_eq!(path, Path::new("/synthetic/file"));
            PathObservation {
                path_state: "present",
                reason: "regular_file",
                checked_at: Some("2026-10-15T01:00:00Z".into()),
                metadata: Some(metadata("100")),
            }
        });
        assert_eq!(
            calls, 1,
            "one logical observation for eligible pathname, no credential/outscope probe"
        );
        assert_eq!(states[0]["state"], "superseded");
        assert_eq!(states[0]["metadata_comparison"], "differs_recorded");
        assert_eq!(states[1]["observation"]["reason"], "out_of_scope");
        assert!(states[1]["observation"]["checked_at"].is_null());
        assert_eq!(states[2]["metadata_comparison"], "matches_recorded");
        assert_eq!(
            states[0]["observation"]["checked_at"],
            states[2]["observation"]["checked_at"]
        );
        assert_eq!(states[3]["observation"]["reason"], "credential_path");
        assert!(states[3]["display_path"].is_null());
        assert_eq!(
            loops["sources"][0]["limitations"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            loops["sources"][1]["limitations"][1]["code"],
            "current_source_not_checked"
        );
    }

    #[test]
    fn unavailable_identities_do_not_merge_and_harness_conflicts_do_not_probe() {
        let ctx = CaptureContext {
            sources: HashMap::new(),
            harnesses: HashMap::from([(
                "S".into(),
                BTreeSet::from(["claude".into(), "pi".into()]),
            )]),
            baselines: Baselines::default(),
        };
        let mut captured = Captured::default();
        assert_eq!(
            captured.add(&ctx, None, Some("claude"), Some(1), "usage:0".into(), true),
            0
        );
        assert_eq!(
            captured.add(&ctx, None, Some("claude"), Some(1), "usage:1".into(), true),
            1
        );
        assert_eq!(
            captured.add(&ctx, None, Some("claude"), Some(1), "usage:0".into(), true),
            0
        );
        captured.add(
            &ctx,
            Some("S"),
            Some("claude"),
            Some(1),
            "loop:s0".into(),
            true,
        );
        captured.add(
            &ctx,
            None,
            Some("pi"),
            Some(2),
            "loop-observation:o3".into(),
            true,
        );
        let states = captured.annotate_with(&mut json!([]), &mut json!({"sources":[]}), |_| {
            panic!("unavailable identity must not probe")
        });
        assert_eq!(states.as_array().unwrap().len(), 4);
        assert!(states[2]["harness"].is_null());
        assert_eq!(states[2]["unavailable_context"], "loop:s0");
        assert_eq!(states[3]["unavailable_context"], "loop-observation:o3");
        for e in states.as_array().unwrap() {
            assert_eq!(e["observation"]["reason"], "identity_unavailable");
        }
    }
}

#[cfg(test)]
mod census_tests {
    use super::*;

    #[test]
    fn census_uses_stored_identity_and_preserves_encounter_order_and_scope() {
        let store = AnalyticsStore::open_in_memory("kid").unwrap();
        let conn = store.connection();
        for (id, h, g, session) in [
            ("A", "claude", 2, "selected"),
            ("B", "pi", 1, "selected"),
            ("X", "codex", 1, "excluded"),
            ("U", "claude", 1, "selected"),
            ("C", "pi", 1, "selected"),
        ] {
            conn.execute(
                "INSERT INTO source_files VALUES (?,?,?,?,?,?,?)",
                params![
                    id,
                    h,
                    format!("/synthetic/{id}.jsonl"),
                    g,
                    0,
                    "prefix",
                    session
                ],
            )
            .unwrap();
        }
        // Deliberately opaque keys prove canonical identity is not parsed from keys.
        for (key, id, h, time, seq) in [
            (
                "opaque-timed",
                "A",
                "claude",
                Some("2026-10-15 00:00:00"),
                0,
            ),
            (
                "opaque-missing",
                "sk-FAKE_UNSAFE_ID",
                "pi",
                Some("2026-10-15 00:00:01"),
                1,
            ),
            ("opaque-untimed", "U", "claude", None, 2),
        ] {
            conn.execute("INSERT INTO usage_facts (source_key,harness,native_session,ts,seq,input_uncached,output_tokens,cache_read,cache_write,reasoning,generation,source_id) VALUES (?,?,?,CAST(? AS TIMESTAMP),?,0,0,0,0,0,1,?)",params![key,h,"selected",time,seq,id]).unwrap();
        }
        let all = store.load_usage().unwrap();
        let timed: Vec<_> = all.iter().filter(|u| u.ts.is_some()).cloned().collect();
        let untimed: Vec<_> = all.iter().filter(|u| u.ts.is_none()).collect();
        let sources = store.load_sources().unwrap();
        let coverage = store.load_source_coverage().unwrap();
        let bindings = BTreeMap::from([
            (
                ("claude".into(), "selected".into()),
                SessionBinding::Linked {
                    rig: "rig".into(),
                    seat: "seat".into(),
                },
            ),
            (
                ("pi".into(), "selected".into()),
                SessionBinding::Linked {
                    rig: "rig".into(),
                    seat: "seat".into(),
                },
            ),
        ]);
        let mut loops = json!({"observations":[{"observation_id":"o0","harness":"pi","generation":1,"source_ref":"s0"},{"observation_id":"o1","harness":"pi","generation":1,"source_ref":null}],"sources":[{"source_id":"B","source_ref":"s0","generation":1,"coverage":{"harness":"pi","in_scope":true},"limitations":[]},{"source_id":"X","source_ref":"s1","generation":1,"coverage":{"harness":"codex","in_scope":false},"limitations":[]}]});
        let captured = Captured::capture(
            &store,
            &timed,
            &untimed,
            &sources,
            &coverage,
            &bindings,
            Some("rig"),
            &loops,
        )
        .unwrap();
        let mut rows = json!([{}, {}]);
        let mut paths = Vec::new();
        let entries = captured.annotate_with(&mut rows, &mut loops, |path| {
            paths.push(path.to_owned());
            PathObservation {
                path_state: "gone",
                reason: "path_not_found",
                checked_at: Some("2026-10-15T00:00:00Z".into()),
                metadata: None,
            }
        });
        let ids: Vec<_> = entries
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["source_id"].as_str(), e["generation"].as_i64()))
            .collect();
        assert_eq!(
            ids,
            vec![
                (Some("A"), Some(1)),
                (None, Some(1)),
                (Some("B"), Some(1)),
                (Some("X"), Some(1)),
                (None, Some(1)),
                (Some("U"), Some(1)),
                (Some("A"), Some(2)),
                (Some("C"), Some(1))
            ]
        );
        assert_eq!(entries[1]["unavailable_context"], "usage:1");
        assert_eq!(entries[4]["unavailable_context"], "loop-observation:o1");
        assert_eq!(entries[3]["observation"]["reason"], "out_of_scope");
        assert_eq!(entries[0]["generation_state"], "superseded");
        assert_eq!(entries[0]["display_path"], "/synthetic/A.jsonl");
        assert_eq!(rows[0]["file_state_ref"], "f0");
        assert_eq!(rows[1]["file_state_ref"], "f1");
        assert_eq!(loops["sources"][0]["file_state_ref"], "f2");
        assert!(!paths.contains(&Path::new("/synthetic/X.jsonl").to_owned()));
        assert_eq!(
            paths
                .iter()
                .filter(|p| p.as_path() == Path::new("/synthetic/A.jsonl"))
                .count(),
            1
        );
    }
}

#[cfg(test)]
mod unavailable_coverage_tests {
    use super::*;

    fn source(store: &AnalyticsStore, id: &str, harness: &str, path: &str) {
        store
            .connection()
            .execute(
                "INSERT INTO source_files VALUES (?,?,?,1,0,'prefix',NULL)",
                params![id, harness, path],
            )
            .unwrap();
    }

    fn loop_source(id: &str, reference: &str, harness: &str) -> Value {
        json!({"source_id":id,"source_ref":reference,"generation":1,"coverage":{"harness":harness,"in_scope":true},"limitations":[]})
    }

    #[test]
    fn unavailable_coverage_is_not_reenumerated_as_a_current_source() {
        let store = AnalyticsStore::open_in_memory("kid").unwrap();
        source(&store, "A\t[redacted]", "claude", "/synthetic/A.jsonl");
        source(&store, "B\t[redacted]", "pi", "/synthetic/B.jsonl");
        source(&store, "C", "codex", "/synthetic/C.jsonl");
        let sources = store.load_sources().unwrap();
        let coverage = store.load_source_coverage().unwrap();
        let mut loops = json!({"sources":[loop_source("A\t[redacted]","s0","claude"),loop_source("B\t[redacted]","s1","pi")],"observations":[]});
        let captured = Captured::capture(
            &store,
            &[],
            &[],
            &sources,
            &coverage,
            &BTreeMap::new(),
            None,
            &loops,
        )
        .unwrap();
        assert_eq!(captured.entries.len(), 3);
        assert_eq!(captured.entries[0].context.as_deref(), Some("loop:s0"));
        assert_eq!(captured.entries[1].context.as_deref(), Some("loop:s1"));
        assert!(captured.entries[0].source_id.is_none());
        assert!(captured.entries[1].source_id.is_none());
        assert_eq!(captured.entries[2].source_id.as_deref(), Some("C"));
        let mut calls = 0;
        captured.annotate_with(&mut json!([]), &mut loops, |path| {
            assert_eq!(path, Path::new("/synthetic/C.jsonl"));
            calls += 1;
            PathObservation {
                path_state: "gone",
                reason: "path_not_found",
                checked_at: Some("2026-10-15T00:00:00Z".into()),
                metadata: None,
            }
        });
        assert_eq!(
            calls, 1,
            "only the genuinely unrepresented current row is observed"
        );
    }

    #[test]
    fn conflicting_coverage_preserves_distinct_usage_and_observation_contexts() {
        let store = AnalyticsStore::open_in_memory("kid").unwrap();
        source(&store, "S", "claude", "/synthetic/S.jsonl");
        store.connection().execute("INSERT INTO usage_facts (source_key,harness,native_session,ts,seq,input_uncached,output_tokens,cache_read,cache_write,reasoning,generation,source_id) VALUES ('opaque','pi','session',TIMESTAMP '2026-10-15 00:00:00',0,0,0,0,0,0,1,'S')",[]).unwrap();
        let usage = store.load_usage().unwrap();
        let sources = store.load_sources().unwrap();
        let coverage = store.load_source_coverage().unwrap();
        let mut loops = json!({"sources":[loop_source("S","s0","claude")],"observations":[{"source_ref":null,"observation_id":"o0","harness":"pi","generation":1}]});
        let captured = Captured::capture(
            &store,
            &usage,
            &[],
            &sources,
            &coverage,
            &BTreeMap::new(),
            None,
            &loops,
        )
        .unwrap();
        let contexts: Vec<_> = captured
            .entries
            .iter()
            .map(|e| e.context.as_deref())
            .collect();
        assert_eq!(
            contexts,
            vec![
                Some("usage:0"),
                Some("loop:s0"),
                Some("loop-observation:o0")
            ]
        );
        assert!(captured.entries[0].harness.is_none());
        assert!(captured.entries[1].harness.is_none());
        captured.annotate_with(&mut json!([{}]), &mut loops, |_| {
            panic!("conflicting identity cannot probe")
        });
    }
}
