use std::path::Path;

use thiserror::Error;
use tokio::sync::mpsc;
use tracing::error;

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

        if let Err(rejection) = check_file(&path, size, &config).await {
            if config.block_executables
                || !matches!(rejection, checks::ScanRejection::ExecutableExtension(_))
            {
                quarantine_file(&path, &config.quarantine_dir).await;
            }

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

async fn check_file(
    path: &Path,
    size: u64,
    config: &ScannerConfig,
) -> Result<(), checks::ScanRejection> {
    checks::check_extension(path, &config.allowed_extensions)?;

    if config.block_executables {
        checks::check_executable_extension(path)?;
    }

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
