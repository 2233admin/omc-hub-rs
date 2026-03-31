//! Toolbox: single-file script tools (bash/python/node)
//! Protocol: TOOLBOX_ACTION=describe → JSON schema, TOOLBOX_ACTION=execute + TOOLBOX_ARGS=json

use crate::protocol::{ToolDef, ToolResult};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct ToolboxEntry {
    pub ns_name: String,
    pub description: String,
    pub input_schema: Value,
    pub script_path: PathBuf,
}

/// Scan a directory for toolbox scripts, run `describe` on each.
pub async fn scan_toolbox(dir: &Path, prefix: &str) -> Vec<ToolboxEntry> {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return vec![];
    };
    let mut tools = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(tool) = describe_script(&path, prefix).await {
            tools.push(tool);
        }
    }
    tools
}

async fn describe_script(path: &Path, prefix: &str) -> Option<ToolboxEntry> {
    let output = run_script(path, "describe", None).await.ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let desc = parse_describe(stdout.trim())?;
    let ns_name = format!("{prefix}__{}", desc.name);
    Some(ToolboxEntry {
        ns_name,
        description: desc.description.clone(),
        input_schema: desc.input_schema.clone(),
        script_path: path.to_path_buf(),
    })
}

/// Execute a toolbox script with given args.
pub async fn execute_script(entry: &ToolboxEntry, args: &Value) -> ToolResult {
    let args_str = serde_json::to_string(args).unwrap_or_else(|_| "{}".into());
    match run_script(&entry.script_path, "execute", Some(&args_str)).await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let text = [stdout.trim(), stderr.trim()]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join("\n");
            let text = if text.is_empty() { "(no output)".into() } else { text };
            if output.status.success() {
                ToolResult::text(text)
            } else {
                ToolResult::error(text)
            }
        }
        Err(e) => ToolResult::error(format!("Script error: {e}")),
    }
}

/// Resolve bash executable: prefer Git Bash on Windows since system PATH rarely has bash.
fn bash_exe() -> &'static str {
    if cfg!(windows) {
        const GIT_BASH: &str = "C:/Program Files/Git/bin/bash.exe";
        if std::path::Path::new(GIT_BASH).exists() {
            return GIT_BASH;
        }
    }
    "bash"
}

async fn run_script(
    path: &Path,
    action: &str,
    args_json: Option<&str>,
) -> std::io::Result<std::process::Output> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let bash = bash_exe();
    let path_str = path.to_str().unwrap_or_default();
    let (cmd, cmd_args): (&str, Vec<&str>) = match ext {
        "py" => ("python", vec![path_str]),
        "mjs" | "js" => ("node", vec![path_str]),
        _ => (bash, vec![path_str]),
    };
    let mut command = Command::new(cmd);
    command.args(&cmd_args);
    command.env("TOOLBOX_ACTION", action);
    if let Some(a) = args_json {
        command.env("TOOLBOX_ARGS", a);
    }
    // Inherit parent env, add toolbox vars
    command.kill_on_drop(true);
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        command.output(),
    )
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "Script timed out (30s)"))?
}

/// Parse describe output: try JSON first, fall back to key:value format.
fn parse_describe(stdout: &str) -> Option<ToolDef> {
    // Try JSON
    if let Ok(def) = serde_json::from_str::<ToolDef>(stdout) {
        if !def.name.is_empty() {
            return Some(def);
        }
    }
    // Try raw JSON with nested structure
    if let Ok(v) = serde_json::from_str::<Value>(stdout) {
        if let Some(name) = v.get("name").and_then(|n| n.as_str()) {
            return Some(ToolDef {
                name: name.into(),
                description: v
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or(name)
                    .into(),
                input_schema: v
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or(serde_json::json!({"type": "object", "properties": {}})),
            });
        }
    }
    // Fallback: key: value lines
    let mut name = None;
    let mut description = None;
    let mut properties = HashMap::new();
    for line in stdout.lines() {
        if let Some((key, val)) = line.split_once(':') {
            let key = key.trim();
            let val = val.trim();
            match key {
                "name" => name = Some(val.to_string()),
                "description" => description = Some(val.to_string()),
                _ => {
                    properties.insert(
                        key.to_string(),
                        serde_json::json!({"type": "string", "description": val}),
                    );
                }
            }
        }
    }
    let name = name?;
    Some(ToolDef {
        description: description.unwrap_or_else(|| name.clone()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": properties,
        }),
        name,
    })
}
