mod config;
mod ssh;
mod ui;

use std::{
    env,
    io::{self, Write},
    path::PathBuf,
};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use keyring::Entry;
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    config::Config,
    ui::{App, NewServerRequest, SubmitResult},
};

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        print_version();
        return Ok(());
    }

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }

    if args.len() > 1 {
        anyhow::bail!("unexpected arguments: {}", args.join(" "));
    }

    if let Some(arg) = args.first() {
        if arg == "--conf" {
            let (config, config_path) = Config::load()?;
            return run_config_tui(config, config_path);
        }

        if arg.starts_with('-') {
            anyhow::bail!("unknown argument: {arg}");
        }

        let (config, _config_path) = Config::load()?;
        return run_login_by_name(config, arg);
    }

    let (config, _config_path) = Config::load()?;
    run_login_selector(config)
}

fn run_login_selector(config: Config) -> Result<()> {
    if config.servers.is_empty() {
        println!("No SSH accounts configured.");
        println!("Run `ashlogin --conf` to add one.");
        return Ok(());
    }

    println!("AshLogin accounts:");
    for (index, server) in config.servers.iter().enumerate() {
        let auth = match server.auth_type {
            config::AuthType::Password => "password",
            config::AuthType::SshKey => "ssh_key",
        };
        println!(
            "  {}. {}  {}@{}:{}  [{}]",
            index + 1,
            server.name,
            server.username,
            server.host,
            server.port,
            auth
        );
    }

    print!("Select account number: ");
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read selection")?;
    let index = input
        .trim()
        .parse::<usize>()
        .context("please enter a valid account number")?;

    let server = config
        .servers
        .get(index.saturating_sub(1))
        .context("selected account number is out of range")?;

    ssh::connect(server)
}

fn run_login_by_name(config: Config, name: &str) -> Result<()> {
    let server = config
        .servers
        .iter()
        .find(|server| server.name == name)
        .with_context(|| format!("no account named `{name}`"))?;

    ssh::connect(server)
}

fn run_config_tui(config: Config, config_path: PathBuf) -> Result<()> {
    let mut app = App::new(config);
    app.status = format!(
        "Loaded {}. 配置模式: a 新增, d 删除, q 退出",
        config_path.display()
    );

    let mut terminal = setup_terminal()?;
    let result = run_config_app(&mut terminal, &mut app, &config_path);
    restore_terminal(&mut terminal)?;
    result
}

fn run_config_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    config_path: &PathBuf,
) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| ui::render(frame, app))?;

        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match app.on_key(key)? {
                    SubmitResult::None => {}
                    SubmitResult::Save(request) => {
                        let message = match persist_new_server(app, config_path, request) {
                            Ok(label) => format!("{label} 已保存"),
                            Err(error) => format!("保存失败: {}", error),
                        };
                        app.complete_add(message);
                    }
                    SubmitResult::DeleteSelected => {
                        let message = match delete_selected_server(app, config_path) {
                            Ok(Some(label)) => format!("{label} 已删除"),
                            Ok(None) => "当前没有可删除的账号".to_string(),
                            Err(error) => format!("删除失败: {}", error),
                        };
                        app.status = message;
                    }
                }
            }
            Event::Paste(text) => app.on_paste(&text),
            _ => {}
        }
    }

    Ok(())
}

fn persist_new_server(
    app: &mut App,
    config_path: &PathBuf,
    request: NewServerRequest,
) -> Result<String> {
    if app
        .config
        .servers
        .iter()
        .any(|server| server.name == request.server.name)
    {
        anyhow::bail!("name 已存在: {}", request.server.name);
    }
    store_password_if_needed(&request)?;

    let label = request.server.name.clone();
    app.add_server(request.server);
    app.config.save_to_path(config_path)?;
    Ok(label)
}

fn delete_selected_server(app: &mut App, config_path: &PathBuf) -> Result<Option<String>> {
    let removed = match app.remove_selected_server() {
        Some(server) => server,
        None => return Ok(None),
    };
    let label = removed.name;
    app.config.save_to_path(config_path)?;
    Ok(Some(label))
}

fn store_password_if_needed(request: &NewServerRequest) -> Result<()> {
    if request.server.auth_type != config::AuthType::Password {
        return Ok(());
    }

    let password = request.password.as_deref().context("password 不能为空")?;
    let entry = Entry::new(
        request.server.keychain_service(),
        &request.server.keychain_account(),
    )
    .context("failed to access macOS Keychain entry")?;
    entry
        .set_password(password)
        .context("failed to save password to macOS Keychain")?;
    Ok(())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("failed to create terminal")
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn print_help() {
    println!("AshLogin {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Usage:");
    println!("  ashlogin           List accounts and log into one");
    println!("  ashlogin <name>    Log into the named account directly");
    println!("  ashlogin --conf    Open the TUI config manager");
    println!("  ashlogin --help    Show this help");
    println!("  ashlogin --version Show version");
    println!();
    println!("Notes:");
    println!("  - Password auth reads secrets from macOS Keychain");
    println!("  - Config file path defaults to ~/.config/ashlogin/config.toml");
}

fn print_version() {
    println!("ashlogin {}", env!("CARGO_PKG_VERSION"));
}
