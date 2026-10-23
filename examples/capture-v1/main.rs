mod contribution;
use contribution::{Example, REGISTRATIONS};
use dendro_connectors::{capture_registry_v1::*, sdk::ExportRoot};
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let root_path = std::path::PathBuf::from(args.next().ok_or_else(|| {
        anyhow::anyhow!("usage: capture_v1 <selected-directory> <direct-child.jsonl>")
    })?);
    let name = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing direct child name"))?;
    let registry = Registry::new(REGISTRATIONS).map_err(anyhow::Error::msg)?;
    assert!(registry.descriptor("example.capture.synthetic").is_some());
    let report = registry.capture(
        &Example,
        &ExportRoot::new(root_path.clone()),
        &root_path.join(name),
        &mut Cursor::default(),
    );
    println!(
        "read={:?} projected={} unprojected={} reasons={:?}",
        report.read, report.counts.projected, report.counts.recognized_unprojected, report.reasons
    );
    println!("amounts={:?}", report.facts);
    if report.read == ReadState::Failed {
        anyhow::bail!("capture failed");
    }
    Ok(())
}
