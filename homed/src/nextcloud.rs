use std::path::Path;

use thiserror::Error;
use tokio::sync::mpsc;

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
        .args(["exec", &config.container_name, "php", "occ", "files:scan"])
        .arg(format!("--path={}", path))
        .output()
        .await?;

    if !output.status.success() {
        eprintln!(
            "Warning: occ files:scan failed (exit code {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
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
) -> Result<(), NextcloudError> {
    while let Some(event) = rx.recv().await {
        let FileEvent::Organized { old_path: _, new_path } = &event else {
            let _ = tx.send(event).await;
            continue;
        };

        if !config.enabled {
            continue;
        }

        let Some(internal_path) = translate_path(&new_path, &config) else {
            eprintln!(
                "Warning: Could not translate path {} to Nextcloud internal path",
                new_path.display()
            );
            continue;
        };

        if let Err(e) = run_occ_scan(&config, &internal_path).await {
            eprintln!(
                "Warning: Nextcloud scan failed for {}: {}",
                new_path.display(),
                e
            );
        }

        let _ = tx.send(event).await;
    }

    Ok(())
}
