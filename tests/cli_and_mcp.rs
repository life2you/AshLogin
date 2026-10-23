use serde_json::Value;
use std::{
    env, fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

fn write_executable(path: &Path, content: &str) {
    fs::write(path, content).expect("should write script");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("set executable permission");
}

#[test]
fn end_to_end_cli_and_mcp_execution() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let temp_root = env::temp_dir().join(format!("ashlogin-e2e-{unique}"));
    let bin_dir = temp_root.join("bin");
    let home_dir = temp_root.join("home");
    fs::create_dir_all(&bin_dir).expect("create bin_dir");
    fs::create_dir_all(home_dir.join(".ssh")).expect("create .ssh");

    write_executable(
        &bin_dir.join("ssh"),
        "#!/bin/sh\necho \"MOCK_SSH_ARGS: $*\"\necho \"MOCK_STDERR\" >&2\nexit 0\n",
    );

    write_executable(
        &bin_dir.join("ssh-keygen"),
        "#!/bin/sh\nhost=\"$2\"\nfile=\"$4\"\nif [ -f \"$file\" ] && grep -q \"$host\" \"$file\"; then\n  echo \"# Host $host found: line 1\"\n  echo \"$host ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI...\"\n  exit 0\nfi\nexit 1\n",
    );

    write_executable(
        &bin_dir.join("ssh-keyscan"),
        "#!/bin/sh\nfor last; do true; done\necho \"$last ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI...\"\nexit 0\n",
    );

    let cfg_path = temp_root.join("config.toml");
    fs::write(
        &cfg_path,
        r#"
[[servers]]
name = "staging"
aliases = ["s"]
host = "203.0.113.20"
user = "developer"
port = 22
ssh_options = ["ServerAliveInterval=30"]
"#,
    )
    .expect("write config.toml");

    let current_path = env::var("PATH").unwrap_or_default();
    let path_env = format!("{}:{current_path}", bin_dir.display());
    let bin = env!("CARGO_BIN_EXE_ashlogin");

    // 1. First run: untrusted host, auto-trust via --trust-host, JSON output
    let out1 = Command::new(bin)
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "staging",
            "--trust-host",
            "--connect-timeout",
            "5",
            "--batch",
            "--json",
            "--",
            "uname",
            "-a",
        ])
        .env("PATH", &path_env)
        .env("HOME", &home_dir)
        .output()
        .expect("run ashlogin cli 1");

    assert!(out1.status.success());
    let json1: Value = serde_json::from_slice(&out1.stdout).expect("valid json on first run");
    assert_eq!(json1["exit_code"], 0);
    assert_eq!(json1["server"], "staging");
    assert!(
        json1["stdout"]
            .as_str()
            .unwrap()
            .contains("-o ConnectTimeout=5 -o BatchMode=yes")
    );

    // 2. Second run: host is now in known_hosts (ssh-keygen -F outputs to its stdout), verify no stdout pollution
    let out2 = Command::new(bin)
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "staging",
            "--json",
            "--",
            "uptime",
        ])
        .env("PATH", &path_env)
        .env("HOME", &home_dir)
        .output()
        .expect("run ashlogin cli 2");

    assert!(out2.status.success());
    let json2: Value = serde_json::from_slice(&out2.stdout).expect("valid json on second run");
    assert_eq!(json2["exit_code"], 0);
    assert_eq!(json2["command"], serde_json::json!(["uptime"]));

    // 3. Third run: MCP run_ssh_command over stdio
    let mut child = Command::new(bin)
        .args(["mcp", "--config", cfg_path.to_str().unwrap()])
        .env("PATH", &path_env)
        .env("HOME", &home_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ashlogin mcp");

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"run_ssh_command","arguments":{{"server":"staging","command":["df","-h"],"connect_timeout":8,"trust_host":true}}}}}}"#
        )
        .expect("write request");
    }

    let mcp_out = child.wait_with_output().expect("wait mcp");
    assert!(mcp_out.status.success());
    let resp: Value = serde_json::from_slice(&mcp_out.stdout).expect("valid mcp json response");
    assert_eq!(resp["id"], 1);
    assert!(resp["result"]["isError"].is_null());
    let tool_text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("tool text");
    let tool_json: Value = serde_json::from_str(tool_text).expect("valid tool payload json");
    assert_eq!(tool_json["exit_code"], 0);
    assert!(
        tool_json["stdout"]
            .as_str()
            .unwrap()
            .contains("developer@203.0.113.20 df -h")
    );

    fs::remove_dir_all(temp_root).expect("cleanup temp_root");
}
