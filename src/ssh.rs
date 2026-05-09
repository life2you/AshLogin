use std::{
    io::{self, Read, Write},
    net::TcpStream,
    path::Path,
    process::Command,
    sync::mpsc,
    thread,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use crossterm::terminal;
use keyring::Entry;
use signal_hook::consts::signal::SIGWINCH;
use signal_hook::iterator::Signals;
use ssh2::Session;

use crate::config::{AuthType, ServerConfig};

const IO_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub fn connect(server: &ServerConfig) -> Result<()> {
    match server.auth_type {
        AuthType::Password => connect_with_password(server),
        AuthType::SshKey => connect_with_ssh_key(server),
    }
}

fn connect_with_password(server: &ServerConfig) -> Result<()> {
    let password = Entry::new(server.keychain_service(), &server.keychain_account())
        .context("failed to access macOS Keychain entry")?
        .get_password()
        .context("failed to read password from macOS Keychain")?;

    let tcp = TcpStream::connect((server.host.as_str(), server.port))
        .with_context(|| format!("failed to connect to {}:{}", server.host, server.port))?;

    let mut session = Session::new().context("failed to create ssh session")?;
    session.set_tcp_stream(tcp);
    session.handshake().context("ssh handshake failed")?;
    session
        .userauth_password(&server.username, &password)
        .context("ssh password authentication failed")?;

    if !session.authenticated() {
        bail!("ssh authentication did not complete");
    }

    run_interactive_shell(session)
}

fn connect_with_ssh_key(server: &ServerConfig) -> Result<()> {
    if let Some(private_key) = &server.private_key {
        let tcp = TcpStream::connect((server.host.as_str(), server.port))
            .with_context(|| format!("failed to connect to {}:{}", server.host, server.port))?;

        let mut session = Session::new().context("failed to create ssh session")?;
        session.set_tcp_stream(tcp);
        session.handshake().context("ssh handshake failed")?;
        session
            .userauth_pubkey_file(&server.username, None, Path::new(private_key), None)
            .context("ssh private key authentication failed")?;

        if !session.authenticated() {
            bail!("ssh authentication did not complete");
        }

        return run_interactive_shell(session);
    }

    let status = Command::new("ssh")
        .arg("-p")
        .arg(server.port.to_string())
        .arg(format!("{}@{}", server.username, server.host))
        .status()
        .context("failed to launch system ssh")?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("system ssh exited with status {}", status))
    }
}

fn run_interactive_shell(session: Session) -> Result<()> {
    let mut channel = session
        .channel_session()
        .context("failed to open ssh channel")?;
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    channel
        .request_pty(
            "xterm-256color",
            None,
            Some((cols as u32, rows as u32, 0, 0)),
        )
        .context("failed to request remote PTY")?;
    channel.shell().context("failed to open remote shell")?;
    session.set_blocking(false);

    let stdin_rx = spawn_stdin_reader();
    let resize_rx = spawn_resize_listener();
    let mut stdout = io::stdout();
    let mut remote_buf = [0_u8; 8192];

    loop {
        while let Ok(bytes) = stdin_rx.try_recv() {
            if bytes.is_empty() {
                channel.send_eof().ok();
                break;
            }
            write_all_nonblocking(&mut channel, &bytes)
                .context("failed to forward terminal input")?;
            channel.flush().ok();
        }

        while let Ok((cols, rows)) = resize_rx.try_recv() {
            channel
                .request_pty_size(cols as u32, rows as u32, None, None)
                .ok();
        }

        match channel.read(&mut remote_buf) {
            Ok(0) => {
                if channel.eof() {
                    break;
                }
            }
            Ok(read) => {
                stdout
                    .write_all(&remote_buf[..read])
                    .context("failed to write remote output")?;
                stdout.flush().ok();
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error).context("failed to read remote shell output"),
        }

        match channel.stderr().read(&mut remote_buf) {
            Ok(0) => {}
            Ok(read) => {
                io::stderr()
                    .write_all(&remote_buf[..read])
                    .context("failed to write remote stderr")?;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error).context("failed to read remote shell stderr"),
        }

        if channel.eof() {
            break;
        }

        thread::sleep(IO_POLL_INTERVAL);
    }

    channel.wait_close().ok();
    let exit_status = channel.exit_status().unwrap_or_default();
    if exit_status == 0 {
        Ok(())
    } else {
        Err(anyhow!("ssh session exited with status {}", exit_status))
    }
}

fn spawn_stdin_reader() -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut buf = [0_u8; 1024];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(Vec::new());
                    break;
                }
                Ok(read) => {
                    if tx.send(buf[..read].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => {
                    let _ = tx.send(Vec::new());
                    break;
                }
            }
        }
    });
    rx
}

fn spawn_resize_listener() -> mpsc::Receiver<(u16, u16)> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut signals = match Signals::new([SIGWINCH]) {
            Ok(signals) => signals,
            Err(_) => return,
        };
        for _signal in signals.forever() {
            let size = terminal::size().unwrap_or((80, 24));
            if tx.send(size).is_err() {
                break;
            }
        }
    });
    rx
}

fn write_all_nonblocking<W: Write>(writer: &mut W, mut buf: &[u8]) -> Result<()> {
    while !buf.is_empty() {
        match writer.write(buf) {
            Ok(0) => bail!("ssh channel closed while writing"),
            Ok(written) => buf = &buf[written..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(IO_POLL_INTERVAL);
            }
            Err(error) => return Err(error).context("ssh channel write failed"),
        }
    }
    Ok(())
}
