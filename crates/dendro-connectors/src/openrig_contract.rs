//! Narrow consumer contract for `openrig.sqlite`.
//!
//! The pin is the schema_migrations tail observed on OpenRig 0.7.6. Columns
//! outside the lists below are additive. A missing required column refuses
//! the source before analytics facts are written.
use anyhow::Result;
use rusqlite::Connection;

pub const MIGRATION_PIN: &str = "141_session_agent_runner.sql";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractClass {
    Supported,
    Additive,
    Drifted,
    Unsupported,
}

impl ContractClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Additive => "additive",
            Self::Drifted => "drifted",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractAssessment {
    pub class: ContractClass,
    pub migration: String,
    pub reason: String,
}

impl ContractAssessment {
    pub fn operator_line(&self, rig_version: &str) -> String {
        format!(
            "dendro: openrig contract={} rig={rig_version} migration={} unknown_events=0 dropped=0 reason={}",
            self.class.as_str(),
            self.migration,
            self.reason
        )
    }

    pub fn summary(&self) -> String {
        format!(
            "contract={} migration={} unknown_events=0 dropped=0",
            self.class.as_str(),
            self.migration
        )
    }
}

struct Column {
    name: &'static str,
    type_has: &'static str,
    required: bool,
}

const EVENTS: &[Column] = &[
    Column {
        name: "seq",
        type_has: "INT",
        required: true,
    },
    Column {
        name: "rig_id",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "node_id",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "type",
        type_has: "TEXT",
        required: true,
    },
    Column {
        name: "payload",
        type_has: "TEXT",
        required: true,
    },
    Column {
        name: "created_at",
        type_has: "TEXT",
        required: true,
    },
];

const TRANSITIONS: &[Column] = &[
    Column {
        name: "transition_id",
        type_has: "INT",
        required: true,
    },
    Column {
        name: "qitem_id",
        type_has: "TEXT",
        required: true,
    },
    Column {
        name: "ts",
        type_has: "TEXT",
        required: true,
    },
    Column {
        name: "state",
        type_has: "TEXT",
        required: true,
    },
    Column {
        name: "transition_note",
        type_has: "TEXT",
        required: true,
    },
    Column {
        name: "actor_session",
        type_has: "TEXT",
        required: true,
    },
    Column {
        name: "closure_reason",
        type_has: "TEXT",
        required: true,
    },
    Column {
        name: "closure_target",
        type_has: "TEXT",
        required: true,
    },
    Column {
        name: "identity_provenance",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "owner_notification_kind",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "owner_notification_level",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "identity_principal",
        type_has: "TEXT",
        required: false,
    },
];

const QUEUE_ITEMS: &[Column] = &[
    Column {
        name: "qitem_id",
        type_has: "TEXT",
        required: true,
    },
    Column {
        name: "ts_created",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "ts_updated",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "source_session",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "destination_session",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "state",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "priority",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "tier",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "tags",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "blocked_on",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "handed_off_to",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "handed_off_from",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "expires_at",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "chain_of_record",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "body",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "closure_reason",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "closure_target",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "closure_required_at",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "claimed_at",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "last_nudge_attempt",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "last_nudge_result",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "last_heartbeat",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "resolution",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "target_repo",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "summary",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "evidence_ref",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "minting_generation_uuid",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "claimed_by_generation_uuid",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "human_intent",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "human_detail",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "reply_to",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "human_questions",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "human_answers",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "last_nudge_input_box",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "delegated_decision",
        type_has: "TEXT",
        required: false,
    },
];

const SNAPSHOTS: &[Column] = &[
    Column {
        name: "id",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "rig_id",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "kind",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "status",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "data",
        type_has: "TEXT",
        required: true,
    },
    Column {
        name: "created_at",
        type_has: "TEXT",
        required: false,
    },
];

const RIGS: &[Column] = &[
    Column {
        name: "id",
        type_has: "TEXT",
        required: true,
    },
    Column {
        name: "name",
        type_has: "TEXT",
        required: true,
    },
    Column {
        name: "created_at",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "updated_at",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "workspace_json",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "archived_at",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "permission_policy",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "rig_policy_origin",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "rig_policy_resolved_target",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "rig_policy_declaring_dir",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "rig_policy_launch_posture",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "claude_managed_block_file",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "non_interruptive",
        type_has: "INT",
        required: false,
    },
    Column {
        name: "install_root",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "env_json",
        type_has: "TEXT",
        required: false,
    },
    Column {
        name: "startup_layers_json",
        type_has: "TEXT",
        required: false,
    },
];

const TABLES: &[(&str, &[Column])] = &[
    ("events", EVENTS),
    ("queue_transitions", TRANSITIONS),
    ("queue_items", QUEUE_ITEMS),
    ("snapshots", SNAPSHOTS),
    ("rigs", RIGS),
];

pub fn known_columns(table: &str) -> &'static [&'static str] {
    match table {
        "events" => &["seq", "rig_id", "node_id", "type", "payload", "created_at"],
        "queue_transitions" => &[
            "transition_id",
            "qitem_id",
            "ts",
            "state",
            "transition_note",
            "actor_session",
            "closure_reason",
            "closure_target",
            "identity_provenance",
            "owner_notification_kind",
            "owner_notification_level",
            "identity_principal",
        ],
        "snapshots" => &["id", "rig_id", "kind", "status", "data", "created_at"],
        _ => &[],
    }
}

pub fn assess(conn: &Connection) -> Result<ContractAssessment> {
    let mut missing_table = None;
    let mut missing_column = None;
    let mut bad_type = None;
    let mut unknown = None;
    for (table, columns) in TABLES {
        if !table_exists(conn, table)? {
            missing_table.get_or_insert_with(|| (*table).to_string());
            continue;
        }
        let present = table_columns(conn, table)?;
        for column in *columns {
            match present.iter().find(|(name, _)| name == column.name) {
                None if column.required => {
                    missing_column.get_or_insert_with(|| format!("{table}.{}", column.name));
                }
                Some((_, declared)) if !declared.to_ascii_uppercase().contains(column.type_has) => {
                    bad_type.get_or_insert_with(|| format!("{table}.{}", column.name));
                }
                _ => {}
            }
        }
        for (name, _) in &present {
            if !columns.iter().any(|column| column.name == name) {
                unknown.get_or_insert_with(|| name.clone());
            }
        }
    }
    let migration = migration_tail(conn)?;
    if missing_table.as_deref() == Some("events")
        || missing_table.as_deref() == Some("queue_transitions")
    {
        return Ok(ContractAssessment {
            class: ContractClass::Unsupported,
            migration: migration.unwrap_or_else(|| "none".into()),
            reason: format!("missing-table:{}", missing_table.unwrap_or_default()),
        });
    }
    if let Some(name) = missing_column {
        return Ok(ContractAssessment {
            class: ContractClass::Unsupported,
            migration: migration.unwrap_or_else(|| "none".into()),
            reason: format!("missing-column:{name}"),
        });
    }
    if let Some(name) = bad_type {
        return Ok(ContractAssessment {
            class: ContractClass::Unsupported,
            migration: migration.unwrap_or_else(|| "none".into()),
            reason: format!("column-type:{name}"),
        });
    }
    let Some(migration) = migration else {
        return Ok(ContractAssessment {
            class: ContractClass::Drifted,
            migration: "none".into(),
            reason: "migrations-unreadable".into(),
        });
    };
    if let Some(name) = unknown {
        return Ok(ContractAssessment {
            class: ContractClass::Additive,
            migration,
            reason: format!("unknown-column:{name}"),
        });
    }
    if migration.as_str() > MIGRATION_PIN {
        return Ok(ContractAssessment {
            class: ContractClass::Additive,
            reason: format!("newer-migration:{migration}"),
            migration,
        });
    }
    if let Some(name) = missing_table {
        return Ok(ContractAssessment {
            class: ContractClass::Drifted,
            migration,
            reason: format!("missing-table:{name}"),
        });
    }
    Ok(ContractAssessment {
        class: ContractClass::Supported,
        migration,
        reason: "none".into(),
    })
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let found: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .ok();
    Ok(found.is_some())
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let rows = stmt.query_map([], |row| Ok((row.get(1)?, row.get(2)?)))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn migration_tail(conn: &Connection) -> Result<Option<String>> {
    if !table_exists(conn, "schema_migrations")? {
        return Ok(None);
    }
    let tail = conn
        .query_row(
            "SELECT name FROM schema_migrations ORDER BY name DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok();
    Ok(tail)
}

pub fn rig_version() -> String {
    let Ok(output) = std::process::Command::new("rig").arg("--version").output() else {
        return "unknown".into();
    };
    if !output.status.success() {
        return "unknown".into();
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("unknown")
        .to_string()
}
