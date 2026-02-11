use crate::config::WatcherConfig;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::Instant;
use thiserror::Error;
use tokio::sync::mpsc;

/// Events representing the file lifecycle within the pipeline.
#[derive(Debug, Clone)]
pub enum FileEvent {
    Detected { path: PathBuf, size: u64 },
    Scanned { path: PathBuf, clean: bool },
    Organized { old_path: PathBuf, new_path: PathBuf },
    Failed { path: PathBuf, error: String },
}

#[derive(Debug, Error)]
pub enum WatcherError {
    #[error("Failed to watch path: {0}")]
    WatchError(#[from] notify::Error),
}

/// Orchestrates filesystem watching and event debouncing.
///
/// Uses a dedicated thread to bridge the blocking `notify` crate with the
/// async runtime to ensure the executor is not stalled by FS events.
pub async fn run_watcher(
    config: WatcherConfig,
    tx: mpsc::Sender<FileEvent>,
) -> Result<(), WatcherError> {
    let (notify_tx, mut notify_rx) = mpsc::channel(100);
    let paths_to_watch = config.paths.clone();

    // Notify uses blocking threads so spawn a dedicated bridge thread
    // to prevent blocking the Tokio reactor
    std::thread::spawn(move || {
        let (std_tx, std_rx) = std::sync::mpsc::channel();

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = std_tx.send(event);
                }
            },
            notify::Config::default(),
        )
            .expect("Failed to create watcher");

        for path in &paths_to_watch {
            watcher
                .watch(path, RecursiveMode::Recursive)
                .expect("Failed to watch path");
        }

        while let Ok(event) = std_rx.recv() {
            if notify_tx.blocking_send(event).is_err() {
                break; // Receiver dropped, likely shutdown
            }
        }
    });

    let debounce_time = Duration::from_millis(config.debounce_ms);
    let mut pending_files: HashMap<PathBuf, Instant> = HashMap::new();
    let mut check_interval = tokio::time::interval(Duration::from_millis(500));

    loop {
        tokio::select! {
            // Handle incoming kernel events. We only care about creation/modification
            Some(event) = notify_rx.recv() => {
                if let EventKind::Create(_) | EventKind::Modify(_) = event.kind {
                    for path in event.paths {
                        // Existence check prevents race conditions where a file is
                        // created and immediately deleted before we process it
                        if path.exists() && path.is_file() {
                            pending_files.insert(path, Instant::now());
                        }
                    }
                }
            }

            // Periodic stability check. Files are "ready" only after X ms of silence
            // TODO: could use a more sophisticated way to avoid false positives
            _ = check_interval.tick() => {
                let now = Instant::now();
                let mut ready_paths = Vec::new();

                // Identify files that haven't received a write event since the last interval
                for (path, last_seen) in &pending_files {
                    if now.duration_since(*last_seen) >= debounce_time {
                        ready_paths.push(path.clone());
                    }
                }

                for path in ready_paths {
                    pending_files.remove(&path);

                    if let Ok(metadata) = std::fs::metadata(&path) {
                        let event = FileEvent::Detected {
                            path: path.clone(),
                            size: metadata.len(),
                        };

                        if tx.send(event).await.is_err() {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
}