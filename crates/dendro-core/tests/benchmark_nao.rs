#[path = "support/semantic_fixture.rs"]
mod fixture;
use dendro_core::{report_json, semantic};
use duckdb::Connection;
use std::time::Instant;

#[test]
fn canonical_semantic_views_latency_and_conservation() -> anyhow::Result<()> {
    let f = fixture::Fixture::new(5500);
    #[cfg(windows)]
    let f = {
        let root = dendro_core::platform_v1::create_user_private_root_with_choice(
            "dendro-nao-benchmark-",
            &dendro_core::platform_v1::StorageRootChoice::UserProfile,
        )?;
        let db = root.path().join("analytics.duckdb");
        dendro_core::lifecycle::private_copy(&f.db, &db, &dendro_core::RUN_CANCEL)?;
        let key = dendro_core::output::load_or_create_key(
            &root.path().join("analytics.duckdb.key"),
            true,
        )?;
        let conn = Connection::open(&db)?;
        conn.execute("UPDATE dendro_meta SET key_id=?", duckdb::params![key.id])?;
        conn.execute_batch("CHECKPOINT")?;
        drop(conn);
        fixture::Fixture {
            dir: root.path().to_path_buf(),
            db,
        }
    };
    let report = report_json(
        &f.db,
        "decisions",
        Some("test-rig"),
        fixture::FROM,
        fixture::TO,
    )?;
    semantic::refresh(&f.db, Some("test-rig"), fixture::FROM, fixture::TO)
        .map_err(|e| e.context("benchmark selected semantic refresh"))?;
    let request = dendro_core::presentation::ReadRequest::open(&f.db)?;
    let config = duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly)?;
    let c = Connection::open_with_flags(request.database(), config)?;
    c.execute_batch("SET threads=1")?;
    let n: i64 = c.query_row("SELECT count(*) FROM usage_facts", [], |r| r.get(0))?;
    assert!(n >= 5000);
    let sums:(i64,i64,i64,i64)=c.query_row("SELECT sum(input_uncached),sum(output),sum(cache_read),sum(cache_write) FROM v_mission_token_burn",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    assert_eq!(sums, (5500 * 11, 5500 * 7, 5500 * 5, 5500 * 3));
    assert_eq!(report["payload"]["tokens"], 5500 * 26);
    let count: i64 = c.query_row(
        "SELECT count(DISTINCT source_key) FROM dendro_semantic_usage",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(count, n);
    let bad:i64=c.query_row("SELECT count(*) FROM v_seat_idle_ranking WHERE observed_secs != active_secs+idle_with_work_secs+idle_between_work_secs+waiting_external_secs+unknown_secs",[],|r|r.get(0))?;
    assert_eq!(bad, 0);
    for (view, budget) in semantic::VIEWS.into_iter().zip([15.0, 10.0, 10.0, 5.0]) {
        // View names are a fixed production allowlist, never user input.
        let sql = format!("SELECT * FROM {view}");
        let run = || -> anyhow::Result<usize> {
            let mut stmt = c.prepare(&sql)?;
            let mut rows = stmt.query([])?;
            let mut count = 0;
            while rows.next()?.is_some() {
                count += 1;
            }
            Ok(count)
        };
        let prepare_start = Instant::now();
        let mut prepared = c.prepare(&sql)?;
        let prepare_ms = prepare_start.elapsed().as_secs_f64() * 1000.0;
        let query_start = Instant::now();
        {
            let mut rows = prepared.query([])?;
            while rows.next()?.is_some() {}
        }
        println!(
            "diagnostic {view}: cold_prepare_ms={prepare_ms:.3} first_execute_ms={:.3}",
            query_start.elapsed().as_secs_f64() * 1000.0
        );
        assert!(run()? > 0);
        let mut timings = Vec::new();
        for _ in 0..20 {
            let start = Instant::now();
            let _ = run()?;
            timings.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        let mean = timings.iter().sum::<f64>() / timings.len() as f64;
        let max = timings.iter().copied().fold(0.0, f64::max);
        println!("view={view} canonical_usage_facts={n} iterations=20 mean_ms={mean:.3} max_ms={max:.3} budget_ms={budget}");
        println!("target_met={}", mean < budget);
        assert!(mean < 20.0, "{view}: hard 20ms mean SLA exceeded");
        assert!(
            max < 20.0,
            "{view}: an iteration exceeded hard 20ms SLA ({max:.3}ms)"
        );
    }
    Ok(())
}
