mod config;

use anyhow::{Context, Result, bail};
use clap::Parser;
use config::{Config, ConfigResolution, Server};
use dialoguer::{Confirm, Select, theme::ColorfulTheme};
use std::{
    fs::{self, OpenOptions},
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    process::{self, Command},
};

#[derive(Debug, Parser)]
#[command(
    name = "ashlogin",
    version,
    about = "Choose a configured host and hand off to system ssh"
)]
struct Cli {
    #[arg(
        value_name = "NAME",
        help = "Server name or alias from the config file"
    )]
    server: Option<String>,

    #[arg(long, value_name = "PATH", help = "Use a specific config file")]
    config: Option<PathBuf>,

    #[arg(long, help = "Print configured servers and exit")]
    list: bool,

    #[arg(long, help = "Print the final ssh command instead of executing it")]
    dry_run: bool,
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
        print_servers(&config);
        return Ok(0);
    }

    let server = match cli.server.as_deref() {
        Some(name) => config.get_server(name)?,
        None => select_server(&config)?,
    };

    if cli.dry_run {
        println!("{}", server.preview_command()?);
        return Ok(0);
    }

    ensure_runtime_requirements(server)?;
    ensure_known_host(server)?;
    launch_ssh(server)
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

fn launch_ssh(server: &Server) -> Result<i32> {
    let mut command = server.build_ssh_command()?;
    let status = command
        .status()
        .with_context(|| format!("failed to launch ssh for `{}`", server.name))?;
    Ok(status.code().unwrap_or(1))
}

fn ensure_runtime_requirements(server: &Server) -> Result<()> {
    if server.uses_password_auth() && !command_exists("sshpass") {
        bail!(
            "server `{}` uses password auth, but `sshpass` was not found in PATH",
            server.name
        );
    }

    Ok(())
}

fn command_exists(program: &str) -> bool {
    Command::new(program)
        .arg("-V")
        .output()
        .map(|output| output.status.success() || output.status.code().is_some())
        .unwrap_or(false)
}

fn ensure_known_host(server: &Server) -> Result<()> {
    let known_hosts_path = known_hosts_path()?;
    let lookup = server.known_hosts_lookup();

    if host_exists_in_known_hosts(&lookup, &known_hosts_path)? {
        return Ok(());
    }

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!(
            "host `{}` is not present in {}. run interactively once to confirm and save its host key",
            lookup,
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

    add_host_to_known_hosts(server, &known_hosts_path)?;
    Ok(())
}

fn known_hosts_path() -> Result<PathBuf> {
    let mut path = dirs::home_dir().context("could not determine the home directory")?;
    path.push(".ssh");
    path.push("known_hosts");
    Ok(path)
}

fn host_exists_in_known_hosts(host: &str, known_hosts_path: &Path) -> Result<bool> {
    if !known_hosts_path.exists() {
        return Ok(false);
    }

    let status = Command::new("ssh-keygen")
        .arg("-F")
        .arg(host)
        .arg("-f")
        .arg(known_hosts_path)
        .status()
        .with_context(|| {
            format!(
                "failed to query {} with ssh-keygen",
                known_hosts_path.display()
            )
        })?;

    Ok(status.success())
}

fn add_host_to_known_hosts(server: &Server, known_hosts_path: &Path) -> Result<()> {
    let parent = known_hosts_path
        .parent()
        .context("known_hosts path should have a parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;

    let mut scan = Command::new("ssh-keyscan");
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

    println!(
        "Saved host key for `{}` to {}.",
        server.known_hosts_lookup(),
        known_hosts_path.display()
    );
    Ok(())
}
