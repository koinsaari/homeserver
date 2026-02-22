use std::path::Path;

use thiserror::Error;

const VIDEO_EXTS: &[&str] = &["mkv", "mp4", "avi", "mov", "wmv", "flv", "webm", "m4v"];
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

    let lower = extension.to_ascii_lowercase();
    if EXECUTABLE_EXTS.contains(&lower.as_str()) {
        Err(ScanRejection::ExecutableExtension(lower))
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
}
