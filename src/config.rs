use std::{env, fs, io::ErrorKind, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "ashlogin";

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub servers: Vec<ServerConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerConfig {
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    pub keychain_service: Option<String>,
    pub keychain_account: Option<String>,
    pub private_key: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthType {
    Password,
    SshKey,
}

fn default_port() -> u16 {
    22
}

impl Config {
    pub fn load() -> Result<(Self, PathBuf)> {
        let path = config_path()?;
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                bootstrap_config_if_missing(&path)?;
                fs::read_to_string(&path)
                    .with_context(|| format!("failed to read config file {}", path.display()))?
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read config file {}", path.display()));
            }
        };
        let config: Self = toml::from_str(&content)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;
        Ok((config, path))
    }

    pub fn save_to_path(&self, path: &PathBuf) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create config directory {}",
                    parent.to_string_lossy()
                )
            })?;
        }
        let content = toml::to_string_pretty(self).context("failed to render config toml")?;
        fs::write(path, content)
            .with_context(|| format!("failed to write config file {}", path.display()))?;
        Ok(())
    }
}

impl ServerConfig {
    pub fn keychain_service(&self) -> &str {
        self.keychain_service.as_deref().unwrap_or("AshLogin")
    }

    pub fn keychain_account(&self) -> String {
        self.keychain_account
            .clone()
            .unwrap_or_else(|| format!("{}@{}", self.username, self.name))
    }
}

pub fn default_config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("failed to resolve home directory")?;
    Ok(home.join(".config").join(APP_NAME).join("config.toml"))
}

fn config_path() -> Result<PathBuf> {
    if let Ok(path) = env::var("ASHLOGIN_CONFIG") {
        return Ok(PathBuf::from(path));
    }

    let cwd_path = env::current_dir()
        .context("failed to resolve current directory")?
        .join("config.toml");
    if cwd_path.exists() {
        return Ok(cwd_path);
    }

    default_config_path()
}

fn bootstrap_config_if_missing(path: &PathBuf) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create config directory {}",
                parent.to_string_lossy()
            )
        })?;
    }
    fs::write(path, default_config_template())
        .with_context(|| format!("failed to create template config {}", path.display()))?;
    Ok(())
}

fn default_config_template() -> &'static str {
    r#"# AshLogin config
# 运行 `ashlogin --conf` 进入配置 TUI。
"#
}
