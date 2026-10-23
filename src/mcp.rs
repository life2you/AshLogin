use crate::config::{Config, ServerSummary, SshExecOptions};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, Write};

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallToolResult {
    pub content: Vec<ToolContent>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
}

fn error_tool_result(message: impl Into<String>) -> CallToolResult {
    CallToolResult {
        content: vec![ToolContent::Text {
            text: message.into(),
        }],
        is_error: Some(true),
    }
}

fn success_tool_result(text: impl Into<String>) -> CallToolResult {
    CallToolResult {
        content: vec![ToolContent::Text { text: text.into() }],
        is_error: None,
    }
}

pub fn get_tools_list() -> Value {
    serde_json::json!([
        {
            "name": "list_servers",
            "description": "List all configured SSH servers with sanitized summaries (passwords are never exposed)",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "preview_ssh_command",
            "description": "Preview sanitized SSH command for a configured server without executing it",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "server": {
                        "type": "string",
                        "description": "Server name or alias"
                    },
                    "command": {
                        "description": "Optional command to execute on remote server (string or array of strings)",
                        "oneOf": [
                            { "type": "string" },
                            { "type": "array", "items": { "type": "string" } }
                        ]
                    },
                    "connect_timeout": {
                        "type": "integer",
                        "description": "SSH connection timeout in seconds"
                    }
                },
                "required": ["server"]
            }
        },
        {
            "name": "run_ssh_command",
            "description": "Execute a command on a configured remote server via SSH and return output",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "server": {
                        "type": "string",
                        "description": "Server name or alias"
                    },
                    "command": {
                        "description": "Command to execute on remote server (string or array of strings)",
                        "oneOf": [
                            { "type": "string" },
                            { "type": "array", "items": { "type": "string" } }
                        ]
                    },
                    "connect_timeout": {
                        "type": "integer",
                        "description": "SSH connection timeout in seconds (default: 15)"
                    },
                    "trust_host": {
                        "type": "boolean",
                        "description": "Automatically trust and add unknown host key to known_hosts without interactive confirmation"
                    }
                },
                "required": ["server", "command"]
            }
        }
    ])
}

fn parse_command_arg(value: Option<&Value>) -> Result<Vec<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Ok(Vec::new())
            } else {
                Ok(vec![trimmed.to_string()])
            }
        }
        Some(Value::Array(arr)) => {
            let mut list = Vec::new();
            for item in arr {
                if let Some(s) = item.as_str() {
                    list.push(s.to_string());
                } else {
                    return Err("command array must contain only strings".to_string());
                }
            }
            if list.iter().all(|s| s.trim().is_empty()) {
                Ok(Vec::new())
            } else {
                Ok(list)
            }
        }
        _ => Err("command must be a string or array of strings".to_string()),
    }
}

fn handle_list_servers(config: &Config) -> CallToolResult {
    let summaries: Vec<ServerSummary> = config.servers.iter().map(ServerSummary::from).collect();
    match serde_json::to_string_pretty(&summaries) {
        Ok(text) => success_tool_result(text),
        Err(err) => error_tool_result(format!("failed to serialize servers: {err:#}")),
    }
}

fn handle_preview_ssh_command(config: &Config, arguments: &Value) -> CallToolResult {
    let server_name = match arguments.get("server").and_then(Value::as_str) {
        Some(name) if !name.trim().is_empty() => name.trim(),
        _ => return error_tool_result("missing or invalid 'server' argument"),
    };

    let server = match config.find_server(server_name) {
        Some(s) => s,
        None => {
            return error_tool_result(format!("server `{server_name}` not found in configuration"));
        }
    };

    let remote_command = match parse_command_arg(arguments.get("command")) {
        Ok(cmd) => cmd,
        Err(err) => return error_tool_result(err),
    };

    let connect_timeout = arguments.get("connect_timeout").and_then(Value::as_u64);

    let options = SshExecOptions {
        connect_timeout,
        batch_mode: false,
        remote_command,
    };

    match server.preview_command_with_options(&options) {
        Ok(preview) => {
            let payload = serde_json::json!({
                "server": server.name,
                "target": server.target(),
                "preview_command": preview,
                "remote_command": options.remote_command,
            });
            match serde_json::to_string_pretty(&payload) {
                Ok(text) => success_tool_result(text),
                Err(err) => error_tool_result(format!("failed to format preview: {err:#}")),
            }
        }
        Err(err) => error_tool_result(format!("failed to generate preview: {err:#}")),
    }
}

fn handle_run_ssh_command(config: &Config, arguments: &Value) -> CallToolResult {
    let server_name = match arguments.get("server").and_then(Value::as_str) {
        Some(name) if !name.trim().is_empty() => name.trim(),
        _ => return error_tool_result("missing or invalid 'server' argument"),
    };

    let server = match config.find_server(server_name) {
        Some(s) => s,
        None => {
            return error_tool_result(format!("server `{server_name}` not found in configuration"));
        }
    };

    let cmd_list = match parse_command_arg(arguments.get("command")) {
        Ok(cmd) => {
            if cmd.is_empty() {
                return error_tool_result("command cannot be empty");
            }
            cmd
        }
        Err(err) => return error_tool_result(err),
    };

    let connect_timeout = arguments
        .get("connect_timeout")
        .and_then(Value::as_u64)
        .unwrap_or(15);

    let trust_host = arguments
        .get("trust_host")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if server.uses_password_auth() && !crate::command_exists("sshpass") {
        return error_tool_result(format!(
            "server `{}` uses password auth, but `sshpass` was not found in PATH",
            server.name
        ));
    }

    let known_hosts_path = match crate::known_hosts_path() {
        Ok(path) => path,
        Err(err) => {
            return error_tool_result(format!("failed to determine known_hosts path: {err:#}"));
        }
    };

    let lookup = server.known_hosts_lookup();
    let is_known = match crate::host_exists_in_known_hosts(&lookup, &known_hosts_path) {
        Ok(known) => known,
        Err(err) => return error_tool_result(format!("failed to check known_hosts: {err:#}")),
    };

    if !is_known {
        if trust_host {
            if let Err(err) = crate::add_host_to_known_hosts(
                server,
                &known_hosts_path,
                Some(connect_timeout),
                true,
            ) {
                return error_tool_result(format!("failed to auto-trust host `{lookup}`: {err:#}"));
            }
        } else {
            return error_tool_result(format!(
                "host `{lookup}` is not present in {}. Set trust_host: true to automatically trust and record its host key.",
                known_hosts_path.display()
            ));
        }
    }

    let options = SshExecOptions {
        connect_timeout: Some(connect_timeout),
        batch_mode: server.password.is_none(),
        remote_command: cmd_list.clone(),
    };

    let mut command = match server.build_ssh_command_with_options(&options) {
        Ok(cmd) => cmd,
        Err(err) => return error_tool_result(format!("failed to build ssh command: {err:#}")),
    };

    command.stdin(std::process::Stdio::null());

    let output = match command.output() {
        Ok(out) => out,
        Err(err) => {
            return error_tool_result(format!(
                "failed to execute command on `{}`: {err:#}",
                server.name
            ));
        }
    };

    let exit_code = output.status.code().unwrap_or(1);
    let stdout_str = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr_str = String::from_utf8_lossy(&output.stderr).to_string();
    let is_error = exit_code != 0;

    let result_data = serde_json::json!({
        "server": server.name,
        "target": server.target(),
        "command": cmd_list,
        "exit_code": exit_code,
        "stdout": stdout_str,
        "stderr": stderr_str,
    });

    CallToolResult {
        content: vec![ToolContent::Text {
            text: serde_json::to_string_pretty(&result_data).unwrap_or_default(),
        }],
        is_error: if is_error { Some(true) } else { None },
    }
}

pub fn handle_message(payload: &str, config: &Config) -> Option<Value> {
    let req: JsonRpcRequest = match serde_json::from_str(payload) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("Failed to parse JSON-RPC request: {err:#}");
            return Some(serde_json::json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": {
                    "code": -32700,
                    "message": "Parse error"
                }
            }));
        }
    };

    let id = match req.id {
        Some(id) if !id.is_null() => id,
        _ => {
            // Notification: silent, no response
            return None;
        }
    };

    match req.method.as_str() {
        "initialize" => {
            let protocol_version = req
                .params
                .as_ref()
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .filter(|v| !v.trim().is_empty())
                .unwrap_or("2024-11-05");

            Some(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": protocol_version,
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "ashlogin",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }))
        }
        "ping" => Some(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {}
        })),
        "tools/list" => {
            let tools = get_tools_list();
            Some(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": tools
                }
            }))
        }
        "tools/call" => {
            let params = req.params.unwrap_or(Value::Null);
            let tool_name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

            let tool_result = match tool_name {
                "list_servers" => handle_list_servers(config),
                "preview_ssh_command" => handle_preview_ssh_command(config, &arguments),
                "run_ssh_command" => handle_run_ssh_command(config, &arguments),
                unknown => error_tool_result(format!("tool `{unknown}` not found")),
            };

            Some(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": tool_result
            }))
        }
        _ => Some(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": "Method not found"
            }
        })),
    }
}

pub fn run_mcp_server_io<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    config: Config,
) -> Result<()> {
    loop {
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let (payload, use_content_length) =
            if trimmed.to_ascii_lowercase().starts_with("content-length:") {
                let len_part = trimmed["content-length:".len()..].trim();
                let content_len: usize = len_part
                    .parse()
                    .with_context(|| format!("invalid Content-Length: {len_part}"))?;

                loop {
                    let mut header_line = String::new();
                    let h_bytes = reader.read_line(&mut header_line)?;
                    if h_bytes == 0 {
                        bail!("unexpected EOF while reading headers");
                    }
                    if header_line.trim().is_empty() {
                        break;
                    }
                }

                let mut buf = vec![0u8; content_len];
                reader.read_exact(&mut buf)?;
                (
                    String::from_utf8(buf).context("payload is not valid utf-8")?,
                    true,
                )
            } else {
                (trimmed.to_string(), false)
            };

        if let Some(response) = handle_message(&payload, &config) {
            let json = serde_json::to_string(&response)?;
            if use_content_length {
                write!(writer, "Content-Length: {}\r\n\r\n{}", json.len(), json)?;
            } else {
                writer.write_all(json.as_bytes())?;
                writer.write_all(b"\n")?;
            }
            writer.flush()?;
        }
    }
    Ok(())
}

pub fn run_mcp_server(config: Config) -> Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    run_mcp_server_io(stdin.lock(), stdout.lock(), config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Server;
    use std::io::Cursor;

    fn test_config() -> Config {
        Config {
            servers: vec![
                Server {
                    name: "prod".into(),
                    aliases: vec!["p".into()],
                    host: "203.0.113.10".into(),
                    user: "deploy".into(),
                    port: 22,
                    description: Some("Production".into()),
                    password: None,
                    identity_file: None,
                    ssh_options: vec!["StrictHostKeyChecking=yes".into()],
                },
                Server {
                    name: "staging".into(),
                    aliases: vec![],
                    host: "203.0.113.20".into(),
                    user: "root".into(),
                    port: 2222,
                    description: None,
                    password: Some("secret".into()),
                    identity_file: None,
                    ssh_options: vec![],
                },
            ],
        }
    }

    #[test]
    fn test_initialize_and_ping() {
        let config = test_config();

        let init_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05"
            }
        });
        let resp = handle_message(&init_req.to_string(), &config).expect("should respond");
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(resp["result"]["serverInfo"]["name"], "ashlogin");

        let ping_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "ping"
        });
        let resp = handle_message(&ping_req.to_string(), &config).expect("should respond");
        assert_eq!(resp["id"], 2);
        assert_eq!(resp["result"], serde_json::json!({}));
    }

    #[test]
    fn test_notification_silence() {
        let config = test_config();
        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        assert!(handle_message(&notif.to_string(), &config).is_none());
    }

    #[test]
    fn test_unknown_method() {
        let config = test_config();
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "foobar"
        });
        let resp = handle_message(&req.to_string(), &config).expect("should respond");
        assert_eq!(resp["id"], 42);
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn test_tools_list() {
        let config = test_config();
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "list-1",
            "method": "tools/list"
        });
        let resp = handle_message(&req.to_string(), &config).expect("should respond");
        assert_eq!(resp["id"], "list-1");
        let tools = resp["result"]["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 3);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            vec!["list_servers", "preview_ssh_command", "run_ssh_command"]
        );
    }

    #[test]
    fn test_tool_call_list_servers() {
        let config = test_config();
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "tools/call",
            "params": {
                "name": "list_servers",
                "arguments": {}
            }
        });
        let resp = handle_message(&req.to_string(), &config).expect("should respond");
        assert_eq!(resp["id"], 10);
        let content_text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content_text.contains("prod"));
        assert!(content_text.contains("staging"));
        assert!(!content_text.contains("secret"));
    }

    #[test]
    fn test_tool_call_preview_ssh_command() {
        let config = test_config();
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/call",
            "params": {
                "name": "preview_ssh_command",
                "arguments": {
                    "server": "prod",
                    "command": "uname -a",
                    "connect_timeout": 5
                }
            }
        });
        let resp = handle_message(&req.to_string(), &config).expect("should respond");
        assert_eq!(resp["id"], 11);
        let content_text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content_text.contains("deploy@203.0.113.10"));
        assert!(content_text.contains("ConnectTimeout=5"));
        assert!(content_text.contains("uname -a"));

        // array command
        let req_arr = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "tools/call",
            "params": {
                "name": "preview_ssh_command",
                "arguments": {
                    "server": "prod",
                    "command": ["echo", "hello world"]
                }
            }
        });
        let resp_arr = handle_message(&req_arr.to_string(), &config).expect("should respond");
        assert_eq!(resp_arr["id"], 12);
        let content_arr = resp_arr["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content_arr.contains("echo 'hello world'"));

        // missing server
        let req_err = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 13,
            "method": "tools/call",
            "params": {
                "name": "preview_ssh_command",
                "arguments": {
                    "server": "nonexistent"
                }
            }
        });
        let resp_err = handle_message(&req_err.to_string(), &config).expect("should respond");
        assert_eq!(resp_err["id"], 13);
        assert_eq!(resp_err["result"]["isError"], true);
    }

    #[test]
    fn test_tool_call_run_ssh_command_validations() {
        let config = test_config();

        // missing server
        let req1 = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "tools/call",
            "params": {
                "name": "run_ssh_command",
                "arguments": {
                    "server": "missing",
                    "command": "uptime"
                }
            }
        });
        let resp1 = handle_message(&req1.to_string(), &config).expect("should respond");
        assert_eq!(resp1["result"]["isError"], true);
        assert!(
            resp1["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("not found")
        );

        // empty command string
        let req2 = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "tools/call",
            "params": {
                "name": "run_ssh_command",
                "arguments": {
                    "server": "prod",
                    "command": "   "
                }
            }
        });
        let resp2 = handle_message(&req2.to_string(), &config).expect("should respond");
        assert_eq!(resp2["result"]["isError"], true);
        assert!(
            resp2["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("cannot be empty")
        );

        // untrusted host with trust_host = false
        let req3 = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "tools/call",
            "params": {
                "name": "run_ssh_command",
                "arguments": {
                    "server": "prod",
                    "command": "echo 1",
                    "trust_host": false
                }
            }
        });
        let resp3 = handle_message(&req3.to_string(), &config).expect("should respond");
        // Unless 203.0.113.10 happens to be in user's real known_hosts, it returns error or executes
        if resp3["result"]["isError"] == true {
            let text = resp3["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("trust_host") || text.contains("known_hosts"));
        }
    }

    #[test]
    fn test_run_mcp_server_io_line_delimited() {
        let config = test_config();
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "ping"
        });
        let input = format!("{}\n", req);
        let mut output = Vec::new();
        run_mcp_server_io(Cursor::new(input), &mut output, config).expect("run should succeed");

        let out_str = String::from_utf8(output).expect("utf-8 output");
        let parsed: Value = serde_json::from_str(&out_str).expect("json parsed");
        assert_eq!(parsed["id"], 99);
        assert_eq!(parsed["result"], serde_json::json!({}));
    }

    #[test]
    fn test_run_mcp_server_io_content_length() {
        let config = test_config();
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "ping"
        });
        let body = req.to_string();
        let input = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let mut output = Vec::new();
        run_mcp_server_io(Cursor::new(input), &mut output, config).expect("run should succeed");

        let out_str = String::from_utf8(output).expect("utf-8 output");
        assert!(out_str.starts_with("Content-Length: "));
        let (_, json_body) = out_str
            .split_once("\r\n\r\n")
            .expect("should contain header separator");
        let parsed: Value = serde_json::from_str(json_body).expect("json parsed");
        assert_eq!(parsed["id"], 100);
        assert_eq!(parsed["result"], serde_json::json!({}));
    }
}
