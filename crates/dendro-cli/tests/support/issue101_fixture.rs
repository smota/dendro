//! Persisted synthetic native-table inputs; never inject overview-derived incidents.
use chrono::{DateTime, Utc};
use dendro_core::overview::{evaluate_overview_snapshot_path, OverviewRequest};
use dendro_core::AnalyticsStore;
use duckdb::{params, Connection};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub const FROM: &str = "2026-10-01T00:00:00Z";
pub const TO: &str = "2026-10-02T00:00:00Z";
pub const RIG: &str = "test-rig";
pub const SEAT: &str = "dev-build@test-rig";
pub type QueueRow<'a> = (
    &'a str,
    i64,
    &'a str,
    &'a str,
    &'a str,
    &'a str,
    Option<&'a str>,
    &'a str,
);
static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct Fixture {
    pub root: PathBuf,
    pub db: PathBuf,
    pub keep: bool,
}

pub fn dt(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

pub fn request() -> OverviewRequest {
    OverviewRequest {
        rig: Some(RIG.into()),
        from: dt(FROM),
        to: dt(TO),
        observation_clock: Some(dt(TO)),
        evidence_as_of: None,
        advisory_options: Default::default(),
        friction_thresholds: Default::default(),
        expect_evidence_digest: None,
    }
}

impl Fixture {
    pub fn empty() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
            .join(".agent-runs/issue101-regressions")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        Self::at(root)
    }

    pub fn at(root: PathBuf) -> Self {
        assert!(
            !root.exists(),
            "fixture root must be new: {}",
            root.display()
        );
        fs::create_dir_all(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            let scratch = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .unwrap()
                .join(".agent-runs");
            // Native path admission also verifies ancestor modes. Mutate only
            // this fixture's authorized scratch ancestors, preserving other bits.
            for dir in root
                .ancestors()
                .skip(1)
                .take_while(|dir| dir.starts_with(&scratch))
            {
                let mode = fs::metadata(dir).unwrap().permissions().mode();
                fs::set_permissions(dir, fs::Permissions::from_mode(mode & !0o022)).unwrap();
            }
        }
        let root = root.canonicalize().unwrap();
        let db = root.join("dendro.duckdb");
        // A new synthetic key is local test state, never an owner's credential.
        let key =
            dendro_core::output::load_or_create_key(&root.join("dendro.duckdb.key"), true).unwrap();
        let store = AnalyticsStore::open_path(&db, &key.id).unwrap();
        store.checkpoint().unwrap();
        drop(store);
        Self {
            root,
            db,
            keep: false,
        }
    }

    pub fn conn(&self) -> Connection {
        Connection::open(&self.db).unwrap()
    }

    pub fn identity(&self, key: &str, session: &str, rig: &str, seat: &str) {
        self.conn()
            .execute(
                "INSERT INTO identity_facts VALUES (?, 'codex', ?, ?, ?, ?::TIMESTAMP)",
                params![key, session, rig, seat, FROM],
            )
            .unwrap();
    }

    pub fn base_identity(&self) {
        self.identity("id:build", "session-build", RIG, SEAT);
    }

    pub fn usage(
        &self,
        key: &str,
        session: &str,
        ts: Option<&str>,
        tokens: i64,
        cost: Option<f64>,
    ) {
        self.conn().execute("INSERT INTO usage_facts (source_key,harness,native_session,ts,seq,input_uncached,output_tokens,cache_read,cache_write,reasoning,currency_total,generation,source_id,model) VALUES (?,'codex',?,?::TIMESTAMP,1,?,0,0,0,0,?,0,'fixture','synthetic-priced')",
            params![key,session,ts,tokens,cost]).unwrap();
    }

    pub fn queue(&self, row: QueueRow<'_>) {
        let (key, seq, item, ts, state, tags, reason, seat) = row;
        self.conn().execute("INSERT INTO queue_facts (source_key,transition_id,qitem_id,ts,state,source_seat,dest_seat,closure_reason,missions,is_recovery,issue_tags) VALUES (?,?,?,?::TIMESTAMP,?,'orch@test-rig',?,?, '[]',false,?)",
            params![key,seq,item,ts,state,seat,reason,tags]).unwrap();
    }

    pub fn activity(&self, key: &str, ts: &str, state: &str, seq: i64) {
        self.conn()
            .execute(
                "INSERT INTO activity_facts VALUES (?,?,?::TIMESTAMP,?,?)",
                params![key, SEAT, ts, state, seq],
            )
            .unwrap();
    }

    pub fn process(&self, key: &str, fingerprint: &str) {
        self.conn().execute("INSERT INTO process_identities (identity_key,root_key,pid,platform,start_time_unix_s,exe_name,harness,cwd_fingerprint,session_link,environment_kind,isolation_hint,first_seen,last_seen,visibility_limits) VALUES (?, ?,1,'linux',0,'codex','codex',?,'session-build','native','none',?::TIMESTAMP,?::TIMESTAMP,'[]')",
            params![key,key,fingerprint,FROM,TO]).unwrap();
    }

    pub fn skill(&self, key: &str, skill: &str, invoked: bool, seat: &str) {
        self.conn()
            .execute(
                "INSERT INTO skill_facts VALUES (?,?,?,?)",
                params![key, seat, skill, invoked],
            )
            .unwrap();
    }

    pub fn tool(
        &self,
        key: &str,
        session: &str,
        ts: Option<&str>,
        name: &str,
        failed: Option<bool>,
    ) {
        self.conn().execute("INSERT INTO tool_facts (source_key,harness,native_session,ts,seq,tool_name,failed,structured_write,generation,source_id) VALUES (?,'codex',?,?::TIMESTAMP,1,?,?,false,0,'fixture')",
            params![key,session,ts,name,failed]).unwrap();
    }

    pub fn overview(&self) -> Value {
        self.overview_with(&request())
    }

    pub fn overview_with(&self, req: &OverviewRequest) -> Value {
        serde_json::to_value(evaluate_overview_snapshot_path(&self.db, req).unwrap()).unwrap()
    }

    pub fn api(&self, rig: &str) -> Value {
        dendro_cli::connected_web::execute_live_query(
            &self.db,
            "direct_overview",
            Some(&format!("{FROM}..{TO}")),
            Some(rig),
        )
        .unwrap()["direct_overview"]
            .clone()
    }

    pub fn populate(&self) {
        self.base_identity();
        self.identity(
            "id:foreign",
            "foreign-session",
            "other-rig",
            "dev-build@other-rig",
        );
        self.process("process:build", "fp-a81f");
        self.conn()
            .execute(
                "INSERT INTO seat_bounds VALUES (?,?::TIMESTAMP,?::TIMESTAMP)",
                params![SEAT, FROM, TO],
            )
            .unwrap();
        self.activity("act:running", "2026-10-01T01:00:00Z", "running", 1);
        self.activity("act:idle", "2026-10-01T02:00:00Z", "idle", 2);
        self.usage(
            "usage:main",
            "session-build",
            Some("2026-10-01T01:00:00Z"),
            160,
            Some(30.0),
        );
        self.usage(
            "usage:foreign",
            "foreign-session",
            Some("2026-10-01T01:00:00Z"),
            9999,
            Some(999.0),
        );
        self.skill("usage:main:skill:0", "tool-alpha", false, SEAT);
        self.skill("usage:main:skill:1", "tool-beta", false, SEAT);
        for (key, seq, item, tags) in [
            ("done:101", 1, "task-101", "[\"101\"]"),
            ("done:91", 2, "task-91", "[\"91\"]"),
            ("done:26", 3, "task-26", "[\"26\"]"),
        ] {
            self.queue((
                key,
                seq,
                item,
                "2026-10-01T01:30:00Z",
                "done",
                tags,
                Some("no-follow-on"),
                SEAT,
            ));
        }
        for i in 1..=7 {
            self.queue((
                &format!("internal:{i}"),
                10 + i,
                &format!("internal-{i}"),
                "2026-10-01T01:45:00Z",
                "done",
                "[]",
                Some("no-follow-on"),
                SEAT,
            ));
        }
        self.queue((
            "stall:created",
            20,
            "live-leaf",
            "2026-10-01T02:59:00Z",
            "pending",
            "[]",
            None,
            SEAT,
        ));
        self.queue((
            "stall:claim",
            21,
            "live-leaf",
            "2026-10-01T03:00:00Z",
            "in-progress",
            "[]",
            None,
            SEAT,
        ));
        self.tool(
            "tool:read:ok",
            "session-build",
            Some("2026-10-01T01:05:00Z"),
            "read",
            Some(false),
        );
        self.tool(
            "tool:read:fail",
            "session-build",
            Some("2026-10-01T01:06:00Z"),
            "read",
            Some(true),
        );
        self.tool(
            "tool:read:unknown",
            "session-build",
            Some("2026-10-01T01:07:00Z"),
            "read",
            None,
        );
        self.conn().execute_batch("CHECKPOINT;").unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
