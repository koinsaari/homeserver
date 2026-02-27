use crate::config::WatcherConfig;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::info;

#[derive(Debug, Clone, Copy)]
pub enum MediaType {
    Photo,
    Video,
}

/// Events representing the file lifecycle within the pipeline.
#[derive(Debug, Clone)]
pub enum FileEvent {
    Detected {
        path: PathBuf,
        size: u64,
    },
    Scanned {
        path: PathBuf,
        clean: bool,
    },
    Enriched {
        path: PathBuf,
        media_type: MediaType,
        datetime: chrono::DateTime<chrono::FixedOffset>,
    },
    Unsorted {
        path: PathBuf,
        media_type: MediaType,
    },
    Organized {
        old_path: PathBuf,
        new_path: PathBuf,
    },
    Cleaned {
        path: PathBuf,
        reason: String,
    },
    Failed {
        path: PathBuf,
        error: String,
    },
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
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) -> Result<(), WatcherError> {
    let (notify_tx, mut notify_rx) = mpsc::channel(100);
    let paths_to_watch = config.paths.clone();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let thread_stop = stop_flag.clone();

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

        loop {
            if thread_stop.load(Ordering::Relaxed) {
                break;
            }
            match std_rx.recv_timeout(Duration::from_secs(1)) {
                Ok(event) => {
                    if notify_tx.blocking_send(event).is_err() {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    let debounce_time = Duration::from_millis(config.debounce_ms);
    let mut pending_files: HashMap<PathBuf, Instant> = HashMap::new();
    let mut check_interval = tokio::time::interval(Duration::from_millis(500));

    // Pick up files that arrived while homed was not running
    let ready_at = Instant::now() - debounce_time;
    for watch_path in &config.paths {
        scan_existing_files(
            watch_path,
            &config.ignore_extensions,
            ready_at,
            &mut pending_files,
        );
    }
    if !pending_files.is_empty() {
        info!(
            count = pending_files.len(),
            "found existing files on startup"
        );
    }

    loop {
        tokio::select! {
            // Handle incoming kernel events. We only care about creation/modification
            Some(event) = notify_rx.recv() => {
                if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                    for path in event.paths {
                        if path.components().any(|c| {
                            c.as_os_str().to_string_lossy().starts_with('.')
                        }) {
                            continue;
                        }
                        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                            if config.ignore_extensions.iter().any(|ie| ie.eq_ignore_ascii_case(ext)) {
                                continue;
                            }
                        }
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

                    if let Ok(metadata) = tokio::fs::metadata(&path).await {
                        let size = metadata.len();
                        if size == 0 {
                            continue;
                        }
                        let event = FileEvent::Detected {
                            path: path.clone(),
                            size,
                        };

                        if tx.send(event).await.is_err() {
                            return Ok(());
                        }
                    }
                }
            }

            _ = shutdown.recv() => {
                stop_flag.store(true, Ordering::Relaxed);
                info!(pending = pending_files.len(), "watcher shutting down, draining pending files");

                // Emit any files that are already debounced before exiting
                for (path, _) in pending_files.drain() {
                    if let Ok(metadata) = tokio::fs::metadata(&path).await {
                        let _ = tx.send(FileEvent::Detected {
                            path: path.clone(),
                            size: metadata.len(),
                        }).await;
                    }
                }

                return Ok(());
            }
        }
    }
}

fn scan_existing_files(
    dir: &PathBuf,
    ignore_extensions: &[String],
    timestamp: Instant,
    pending: &mut HashMap<PathBuf, Instant>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if path.is_dir() {
            if name_str.starts_with('.') {
                continue;
            }
            scan_existing_files(&path, ignore_extensions, timestamp, pending);
            continue;
        }

        if !path.is_file() {
            continue;
        }

        if name_str.starts_with('.') {
            continue;
        }

        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if ignore_extensions
                .iter()
                .any(|ie| ie.eq_ignore_ascii_case(ext))
            {
                continue;
            }
        }

        pending.insert(path, timestamp);
    }
}
