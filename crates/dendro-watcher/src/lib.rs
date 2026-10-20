#![deny(unsafe_code)]

use anyhow::Result;
use notify_debouncer_mini::{new_debouncer, DebouncedEvent};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::info;

pub struct DendroWatcher {
    watch_paths: Vec<PathBuf>,
    debounce_duration: Duration,
}

impl DendroWatcher {
    pub fn new(debounce_duration: Duration) -> Self {
        Self {
            watch_paths: Vec::new(),
            debounce_duration,
        }
    }

    pub fn add_path(&mut self, path: PathBuf) {
        if path.exists() {
            self.watch_paths.push(path);
        }
    }

    pub async fn run(self, tx: mpsc::Sender<Vec<DebouncedEvent>>) -> Result<()> {
        let (sync_tx, sync_rx) = std::sync::mpsc::channel();
        let mut debouncer = new_debouncer(self.debounce_duration, sync_tx)?;

        for path in &self.watch_paths {
            info!("Watching path: {}", path.display());
            debouncer
                .watcher()
                .watch(path, notify::RecursiveMode::Recursive)?;
        }

        tokio::task::spawn_blocking(move || {
            let _debouncer = debouncer;
            while let Ok(events) = sync_rx.recv() {
                if let Ok(valid_events) = events {
                    if tx.blocking_send(valid_events).is_err() {
                        break;
                    }
                }
            }
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn watcher_retains_debouncer_and_delivers_file_events() {
        let dir = std::env::temp_dir().join(format!(
            "dendro-watcher-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let watched_file = dir.join("events.jsonl");
        fs::write(&watched_file, b"initial\n").unwrap();

        let mut watcher = DendroWatcher::new(Duration::from_millis(50));
        watcher.add_path(dir.clone());
        let (tx, mut rx) = mpsc::channel(8);
        watcher.run(tx).await.unwrap();

        tokio::time::sleep(Duration::from_millis(60)).await;
        fs::write(&watched_file, b"initial\nappended\n").unwrap();

        let received = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("watcher timed out")
            .expect("watcher channel closed prematurely");
        assert!(!received.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}
