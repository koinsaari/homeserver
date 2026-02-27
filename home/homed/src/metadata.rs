use std::path::Path;

use chrono::{DateTime, Datelike, FixedOffset, NaiveDateTime, Offset, TimeZone, Utc};
use nom_exif::{EntryValue, ExifIter, ExifTag, MediaParser, MediaSource, TrackInfo, TrackInfoTag};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::config::OrganizerConfig;
use crate::watcher::{FileEvent, MediaType};

fn classify_media_type(path: &Path, config: &OrganizerConfig) -> Option<MediaType> {
    let extension = path.extension().and_then(|ext| ext.to_str())?;

    let lower = extension.to_ascii_lowercase();

    if config
        .photo_extensions
        .iter()
        .any(|ext| ext.eq_ignore_ascii_case(&lower))
    {
        Some(MediaType::Photo)
    } else if config
        .video_extensions
        .iter()
        .any(|ext| ext.eq_ignore_ascii_case(&lower))
    {
        Some(MediaType::Video)
    } else {
        None
    }
}

fn extract_photo_datetime(path: &Path) -> Option<DateTime<FixedOffset>> {
    let mut parser = MediaParser::new();

    let ms = MediaSource::file_path(path).ok()?;

    if !ms.has_exif() {
        return None;
    }

    let iter: ExifIter = parser.parse(ms).ok()?;
    let exif: nom_exif::Exif = iter.into();

    match exif.get(ExifTag::DateTimeOriginal) {
        Some(EntryValue::Time(dt)) => Some(*dt),
        Some(EntryValue::NaiveDateTime(ndt)) => {
            let fixed = Utc.fix().from_utc_datetime(ndt);
            Some(fixed)
        }
        _ => None,
    }
}

fn extract_video_datetime(path: &Path) -> Option<DateTime<FixedOffset>> {
    let mut parser = MediaParser::new();

    let ms = MediaSource::file_path(path).ok()?;

    if !ms.has_track() {
        return None;
    }

    let info: TrackInfo = parser.parse(ms).ok()?;

    match info.get(TrackInfoTag::CreateDate) {
        Some(EntryValue::Time(dt)) => Some(*dt),
        _ => None,
    }
}

/// Attempts to parse a date from filenames like "IMG_20260211_143022.jpg"
/// or "20260211_143022.jpg". Returns None if no date pattern is found.
fn extract_datetime_from_filename(path: &Path) -> Option<DateTime<FixedOffset>> {
    let stem = path.file_stem()?.to_str()?;

    // Try YYYYMMDD_HHMMSS pattern anywhere in the filename
    let digits: String = stem.chars().filter(|c| c.is_ascii_digit()).collect();

    if digits.len() >= 14 {
        let date_str = &digits[..14];
        let naive = NaiveDateTime::parse_from_str(date_str, "%Y%m%d%H%M%S").ok()?;
        return Some(Utc.fix().from_utc_datetime(&naive));
    }

    if digits.len() >= 8 {
        let date_str = &digits[..8];
        let naive =
            NaiveDateTime::parse_from_str(&format!("{}000000", date_str), "%Y%m%d%H%M%S").ok()?;
        return Some(Utc.fix().from_utc_datetime(&naive));
    }

    None
}

async fn fallback_to_mtime(path: &Path) -> Option<DateTime<FixedOffset>> {
    let metadata = tokio::fs::metadata(path).await.ok()?;
    let mtime = metadata.modified().ok()?;
    let utc: DateTime<Utc> = mtime.into();
    Some(Utc.fix().from_utc_datetime(&utc.naive_utc()))
}

/// Extracts the best available datetime by EXIF/track metadata ->
/// filename pattern -> file modification time.
/// Dates before min_valid_year are considered invalid (e.g., 1970 Unix epoch).
async fn extract_best_datetime(
    path: &Path,
    media_type: MediaType,
    min_valid_year: i32,
) -> Option<DateTime<FixedOffset>> {
    let owned_path = path.to_path_buf();
    let exif_result = tokio::task::spawn_blocking(move || match media_type {
        MediaType::Photo => extract_photo_datetime(&owned_path),
        MediaType::Video => extract_video_datetime(&owned_path),
    })
    .await
    .ok()
    .flatten();

    if let Some(dt) = exif_result {
        if dt.year() >= min_valid_year {
            return Some(dt);
        }
    }

    if let Some(dt) = extract_datetime_from_filename(path) {
        if dt.year() >= min_valid_year {
            return Some(dt);
        }
    }

    fallback_to_mtime(path).await
}

#[derive(Debug, Error)]
pub enum MetadataError {}

/// Classifies files as photo/video and extracts timestamps.
///
/// Non-media files are rejected with a Failed event. Files without
/// any extractable datetime are also rejected since we can't name them.
pub async fn run_metadata(
    config: OrganizerConfig,
    mut rx: mpsc::Receiver<FileEvent>,
    tx: mpsc::Sender<FileEvent>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) -> Result<(), MetadataError> {
    loop {
        let event = tokio::select! {
            Some(event) = rx.recv() => event,
            _ = shutdown.recv() => break,
            else => break,
        };
        let path = match event {
            FileEvent::Detected { path, .. } => path,
            other => {
                let _ = tx.send(other).await;
                continue;
            }
        };

        let Some(media_type) = classify_media_type(&path, &config) else {
            let _ = tx
                .send(FileEvent::Failed {
                    path,
                    error: "Unsupported media type".to_string(),
                })
                .await;
            continue;
        };

        let Some(datetime) = extract_best_datetime(&path, media_type, config.min_valid_year).await
        else {
            let _ = tx
                .send(FileEvent::Failed {
                    path,
                    error: "Could not extract datetime".to_string(),
                })
                .await;
            continue;
        };

        let _ = tx
            .send(FileEvent::Enriched {
                path,
                media_type,
                datetime,
            })
            .await;
    }

    Ok(())
}
