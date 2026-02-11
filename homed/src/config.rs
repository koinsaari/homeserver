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
    pub watcher: WatcherConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WatcherConfig {
    pub paths: Vec<PathBuf>,
    pub debounce_ms: u64,
}

impl Config {
    pub fn load(path: &str) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        config.validate()?;

        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.watcher.paths.is_empty() {
            return Err(ConfigError::ValidationError(
                "watcher.paths cannot be empty".to_string()
            ));
        }

        if self.watcher.debounce_ms < 100 || self.watcher.debounce_ms > 60_000 {
            return Err(ConfigError::ValidationError(
                format!("watcher.debounce_ms must be between 100 and 60000, got {}",
                    self.watcher.debounce_ms)
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_validation() {
        let config = Config {
            watcher: WatcherConfig {
                paths: vec![PathBuf::from("/tmp")],
                debounce_ms: 5000,
            },
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_empty_paths_fails() {
        let config = Config {
            watcher: WatcherConfig {
                paths: vec![],
                debounce_ms: 5000,
            },
        };

        assert!(config.validate().is_err());
    }
}
