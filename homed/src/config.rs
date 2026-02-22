use serde::Deserialize;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    ReadError(#[from] std::io::Error),

    #[error("Failed to parse config file: {0}")]
    ParseError(#[from] toml::de::Error),

    #[error("Config validation failed: {0}")]
    ValidationError(String),
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub photos: PhotosConfig,
    pub media: MediaConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PhotosConfig {
    pub watcher: WatcherConfig,
    pub organizer: OrganizerConfig,
    pub nextcloud: NextcloudConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MediaConfig {
    pub watcher: WatcherConfig,
    pub scanner: ScannerConfig,
    pub mover: MoverConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WatcherConfig {
    pub paths: Vec<PathBuf>,
    pub debounce_ms: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ScannerConfig {
    pub quarantine_dir: PathBuf,
    pub allowed_extensions: Vec<String>,
    pub block_executables: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OrganizerConfig {
    pub enabled: bool,
    pub photos_dir: PathBuf,
    pub photo_prefix: String,
    pub video_prefix: String,
    pub photo_extensions: Vec<String>,
    pub video_extensions: Vec<String>,
    pub file_owner: Option<String>,
    pub file_group: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct NextcloudConfig {
    pub enabled: bool,
    pub container_name: String,
    pub username: String,
    pub data_dir: PathBuf,
    pub internal_prefix: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MoverConfig {
    pub enabled: bool,
    pub source: PathBuf,
    pub destination: PathBuf,
}

impl Config {
    pub fn load(path: &str) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        config.validate()?;

        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        Self::validate_watcher(&self.photos.watcher, "photos")?;
        Self::validate_watcher(&self.media.watcher, "media")?;

        Ok(())
    }

    fn validate_watcher(watcher: &WatcherConfig, name: &str) -> Result<(), ConfigError> {
        if watcher.paths.is_empty() {
            return Err(ConfigError::ValidationError(
                format!("{}.watcher.paths cannot be empty", name)
            ));
        }

        if watcher.debounce_ms < 100 || watcher.debounce_ms > 60_000 {
            return Err(ConfigError::ValidationError(
                format!("{}.watcher.debounce_ms must be between 100 and 60000, got {}",
                    name, watcher.debounce_ms)
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config {
            photos: PhotosConfig {
                watcher: WatcherConfig {
                    paths: vec![PathBuf::from("/tmp/photos")],
                    debounce_ms: 5000,
                },
                organizer: OrganizerConfig {
                    enabled: false,
                    photos_dir: Default::default(),
                    photo_prefix: "IMG".to_string(),
                    video_prefix: "VID".to_string(),
                    photo_extensions: vec![],
                    video_extensions: vec![],
                    file_owner: None,
                    file_group: None,
                },
                nextcloud: NextcloudConfig {
                    enabled: false,
                    container_name: "nextcloud".to_string(),
                    username: "admin".to_string(),
                    data_dir: Default::default(),
                    internal_prefix: "/admin/files".to_string(),
                },
            },
            media: MediaConfig {
                watcher: WatcherConfig {
                    paths: vec![PathBuf::from("/tmp/media")],
                    debounce_ms: 5000,
                },
                scanner: ScannerConfig {
                    quarantine_dir: Default::default(),
                    allowed_extensions: vec![],
                    block_executables: false,
                },
                mover: MoverConfig {
                    enabled: false,
                    source: Default::default(),
                    destination: Default::default(),
                },
            },
        }
    }

    #[test]
    fn test_valid_config() {
        assert!(test_config().validate().is_ok());
    }

    #[test]
    fn test_empty_photos_paths_fails() {
        let mut config = test_config();
        config.photos.watcher.paths = vec![];
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_empty_media_paths_fails() {
        let mut config = test_config();
        config.media.watcher.paths = vec![];
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_bad_debounce_fails() {
        let mut config = test_config();
        config.media.watcher.debounce_ms = 50;
        assert!(config.validate().is_err());
    }
}
