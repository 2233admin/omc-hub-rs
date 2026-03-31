//! omc-hub-rs — Lightweight MCP Hub
//! Replaces OMC's 663MB bun+haiku stack with <5MB Rust.

mod child;
mod config;
mod hub;
mod omc_tools;
mod protocol;
mod toolbox;

use hub::Hub;
use protocol::{JsonRpcRequest, JsonRpcResponse};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    // Capture stdin BEFORE tokio runtime starts (tokio can interfere on Windows).
    let (line_tx, line_rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let stdin = std::io::stdin();
        let reader = stdin.lock();
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    if l.trim().is_empty() {
                        continue;
                    }
                    if line_tx.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Now start tokio runtime
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async_main(line_rx));
}

async fn async_main(line_rx: std::sync::mpsc::Receiver<String>) {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "omc_hub=info".parse().unwrap()),
        )
        .init();

    let base_dir = resolve_base_dir();
    let state_dir = resolve_state_dir(&base_dir);
    tracing::info!(
        "omc-hub-rs starting, base_dir={}, state_dir={}",
        base_dir.display(),
        state_dir.display()
    );

    let mut hub = Hub::new(base_dir, state_dir).await;

    // Bridge std::sync::mpsc into tokio
    let (async_tx, mut async_rx) = tokio::sync::mpsc::channel::<String>(64);
    tokio::task::spawn_blocking(move || {
        while let Ok(line) = line_rx.recv() {
            if async_tx.blocking_send(line).is_err() {
                break;
            }
        }
    });

    // Signal handler
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn({
        let shutdown_tx = shutdown_tx.clone();
        async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = shutdown_tx.send(true);
        }
    });

    // Main loop
    loop {
        tokio::select! {
            line = async_rx.recv() => {
                match line {
                    Some(line) => {
                        let resp = handle_message(&line, &mut hub).await;
                        if let Some(resp) = resp {
                            let mut out = serde_json::to_string(&resp).unwrap();
                            out.push('\n');
                            use std::io::Write;
                            let stdout = std::io::stdout();
                            let mut lock = stdout.lock();
                            let _ = lock.write_all(out.as_bytes());
                            let _ = lock.flush();
                        }
                    }
                    None => break,
                }
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }

    hub.shutdown().await;
    tracing::info!("omc-hub-rs shutdown complete");
}

async fn handle_message(line: &str, hub: &mut Hub) -> Option<JsonRpcResponse> {
    let req: JsonRpcRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            return Some(JsonRpcResponse::error(
                None,
                -32700,
                format!("Parse error: {e}"),
            ));
        }
    };

    req.id.as_ref()?;

    let id = req.id.clone();

    match req.method.as_str() {
        "initialize" => Some(JsonRpcResponse::success(
            id,
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": { "listChanged": true }
                },
                "serverInfo": {
                    "name": "omc-hub-rs",
                    "version": "0.1.0"
                }
            }),
        )),

        "tools/list" => {
            let tools = hub.list_tools();
            Some(JsonRpcResponse::success(
                id,
                serde_json::json!({ "tools": tools }),
            ))
        }

        "tools/call" => {
            let name = req.params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = req
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));

            let gen_before = hub.tool_generation();
            let result = hub.call_tool(name, args).await;
            hub.flush_stats().await;

            let notify = hub.tools_changed_since(gen_before);
            let resp = JsonRpcResponse::success(id, serde_json::to_value(&result).unwrap());

            // Send tools/list_changed notification after response
            if notify {
                let notif = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/tools/list_changed"
                });
                let mut out = serde_json::to_string(&notif).unwrap();
                out.push('\n');
                use std::io::Write;
                let stdout = std::io::stdout();
                let mut lock = stdout.lock();
                let _ = lock.write_all(out.as_bytes());
                let _ = lock.flush();
            }

            Some(resp)
        }

        "ping" => Some(JsonRpcResponse::success(id, serde_json::json!({}))),

        _ => Some(JsonRpcResponse::error(
            id,
            -32601,
            format!("Method not found: {}", req.method),
        )),
    }
}

fn resolve_base_dir() -> PathBuf {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "--config"
            && let Some(path) = args.get(i + 1) {
                return PathBuf::from(path);
            }
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home).join(".omc").join("mcp-hub");
    }
    PathBuf::from(".omc/mcp-hub")
}

/// Resolve state directory for OMC native tools.
/// Priority: --state-dir arg > parent of --config > ~/.omc
fn resolve_state_dir(base_dir: &Path) -> PathBuf {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "--state-dir"
            && let Some(path) = args.get(i + 1) {
                return PathBuf::from(path);
            }
    }
    // Infer from base_dir: ~/.omc/mcp-hub → ~/.omc
    if let Some(parent) = base_dir.parent() {
        return parent.to_path_buf();
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home).join(".omc");
    }
    PathBuf::from(".omc")
}
