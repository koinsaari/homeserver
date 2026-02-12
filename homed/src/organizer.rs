use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, FixedOffset};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::config::OrganizerConfig;
use crate::watcher::{FileEvent, MediaType};

#[derive(Debug, Error)]
pub enum OrganizerError {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Builds the destination path avoiding collisions.
///
/// Format: `photos_dir/YYYY/YYYY-MM/PREFIX_YYYYMMDD_HHMMSS.ext`
/// If that path exists, appends `_1`, `_2`, etc.
fn build_target_path(
    config: &OrganizerConfig,
    media_type: MediaType,
    datetime: &DateTime<FixedOffset>,
    extension: &str,
) -> PathBuf {
    let prefix = match media_type {
        MediaType::Photo => &config.photo_prefix,
        MediaType::Video => &config.video_prefix,
    };

    let year = format!("{}", datetime.format("%Y"));
    let month = format!("{}-{:02}", datetime.year(), datetime.month());
    let timestamp = format!("{}", datetime.format("%Y%m%d_%H%M%S"));

    let dir = config.photos_dir.join(&year).join(&month);
    let base_name = format!("{}_{}.{}", prefix, timestamp, extension);
    let candidate = dir.join(&base_name);

    if !candidate.exists() {
        return candidate;
    }

    // Collision probe: _1, _2, _3...
    for suffix in 1u32.. {
        let name = format!("{}_{}_{}.{}", prefix, timestamp, suffix, extension);
        let candidate = dir.join(&name);
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!()
}

/// Moves a file across filesystems safely by copy -> sync -> delete.
///
/// `tokio::fs::rename` only works within the same filesystem (SSD→SSD).
/// For cross-device moves (SSD→HDD), we must copy the data, sync to
/// ensure it's flushed to disk, then delete the original.
async fn move_safe(source: &Path, dest: &Path) -> Result<(), OrganizerError> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    if tokio::fs::rename(source, dest).await.is_ok() {
        return Ok(());
    }

    // Cross-device fallback
    tokio::fs::copy(source, dest).await?;

    let dest_path = dest.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<(), std::io::Error> {
        let file = std::fs::File::open(&dest_path)?;
        file.sync_all()?;
        Ok(())
    })
    .await
    .expect("sync task panicked")?;

    tokio::fs::remove_file(source).await?;

    Ok(())
}

/// Organizes files into date-based directories with timestamp naming.
pub async fn run_organizer(
    config: OrganizerConfig,
    mut rx: mpsc::Receiver<FileEvent>,
    tx: mpsc::Sender<FileEvent>,
) -> Result<(), OrganizerError> {
    while let Some(event) = rx.recv().await {
        let FileEvent::Enriched { path, media_type, datetime } = event else {
            continue;
        };

        if !config.enabled {
            continue;
        }

        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("bin")
            .to_ascii_lowercase();

        let target = build_target_path(&config, media_type, &datetime, &extension);

        match move_safe(&path, &target).await {
            Ok(()) => {
                let _ = tx.send(FileEvent::Organized {
                    old_path: path,
                    new_path: target,
                }).await;
            }
            Err(e) => {
                let _ = tx.send(FileEvent::Failed {
                    path,
                    error: format!("Failed to organize: {}", e),
                }).await;
            }
        }
    }

    Ok(())
}
