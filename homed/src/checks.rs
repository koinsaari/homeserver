use std::path::Path;

use thiserror::Error;
use tokio::io::AsyncReadExt;

const VIDEO_EXTS: &[&str] = &["mkv", "mp4", "mov", "webm"];
const SUBTITLE_EXTS: &[&str] = &["srt", "ass"];
const MIN_VIDEO_SIZE: u64 = 1024;

const EXECUTABLE_EXTS: &[&str] = &[
    "exe", "bat", "cmd", "com", "sh", "bash", "zsh", "py", "pyc", "pl", "rb", "jar", "app", "run",
];

#[derive(Debug, Error)]
pub enum ScanRejection {
    #[error("extension not allowed: .{0}")]
    ExtensionBlocked(String),

    #[error("executable extension blocked: .{0}")]
    ExecutableExtension(String),

    #[error("video file suspiciously small ({0} bytes)")]
    FileTooSmall(u64),

    #[error("file type mismatch: expected .{expected}, detected .{actual}")]
    TypeMismatch { expected: String, actual: String },

    #[error("unrecognized file type for .{0}")]
    UnrecognizedType(String),

    #[error("subtitle file is not valid UTF-8")]
    InvalidSubtitleEncoding,

    #[error("failed to read file: {0}")]
    IoError(#[from] std::io::Error),
}

pub fn check_extension(path: &Path, allowed: &[String]) -> Result<(), ScanRejection> {
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");

    let is_allowed = allowed.iter().any(|a| a.eq_ignore_ascii_case(extension));

    if is_allowed {
        Ok(())
    } else {
        Err(ScanRejection::ExtensionBlocked(extension.to_string()))
    }
}

pub fn check_executable_extension(path: &Path) -> Result<(), ScanRejection> {
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");

    if EXECUTABLE_EXTS
        .iter()
        .any(|&ext| ext.eq_ignore_ascii_case(extension))
    {
        Err(ScanRejection::ExecutableExtension(
            extension.to_ascii_lowercase(),
        ))
    } else {
        Ok(())
    }
}

pub fn check_file_size(path: &Path, size: u64) -> Result<(), ScanRejection> {
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    let lower = extension.to_ascii_lowercase();

    if VIDEO_EXTS.contains(&lower.as_str()) && size < MIN_VIDEO_SIZE {
        Err(ScanRejection::FileTooSmall(size))
    } else {
        Ok(())
    }
}

pub async fn check_file_type(path: &Path) -> Result<(), ScanRejection> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut file = tokio::fs::File::open(path).await?;
    let mut buf = [0u8; 8192];
    let n = file.read(&mut buf).await?;
    let bytes = &buf[..n];

    match infer::get(bytes) {
        Some(kind) if is_compatible(&extension, kind.extension()) => Ok(()),
        Some(kind) => Err(ScanRejection::TypeMismatch {
            expected: extension,
            actual: kind.extension().to_string(),
        }),
        None if SUBTITLE_EXTS.contains(&extension.as_str()) => {
            if std::str::from_utf8(bytes).is_ok() {
                Ok(())
            } else {
                Err(ScanRejection::InvalidSubtitleEncoding)
            }
        }
        // Some older encodings might have non-standard headers that infer can't identify
        // so for now just let them through
        None => Ok(()),
    }
}

/// infer detects MKV as "webm" and MOV as "mp4" since they share container formats
fn is_compatible(claimed: &str, detected: &str) -> bool {
    if claimed == detected {
        return true;
    }

    const ALIASES: &[&[&str]] = &[&["mkv", "webm"], &["mp4", "mov", "m4a"]];

    ALIASES
        .iter()
        .any(|group| group.contains(&claimed) && group.contains(&detected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowed_extension() {
        let allowed = vec!["mkv".to_string(), "mp4".to_string()];
        assert!(check_extension(Path::new("movie.mkv"), &allowed).is_ok());
        assert!(check_extension(Path::new("movie.MKV"), &allowed).is_ok());
    }

    #[test]
    fn test_blocked_extension() {
        let allowed = vec!["mkv".to_string()];
        let result = check_extension(Path::new("file.zip"), &allowed);
        assert!(matches!(result, Err(ScanRejection::ExtensionBlocked(_))));
    }

    #[test]
    fn test_executable_detected() {
        let result = check_executable_extension(Path::new("virus.exe"));
        assert!(matches!(result, Err(ScanRejection::ExecutableExtension(_))));
    }

    #[test]
    fn test_media_not_executable() {
        assert!(check_executable_extension(Path::new("movie.mkv")).is_ok());
    }

    #[test]
    fn test_executable_as_last_extension() {
        let result = check_executable_extension(Path::new("movie.mkv.exe"));
        assert!(matches!(result, Err(ScanRejection::ExecutableExtension(_))));
    }

    #[test]
    fn test_disguised_executable_passes_extension_check() {
        assert!(check_executable_extension(Path::new("virus.exe.mkv")).is_ok());
    }

    #[test]
    fn test_no_false_positive_on_dotted_filenames() {
        assert!(
            check_executable_extension(Path::new("Some.Show.S01E05.episode.title.sh.mkv")).is_ok()
        );
    }

    #[test]
    fn test_small_video_rejected() {
        let result = check_file_size(Path::new("tiny.mkv"), 500);
        assert!(matches!(result, Err(ScanRejection::FileTooSmall(500))));
    }

    #[test]
    fn test_normal_video_passes() {
        assert!(check_file_size(Path::new("movie.mkv"), 1_000_000).is_ok());
    }

    #[test]
    fn test_small_subtitle_passes() {
        assert!(check_file_size(Path::new("subs.srt"), 100).is_ok());
    }

    #[tokio::test]
    async fn test_pe_disguised_as_mkv_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake.mkv");
        tokio::fs::write(&path, b"MZ\x90\x00fake_pe_content")
            .await
            .unwrap();
        let result = check_file_type(&path).await;
        assert!(matches!(result, Err(ScanRejection::TypeMismatch { .. })));
    }

    #[tokio::test]
    async fn test_elf_disguised_as_mp4_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake.mp4");
        let mut elf_header = [0u8; 64];
        elf_header[0..4].copy_from_slice(&[0x7F, 0x45, 0x4C, 0x46]);
        elf_header[4] = 0x02; // 64-bit
        elf_header[5] = 0x01; // little-endian
        elf_header[6] = 0x01; // version
        tokio::fs::write(&path, &elf_header).await.unwrap();
        let result = check_file_type(&path).await;
        assert!(matches!(result, Err(ScanRejection::TypeMismatch { .. })));
    }

    #[tokio::test]
    async fn test_subtitle_text_passes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subs.srt");
        tokio::fs::write(&path, b"1\n00:00:01,000 --> 00:00:02,000\nHello")
            .await
            .unwrap();
        assert!(check_file_type(&path).await.is_ok());
    }

    #[tokio::test]
    async fn test_real_mkv_header_passes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("real.mkv");
        tokio::fs::write(&path, b"\x1a\x45\xdf\xa3matroska")
            .await
            .unwrap();
        assert!(check_file_type(&path).await.is_ok());
    }

    #[tokio::test]
    async fn test_unrecognized_video_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mystery.mkv");
        tokio::fs::write(&path, b"not a real video header at all")
            .await
            .unwrap();
        assert!(check_file_type(&path).await.is_ok());
    }

    #[tokio::test]
    async fn test_binary_subtitle_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.srt");
        tokio::fs::write(&path, &[0xFF, 0xFE, 0x00, 0x80, 0xC0])
            .await
            .unwrap();
        let result = check_file_type(&path).await;
        assert!(matches!(
            result,
            Err(ScanRejection::InvalidSubtitleEncoding)
        ));
    }
}
