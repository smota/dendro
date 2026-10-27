//! A05: O(1) tripwire outbox and post-commit I22 dispatcher.
use duckdb::Connection;

use dendro_core::migration_v5::apply_schema5;
use dendro_core::tripwire_v1::{
    dispatch_after_commit, dispatch_with, install_usage_tripwire, recover_undispatched,
    render_agent_prompt, trajectory_vortex, undispatched, validate_trigger_sql, TripwireError,
    NOTIFY_TRIGGER_BODY, WATERMARK_TRIGGER_BODY,
};

#[test]
fn statement_trigger_is_o1_and_dispatcher_stays_out_of_the_prompt() {
    assert!(validate_trigger_sql(WATERMARK_TRIGGER_BODY).is_ok());
    assert!(validate_trigger_sql(NOTIFY_TRIGGER_BODY).is_ok());
    assert!(validate_trigger_sql(
        "INSERT INTO _dendro_tripwire_outbox SELECT count(*) FROM usage_facts"
    )
    .is_err());
    assert!(validate_trigger_sql(
        "UPDATE _dendro_tripwire_watermark SET counter = counter + 1 WHERE id = 1 AND quantile_cont(counter, 0.5) > 0"
    )
    .is_err());
    assert!(validate_trigger_sql(
        "INSERT INTO _dendro_tripwire_outbox SELECT w.counter, centroid(w.counter) FROM _dendro_tripwire_watermark w JOIN _dendro_tripwire_threshold t ON w.id = t.id"
    )
    .is_err());

    let conn = Connection::open_in_memory().unwrap();
    apply_schema5(&conn).unwrap();
    conn.execute_batch(
        "CREATE TABLE usage_facts (source_key VARCHAR, harness VARCHAR);
         UPDATE _dendro_tripwire_watermark SET subject_key = 'sess-1' WHERE id = 1;",
    )
    .unwrap();
    install_usage_tripwire(&conn).unwrap();
    insert_trajectory(&conn, "o1", 1, 1.0);
    insert_trajectory(&conn, "o2", 2, 0.0);

    conn.execute_batch(
        "BEGIN TRANSACTION;
         INSERT INTO usage_facts VALUES ('rolled', 'codex');
         ROLLBACK;",
    )
    .unwrap();
    assert_eq!(
        count(&conn, "SELECT count(*) FROM _dendro_tripwire_outbox"),
        2,
        "rolled-back usage insert must not add an outbox row"
    );
    assert_eq!(count(&conn, "SELECT count(*) FROM usage_facts"), 0);
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM _dendro_watermarks WHERE subject_key = 'sess-1'"
        ),
        2
    );

    let before_usage = count(&conn, "SELECT count(*) FROM _dendro_tripwire_outbox");
    conn.execute(
        "INSERT INTO usage_facts VALUES ('a', 'codex'), ('b', 'codex'), ('c', 'codex')",
        [],
    )
    .unwrap();
    assert_eq!(
        count(&conn, "SELECT count(*) FROM _dendro_tripwire_outbox") - before_usage,
        1,
        "a multi-row statement must enqueue one notification"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM _dendro_tripwire_outbox WHERE dispatched_at IS NULL"
        ),
        3
    );

    let notices = dispatch_after_commit(&conn).expect("dispatch");
    assert_eq!(notices.len(), 3);
    assert_eq!(notices[0].insight_id, "I22");
    assert_eq!(notices[0].channel, "operator-pull");
    assert!(!notices[0].enters_agent_prompt);
    assert!(!notices[0].recovery);
    assert!(notices[0].status_evidence_present());
    assert_eq!(
        render_agent_prompt("agent prompt stays", &notices[0]),
        "agent prompt stays"
    );
    assert!(notices.iter().any(|notice| {
        notice.measures.iter().any(|(id, _)| {
            id == "I22.cosine_distance" || id == "I22.distance_threshold" || id == "I22.tripped"
        }) || (notice.evidence.iter().any(|id| id == "o1")
            && notice.evidence.iter().any(|id| id == "o2"))
    }));
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM _dendro_tripwire_outbox WHERE dispatched_at IS NULL"
        ),
        0
    );

    assert!(dispatch_after_commit(&conn).unwrap().is_empty());
    assert!(recover_undispatched(&conn).unwrap().is_empty());

    conn.execute("INSERT INTO usage_facts VALUES ('d', 'codex')", [])
        .unwrap();
    let failed = dispatch_with(&conn, false, |_conn, _row| {
        Err(TripwireError::Analyzer("analyzer unavailable".into()))
    });
    assert!(failed.is_err());
    assert_eq!(
        undispatched(&conn).unwrap().len(),
        1,
        "failure must leave dispatched_at null"
    );
    let facts: i64 = count(&conn, "SELECT count(*) FROM usage_facts");
    assert_eq!(facts, 4, "dispatcher failure rolled back a committed fact");
}

#[test]
fn scoped_i22_keeps_the_full_reference_cohort() {
    let conn = Connection::open_in_memory().unwrap();
    apply_schema5(&conn).unwrap();
    for index in 0..5 {
        insert_trajectory_subject(&conn, &format!("r{index}"), &format!("ref-{index}"), 1, 1.0);
    }
    let answer = trajectory_vortex(&conn, "ref-0").expect("scoped vortex");
    assert_eq!(answer.status, "computed");
    assert_ne!(answer.status, "refused:insufficient_sample");
    let cohort = answer
        .measures
        .iter()
        .find(|(id, _)| id == "I22.cohort")
        .map(|(_, value)| value.parse::<i64>().expect("cohort"))
        .expect("I22.cohort");
    assert_ne!(cohort, 1);
    assert!(
        cohort >= 5,
        "cohort {cohort} must be the full reference, not the scoped subject"
    );
}

fn insert_trajectory(conn: &Connection, id: &str, turn: i64, first: f64) {
    insert_trajectory_subject(conn, id, "sess-1", turn, first);
}

fn insert_trajectory_subject(conn: &Connection, id: &str, subject: &str, turn: i64, first: f64) {
    let mut values = vec![first];
    values.extend(std::iter::repeat_n(0.0, 31));
    let list = values
        .iter()
        .map(|value| format!("{value}"))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute_batch(&format!(
        "INSERT INTO harness_trajectory (observation_id, subject_key, turn_k, embedding)
         VALUES ('{id}', '{subject}', {turn}, [{list}]::FLOAT[32])"
    ))
    .unwrap_or_else(|error| panic!("trajectory insert failed: {error}"));
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

trait EvidencePresent {
    fn status_evidence_present(&self) -> bool;
}

impl EvidencePresent for dendro_core::tripwire_v1::OperatorAdvisory {
    fn status_evidence_present(&self) -> bool {
        self.evidence.iter().any(|id| id == "o1") && self.evidence.iter().any(|id| id == "o2")
    }
}
