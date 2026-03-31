//! Child MCP client — stdio subprocess and HTTP proxy transports.
//! Handles JSON-RPC id mapping and process lifecycle.

use crate::config::McpServerConfig;
use crate::protocol::ToolDef;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A connected child MCP server (stdio or HTTP).
pub enum ChildMcp {
    Stdio(StdioChild),
    Http(HttpChild),
}

/// Pending request awaiting response from child.
type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

pub struct StdioChild {
    tx: mpsc::Sender<String>,       // send JSON lines to child stdin
    pending: PendingMap,
    _child: Arc<Mutex<Child>>,       // kept alive; killed on drop
    _reader_handle: tokio::task::JoinHandle<()>,
}

pub struct HttpChild {
    url: String,
    headers: HashMap<String, String>,
    client: reqwest::Client,
}

impl ChildMcp {
    /// Connect to a child MCP server, perform initialize handshake.
    pub async fn connect(config: &McpServerConfig) -> Result<Self, String> {
        if config.is_http() {
            // TODO: SSE (Server-Sent Events) transport is declared in config schema but not yet
            // implemented. Configs with `"type": "sse"` currently fall through to the same
            // streamable-http POST path, which will likely fail. Full SSE support (persistent
            // event stream + request/response multiplexing) is tracked for a future release.
            if config.transport_type.as_deref() == Some("sse") {
                return Err("SSE transport is not yet implemented. Use \"streamable-http\" instead.".to_string());
            }
            let url = config.url.as_deref().ok_or("HTTP transport requires url")?;
            Ok(ChildMcp::Http(HttpChild {
                url: url.to_string(),
                headers: config.headers.clone().unwrap_or_default(),
                client: reqwest::Client::new(),
            }))
        } else {
            let cmd = config.command.as_deref().ok_or("stdio transport requires command")?;
            let mut child_proc = Command::new(cmd)
                .args(&config.args)
                .envs(&config.env)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("Failed to spawn {cmd}: {e}"))?;

            let child_stdin = child_proc.stdin.take().ok_or("No stdin")?;
            let child_stdout = child_proc.stdout.take().ok_or("No stdout")?;

            let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
            let pending_clone = pending.clone();

            // Stdin writer channel
            let (tx, mut rx) = mpsc::channel::<String>(64);

            // Writer task: drains channel → child stdin
            let mut writer = child_stdin;
            tokio::spawn(async move {
                while let Some(line) = rx.recv().await {
                    if writer.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                    if writer.write_all(b"\n").await.is_err() {
                        break;
                    }
                    let _ = writer.flush().await;
                }
            });

            // Reader task: child stdout → resolve pending requests
            let reader_handle = tokio::spawn(async move {
                let mut reader = BufReader::new(child_stdout);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) | Err(_) => break, // EOF or error
                        Ok(_) => {}
                    }
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(val) = serde_json::from_str::<Value>(trimmed)
                        && let Some(id) = val.get("id").and_then(|v| v.as_u64()) {
                            let mut map = pending_clone.lock().await;
                            if let Some(sender) = map.remove(&id) {
                                let _ = sender.send(val);
                            }
                        }
                        // Notifications (no id) are silently dropped
                }
            });

            let child = Arc::new(Mutex::new(child_proc));

            let stdio = StdioChild {
                tx,
                pending,
                _child: child,
                _reader_handle: reader_handle,
            };

            // Initialize handshake
            stdio.send_request("initialize", serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "omc-hub-rs", "version": "0.1.0" }
            })).await.map_err(|e| format!("Initialize handshake failed: {e}"))?;

            // Send initialized notification (no response expected)
            let notif = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            });
            let _ = stdio.tx.send(serde_json::to_string(&notif).unwrap()).await;

            Ok(ChildMcp::Stdio(stdio))
        }
    }

    /// List tools from child MCP.
    pub async fn list_tools(&self) -> Result<Vec<ToolDef>, String> {
        match self {
            ChildMcp::Stdio(s) => {
                let resp = s.send_request("tools/list", serde_json::json!({})).await?;
                parse_tools_response(resp)
            }
            ChildMcp::Http(h) => {
                let resp = h.send_rpc("tools/list", serde_json::json!({})).await?;
                parse_tools_response(resp)
            }
        }
    }

    /// Call a tool on the child MCP.
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<Value, String> {
        let params = serde_json::json!({ "name": name, "arguments": args });
        match self {
            ChildMcp::Stdio(s) => s.send_request("tools/call", params).await,
            ChildMcp::Http(h) => h.send_rpc("tools/call", params).await,
        }
    }

    /// Shut down the child.
    pub async fn close(self) {
        match self {
            ChildMcp::Stdio(s) => {
                drop(s.tx); // close stdin → child should exit
                let mut child = s._child.lock().await;
                let _ = child.kill().await;
            }
            ChildMcp::Http(_) => {} // nothing to close
        }
    }
}

impl StdioChild {
    async fn send_request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let (resp_tx, resp_rx) = oneshot::channel();
        {
            let mut map = self.pending.lock().await;
            map.insert(id, resp_tx);
        }
        self.tx
            .send(serde_json::to_string(&req).unwrap())
            .await
            .map_err(|_| "Child stdin closed".to_string())?;

        tokio::time::timeout(std::time::Duration::from_secs(30), resp_rx)
            .await
            .map_err(|_| "Child response timeout (30s)".to_string())?
            .map_err(|_| "Child response channel closed".to_string())
    }
}

impl HttpChild {
    async fn send_rpc(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut req = self.client.post(&self.url).json(&body);
        for (k, v) in &self.headers {
            req = req.header(k, v);
        }
        let resp = req.send().await.map_err(|e| format!("HTTP error: {e}"))?;
        let val: Value = resp.json().await.map_err(|e| format!("JSON parse error: {e}"))?;
        Ok(val)
    }
}

fn parse_tools_response(resp: Value) -> Result<Vec<ToolDef>, String> {
    let tools_val = resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .or_else(|| resp.get("tools"))
        .ok_or("No tools in response")?;
    serde_json::from_value(tools_val.clone()).map_err(|e| format!("Parse tools: {e}"))
}
