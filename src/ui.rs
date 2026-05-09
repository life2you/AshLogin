use std::path::PathBuf;

use anyhow::{Result, bail};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use crate::config::{AuthType, Config, ServerConfig};

pub struct App {
    pub config: Config,
    pub status: String,
    pub should_quit: bool,
    selected: usize,
    mode: Mode,
}

enum Mode {
    List,
    Add(AddServerForm),
}

pub enum SubmitResult {
    None,
    Save(NewServerRequest),
    DeleteSelected,
}

pub struct NewServerRequest {
    pub server: ServerConfig,
    pub password: Option<String>,
}

struct AddServerForm {
    name: String,
    host: String,
    port: String,
    username: String,
    auth_type: AuthType,
    keychain_service: String,
    keychain_account: String,
    private_key: String,
    password: String,
    focus: usize,
}

impl Default for AddServerForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            port: "22".to_string(),
            username: String::new(),
            auth_type: AuthType::Password,
            keychain_service: String::new(),
            keychain_account: String::new(),
            private_key: String::new(),
            password: String::new(),
            focus: 0,
        }
    }
}

impl App {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            status: "配置模式: a 新增, d 删除, q 退出".to_string(),
            should_quit: false,
            selected: 0,
            mode: Mode::List,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Result<SubmitResult> {
        match &mut self.mode {
            Mode::List => self.on_list_key(key),
            Mode::Add(form) => {
                if matches!(key.code, KeyCode::Esc) {
                    self.cancel_add();
                    return Ok(SubmitResult::None);
                }
                form.on_key(key)
            }
        }
    }

    pub fn on_paste(&mut self, text: &str) {
        if let Mode::Add(form) = &mut self.mode {
            form.paste_current(text);
        }
    }

    pub fn add_server(&mut self, server: ServerConfig) {
        self.config.servers.push(server);
        self.selected = self.config.servers.len().saturating_sub(1);
    }

    pub fn remove_selected_server(&mut self) -> Option<ServerConfig> {
        if self.config.servers.is_empty() {
            return None;
        }
        let removed = self.config.servers.remove(self.selected);
        if self.selected >= self.config.servers.len() && !self.config.servers.is_empty() {
            self.selected = self.config.servers.len() - 1;
        } else if self.config.servers.is_empty() {
            self.selected = 0;
        }
        Some(removed)
    }

    pub fn complete_add(&mut self, message: String) {
        self.mode = Mode::List;
        self.status = message;
    }

    pub fn cancel_add(&mut self) {
        self.mode = Mode::List;
        self.status = "已取消新增账号".to_string();
    }

    fn on_list_key(&mut self, key: KeyEvent) -> Result<SubmitResult> {
        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                Ok(SubmitResult::None)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.config.servers.is_empty() {
                    self.selected = (self.selected + 1) % self.config.servers.len();
                }
                Ok(SubmitResult::None)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.config.servers.is_empty() {
                    self.selected = if self.selected == 0 {
                        self.config.servers.len() - 1
                    } else {
                        self.selected - 1
                    };
                }
                Ok(SubmitResult::None)
            }
            KeyCode::Char('a') => {
                self.mode = Mode::Add(AddServerForm::default());
                self.status = "新增账号: Tab 切换字段, Enter 下一项/提交, Esc 取消".to_string();
                Ok(SubmitResult::None)
            }
            KeyCode::Char('d') => Ok(SubmitResult::DeleteSelected),
            _ => Ok(SubmitResult::None),
        }
    }
}

impl AddServerForm {
    fn on_key(&mut self, key: KeyEvent) -> Result<SubmitResult> {
        let fields = self.fields();
        let last = fields.len().saturating_sub(1);
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                self.focus = (self.focus + 1).min(last);
                return Ok(SubmitResult::None);
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.focus = self.focus.saturating_sub(1);
                return Ok(SubmitResult::None);
            }
            KeyCode::Left | KeyCode::Right => {
                if fields[self.focus] == Field::AuthType {
                    self.toggle_auth_type();
                }
                return Ok(SubmitResult::None);
            }
            KeyCode::Enter => {
                if self.focus == last {
                    let request = self.build_request()?;
                    return Ok(SubmitResult::Save(request));
                }
                self.focus += 1;
                return Ok(SubmitResult::None);
            }
            KeyCode::Backspace => {
                self.edit_current(|s| {
                    s.pop();
                });
                return Ok(SubmitResult::None);
            }
            KeyCode::Char(' ') if fields[self.focus] == Field::AuthType => {
                self.toggle_auth_type();
                return Ok(SubmitResult::None);
            }
            KeyCode::Char(ch) => {
                if fields[self.focus] == Field::Port && !ch.is_ascii_digit() {
                    return Ok(SubmitResult::None);
                }
                self.edit_current(|s| s.push(ch));
                return Ok(SubmitResult::None);
            }
            _ => {}
        }
        Ok(SubmitResult::None)
    }

    fn build_request(&self) -> Result<NewServerRequest> {
        let name = required("name", &self.name)?;
        let host = required("host", &self.host)?;
        let username = required("username", &self.username)?;

        let port = if self.port.trim().is_empty() {
            22
        } else {
            self.port
                .trim()
                .parse::<u16>()
                .map_err(|_| anyhow::anyhow!("port 必须是 1-65535 的数字"))?
        };

        let mut request = NewServerRequest {
            server: ServerConfig {
                name,
                host,
                port,
                username,
                auth_type: self.auth_type,
                keychain_service: None,
                keychain_account: None,
                private_key: None,
            },
            password: None,
        };

        match self.auth_type {
            AuthType::Password => {
                let service = if self.keychain_service.trim().is_empty() {
                    "AshLogin".to_string()
                } else {
                    self.keychain_service.trim().to_string()
                };
                request.server.keychain_service = Some(service);

                let account = if self.keychain_account.trim().is_empty() {
                    self.derived_keychain_account()
                } else {
                    self.keychain_account.trim().to_string()
                };
                request.server.keychain_account = Some(account);

                let password = required("password", &self.password)?;
                request.password = Some(password);
            }
            AuthType::SshKey => {
                if !self.private_key.trim().is_empty() {
                    request.server.private_key = Some(PathBuf::from(self.private_key.trim()));
                }
            }
        }

        Ok(request)
    }

    fn toggle_auth_type(&mut self) {
        self.auth_type = match self.auth_type {
            AuthType::Password => AuthType::SshKey,
            AuthType::SshKey => AuthType::Password,
        };
        let max_focus = self.fields().len().saturating_sub(1);
        self.focus = self.focus.min(max_focus);
    }

    fn edit_current(&mut self, edit: impl FnOnce(&mut String)) {
        match self.fields()[self.focus] {
            Field::Name => edit(&mut self.name),
            Field::Host => edit(&mut self.host),
            Field::Port => edit(&mut self.port),
            Field::Username => edit(&mut self.username),
            Field::KeychainService => edit(&mut self.keychain_service),
            Field::KeychainAccount => edit(&mut self.keychain_account),
            Field::PrivateKey => edit(&mut self.private_key),
            Field::Password => edit(&mut self.password),
            Field::AuthType => {}
        }
    }

    fn paste_current(&mut self, text: &str) {
        let normalized = text.replace('\n', "").replace('\r', "");
        if normalized.is_empty() {
            return;
        }
        match self.fields()[self.focus] {
            Field::Name => self.name.push_str(&normalized),
            Field::Host => self.host.push_str(&normalized),
            Field::Port => {
                let digits: String = normalized
                    .chars()
                    .filter(|ch| ch.is_ascii_digit())
                    .collect();
                self.port.push_str(&digits);
            }
            Field::Username => self.username.push_str(&normalized),
            Field::KeychainService => self.keychain_service.push_str(&normalized),
            Field::KeychainAccount => self.keychain_account.push_str(&normalized),
            Field::PrivateKey => self.private_key.push_str(&normalized),
            Field::Password => self.password.push_str(&normalized),
            Field::AuthType => {}
        }
    }

    fn fields(&self) -> Vec<Field> {
        let mut fields = vec![
            Field::Name,
            Field::Host,
            Field::Port,
            Field::Username,
            Field::AuthType,
        ];
        match self.auth_type {
            AuthType::Password => {
                fields.push(Field::KeychainService);
                fields.push(Field::KeychainAccount);
                fields.push(Field::Password);
            }
            AuthType::SshKey => fields.push(Field::PrivateKey),
        }
        fields
    }

    fn derived_keychain_account(&self) -> String {
        let username = self.username.trim();
        let name = self.name.trim();
        if username.is_empty() || name.is_empty() {
            String::new()
        } else {
            format!("{username}@{name}")
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Host,
    Port,
    Username,
    AuthType,
    KeychainService,
    KeychainAccount,
    PrivateKey,
    Password,
}

fn required(label: &str, value: &str) -> Result<String> {
    if value.trim().is_empty() {
        bail!("{label} 不能为空");
    }
    Ok(value.trim().to_string())
}

pub fn render(frame: &mut Frame, app: &App) {
    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .split(frame.area());

    let title =
        Paragraph::new("AshLogin").block(Block::default().borders(Borders::ALL).title("Config"));
    frame.render_widget(title, layout[0]);

    render_servers(frame, app, layout[1]);

    let footer = Paragraph::new(app.status.as_str())
        .block(Block::default().borders(Borders::ALL).title("Status"));
    frame.render_widget(footer, layout[2]);

    if let Mode::Add(form) = &app.mode {
        render_add_dialog(frame, form);
    }
}

fn render_servers(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = if app.config.servers.is_empty() {
        vec![ListItem::new("还没有账号，按 a 新增")]
    } else {
        app.config
            .servers
            .iter()
            .map(|server| {
                let auth = match server.auth_type {
                    AuthType::Password => "password",
                    AuthType::SshKey => "ssh_key",
                };
                ListItem::new(format!(
                    "{}  {}@{}:{}  [{}]",
                    server.name, server.username, server.host, server.port, auth
                ))
            })
            .collect()
    };
    let selected = if app.config.servers.is_empty() {
        None
    } else {
        Some(app.selected)
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Servers"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    let mut state = ListState::default().with_selected(selected);
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_add_dialog(frame: &mut Frame, form: &AddServerForm) {
    let area = centered(frame.area(), 80, 14);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("新增账号 (Esc 取消)");
    frame.render_widget(block.clone(), area);

    let inner = block.inner(area);
    let fields = form.fields();
    let rows = Layout::vertical(vec![Constraint::Length(1); fields.len()]).split(inner);
    for (i, field) in fields.into_iter().enumerate() {
        let focused = i == form.focus;
        let prefix = if focused { "> " } else { "  " };
        let line = match field {
            Field::Name => format!("{prefix}name: {}", form.name),
            Field::Host => format!("{prefix}host: {}", form.host),
            Field::Port => format!("{prefix}port: {}", form.port),
            Field::Username => format!("{prefix}username: {}", form.username),
            Field::AuthType => format!(
                "{prefix}auth_type: {}",
                match form.auth_type {
                    AuthType::Password => "password",
                    AuthType::SshKey => "ssh_key",
                }
            ),
            Field::KeychainService => {
                let shown = if form.keychain_service.trim().is_empty() {
                    "AshLogin".to_string()
                } else {
                    form.keychain_service.clone()
                };
                format!("{prefix}keychain_service: {shown}")
            }
            Field::KeychainAccount => {
                let shown = if form.keychain_account.trim().is_empty() {
                    form.derived_keychain_account()
                } else {
                    form.keychain_account.clone()
                };
                format!("{prefix}keychain_account: {shown}")
            }
            Field::PrivateKey => format!("{prefix}private_key: {}", form.private_key),
            Field::Password => format!(
                "{prefix}password: {}",
                "*".repeat(form.password.chars().count())
            ),
        };
        let style = if focused {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        frame.render_widget(Paragraph::new(line).style(style), rows[i]);
    }
}

fn centered(area: Rect, width_percent: u16, height: u16) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)])
        .flex(ratatui::layout::Flex::Center)
        .split(area)[0];
    Layout::horizontal([Constraint::Percentage(width_percent)])
        .flex(ratatui::layout::Flex::Center)
        .split(vertical)[0]
}
