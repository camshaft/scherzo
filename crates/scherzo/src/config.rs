use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

/// Main configuration for the Scherzo runtime
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Server configuration
    #[serde(default)]
    pub server: ServerConfig,

    /// List of plugin paths to load at boot
    #[serde(default)]
    pub plugins: Vec<String>,

    /// Job storage configuration
    #[serde(default)]
    pub jobs: JobsConfig,
}

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Port to bind the server to
    #[serde(default = "default_port")]
    pub port: u16,

    /// Hostname/address to bind to
    #[serde(default = "default_host")]
    pub host: String,

    /// Authentication configuration
    pub auth: Option<AuthConfig>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
            auth: None,
        }
    }
}

/// Authentication configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Username for basic auth
    pub username: String,

    /// Password hash (bcrypt) for basic auth
    pub password_hash: String,
}

/// Jobs configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobsConfig {
    /// Directory to store uploaded jobs
    #[serde(default = "default_jobs_dir")]
    pub storage_dir: String,

    /// Maximum job size in bytes (default 100MB)
    #[serde(default = "default_max_job_size")]
    pub max_size_bytes: u64,
}

impl Default for JobsConfig {
    fn default() -> Self {
        Self {
            storage_dir: default_jobs_dir(),
            max_size_bytes: default_max_job_size(),
        }
    }
}

fn default_port() -> u16 {
    3000
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_jobs_dir() -> String {
    "./jobs".to_string()
}

fn default_max_job_size() -> u64 {
    100 * 1024 * 1024 // 100MB
}

impl Config {
    /// Load configuration from a file, auto-detecting TOML or JSON format
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;

        // Try to determine format from extension
        let extension = path.extension().and_then(|s| s.to_str());

        match extension {
            Some("toml") => Self::from_toml(&content),
            Some("json") => Self::from_json(&content),
            _ => {
                // Try TOML first (preferred), fall back to JSON
                Self::from_toml(&content).or_else(|_| Self::from_json(&content))
            }
        }
    }

    /// Parse configuration from TOML string
    pub fn from_toml(content: &str) -> Result<Self> {
        toml::from_str(content).context("failed to parse config as TOML")
    }

    /// Parse configuration from JSON string
    pub fn from_json(content: &str) -> Result<Self> {
        serde_json::from_str(content).context("failed to parse config as JSON")
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Ensure storage directory is valid
        if self.jobs.storage_dir.is_empty() {
            anyhow::bail!("jobs.storage_dir cannot be empty");
        }

        // Validate auth if present
        if let Some(auth) = &self.server.auth {
            if auth.username.is_empty() {
                anyhow::bail!("server.auth.username cannot be empty");
            }
            if auth.password_hash.is_empty() {
                anyhow::bail!("server.auth.password_hash cannot be empty");
            }
        }

        Ok(())
    }
}

/// Helper function to hash a password with bcrypt
#[allow(dead_code)]
pub fn hash_password(password: &str) -> Result<String> {
    bcrypt::hash(password, bcrypt::DEFAULT_COST).context("failed to hash password")
}

/// Helper function to verify a password against a hash
pub fn verify_password(password: &str, hash: &str) -> bool {
    bcrypt::verify(password, hash).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_toml() {
        let toml = r#"
[server]
port = 8080
host = "0.0.0.0"

[server.auth]
username = "admin"
password_hash = "$2b$12$..."

plugins = ["/path/to/plugin.wasm"]

[jobs]
storage_dir = "/var/lib/scherzo/jobs"
max_size_bytes = 52428800
"#;

        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.host, "0.0.0.0");
    }

    #[test]
    fn test_parse_json() {
        let json = r#"{
            "server": {
                "port": 8080,
                "host": "0.0.0.0",
                "auth": {
                    "username": "admin",
                    "password_hash": "$2b$12$..."
                }
            },
            "plugins": ["/path/to/plugin.wasm"],
            "jobs": {
                "storage_dir": "/var/lib/scherzo/jobs",
                "max_size_bytes": 52428800
            }
        }"#;

        let config = Config::from_json(json).unwrap();
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.host, "0.0.0.0");
    }

    #[test]
    fn test_defaults() {
        let config = Config::from_toml("").unwrap();
        assert_eq!(config.server.port, 3000);
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.jobs.storage_dir, "./jobs");
    }

    #[test]
    fn test_password_hashing() {
        let password = "test123";
        let hash = hash_password(password).unwrap();
        assert!(verify_password(password, &hash));
        assert!(!verify_password("wrong", &hash));
    }
}
