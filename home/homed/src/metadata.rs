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

/// Extracts datetime from EXIF/track metadata or filename pattern.
/// Dates before min_valid_year are considered invalid (e.g., 1970 Unix epoch).
/// Returns None if no valid date is found since the file should go to the unsorted folder.
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

    None
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

        match extract_best_datetime(&path, media_type, config.min_valid_year).await {
            Some(datetime) => {
                let _ = tx
                    .send(FileEvent::Enriched {
                        path,
                        media_type,
                        datetime,
                    })
                    .await;
            }
            None => {
                let _ = tx.send(FileEvent::Unsorted { path, media_type }).await;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_filename_with_full_timestamp() {
        let path = PathBuf::from("/photos/IMG_20260211_143022.jpg");
        let dt = extract_datetime_from_filename(&path).unwrap();
        assert_eq!(dt.year(), 2026);
        assert_eq!(dt.month(), 2);
        assert_eq!(dt.day(), 11);
    }

    #[test]
    fn test_filename_with_date_only() {
        let path = PathBuf::from("/photos/20260315.jpg");
        let dt = extract_datetime_from_filename(&path).unwrap();
        assert_eq!(dt.year(), 2026);
        assert_eq!(dt.month(), 3);
        assert_eq!(dt.day(), 15);
    }

    #[test]
    fn test_filename_with_prefix_and_timestamp() {
        let path = PathBuf::from("/photos/VID_20251225_180000.mp4");
        let dt = extract_datetime_from_filename(&path).unwrap();
        assert_eq!(dt.year(), 2025);
        assert_eq!(dt.month(), 12);
        assert_eq!(dt.day(), 25);
    }

    #[test]
    fn test_filename_no_date_returns_none() {
        let path = PathBuf::from("/photos/vacation_photo.jpg");
        assert!(extract_datetime_from_filename(&path).is_none());
    }

    #[test]
    fn test_filename_short_digits_returns_none() {
        let path = PathBuf::from("/photos/IMG_123.jpg");
        assert!(extract_datetime_from_filename(&path).is_none());
    }

    #[test]
    fn test_filename_invalid_date_returns_none() {
        let path = PathBuf::from("/photos/99999999_999999.jpg");
        assert!(extract_datetime_from_filename(&path).is_none());
    }

    #[test]
    fn test_min_valid_year_rejects_old_dates() {
        let path = PathBuf::from("/photos/19700101_000000.jpg");
        let dt = extract_datetime_from_filename(&path).unwrap();
        assert_eq!(dt.year(), 1970);
        assert!(dt.year() < 2000);
    }

    #[test]
    fn test_min_valid_year_accepts_recent_dates() {
        let path = PathBuf::from("/photos/20240615_120000.jpg");
        let dt = extract_datetime_from_filename(&path).unwrap();
        assert!(dt.year() >= 2000);
    }

    #[tokio::test]
    async fn test_extract_best_datetime_rejects_pre_2000() {
        let path = PathBuf::from("/photos/19991231_235959.jpg");
        let result = extract_best_datetime(&path, MediaType::Photo, 2000).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_extract_best_datetime_accepts_post_2000() {
        let path = PathBuf::from("/photos/IMG_20260211_143022.jpg");
        let result = extract_best_datetime(&path, MediaType::Photo, 2000).await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().year(), 2026);
    }

    #[tokio::test]
    async fn test_extract_best_datetime_no_date_returns_none() {
        let path = PathBuf::from("/photos/random_photo.jpg");
        let result = extract_best_datetime(&path, MediaType::Photo, 2000).await;
        assert!(result.is_none());
    }
}
