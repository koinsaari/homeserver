use std::path::Path;

use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::config::MoverConfig;
use crate::watcher::FileEvent;

#[derive(Debug, Error)]
pub enum MoverError {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Hardlinks `source` to `dest`, falling back to copy for cross-device.
async fn hardlink_or_copy(source: &Path, dest: &Path) -> Result<(), MoverError> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let src = source.to_path_buf();
    let dst = dest.to_path_buf();

    let result = tokio::task::spawn_blocking(move || std::fs::hard_link(&src, &dst))
        .await
        .expect("hardlink task panicked");

    if result.is_ok() {
        return Ok(());
    }

    tokio::fs::copy(source, dest).await?;
    Ok(())
}

/// Hardlinks scanned files from source to destination and preserves subdirectory structure.
pub async fn run_mover(
    config: MoverConfig,
    mut rx: mpsc::Receiver<FileEvent>,
    tx: mpsc::Sender<FileEvent>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) -> Result<(), MoverError> {
    if config.enabled {
        tokio::fs::create_dir_all(&config.destination).await?;
    }

    loop {
        let event = tokio::select! {
            Some(event) = rx.recv() => event,
            _ = shutdown.recv() => break,
            else => break,
        };

        let FileEvent::Scanned { ref path, clean } = event else {
            let _ = tx.send(event).await;
            continue;
        };

        if !clean || !config.enabled {
            let _ = tx.send(event).await;
            continue;
        }

        let Ok(relative) = path.strip_prefix(&config.source) else {
            let _ = tx.send(event).await;
            continue;
        };

        let destination = config.destination.join(relative);

        if destination.exists() {
            let _ = tx.send(event).await;
            continue;
        }

        match hardlink_or_copy(path, &destination).await {
            Ok(()) => {
                info!(
                    from = %path.display(),
                    to = %destination.display(),
                    "file linked to import directory"
                );

                let _ = tx
                    .send(FileEvent::Organized {
                        old_path: path.clone(),
                        new_path: destination,
                    })
                    .await;
            }
            Err(e) => {
                warn!(
                    path = %path.display(),
                    dest = %destination.display(),
                    error = %e,
                    "failed to link file"
                );

                let _ = tx
                    .send(FileEvent::Failed {
                        path: path.clone(),
                        error: format!("failed to link: {}", e),
                    })
                    .await;
            }
        }
    }

    Ok(())
}
