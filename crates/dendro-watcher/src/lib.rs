use std::path::PathBuf;
use std::time::Duration;
use anyhow::Result;
use notify_debouncer_mini::{new_debouncer, DebouncedEvent};
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
