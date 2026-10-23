pub mod config;
pub mod mcp;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use config::{Config, ConfigResolution, Server, ServerSummary, SshExecOptions};
use dialoguer::{Confirm, Select, theme::ColorfulTheme};
use std::{
    fs::{self, OpenOptions},
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    process::{self, Command},
};

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Subcommands {
    #[command(about = "Start MCP server on stdio")]
    Mcp,
}

#[derive(Debug, Parser, PartialEq, Eq)]
#[command(
    name = "ashlogin",
    version,
    about = "Choose a configured host and hand off to system ssh"
)]
pub struct Cli {
    #[command(subcommand)]
    pub subcommand: Option<Subcommands>,

    #[arg(
        value_name = "NAME",
        help = "Server name or alias from the config file"
    )]
    pub server: Option<String>,

    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "Use a specific config file"
    )]
    pub config: Option<PathBuf>,

    #[arg(long, help = "Print configured servers and exit")]
    pub list: bool,

    #[arg(long, help = "Print the final ssh command instead of executing it")]
    pub dry_run: bool,

    #[arg(long, help = "Output as JSON")]
    pub json: bool,

    #[arg(long, help = "Trust and add unknown host key non-interactively")]
    pub trust_host: bool,

    #[arg(long, value_name = "SECONDS", help = "SSH connect timeout in seconds")]
    pub connect_timeout: Option<u64>,

    #[arg(long, help = "Enable non-interactive SSH BatchMode")]
    pub batch: bool,

    #[arg(long, help = "Start MCP server")]
    pub mcp: bool,

    #[arg(
        last = true,
        value_name = "COMMAND",
        help = "Command and arguments to execute on the remote server"
    )]
    pub command: Vec<String>,
}

impl Cli {
    pub fn is_mcp(&self) -> bool {
        self.mcp || matches!(self.subcommand, Some(Subcommands::Mcp))
    }
}

fn main() {
    match run() {
        Ok(code) => process::exit(code),
        Err(error) => {
            eprintln!("error: {error:#}");
            process::exit(1);
        }
    }
}

fn run() -> Result<i32> {
    let cli = Cli::parse();

    if cli.is_mcp() {
        let config_path = match config::resolve_config_path(cli.config)? {
            ConfigResolution::Ready(path) => path,
            ConfigResolution::CreatedDefault(path) => {
                eprintln!(
                    "Created a default config at {}.\nEdit that file with your servers.",
                    path.display()
                );
                path
            }
        };
        let config = Config::load_from_path(&config_path)
            .with_context(|| format!("failed to load {}", config_path.display()))?;
        mcp::run_mcp_server(config)?;
        return Ok(0);
    }

    if cli.json && !cli.list && !cli.dry_run && cli.command.is_empty() {
        bail!("--json requires --list, --dry-run, or a remote command (-- <COMMAND>...)");
    }

    let config_path = match config::resolve_config_path(cli.config)? {
        ConfigResolution::Ready(path) => path,
        ConfigResolution::CreatedDefault(path) => {
            println!(
                "Created a default config at {}.\nEdit that file with your servers, then run ashlogin again.",
                path.display()
            );
            return Ok(0);
        }
    };
    let config = Config::load_from_path(&config_path)
        .with_context(|| format!("failed to load {}", config_path.display()))?;

    if cli.list {
        if cli.json {
            let summaries: Vec<ServerSummary> =
                config.servers.iter().map(ServerSummary::from).collect();
            let json = serde_json::to_string_pretty(&summaries)?;
            println!("{json}");
        } else {
            print_servers(&config);
        }
        return Ok(0);
    }

    let server = match cli.server.as_deref() {
        Some(name) => config.get_server(name)?,
        None => select_server(&config)?,
    };

    if cli.batch && server.uses_password_auth() {
        eprintln!(
            "warning: --batch is ignored for password-authenticated server `{}`",
            server.name
        );
    }

    let exec_options = SshExecOptions {
        connect_timeout: cli.connect_timeout,
        batch_mode: cli.batch,
        remote_command: cli.command.clone(),
    };

    if cli.dry_run {
        let preview = server.preview_command_with_options(&exec_options)?;
        if cli.json {
            let output = serde_json::json!({
                "server": server.name,
                "target": server.target(),
                "port": server.port,
                "uses_password": server.uses_password_auth(),
                "preview_command": preview,
                "remote_command": cli.command,
            });
            let json = serde_json::to_string_pretty(&output)?;
            println!("{json}");
        } else {
            println!("{preview}");
        }
        return Ok(0);
    }

    ensure_runtime_requirements(server)?;
    ensure_known_host(server, cli.trust_host, cli.connect_timeout, cli.json)?;

    if !cli.command.is_empty() {
        let mut command = server.build_ssh_command_with_options(&exec_options)?;
        if cli.json {
            command.stdin(std::process::Stdio::null());
            let output = command.output().with_context(|| {
                format!("failed to execute remote command on `{}`", server.name)
            })?;
            let exit_code = output.status.code().unwrap_or(1);
            let stdout_str = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr_str = String::from_utf8_lossy(&output.stderr).to_string();

            let result = serde_json::json!({
                "server": server.name,
                "target": server.target(),
                "command": cli.command,
                "exit_code": exit_code,
                "stdout": stdout_str,
                "stderr": stderr_str,
            });
            let json = serde_json::to_string_pretty(&result)?;
            println!("{json}");
            return Ok(exit_code);
        }

        let status = command
            .status()
            .with_context(|| format!("failed to execute remote command on `{}`", server.name))?;
        return Ok(status.code().unwrap_or(1));
    }

    launch_ssh(server, &exec_options)
}

fn print_servers(config: &Config) {
    for server in &config.servers {
        println!("{}", server.list_line());
    }
}

fn select_server(config: &Config) -> Result<&Server> {
    if config.servers.len() == 1 {
        return Ok(&config.servers[0]);
    }

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!(
            "multiple servers are configured. use `ashlogin --list` or `ashlogin <name>` in non-interactive environments"
        );
    }

    let labels = config
        .servers
        .iter()
        .map(Server::menu_label)
        .collect::<Vec<_>>();

    let selected = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select a server")
        .items(&labels)
        .default(0)
        .interact()
        .context("server selection aborted")?;

    Ok(&config.servers[selected])
}

fn launch_ssh(server: &Server, options: &SshExecOptions) -> Result<i32> {
    let mut command = server.build_ssh_command_with_options(options)?;
    let status = command
        .status()
        .with_context(|| format!("failed to launch ssh for `{}`", server.name))?;
    Ok(status.code().unwrap_or(1))
}

pub fn ensure_runtime_requirements(server: &Server) -> Result<()> {
    if server.uses_password_auth() && !command_exists("sshpass") {
        bail!(
            "server `{}` uses password auth, but `sshpass` was not found in PATH",
            server.name
        );
    }

    Ok(())
}

pub fn command_exists(program: &str) -> bool {
    Command::new(program)
        .arg("-V")
        .output()
        .map(|output| output.status.success() || output.status.code().is_some())
        .unwrap_or(false)
}

pub fn ensure_known_host(
    server: &Server,
    trust_host: bool,
    connect_timeout: Option<u64>,
    quiet: bool,
) -> Result<()> {
    let known_hosts_path = known_hosts_path()?;
    let lookup = server.known_hosts_lookup();

    if host_exists_in_known_hosts(&lookup, &known_hosts_path)? {
        return Ok(());
    }

    if trust_host {
        add_host_to_known_hosts(server, &known_hosts_path, connect_timeout, quiet)?;
        return Ok(());
    }

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!(
            "host `{lookup}` is not present in {}. run interactively once to confirm and save its host key or use --trust-host",
            known_hosts_path.display()
        );
    }

    let confirmed = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!(
            "Host `{}` is not trusted yet. Fetch its SSH host key and save it to {}?",
            lookup,
            known_hosts_path.display()
        ))
        .default(true)
        .interact()
        .context("host key confirmation aborted")?;

    if !confirmed {
        bail!("aborted because host `{lookup}` is not trusted yet");
    }

    add_host_to_known_hosts(server, &known_hosts_path, connect_timeout, quiet)?;
    Ok(())
}

pub fn known_hosts_path() -> Result<PathBuf> {
    let mut path = dirs::home_dir().context("could not determine the home directory")?;
    path.push(".ssh");
    path.push("known_hosts");
    Ok(path)
}

pub fn host_exists_in_known_hosts(host: &str, known_hosts_path: &Path) -> Result<bool> {
    if !known_hosts_path.exists() {
        return Ok(false);
    }

    let output = Command::new("ssh-keygen")
        .arg("-F")
        .arg(host)
        .arg("-f")
        .arg(known_hosts_path)
        .output()
        .with_context(|| {
            format!(
                "failed to query {} with ssh-keygen",
                known_hosts_path.display()
            )
        })?;

    Ok(output.status.success())
}

pub fn add_host_to_known_hosts(
    server: &Server,
    known_hosts_path: &Path,
    connect_timeout: Option<u64>,
    quiet: bool,
) -> Result<()> {
    let parent = known_hosts_path
        .parent()
        .context("known_hosts path should have a parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;

    let mut scan = Command::new("ssh-keyscan");
    if let Some(timeout) = connect_timeout {
        scan.arg("-T").arg(timeout.to_string());
    }
    scan.arg("-H");
    if server.port != 22 {
        scan.arg("-p").arg(server.port.to_string());
    }
    scan.arg(server.host.trim());

    let output = scan
        .output()
        .with_context(|| format!("failed to run ssh-keyscan for `{}`", server.host.trim()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "failed to fetch host key for `{}`: {}",
            server.host.trim(),
            stderr.trim()
        );
    }

    if output.stdout.is_empty() {
        bail!(
            "ssh-keyscan returned no host key data for `{}`",
            server.host.trim()
        );
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(known_hosts_path)
        .with_context(|| format!("failed to open {}", known_hosts_path.display()))?;

    use std::io::Write;
    file.write_all(&output.stdout)
        .with_context(|| format!("failed to append to {}", known_hosts_path.display()))?;

    if quiet {
        eprintln!(
            "Saved host key for `{}` to {}.",
            server.known_hosts_lookup(),
            known_hosts_path.display()
        );
    } else {
        println!(
            "Saved host key for `{}` to {}.",
            server.known_hosts_lookup(),
            known_hosts_path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parsing_no_args() {
        let cli = Cli::try_parse_from(["ashlogin"]).expect("should parse");
        assert_eq!(cli.server, None);
        assert!(cli.command.is_empty());
        assert!(!cli.is_mcp());
        assert!(!cli.list);
        assert!(!cli.dry_run);
        assert!(!cli.json);
        assert!(!cli.trust_host);
        assert_eq!(cli.connect_timeout, None);
        assert!(!cli.batch);
    }

    #[test]
    fn test_cli_parsing_server_only() {
        let cli = Cli::try_parse_from(["ashlogin", "prod"]).expect("should parse");
        assert_eq!(cli.server.as_deref(), Some("prod"));
        assert!(cli.command.is_empty());
        assert!(!cli.is_mcp());
    }

    #[test]
    fn test_cli_parsing_server_with_remote_command() {
        let cli =
            Cli::try_parse_from(["ashlogin", "prod", "--", "uname", "-a"]).expect("should parse");
        assert_eq!(cli.server.as_deref(), Some("prod"));
        assert_eq!(cli.command, vec!["uname", "-a"]);
        assert!(!cli.is_mcp());
    }

    #[test]
    fn test_cli_parsing_mcp_subcommand() {
        let cli = Cli::try_parse_from(["ashlogin", "mcp"]).expect("should parse");
        assert!(cli.is_mcp());
        assert_eq!(cli.subcommand, Some(Subcommands::Mcp));
        assert_eq!(cli.server, None);
    }

    #[test]
    fn test_cli_parsing_mcp_flag() {
        let cli = Cli::try_parse_from(["ashlogin", "--mcp"]).expect("should parse");
        assert!(cli.is_mcp());
        assert!(cli.mcp);
    }

    #[test]
    fn test_cli_parsing_mcp_with_config() {
        let cli1 = Cli::try_parse_from(["ashlogin", "mcp", "--config", "myconfig.toml"])
            .expect("should parse");
        assert!(cli1.is_mcp());
        assert_eq!(cli1.config, Some(PathBuf::from("myconfig.toml")));

        let cli2 = Cli::try_parse_from(["ashlogin", "--config", "myconfig.toml", "mcp"])
            .expect("should parse");
        assert!(cli2.is_mcp());
        assert_eq!(cli2.config, Some(PathBuf::from("myconfig.toml")));
    }

    #[test]
    fn test_cli_parsing_all_flags() {
        let cli = Cli::try_parse_from([
            "ashlogin",
            "prod",
            "--json",
            "--trust-host",
            "--connect-timeout",
            "10",
            "--batch",
            "--",
            "whoami",
        ])
        .expect("should parse");

        assert_eq!(cli.server.as_deref(), Some("prod"));
        assert!(cli.json);
        assert!(cli.trust_host);
        assert_eq!(cli.connect_timeout, Some(10));
        assert!(cli.batch);
        assert_eq!(cli.command, vec!["whoami"]);
    }

    #[test]
    fn test_json_flag_requires_list_dry_run_or_remote_command() {
        // When --json is passed without --list, --dry-run, or remote command
        let cli = Cli::try_parse_from(["ashlogin", "prod", "--json"]).expect("should parse CLI");
        let requires_error = cli.json && !cli.list && !cli.dry_run && cli.command.is_empty();
        assert!(requires_error);

        // When remote command is present
        let cli = Cli::try_parse_from(["ashlogin", "prod", "--json", "--", "ls"]).unwrap();
        let requires_error = cli.json && !cli.list && !cli.dry_run && cli.command.is_empty();
        assert!(!requires_error);

        // When --list is present
        let cli = Cli::try_parse_from(["ashlogin", "--list", "--json"]).unwrap();
        let requires_error = cli.json && !cli.list && !cli.dry_run && cli.command.is_empty();
        assert!(!requires_error);

        // When --dry-run is present
        let cli = Cli::try_parse_from(["ashlogin", "prod", "--dry-run", "--json"]).unwrap();
        let requires_error = cli.json && !cli.list && !cli.dry_run && cli.command.is_empty();
        assert!(!requires_error);
    }
}
