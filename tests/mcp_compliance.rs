//! MCP Protocol Compliance + Process Verification Tests
//!
//! Spawns omc-hub as a subprocess, sends JSON-RPC 2.0 messages via stdin,
//! validates responses against MCP spec (2024-11-05).

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

/// Persistent session that keeps a BufReader across multiple RPC calls.
struct McpSession {
    stdin: Option<ChildStdin>,
    reader: BufReader<ChildStdout>,
    child: Child,
}

impl McpSession {
    fn spawn() -> Self {
        let tmp = std::env::temp_dir().join(format!(
            "omc-hub-test-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(tmp.join("skills"));
        let _ = std::fs::create_dir_all(tmp.join("toolbox"));

        let binary = if cfg!(target_os = "windows") {
            "target/debug/omc-hub.exe"
        } else {
            "target/debug/omc-hub"
        };

        let mut child = Command::new(binary)
            .arg("--config")
            .arg(tmp.to_str().unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .spawn()
            .expect("Failed to spawn omc-hub");

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let reader = BufReader::new(stdout);

        Self { stdin: Some(stdin), reader, child }
    }

    /// Send a JSON-RPC request and read one response line.
    fn rpc(&mut self, req: Value) -> Value {
        let mut line = serde_json::to_string(&req).unwrap();
        line.push('\n');
        let stdin = self.stdin.as_mut().expect("stdin closed");
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.flush().unwrap();

        let mut buf = String::new();
        self.reader.read_line(&mut buf).unwrap();
        serde_json::from_str(buf.trim()).expect("valid JSON response")
    }

    /// Read the next line from stdout (for notifications).
    fn read_line(&mut self) -> Value {
        let mut buf = String::new();
        self.reader.read_line(&mut buf).unwrap();
        serde_json::from_str(buf.trim()).expect("valid JSON")
    }

    /// Send raw text (not necessarily valid JSON).
    fn send_raw(&mut self, text: &str) {
        let stdin = self.stdin.as_mut().expect("stdin closed");
        stdin.write_all(text.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    /// Initialize handshake (convenience).
    fn initialize(&mut self) -> Value {
        self.rpc(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0.0.1" }
            }
        }))
    }

}

impl Drop for McpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

// ═══════════════════════════════════════════════════
// #66: MCP Protocol Compliance Tests
// ═══════════════════════════════════════════════════

#[test]
fn test_initialize_response_format() {
    let mut s = McpSession::spawn();
    let resp = s.initialize();

    assert_eq!(resp["jsonrpc"], "2.0", "jsonrpc field must be '2.0'");
    assert_eq!(resp["id"], 1, "id must match request");
    assert!(resp.get("result").is_some(), "initialize must return result");
    assert!(resp.get("error").is_none(), "initialize must not return error");

    let result = &resp["result"];
    assert_eq!(result["protocolVersion"], "2024-11-05");
    assert!(result.get("capabilities").is_some(), "must include capabilities");

    let server_info = result.get("serverInfo").expect("serverInfo required");
    assert!(server_info.get("name").is_some(), "serverInfo must have name");
}

#[test]
fn test_tools_list_response_format() {
    let mut s = McpSession::spawn();
    s.initialize();

    let resp = s.rpc(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);
    assert!(resp.get("error").is_none());

    let tools = resp["result"]["tools"].as_array().expect("tools must be array");

    for tool in tools {
        assert!(tool.get("name").and_then(|n| n.as_str()).is_some(), "tool must have string name");
        assert!(tool.get("description").and_then(|d| d.as_str()).is_some(), "tool must have description");
        let schema = tool.get("inputSchema").expect("tool must have inputSchema");
        assert_eq!(schema["type"], "object", "inputSchema.type must be 'object'");
    }

    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"hub_load_skill"), "must have hub_load_skill");
    assert!(names.contains(&"hub_list_skills"), "must have hub_list_skills");
    assert!(names.contains(&"hub_stats"), "must have hub_stats");
    assert!(names.contains(&"state_read"), "must have state_read");
    assert!(names.contains(&"notepad_read"), "must have notepad_read");
}

#[test]
fn test_tools_call_success() {
    let mut s = McpSession::spawn();
    s.initialize();

    let resp = s.rpc(json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "hub_list_skills", "arguments": {} }
    }));

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 3);

    let content = resp["result"]["content"].as_array().expect("content must be array");
    assert!(!content.is_empty());
    assert_eq!(content[0]["type"], "text");
    assert!(content[0].get("text").is_some());
}

#[test]
fn test_tools_call_unknown_tool_returns_error_in_content() {
    let mut s = McpSession::spawn();
    s.initialize();

    let resp = s.rpc(json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": { "name": "nonexistent_tool_xyz", "arguments": {} }
    }));

    assert_eq!(resp["id"], 4);
    assert_eq!(resp["result"]["isError"], true, "unknown tool must set isError=true");
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Unknown tool"));
}

#[test]
fn test_ping_response() {
    let mut s = McpSession::spawn();
    let resp = s.rpc(json!({"jsonrpc":"2.0","id":5,"method":"ping","params":{}}));

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 5);
    assert!(resp.get("result").is_some());
    assert!(resp.get("error").is_none());
}

#[test]
fn test_unknown_method_returns_error() {
    let mut s = McpSession::spawn();
    let resp = s.rpc(json!({"jsonrpc":"2.0","id":6,"method":"nonexistent/method","params":{}}));

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 6);

    let err = resp.get("error").expect("unknown method must return error");
    assert_eq!(err["code"], -32601);
    assert!(err["message"].as_str().unwrap().contains("Method not found"));
}

#[test]
fn test_parse_error_returns_32700() {
    let mut s = McpSession::spawn();
    s.send_raw("this is not json");
    let resp = s.read_line();

    assert_eq!(resp["jsonrpc"], "2.0");
    let err = resp.get("error").expect("parse error must return error");
    assert_eq!(err["code"], -32700);
}

#[test]
fn test_notification_no_response() {
    let mut s = McpSession::spawn();
    s.initialize();

    // Send notification (no id) — should NOT get a response
    s.send_raw(&serde_json::to_string(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})).unwrap());

    // Send a ping right after — if notification was swallowed, we get ping response
    let resp = s.rpc(json!({"jsonrpc":"2.0","id":99,"method":"ping","params":{}}));
    assert_eq!(resp["id"], 99, "notification must not produce a response");
}

#[test]
fn test_id_types_string_and_number() {
    let mut s = McpSession::spawn();

    // String id
    let resp = s.rpc(json!({"jsonrpc":"2.0","id":"abc-123","method":"ping","params":{}}));
    assert_eq!(resp["id"], "abc-123", "string id must be echoed back");

    // Null id — serde Option<Value> treats JSON null as None → no response (like notification)
    // Verify it doesn't crash by sending null id then a normal ping
    s.send_raw(r#"{"jsonrpc":"2.0","id":null,"method":"ping","params":{}}"#);
    let resp = s.rpc(json!({"jsonrpc":"2.0","id":999,"method":"ping","params":{}}));
    assert_eq!(resp["id"], 999, "server must still respond after null-id request");
}

#[test]
fn test_tools_list_changed_notification_after_load() {
    let mut s = McpSession::spawn();
    s.initialize();

    // Load a nonexistent skill — returns an error; the tool set did NOT change,
    // so NO tools/list_changed notification should be emitted.
    let resp = s.rpc(json!({
        "jsonrpc": "2.0", "id": 10, "method": "tools/call",
        "params": {"name": "hub_load_skill", "arguments": {"skill": "nonexistent"}}
    }));
    assert_eq!(resp["id"], 10, "response id must match");

    // If a spurious notification were sent we would see it before the next response.
    // Verify it was NOT sent by immediately sending a ping and receiving it as the
    // very next line (no interleaved notification).
    let ping_resp = s.rpc(json!({"jsonrpc":"2.0","id":11,"method":"ping","params":{}}));
    assert_eq!(ping_resp["id"], 11, "no notification should be interleaved after a failed load");
}

#[test]
fn test_omc_state_roundtrip() {
    let mut s = McpSession::spawn();
    s.initialize();

    // Write state
    let resp = s.rpc(json!({
        "jsonrpc":"2.0","id":20,"method":"tools/call",
        "params":{"name":"state_write","arguments":{
            "mode":"test-compliance","active":true,"current_phase":"testing"
        }}
    }));
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("State written"), "state_write should confirm: {text}");

    // Read state back
    let resp = s.rpc(json!({
        "jsonrpc":"2.0","id":21,"method":"tools/call",
        "params":{"name":"state_read","arguments":{"mode":"test-compliance"}}
    }));
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let state: Value = serde_json::from_str(text).expect("state must be valid JSON");
    assert_eq!(state["active"], true);
    assert_eq!(state["current_phase"], "testing");
    assert_eq!(state["mode"], "test-compliance");

    // Clear state
    let resp = s.rpc(json!({
        "jsonrpc":"2.0","id":22,"method":"tools/call",
        "params":{"name":"state_clear","arguments":{"mode":"test-compliance"}}
    }));
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("cleared") || text.contains("clear"), "state_clear should confirm");
}

// ═══════════════════════════════════════════════════
// #67: Memory & Process Verification
// ═══════════════════════════════════════════════════

#[test]
fn test_binary_size_under_15mb() {
    let binary = if cfg!(target_os = "windows") {
        "target/debug/omc-hub.exe"
    } else {
        "target/debug/omc-hub"
    };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(binary);
    let meta = std::fs::metadata(&path).expect("binary must exist");
    let size_mb = meta.len() as f64 / (1024.0 * 1024.0);
    // Debug binaries include full symbols — Linux debug can be 80MB+.
    // Only enforce strict limit on release builds.
    let limit = if cfg!(debug_assertions) { 120.0 } else { 15.0 };
    assert!(size_mb < limit, "binary should be under {limit}MB, got {size_mb:.1}MB");
    eprintln!("Binary size: {size_mb:.1}MB (limit: {limit}MB)");
}

#[test]
fn test_startup_time_under_2s() {
    let t0 = Instant::now();
    let mut s = McpSession::spawn();
    let resp = s.initialize();
    let startup = t0.elapsed();

    assert!(resp.get("result").is_some(), "must initialize");
    assert!(startup < Duration::from_secs(2), "startup must be <2s, got {:?}", startup);
    eprintln!("Startup time: {:?}", startup);
}

#[test]
fn test_graceful_shutdown_on_stdin_close() {
    let mut s = McpSession::spawn();
    s.initialize();

    // Close stdin → should trigger graceful shutdown
    drop(s.stdin.take());

    let t0 = Instant::now();
    loop {
        match s.child.try_wait() {
            Ok(Some(status)) => {
                eprintln!("Process exited with: {:?} in {:?}", status, t0.elapsed());
                assert!(t0.elapsed() < Duration::from_secs(5), "should exit within 5s");
                return;
            }
            Ok(None) => {
                if t0.elapsed() > Duration::from_secs(5) {
                    let _ = s.child.kill();
                    panic!("Process did not exit within 5s after stdin close");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => panic!("Error waiting: {e}"),
        }
    }
}

#[test]
fn test_multiple_rapid_requests() {
    let mut s = McpSession::spawn();
    s.initialize();

    for i in 100..120 {
        let resp = s.rpc(json!({"jsonrpc":"2.0","id":i,"method":"ping","params":{}}));
        assert_eq!(resp["id"], i, "response id must match request id {i}");
        assert!(resp.get("result").is_some(), "ping {i} must have result");
    }
}

#[test]
fn test_hub_stats_tracking() {
    let mut s = McpSession::spawn();
    s.initialize();

    // Make tool calls that get tracked
    let _ = s.rpc(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
        "name":"state_list_active","arguments":{}
    }}));
    let _ = s.rpc(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
        "name":"notepad_stats","arguments":{}
    }}));

    // Check stats
    let resp = s.rpc(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
        "name":"hub_stats","arguments":{}
    }}));
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let stats: Value = serde_json::from_str(text).expect("stats must be valid JSON");
    assert!(stats.get("state_list_active").is_some(), "stats must track state_list_active");
    assert_eq!(stats["state_list_active"]["calls"], 1);
}
