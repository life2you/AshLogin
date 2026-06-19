use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const CONFIG_FILE_NAME: &str = "config.toml";
const APP_CONFIG_DIR: &str = "ashlogin";
const XDG_CONFIG_DIR: &str = ".config";
const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../config.toml.example");

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub servers: Vec<Server>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Server {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub host: String,
    pub user: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub identity_file: Option<PathBuf>,
    #[serde(default)]
    pub ssh_options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigResolution {
    Ready(PathBuf),
    CreatedDefault(PathBuf),
}

fn default_port() -> u16 {
    22
}

impl Config {
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let config: Self = toml::from_str(&raw)
            .with_context(|| format!("failed to parse TOML in {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn get_server(&self, query: &str) -> Result<&Server> {
        self.find_server(query)
            .ok_or_else(|| anyhow!("server `{query}` not found"))
    }

    pub fn find_server(&self, query: &str) -> Option<&Server> {
        let query = normalize_lookup_key(query);
        self.servers.iter().find(|server| {
            normalize_lookup_key(&server.name) == query
                || server
                    .aliases
                    .iter()
                    .any(|alias| normalize_lookup_key(alias) == query)
        })
    }

    fn validate(&self) -> Result<()> {
        if self.servers.is_empty() {
            bail!("config does not define any [[servers]] entries");
        }

        let mut known_names = BTreeSet::new();

        for server in &self.servers {
            let name = server.name.trim();
            if name.is_empty() {
                bail!("server name cannot be empty");
            }

            if server.host.trim().is_empty() {
                bail!("server `{name}` is missing host");
            }

            if server.user.trim().is_empty() {
                bail!("server `{name}` is missing user");
            }

            if matches!(server.password.as_deref(), Some(password) if password.trim().is_empty()) {
                bail!("server `{name}` contains an empty password");
            }

            for candidate in std::iter::once(name).chain(server.aliases.iter().map(String::as_str))
            {
                let normalized = normalize_lookup_key(candidate);
                if normalized.is_empty() {
                    bail!("server `{name}` contains an empty alias");
                }

                if !known_names.insert(normalized) {
                    bail!("duplicate server name or alias `{candidate}`");
                }
            }
        }

        Ok(())
    }
}

impl Server {
    pub fn target(&self) -> String {
        format!("{}@{}", self.user.trim(), self.host.trim())
    }

    pub fn target_with_port(&self) -> String {
        if self.port == 22 {
            self.target()
        } else {
            format!("{}:{}", self.target(), self.port)
        }
    }

    pub fn known_hosts_lookup(&self) -> String {
        if self.port == 22 {
            self.host.trim().to_string()
        } else {
            format!("[{}]:{}", self.host.trim(), self.port)
        }
    }

    pub fn menu_label(&self) -> String {
        let mut label = format!("{:<18} {}", self.name.trim(), self.target_with_port());

        if let Some(description) = self.description_text() {
            label.push_str("  ");
            label.push_str(description);
        }

        if !self.aliases.is_empty() {
            label.push_str("  [");
            label.push_str(&self.aliases.join(", "));
            label.push(']');
        }

        label
    }

    pub fn list_line(&self) -> String {
        self.menu_label()
    }

    pub fn preview_command(&self) -> Result<String> {
        let mut parts = Vec::new();

        if self.password.is_some() {
            parts.push("SSHPASS=***".to_string());
            parts.push("sshpass".to_string());
            parts.push("-e".to_string());
        }

        parts.push("ssh".to_string());

        if self.port != 22 {
            parts.push("-p".to_string());
            parts.push(self.port.to_string());
        }

        if let Some(identity_file) = &self.identity_file {
            parts.push("-i".to_string());
            parts.push(shell_quote(
                &expand_user_path(identity_file)?.display().to_string(),
            ));
        }

        for option in &self.ssh_options {
            parts.push("-o".to_string());
            parts.push(shell_quote(option));
        }

        parts.push(shell_quote(&self.target()));
        Ok(parts.join(" "))
    }

    pub fn build_ssh_command(&self) -> Result<Command> {
        let mut command = if let Some(password) = &self.password {
            let mut command = Command::new("sshpass");
            command.arg("-e");
            command.env("SSHPASS", password);
            command.arg("ssh");
            command
        } else {
            Command::new("ssh")
        };

        if self.port != 22 {
            command.arg("-p").arg(self.port.to_string());
        }

        if let Some(identity_file) = &self.identity_file {
            command.arg("-i").arg(expand_user_path(identity_file)?);
        }

        for option in &self.ssh_options {
            command.arg("-o").arg(option);
        }

        command.arg(self.target());
        Ok(command)
    }

    pub fn uses_password_auth(&self) -> bool {
        self.password.is_some()
    }

    fn description_text(&self) -> Option<&str> {
        self.description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

pub fn resolve_config_path(explicit: Option<PathBuf>) -> Result<ConfigResolution> {
    if let Some(path) = explicit {
        return ensure_existing_file(path, "--config").map(ConfigResolution::Ready);
    }

    if let Ok(path) = env::var("ASHLOGIN_CONFIG") {
        return ensure_existing_file(PathBuf::from(path), "ASHLOGIN_CONFIG")
            .map(ConfigResolution::Ready);
    }

    let home_config = default_config_path()?;
    if home_config.is_file() {
        return Ok(ConfigResolution::Ready(home_config));
    }

    create_default_config(&home_config)?;
    Ok(ConfigResolution::CreatedDefault(home_config))
}

fn ensure_existing_file(path: PathBuf, source: &str) -> Result<PathBuf> {
    let expanded = expand_user_path(&path)?;
    if expanded.is_file() {
        return Ok(expanded);
    }

    bail!("{source} points to a missing file: {}", expanded.display())
}

fn default_config_path() -> Result<PathBuf> {
    let mut path = dirs::home_dir().context("could not determine the home directory")?;
    path.push(XDG_CONFIG_DIR);
    path.push(APP_CONFIG_DIR);
    path.push(CONFIG_FILE_NAME);
    Ok(path)
}

fn create_default_config(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .context("default config path should have a parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    fs::write(path, DEFAULT_CONFIG_TEMPLATE)
        .with_context(|| format!("failed to write default config file {}", path.display()))?;
    Ok(())
}

fn normalize_lookup_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn expand_user_path(path: &Path) -> Result<PathBuf> {
    let raw = path.to_string_lossy();

    if raw == "~" {
        return dirs::home_dir().context("could not expand `~` to the home directory");
    }

    if let Some(suffix) = raw.strip_prefix("~/") {
        return Ok(dirs::home_dir()
            .context("could not expand `~` to the home directory")?
            .join(suffix));
    }

    Ok(path.to_path_buf())
}

fn shell_quote(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"-_./:=@".contains(&byte))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigResolution, Server};
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn parses_example_like_config() {
        let input = r#"
            [[servers]]
            name = "prod"
            aliases = ["p"]
            host = "203.0.113.10"
            user = "deploy"
            port = 22
            description = "Main production host"
            password = "secret"
            identity_file = "~/.ssh/id_ed25519"
            ssh_options = ["IdentitiesOnly=yes"]
        "#;

        let config: Config = toml::from_str(input).expect("config should parse");
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].name, "prod");
        assert_eq!(config.servers[0].aliases, vec!["p"]);
        assert_eq!(config.servers[0].host, "203.0.113.10");
        assert_eq!(config.servers[0].password.as_deref(), Some("secret"));
    }

    #[test]
    fn finds_server_by_name_or_alias_case_insensitively() {
        let config = Config {
            servers: vec![Server {
                name: "Prod".into(),
                aliases: vec!["p".into()],
                host: "203.0.113.10".into(),
                user: "deploy".into(),
                port: 22,
                description: None,
                password: None,
                identity_file: None,
                ssh_options: Vec::new(),
            }],
        };

        assert!(config.find_server("prod").is_some());
        assert!(config.find_server("P").is_some());
        assert!(config.find_server("missing").is_none());
    }

    #[test]
    fn renders_preview_command() {
        let server = Server {
            name: "prod".into(),
            aliases: Vec::new(),
            host: "203.0.113.10".into(),
            user: "deploy".into(),
            port: 2222,
            description: None,
            password: Some("super-secret".into()),
            identity_file: Some("~/.ssh/id_ed25519".into()),
            ssh_options: vec!["IdentitiesOnly=yes".into()],
        };

        let preview = server.preview_command().expect("preview should build");
        assert!(preview.contains("SSHPASS=*** sshpass -e ssh"));
        assert!(preview.contains("ssh -p 2222"));
        assert!(preview.contains("-i"));
        assert!(preview.contains("deploy@203.0.113.10"));
        assert!(!preview.contains("super-secret"));
    }

    #[test]
    fn builds_sshpass_command_when_password_is_present() {
        let server = Server {
            name: "prod".into(),
            aliases: Vec::new(),
            host: "203.0.113.10".into(),
            user: "deploy".into(),
            port: 22,
            description: None,
            password: Some("super-secret".into()),
            identity_file: None,
            ssh_options: Vec::new(),
        };

        let command = server
            .build_ssh_command()
            .expect("sshpass command should build");

        assert_eq!(command.get_program(), std::ffi::OsStr::new("sshpass"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![
                std::ffi::OsStr::new("-e"),
                std::ffi::OsStr::new("ssh"),
                std::ffi::OsStr::new("deploy@203.0.113.10"),
            ]
        );
        assert_eq!(
            command.get_envs().collect::<Vec<_>>(),
            vec![(
                std::ffi::OsStr::new("SSHPASS"),
                Some(std::ffi::OsStr::new("super-secret"))
            )]
        );
    }

    #[test]
    fn known_hosts_lookup_uses_bracket_notation_for_custom_port() {
        let server = Server {
            name: "prod".into(),
            aliases: Vec::new(),
            host: "203.0.113.10".into(),
            user: "deploy".into(),
            port: 2200,
            description: None,
            password: None,
            identity_file: None,
            ssh_options: Vec::new(),
        };

        assert_eq!(server.known_hosts_lookup(), "[203.0.113.10]:2200");
    }

    #[test]
    fn builds_default_config_path_under_dot_config() {
        let path = super::default_config_path().expect("default config path should build");
        let path_text = path.display().to_string();

        assert!(path_text.contains(".config/ashlogin/config.toml"));
    }

    #[test]
    fn creates_default_config_file_with_template() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let temp_root = std::env::temp_dir().join(format!("ashlogin-test-{unique}"));
        let config_path = temp_root.join(".config/ashlogin/config.toml");

        super::create_default_config(&config_path).expect("default config should be created");

        let written = fs::read_to_string(&config_path).expect("config file should exist");
        assert_eq!(written, super::DEFAULT_CONFIG_TEMPLATE);

        fs::remove_dir_all(temp_root).expect("temp config directory should be removable");
    }

    #[test]
    fn explicit_path_resolution_requires_existing_file() {
        let result = super::resolve_config_path(Some("/definitely/missing/config.toml".into()));
        assert!(result.is_err());
    }

    #[test]
    fn created_default_variant_exposes_path_shape() {
        let resolution = ConfigResolution::CreatedDefault(PathBuf::from(
            "/tmp/example/.config/ashlogin/config.toml",
        ));

        match resolution {
            ConfigResolution::CreatedDefault(path) => {
                assert!(path.ends_with("ashlogin/config.toml"));
            }
            ConfigResolution::Ready(_) => panic!("expected created default variant"),
        }
    }
}
