use crate::error::{Result, WindchillError};
use config::{Config as ConfigBuilder, Environment, File};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const BASE_URL_PLACEHOLDER: &str = "https://your-windchill-server.example.com/Windchill";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub base_url: String,

    #[serde(skip)]
    pub username: Option<String>,

    #[serde(skip)]
    pub auth_token: Option<String>,
}

impl Config {
    /// Load configuration from file, environment variables, and CLI arguments.
    /// Priority: CLI args > Environment variables > Config file > empty.
    /// Returns an error if no base URL can be resolved.
    pub fn load(base_url_override: Option<String>) -> Result<Self> {
        let config_path = Self::config_path()?;

        let mut builder = ConfigBuilder::builder()
            .add_source(File::from(config_path).required(false))
            .add_source(Environment::with_prefix("WINDCHILL"));

        if let Some(url) = base_url_override {
            builder = builder.set_override("base_url", url)?;
        }

        let config: Config = builder.build()?.try_deserialize()?;

        if config.base_url.is_empty() || config.base_url == BASE_URL_PLACEHOLDER {
            return Err(WindchillError::Other(
                "No Windchill base URL configured. Set via --baseurl, \
                 the WINDCHILL_BASE_URL environment variable, or \
                 ~/.config/windchill/config.toml (run `windchill init`)."
                    .to_string(),
            ));
        }

        Ok(config)
    }

    /// Get the configuration file path (~/.config/windchill/config.toml)
    fn config_path() -> Result<PathBuf> {
        let home = dirs::config_dir().ok_or_else(|| {
            WindchillError::ConfigError(config::ConfigError::Message(
                "Cannot find config directory".to_string(),
            ))
        })?;

        Ok(home.join("windchill").join("config.toml"))
    }

    /// Create a default config file if it doesn't exist.
    /// The generated file contains a placeholder base URL the user must edit.
    pub fn create_default_config() -> Result<PathBuf> {
        let config_path = Self::config_path()?;

        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        if !config_path.exists() {
            let scaffold = format!("base_url = \"{}\"\n", BASE_URL_PLACEHOLDER);
            std::fs::write(&config_path, scaffold)?;
        }

        Ok(config_path)
    }
}
