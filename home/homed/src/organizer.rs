use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, FixedOffset};
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::warn;

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
///
/// On copy failure, cleans up any partial destination file.
async fn move_safe(source: &Path, dest: &Path) -> Result<(), OrganizerError> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    if tokio::fs::rename(source, dest).await.is_ok() {
        return Ok(());
    }

    // Cross-device fallback with cleanup on failure
    if let Err(e) = tokio::fs::copy(source, dest).await {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(e.into());
    }

    let dest_path = dest.to_path_buf();
    if let Err(e) = tokio::task::spawn_blocking(move || -> Result<(), std::io::Error> {
        let file = std::fs::File::open(&dest_path)?;
        file.sync_all()?;
        Ok(())
    })
    .await
    .expect("sync task panicked")
    {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(e.into());
    }

    tokio::fs::remove_file(source).await?;

    Ok(())
}

/// Builds the destination path for unsorted files, avoiding collisions.
///
/// Format: `photos_dir/unsorted_dir/original_name.ext`
/// If that path exists, appends `_1`, `_2`, etc. before the extension.
fn build_unsorted_path(dir: &Path, filename: &std::ffi::OsStr) -> PathBuf {
    let candidate = dir.join(filename);

    if !candidate.exists() {
        return candidate;
    }

    let name = filename.to_string_lossy();
    let (stem, ext) = match name.rfind('.') {
        Some(pos) => (&name[..pos], Some(&name[pos + 1..])),
        None => (name.as_ref(), None),
    };

    for suffix in 1u32.. {
        let new_name = match ext {
            Some(e) => format!("{}_{}.{}", stem, suffix, e),
            None => format!("{}_{}", stem, suffix),
        };
        let candidate = dir.join(&new_name);
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!()
}

/// Changes file ownership to allow Nextcloud (www-data) to read it.
async fn apply_ownership(path: &Path, owner: &str, group: &str) {
    let owner_group = format!("{}:{}", owner, group);

    let result = tokio::process::Command::new("chown")
        .arg(&owner_group)
        .arg(path)
        .output()
        .await;

    if let Err(e) = result {
        warn!(path = %path.display(), error = %e, "chown failed");
    }
}

/// Moves an unsorted file to an unsorted directory with the original filename.
/// Handles collisions by appending _1, _2, etc.
async fn handle_unsorted(
    config: &OrganizerConfig,
    path: &Path,
    tx: &mpsc::Sender<FileEvent>,
) -> Result<(), OrganizerError> {
    let Some(unsorted_dir) = &config.unsorted_dir else {
        let _ = tx
            .send(FileEvent::Failed {
                path: path.to_path_buf(),
                error: "No valid date and unsorted_dir not configured".to_string(),
            })
            .await;
        return Ok(());
    };

    let filename = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("unknown"));
    let unsorted_path = config.photos_dir.join(unsorted_dir);
    let target = build_unsorted_path(&unsorted_path, filename);

    match move_safe(path, &target).await {
        Ok(()) => {
            if let (Some(owner), Some(group)) = (&config.file_owner, &config.file_group) {
                apply_ownership(&target, owner, group).await;
            }

            let _ = tx
                .send(FileEvent::Organized {
                    old_path: path.to_path_buf(),
                    new_path: target,
                })
                .await;
        }
        Err(e) => {
            let _ = tx
                .send(FileEvent::Failed {
                    path: path.to_path_buf(),
                    error: format!("Failed to move to unsorted: {}", e),
                })
                .await;
        }
    }

    Ok(())
}

/// Organizes files into date-based directories with timestamp naming.
pub async fn run_organizer(
    config: OrganizerConfig,
    mut rx: mpsc::Receiver<FileEvent>,
    tx: mpsc::Sender<FileEvent>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) -> Result<(), OrganizerError> {
    loop {
        let event = tokio::select! {
            Some(event) = rx.recv() => event,
            _ = shutdown.recv() => break,
            else => break,
        };

        if !config.enabled {
            let _ = tx.send(event).await;
            continue;
        }

        match event {
            FileEvent::Enriched {
                path,
                media_type,
                datetime,
            } => {
                let extension = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("bin")
                    .to_ascii_lowercase();

                let target = build_target_path(&config, media_type, &datetime, &extension);

                match move_safe(&path, &target).await {
                    Ok(()) => {
                        if let (Some(owner), Some(group)) = (&config.file_owner, &config.file_group)
                        {
                            apply_ownership(&target, owner, group).await;
                        }

                        let _ = tx
                            .send(FileEvent::Organized {
                                old_path: path,
                                new_path: target,
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = tx
                            .send(FileEvent::Failed {
                                path,
                                error: format!("Failed to organize: {}", e),
                            })
                            .await;
                    }
                }
            }
            FileEvent::Unsorted { path, .. } => {
                let _ = handle_unsorted(&config, &path, &tx).await;
            }
            other => {
                let _ = tx.send(other).await;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use tempfile::tempdir;

    #[test]
    fn test_build_unsorted_path_no_collision() {
        let dir = tempdir().unwrap();
        let filename = OsStr::new("photo.jpg");
        let result = build_unsorted_path(dir.path(), filename);
        assert_eq!(result.file_name().unwrap(), "photo.jpg");
    }

    #[test]
    fn test_build_unsorted_path_with_collision() {
        let dir = tempdir().unwrap();
        let filename = OsStr::new("photo.jpg");

        std::fs::write(dir.path().join("photo.jpg"), "test").unwrap();

        let result = build_unsorted_path(dir.path(), filename);
        assert_eq!(result.file_name().unwrap(), "photo_1.jpg");
    }

    #[test]
    fn test_build_unsorted_path_multiple_collisions() {
        let dir = tempdir().unwrap();
        let filename = OsStr::new("photo.jpg");

        std::fs::write(dir.path().join("photo.jpg"), "test").unwrap();
        std::fs::write(dir.path().join("photo_1.jpg"), "test").unwrap();
        std::fs::write(dir.path().join("photo_2.jpg"), "test").unwrap();

        let result = build_unsorted_path(dir.path(), filename);
        assert_eq!(result.file_name().unwrap(), "photo_3.jpg");
    }

    #[test]
    fn test_build_unsorted_path_no_extension() {
        let dir = tempdir().unwrap();
        let filename = OsStr::new("README");

        std::fs::write(dir.path().join("README"), "test").unwrap();

        let result = build_unsorted_path(dir.path(), filename);
        assert_eq!(result.file_name().unwrap(), "README_1");
    }
}
