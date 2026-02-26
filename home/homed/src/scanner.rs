use std::path::Path;

use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::checks;
use crate::config::ScannerConfig;
use crate::watcher::FileEvent;

#[derive(Debug, Error)]
pub enum ScannerError {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
}

pub async fn run_scanner(
    config: ScannerConfig,
    mut rx: mpsc::Receiver<FileEvent>,
    tx: mpsc::Sender<FileEvent>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) -> Result<(), ScannerError> {
    tokio::fs::create_dir_all(&config.quarantine_dir).await?;

    loop {
        let event = tokio::select! {
            Some(event) = rx.recv() => event,
            _ = shutdown.recv() => break,
            else => break,
        };
        let FileEvent::Detected { path, size } = event else {
            continue;
        };

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if config.delete_junk && is_junk(&ext, &config.junk_extensions) {
            if let Err(e) = tokio::fs::remove_file(&path).await {
                error!(path = %path.display(), error = %e, "failed to delete junk file");
            } else {
                try_remove_empty_parent(&path).await;
                let _ = tx
                    .send(FileEvent::Cleaned {
                        path,
                        reason: format!("junk extension: .{ext}"),
                    })
                    .await;
            }
            continue;
        }

        if config.block_executables {
            if let Err(rejection) = checks::check_executable_extension(&path) {
                quarantine_file(&path, &config.quarantine_dir).await;
                try_remove_empty_parent(&path).await;
                let _ = tx
                    .send(FileEvent::Failed {
                        path,
                        error: rejection.to_string(),
                    })
                    .await;
                continue;
            }
        }

        if let Err(_rejection) = checks::check_extension(&path, &config.allowed_extensions) {
            if config.delete_junk {
                if let Err(e) = tokio::fs::remove_file(&path).await {
                    error!(path = %path.display(), error = %e, "failed to delete file");
                } else {
                    try_remove_empty_parent(&path).await;
                    let _ = tx
                        .send(FileEvent::Cleaned {
                            path,
                            reason: format!("extension not allowed: .{ext}"),
                        })
                        .await;
                }
            }
            continue;
        }

        // Size and magic byte checks
        if let Err(rejection) = check_content(&path, size).await {
            quarantine_file(&path, &config.quarantine_dir).await;
            try_remove_empty_parent(&path).await;
            let _ = tx
                .send(FileEvent::Failed {
                    path,
                    error: rejection.to_string(),
                })
                .await;
            continue;
        }

        let _ = tx.send(FileEvent::Scanned { path, clean: true }).await;
    }

    Ok(())
}

async fn check_content(path: &Path, size: u64) -> Result<(), checks::ScanRejection> {
    checks::check_file_size(path, size)?;
    checks::check_file_type(path).await?;
    Ok(())
}

async fn quarantine_file(path: &Path, quarantine_dir: &Path) {
    let filename = path
        .file_name()
        .unwrap_or(std::ffi::OsStr::new("unknown_file"));
    let quarantine_path = quarantine_dir.join(filename);

    if let Err(e) = tokio::fs::rename(path, &quarantine_path).await {
        error!(path = %path.display(), error = %e, "failed to quarantine file");
    }
}

fn is_junk(ext: &str, junk_extensions: &[String]) -> bool {
    junk_extensions.iter().any(|j| j.eq_ignore_ascii_case(ext))
}

async fn try_remove_empty_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        if tokio::fs::remove_dir(parent).await.is_ok() {
            info!(path = %parent.display(), "removed empty directory");
        }
    }
}
