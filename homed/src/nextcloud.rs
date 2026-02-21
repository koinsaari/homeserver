use std::path::Path;

use thiserror::Error;
use tokio::sync::mpsc;
use tracing::warn;

use crate::config::NextcloudConfig;
use crate::watcher::FileEvent;

#[derive(Debug, Error)]
pub enum NextcloudError {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Translates a host filesystem path to a Nextcloud internal path.
///
/// Example:
///   Host: /mnt/hot/nextcloud-data/admin/files/Photos/2026/2026-02/IMG_20260203_122134.jpg
///   Internal: /admin/files/Photos/2026/2026-02/IMG_20250203_122938.jpg
fn translate_path(host_path: &Path, config: &NextcloudConfig) -> Option<String> {
    let relative = host_path.strip_prefix(&config.data_dir).ok()?;
    let username_prefix = format!("{}/files/", config.username);
    let relative_str = relative.to_str()?;

    if let Some(stripped) = relative_str.strip_prefix(&username_prefix) {
        Some(format!("{}/{}", config.internal_prefix, stripped))
    } else {
        Some(format!("{}/{}", config.internal_prefix, relative_str))
    }
}

/// Runs `occ files:scan --path=<path>` via docker exec.
async fn run_occ_scan(config: &NextcloudConfig, path: &str) -> Result<(), NextcloudError> {
    let output = tokio::process::Command::new("docker")
        .args(["exec", "--user", "www-data", &config.container_name, "php", "occ", "files:scan"])
        .arg(format!("--path={}", path))
        .output()
        .await?;

    if !output.status.success() {
        warn!(
            exit_code = ?output.status.code(),
            stderr = %String::from_utf8_lossy(&output.stderr),
            "occ files:scan failed"
        );
    }

    Ok(())
}

/// Listens for Organized events and triggers Nextcloud file scans.
///
/// Logs warnings on failure but doesn't block the pipeline.
/// Forwards all events downstream for logging/alerting.
pub async fn run_nextcloud(
    config: NextcloudConfig,
    mut rx: mpsc::Receiver<FileEvent>,
    tx: mpsc::Sender<FileEvent>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) -> Result<(), NextcloudError> {
    loop {
        let event = tokio::select! {
            Some(event) = rx.recv() => event,
            _ = shutdown.recv() => break,
            else => break,
        };
        let FileEvent::Organized { old_path, new_path } = &event else {
            let _ = tx.send(event).await;
            continue;
        };

        if !config.enabled {
            let _ = tx.send(event).await;
            continue;
        }

        let Some(internal_path) = translate_path(new_path, &config) else {
            let _ = tx.send(event).await;
            continue;
        };

        if let Err(e) = run_occ_scan(&config, &internal_path).await {
            warn!(path = %new_path.display(), error = %e, "nextcloud scan failed");
        }

        // Scan old path's parent to remove ghost entries from Nextcloud DB
        if let Some(old_internal) = old_path.parent()
            .and_then(|p| translate_path(p, &config))
        {
            if let Err(e) = run_occ_scan(&config, &old_internal).await {
                warn!(path = %old_internal, error = %e, "nextcloud cleanup scan failed");
            }
        }

        let _ = tx.send(event).await;
    }

    Ok(())
}
