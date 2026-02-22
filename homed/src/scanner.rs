use crate::config::ScannerConfig;
use crate::watcher::FileEvent;
use std::path::Path;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::error;

#[derive(Debug, Error)]
pub enum ScannerError {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
}

fn is_extension_allowed(path: &Path, allowed: &[String]) -> bool {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");

    allowed.iter().any(|allowed_ext| {
        allowed_ext.eq_ignore_ascii_case(extension)
    })
}

fn is_executable(path: &Path) -> bool {
    const EXECUTABLE_EXTS: &[&str] = &[
        "exe", "bat", "cmd", "com", "sh", "bash", "zsh",
        "py", "pyc", "pl", "rb", "jar", "app", "run"
    ];

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");

    EXECUTABLE_EXTS.contains(&extension)
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
        let FileEvent::Detected { path, size: _ } = event else {
            continue;
        };

        if !is_extension_allowed(&path, &config.allowed_extensions) {
            let _ = tx.send(FileEvent::Failed {
                path,
                error: "File extension not allowed".to_string(),
            }).await;
            continue;
        }

        if config.block_executables && is_executable(&path) {
            quarantine_file(&path, &config.quarantine_dir).await;

            let _ = tx.send(FileEvent::Failed {
                path,
                error: "Executable file blocked".to_string(),
            }).await;
            continue;
        }

        let _ = tx.send(FileEvent::Scanned {
            path,
            clean: true,
        }).await;
    }

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
