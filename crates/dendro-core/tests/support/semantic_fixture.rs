use dendro_core::AnalyticsStore;
use duckdb::Connection;
use std::path::PathBuf;

pub const FROM: &str = "2026-10-01T00:00:00Z";
pub const TO: &str = "2026-10-02T00:00:00Z";
pub struct Fixture {
    pub dir: PathBuf,
    pub db: PathBuf,
}
impl Fixture {
    pub fn new(facts: usize) -> Self {
        let dir = std::env::current_dir()
            .unwrap()
            .join("target/semantic-tests")
            .join(format!("{}-{}", std::process::id(), rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("analytics.duckdb");
        drop(AnalyticsStore::open_path(&db, "fixture").unwrap());
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("INSERT INTO identity_facts VALUES ('id:build','claude','session-build','test-rig','dev-build@test-rig','2026-10-01 00:00:00');
          INSERT INTO identity_facts VALUES ('id:review','codex','session-review','test-rig','rev-review@test-rig','2026-10-01 00:00:00');
          INSERT INTO source_files VALUES ('file','claude','synthetic.jsonl',0,100,'fixture','session-build');
          INSERT INTO seat_bounds VALUES ('dev-build@test-rig','2026-10-01 00:00:00','2026-10-02 00:00:00'),('rev-review@test-rig','2026-10-01 00:00:00','2026-10-02 00:00:00');
          INSERT INTO seat_roles VALUES ('dev-build@test-rig','builder','fixture'),('rev-review@test-rig','reviewer','fixture');
          INSERT INTO activity_facts VALUES ('act:1','dev-build@test-rig','2026-10-01 00:00:00','running',1),('act:2','dev-build@test-rig','2026-10-01 12:00:00','idle',2),('act:3','rev-review@test-rig','2026-10-01 00:00:00','running',1);
          INSERT INTO queue_facts (source_key,transition_id,qitem_id,ts,state,source_seat,dest_seat,missions,chain_id,issue_tags,phase_tags) VALUES
          ('q:1',1,'item1','2026-10-01 00:00:00','pending','orch-arch@test-rig','dev-build@test-rig','[\"semantic\"]','chain','[\"16\"]','[\"build\"]'),
          ('q:2',2,'item1','2026-10-01 00:00:01','in-progress','orch-arch@test-rig','dev-build@test-rig','[\"semantic\"]','chain','[\"16\"]','[\"build\"]'),
          ('q:3',3,'item1','2026-10-01 11:00:00','done','orch-arch@test-rig','dev-build@test-rig','[\"semantic\"]','chain','[\"16\"]','[\"build\"]'),
          ('q:4',4,'item2','2026-10-01 11:00:01','pending','dev-build@test-rig','rev-review@test-rig','[\"semantic\"]','chain','[\"16\"]','[\"review\"]'),
          ('q:5',5,'item2','2026-10-01 11:00:02','in-progress','dev-build@test-rig','rev-review@test-rig','[\"semantic\"]','chain','[\"16\"]','[\"review\"]'),
          ('q:6',6,'item2','2026-10-01 13:00:00','done','dev-build@test-rig','rev-review@test-rig','[\"semantic\"]','chain','[\"16\"]','[\"review\"]');").unwrap();
        conn.execute_batch("UPDATE queue_facts SET source_key='events:queue.created:item1' WHERE source_key='q:1';
          UPDATE queue_facts SET state='handed-off',source_seat='dev-build@test-rig',closure_reason='handed_off_to',closure_target='rev-review@test-rig' WHERE source_key='q:3';
          UPDATE queue_facts SET source_key='events:queue.created:item2',parent_qitem_id='item1',closure_reason='handed_off_from',closure_target='item1' WHERE source_key='q:4'").unwrap();
        conn.execute("INSERT INTO usage_facts (source_key,harness,native_session,ts,seq,input_uncached,output_tokens,cache_read,cache_write,reasoning,generation,source_id,model,effort)
          SELECT 'file:0:' || i,'claude','session-build',CAST('2026-10-01 00:00:02' AS TIMESTAMP) + i * INTERVAL '1 second',i,11,7,5,3,0,0,'file','claude-fixture','medium' FROM range(1,?) t(i)",duckdb::params![facts as i64+1]).unwrap();
        conn.execute_batch("INSERT INTO tool_facts (source_key,harness,native_session,ts,seq,tool_name,structured_write,generation,source_id,category) VALUES ('tool:1','claude','session-build','2026-10-01 01:00:00',1,'Edit',true,0,'file','eng_code_edit')").unwrap();
        drop(conn);
        Self { dir, db }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
