//! Content-free cutoffs for reproducible reads of live sources.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutoffManifest {
    /// Content digest of accepted analytics history beside the manifest. Present
    /// only when capture included facts from previous ingests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analytics_snapshot_sha256: Option<String>,
    #[serde(default)]
    pub files: Vec<FileCutoff>,
    #[serde(default)]
    pub openrig: Vec<OpenRigCutoff>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCutoff {
    #[serde(
        serialize_with = "serialize_identifier",
        deserialize_with = "deserialize_identifier"
    )]
    pub source_id: String,
    #[serde(
        serialize_with = "serialize_identifier",
        deserialize_with = "deserialize_identifier"
    )]
    pub harness: String,
    pub byte_length: u64,
    pub prefix_sha256: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRigCutoff {
    #[serde(
        serialize_with = "serialize_identifier",
        deserialize_with = "deserialize_identifier"
    )]
    pub source_id: String,
    pub events_max_seq: Option<i64>,
    pub events_count: u64,
    pub events_digest: String,
    pub queue_transitions_max_id: Option<i64>,
    pub queue_transitions_count: u64,
    pub queue_transitions_digest: String,
    pub snapshots_max_rowid: Option<i64>,
    pub snapshots_count: u64,
    pub snapshots_digest: String,
    pub queue_items: Vec<QueueItemCutoff>,
    pub rigs_count: u64,
    pub rigs_digest: String,
}

/// Only field digests are retained, never the mutable source values.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueItemCutoff {
    #[serde(
        serialize_with = "serialize_identifier",
        deserialize_with = "deserialize_identifier"
    )]
    pub qitem_id: String,
    pub destination_session_digest: String,
    pub ts_updated_digest: String,
    pub chain_of_record_digest: String,
    pub handed_off_from_digest: String,
    pub tags_digest: String,
    #[serde(default)]
    pub mission_tags_digest: String,
    /// Absent in manifests captured before wake-result attribution existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_nudge_result_digest: Option<String>,
    /// Issue, phase, and recovery tags used by additive analytics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analytics_tags_digest: Option<String>,
}

impl CutoffManifest {
    /// Apply the same privacy boundary to loaded and newly captured identifiers.
    pub fn redact_identifiers(&mut self) {
        for file in &mut self.files {
            file.source_id = crate::redact(&file.source_id);
            file.harness = crate::redact(&file.harness);
        }
        for source in &mut self.openrig {
            source.source_id = crate::redact(&source.source_id);
            for item in &mut source.queue_items {
                item.qitem_id = crate::redact(&item.qitem_id);
            }
        }
    }
}

fn serialize_identifier<S: serde::Serializer>(
    value: &str,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&crate::redact(value))
}

fn deserialize_identifier<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    String::deserialize(deserializer).map(|value| crate::redact(&value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_redacts_identifiers_at_both_boundaries() {
        let canary = "ghp_manifestcanary123456789012345678901234567890";
        let manifest = CutoffManifest {
            files: vec![FileCutoff {
                source_id: canary.into(),
                harness: canary.into(),
                byte_length: 0,
                prefix_sha256: "00".repeat(32),
            }],
            openrig: vec![OpenRigCutoff {
                source_id: canary.into(),
                queue_items: vec![QueueItemCutoff {
                    qitem_id: canary.into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let serialized = serde_json::to_string(&manifest).unwrap();
        assert!(!serialized.contains("manifestcanary"));
        let loaded: CutoffManifest = serde_json::from_value(serde_json::json!({"files":[{"source_id":canary,"harness":canary,"byte_length":0,"prefix_sha256":"00"}],"openrig":[]})).unwrap();
        assert!(!loaded.files[0].source_id.contains("manifestcanary"));
        assert!(!loaded.files[0].harness.contains("manifestcanary"));
    }
}
