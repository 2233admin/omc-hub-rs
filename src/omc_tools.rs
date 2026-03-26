//! OMC native tools — state, notepad, project memory, trace, session search.
//! Pure file I/O, no MCP subprocess needed.

use crate::protocol::{ToolDef, ToolResult};
use serde_json::Value;
use std::path::PathBuf;

pub struct OmcTools {
    omc_dir: PathBuf, // ~/.omc
}

impl OmcTools {
    pub fn new(omc_dir: PathBuf) -> Self {
        Self { omc_dir }
    }

    pub fn tool_defs(&self) -> Vec<ToolDef> {
        vec![
            // ── State tools ──
            tool("state_read", "Read state for a mode (ralph, ultrawork, autopilot, team, etc.)", json_obj(&[
                ("mode", "string", "Mode name", true),
            ])),
            tool("state_write", "Write/update state for a mode. Creates dirs if needed.", json_obj(&[
                ("mode", "string", "Mode name", true),
                ("active", "boolean", "Whether mode is active", false),
                ("current_phase", "string", "Current phase", false),
                ("iteration", "number", "Current iteration", false),
                ("max_iterations", "number", "Max iterations", false),
                ("state", "object", "Arbitrary state data to merge", false),
            ])),
            tool("state_clear", "Clear/delete state for a mode.", json_obj(&[
                ("mode", "string", "Mode name", true),
            ])),
            tool("state_list_active", "List all currently active modes.", json_obj(&[])),
            tool("state_get_status", "Get detailed status for a mode or all modes.", json_obj(&[
                ("mode", "string", "Mode name (omit for all)", false),
            ])),
            // ── Notepad tools ──
            tool("notepad_read", "Read notepad content. Full or specific section.", json_obj(&[
                ("section", "string", "Section: priority, working, manual (omit for all)", false),
            ])),
            tool("notepad_write_priority", "Write to Priority Context section (replaces existing).", json_obj(&[
                ("content", "string", "Content to write", true),
            ])),
            tool("notepad_write_working", "Add timestamped entry to Working Memory section.", json_obj(&[
                ("content", "string", "Content to add", true),
            ])),
            tool("notepad_write_manual", "Add entry to MANUAL section (never auto-pruned).", json_obj(&[
                ("content", "string", "Content to add", true),
            ])),
            tool("notepad_prune", "Prune Working Memory entries older than N days.", json_obj(&[
                ("days", "number", "Days to keep (default: 7)", false),
            ])),
            tool("notepad_stats", "Get notepad statistics (size, entry count).", json_obj(&[])),
            // ── Project Memory tools ──
            tool("project_memory_read", "Read project memory. Full or specific section.", json_obj(&[
                ("section", "string", "Section name (omit for all)", false),
            ])),
            tool("project_memory_write", "Write/update project memory.", json_obj(&[
                ("content", "object", "Memory content (replaces or merges)", true),
                ("merge", "boolean", "Merge with existing (default: false)", false),
            ])),
            tool("project_memory_add_note", "Add a categorized note to project memory.", json_obj(&[
                ("category", "string", "Note category", true),
                ("content", "string", "Note content", true),
            ])),
            tool("project_memory_add_directive", "Add a user directive to project memory.", json_obj(&[
                ("directive", "string", "Directive text", true),
            ])),
            // ── Trace tools ──
            tool("trace_timeline", "Show chronological agent flow trace timeline.", json_obj(&[
                ("session_id", "string", "Session ID (omit for current)", false),
                ("limit", "number", "Max entries", false),
            ])),
            tool("trace_summary", "Show aggregate statistics for trace session.", json_obj(&[
                ("session_id", "string", "Session ID (omit for current)", false),
            ])),
            // ── Session search ──
            tool("session_search", "Search prior session history and transcripts.", json_obj(&[
                ("query", "string", "Search query", true),
                ("limit", "number", "Max results (default: 10)", false),
            ])),
            // ── AST grep (delegates to sg CLI) ──
            tool("ast_grep_search", "Search for code patterns using AST matching (sg CLI).", json_obj(&[
                ("pattern", "string", "AST pattern to search", true),
                ("path", "string", "File or directory to search", false),
                ("lang", "string", "Language (js, ts, py, rs, go, etc.)", false),
            ])),
            tool("ast_grep_replace", "Replace code patterns using AST matching (sg CLI).", json_obj(&[
                ("pattern", "string", "AST pattern to match", true),
                ("rewrite", "string", "Replacement pattern", true),
                ("path", "string", "File or directory", false),
                ("lang", "string", "Language", false),
            ])),
        ]
    }

    pub async fn call(&self, name: &str, args: Value) -> Option<ToolResult> {
        match name {
            "state_read" => Some(self.state_read(&args).await),
            "state_write" => Some(self.state_write(&args).await),
            "state_clear" => Some(self.state_clear(&args).await),
            "state_list_active" => Some(self.state_list_active().await),
            "state_get_status" => Some(self.state_get_status(&args).await),
            "notepad_read" => Some(self.notepad_read(&args).await),
            "notepad_write_priority" => Some(self.notepad_write_priority(&args).await),
            "notepad_write_working" => Some(self.notepad_write_working(&args).await),
            "notepad_write_manual" => Some(self.notepad_write_manual(&args).await),
            "notepad_prune" => Some(self.notepad_prune(&args).await),
            "notepad_stats" => Some(self.notepad_stats().await),
            "project_memory_read" => Some(self.pm_read(&args).await),
            "project_memory_write" => Some(self.pm_write(&args).await),
            "project_memory_add_note" => Some(self.pm_add_note(&args).await),
            "project_memory_add_directive" => Some(self.pm_add_directive(&args).await),
            "trace_timeline" => Some(self.trace_timeline(&args).await),
            "trace_summary" => Some(self.trace_summary(&args).await),
            "session_search" => Some(self.session_search(&args).await),
            "ast_grep_search" => Some(self.ast_grep_search(&args).await),
            "ast_grep_replace" => Some(self.ast_grep_replace(&args).await),
            _ => None,
        }
    }

    // ── State ────────────────────────────────────────

    fn state_path(&self, mode: &str) -> PathBuf {
        self.omc_dir.join("state").join(format!("{mode}-state.json"))
    }

    async fn state_read(&self, args: &Value) -> ToolResult {
        let mode = str_arg(args, "mode");
        let path = self.state_path(&mode);
        match tokio::fs::read_to_string(&path).await {
            Ok(data) => ToolResult::text(data),
            Err(_) => ToolResult::text(format!("{{\"active\": false, \"mode\": \"{mode}\"}}")),
        }
    }

    async fn state_write(&self, args: &Value) -> ToolResult {
        let mode = str_arg(args, "mode");
        let path = self.state_path(&mode);
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        // Read existing, merge
        let mut existing: Value = match tokio::fs::read_to_string(&path).await {
            Ok(d) => serde_json::from_str(&d).unwrap_or(Value::Object(Default::default())),
            Err(_) => Value::Object(Default::default()),
        };
        let obj = existing.as_object_mut().unwrap();
        // Merge top-level fields from args
        for key in &["active", "current_phase", "iteration", "max_iterations"] {
            if let Some(v) = args.get(key) {
                obj.insert(key.to_string(), v.clone());
            }
        }
        obj.insert("mode".to_string(), Value::String(mode.clone()));
        obj.insert("updatedAt".to_string(), Value::String(chrono::Local::now().to_rfc3339()));
        // Merge nested state object
        if let Some(state) = args.get("state").and_then(|s| s.as_object()) {
            for (k, v) in state {
                obj.insert(k.clone(), v.clone());
            }
        }
        match tokio::fs::write(&path, serde_json::to_string_pretty(&existing).unwrap()).await {
            Ok(_) => ToolResult::text(format!("State written for mode '{mode}'")),
            Err(e) => ToolResult::error(format!("Write failed: {e}")),
        }
    }

    async fn state_clear(&self, args: &Value) -> ToolResult {
        let mode = str_arg(args, "mode");
        let path = self.state_path(&mode);
        match tokio::fs::remove_file(&path).await {
            Ok(_) => ToolResult::text(format!("State cleared for '{mode}'")),
            Err(_) => ToolResult::text(format!("No state file for '{mode}' (already clear)")),
        }
    }

    async fn state_list_active(&self) -> ToolResult {
        let dir = self.omc_dir.join("state");
        let mut active = Vec::new();
        if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with("-state.json") {
                    if let Ok(data) = tokio::fs::read_to_string(entry.path()).await {
                        if let Ok(v) = serde_json::from_str::<Value>(&data) {
                            if v.get("active").and_then(|a| a.as_bool()).unwrap_or(false) {
                                let mode = name.trim_end_matches("-state.json");
                                active.push(mode.to_string());
                            }
                        }
                    }
                }
            }
        }
        ToolResult::text(serde_json::json!({"activeModes": active}).to_string())
    }

    async fn state_get_status(&self, args: &Value) -> ToolResult {
        let mode = args.get("mode").and_then(|m| m.as_str());
        if mode.is_some() {
            return self.state_read(args).await;
        }
        // All modes
        self.state_list_active().await
    }

    // ── Notepad ──────────────────────────────────────

    fn notepad_path(&self) -> PathBuf {
        self.omc_dir.join("notepad.md")
    }

    async fn notepad_read(&self, args: &Value) -> ToolResult {
        let content = tokio::fs::read_to_string(self.notepad_path())
            .await
            .unwrap_or_else(|_| "# Notepad\n\n(empty)".to_string());
        let section = args.get("section").and_then(|s| s.as_str());
        if let Some(section) = section {
            // Extract section
            let marker = format!("## {}", section_title(section));
            if let Some(start) = content.find(&marker) {
                let rest = &content[start..];
                let end = rest[marker.len()..].find("\n## ").map(|i| i + marker.len()).unwrap_or(rest.len());
                return ToolResult::text(rest[..end].to_string());
            }
            return ToolResult::text(format!("Section '{section}' not found"));
        }
        ToolResult::text(content)
    }

    async fn notepad_write_priority(&self, args: &Value) -> ToolResult {
        let content = str_arg(args, "content");
        self.notepad_update_section("Priority Context", &content, true).await
    }

    async fn notepad_write_working(&self, args: &Value) -> ToolResult {
        let content = str_arg(args, "content");
        let entry = format!("- [{}] {}", chrono::Local::now().format("%Y-%m-%d %H:%M"), content);
        self.notepad_update_section("Working Memory", &entry, false).await
    }

    async fn notepad_write_manual(&self, args: &Value) -> ToolResult {
        let content = str_arg(args, "content");
        let entry = format!("- {}", content);
        self.notepad_update_section("MANUAL", &entry, false).await
    }

    async fn notepad_update_section(&self, section: &str, content: &str, replace: bool) -> ToolResult {
        let path = self.notepad_path();
        let mut full = tokio::fs::read_to_string(&path).await.unwrap_or_else(|_| {
            "# Notepad\n\n## Priority Context\n\n## Working Memory\n\n## MANUAL\n".to_string()
        });
        let marker = format!("## {section}");
        if let Some(start) = full.find(&marker) {
            let section_start = start + marker.len();
            let next_section = full[section_start..].find("\n## ").map(|i| section_start + i);
            let section_end = next_section.unwrap_or(full.len());
            if replace {
                let replacement = format!("{marker}\n{content}\n");
                full.replace_range(start..section_end, &replacement);
            } else {
                let insert_pos = section_end;
                full.insert_str(insert_pos, &format!("{content}\n"));
            }
        } else {
            full.push_str(&format!("\n{marker}\n{content}\n"));
        }
        match tokio::fs::write(&path, &full).await {
            Ok(_) => ToolResult::text(format!("Notepad {section} updated")),
            Err(e) => ToolResult::error(format!("Write failed: {e}")),
        }
    }

    async fn notepad_prune(&self, args: &Value) -> ToolResult {
        let _days = args.get("days").and_then(|d| d.as_u64()).unwrap_or(7);
        // Simple prune: just report, actual date parsing is complex
        ToolResult::text("Prune not yet implemented in Rust hub — use OMC bridge for now")
    }

    async fn notepad_stats(&self) -> ToolResult {
        let path = self.notepad_path();
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                let lines = content.lines().count();
                let size = content.len();
                let entries = content.matches("\n- ").count();
                ToolResult::text(serde_json::json!({
                    "lines": lines, "bytes": size, "entries": entries
                }).to_string())
            }
            Err(_) => ToolResult::text("{\"lines\":0,\"bytes\":0,\"entries\":0}".to_string()),
        }
    }

    // ── Project Memory ───────────────────────────────

    fn pm_path(&self) -> PathBuf {
        self.omc_dir.join("project-memory.json")
    }

    async fn pm_read(&self, _args: &Value) -> ToolResult {
        match tokio::fs::read_to_string(self.pm_path()).await {
            Ok(d) => ToolResult::text(d),
            Err(_) => ToolResult::text("{}"),
        }
    }

    async fn pm_write(&self, args: &Value) -> ToolResult {
        let merge = args.get("merge").and_then(|m| m.as_bool()).unwrap_or(false);
        let content = args.get("content").cloned().unwrap_or(Value::Object(Default::default()));
        let path = self.pm_path();

        let final_val = if merge {
            let mut existing: Value = match tokio::fs::read_to_string(&path).await {
                Ok(d) => serde_json::from_str(&d).unwrap_or(Value::Object(Default::default())),
                Err(_) => Value::Object(Default::default()),
            };
            if let (Some(e), Some(c)) = (existing.as_object_mut(), content.as_object()) {
                for (k, v) in c {
                    e.insert(k.clone(), v.clone());
                }
            }
            existing
        } else {
            content
        };

        match tokio::fs::write(&path, serde_json::to_string_pretty(&final_val).unwrap()).await {
            Ok(_) => ToolResult::text("Project memory updated"),
            Err(e) => ToolResult::error(format!("Write failed: {e}")),
        }
    }

    async fn pm_add_note(&self, args: &Value) -> ToolResult {
        let category = str_arg(args, "category");
        let content = str_arg(args, "content");
        let path = self.pm_path();
        let mut pm: Value = match tokio::fs::read_to_string(&path).await {
            Ok(d) => serde_json::from_str(&d).unwrap_or(Value::Object(Default::default())),
            Err(_) => Value::Object(Default::default()),
        };
        let notes = pm.as_object_mut().unwrap()
            .entry("notes").or_insert(Value::Array(vec![]));
        if let Some(arr) = notes.as_array_mut() {
            arr.push(serde_json::json!({
                "category": category,
                "content": content,
                "addedAt": chrono::Local::now().to_rfc3339(),
            }));
        }
        let _ = tokio::fs::write(&path, serde_json::to_string_pretty(&pm).unwrap()).await;
        ToolResult::text(format!("Note added to category '{category}'"))
    }

    async fn pm_add_directive(&self, args: &Value) -> ToolResult {
        let directive = str_arg(args, "directive");
        let path = self.pm_path();
        let mut pm: Value = match tokio::fs::read_to_string(&path).await {
            Ok(d) => serde_json::from_str(&d).unwrap_or(Value::Object(Default::default())),
            Err(_) => Value::Object(Default::default()),
        };
        let directives = pm.as_object_mut().unwrap()
            .entry("directives").or_insert(Value::Array(vec![]));
        if let Some(arr) = directives.as_array_mut() {
            arr.push(serde_json::json!({
                "text": directive,
                "addedAt": chrono::Local::now().to_rfc3339(),
            }));
        }
        let _ = tokio::fs::write(&path, serde_json::to_string_pretty(&pm).unwrap()).await;
        ToolResult::text("Directive added")
    }

    // ── Trace ────────────────────────────────────────

    async fn trace_timeline(&self, args: &Value) -> ToolResult {
        let session_id = args.get("session_id").and_then(|s| s.as_str());
        // Look for trace files in state directory
        let dir = self.omc_dir.join("state");
        let _pattern = if let Some(sid) = session_id {
            format!("agent-replay-{sid}.jsonl")
        } else {
            "agent-replay-*.jsonl".to_string()
        };
        let mut entries = Vec::new();
        if let Ok(mut rd) = tokio::fs::read_dir(&dir).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("agent-replay-") && name.ends_with(".jsonl") {
                    if session_id.is_none() || name.contains(session_id.unwrap_or("")) {
                        if let Ok(data) = tokio::fs::read_to_string(entry.path()).await {
                            let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(50) as usize;
                            for line in data.lines().take(limit) {
                                entries.push(line.to_string());
                            }
                        }
                    }
                }
            }
        }
        if entries.is_empty() {
            ToolResult::text("No trace data found")
        } else {
            ToolResult::text(format!("[{}]", entries.join(",")))
        }
    }

    async fn trace_summary(&self, args: &Value) -> ToolResult {
        let tl = self.trace_timeline(args).await;
        // Simple summary: count entries
        let text = &tl.content[0].text;
        let count = text.matches('{').count();
        ToolResult::text(format!("{{\"totalEvents\": {count}}}"))
    }

    // ── Session search ───────────────────────────────

    async fn session_search(&self, args: &Value) -> ToolResult {
        let query = str_arg(args, "query").to_lowercase();
        let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(10) as usize;
        let sessions_dir = self.omc_dir.join("sessions");
        let mut results = Vec::new();

        if let Ok(mut entries) = tokio::fs::read_dir(&sessions_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if results.len() >= limit { break; }
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "json" || e == "md") {
                    if let Ok(data) = tokio::fs::read_to_string(&path).await {
                        if data.to_lowercase().contains(&query) {
                            results.push(serde_json::json!({
                                "file": path.file_name().unwrap().to_string_lossy(),
                                "snippet": data.to_lowercase()
                                    .find(&query)
                                    .map(|i| {
                                        let start = i.saturating_sub(50);
                                        let end = (i + query.len() + 50).min(data.len());
                                        data[start..end].to_string()
                                    })
                                    .unwrap_or_default(),
                            }));
                        }
                    }
                }
            }
        }
        ToolResult::text(serde_json::to_string_pretty(&results).unwrap_or("[]".into()))
    }

    // ── AST grep (delegates to sg CLI) ───────────────

    async fn ast_grep_search(&self, args: &Value) -> ToolResult {
        let pattern = str_arg(args, "pattern");
        let path = args.get("path").and_then(|p| p.as_str()).unwrap_or(".");
        let lang = args.get("lang").and_then(|l| l.as_str());
        let mut cmd = tokio::process::Command::new("sg");
        cmd.arg("run").arg("--pattern").arg(&pattern).arg(path);
        if let Some(l) = lang {
            cmd.arg("--lang").arg(l);
        }
        cmd.arg("--json");
        run_cmd(cmd).await
    }

    async fn ast_grep_replace(&self, args: &Value) -> ToolResult {
        let pattern = str_arg(args, "pattern");
        let rewrite = str_arg(args, "rewrite");
        let path = args.get("path").and_then(|p| p.as_str()).unwrap_or(".");
        let lang = args.get("lang").and_then(|l| l.as_str());
        let mut cmd = tokio::process::Command::new("sg");
        cmd.arg("run").arg("--pattern").arg(&pattern)
            .arg("--rewrite").arg(&rewrite).arg(path);
        if let Some(l) = lang {
            cmd.arg("--lang").arg(l);
        }
        cmd.arg("--json");
        run_cmd(cmd).await
    }
}

// ── Helpers ──────────────────────────────────────

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn section_title(section: &str) -> &str {
    match section {
        "priority" => "Priority Context",
        "working" => "Working Memory",
        "manual" => "MANUAL",
        _ => section,
    }
}

fn tool(name: &str, desc: &str, schema: Value) -> ToolDef {
    ToolDef {
        name: name.to_string(),
        description: desc.to_string(),
        input_schema: schema,
    }
}

fn json_obj(fields: &[(&str, &str, &str, bool)]) -> Value {
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();
    for (name, typ, desc, req) in fields {
        props.insert(name.to_string(), serde_json::json!({
            "type": typ,
            "description": desc,
        }));
        if *req {
            required.push(Value::String(name.to_string()));
        }
    }
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": props,
    });
    if !required.is_empty() {
        schema.as_object_mut().unwrap().insert("required".to_string(), Value::Array(required));
    }
    schema
}

async fn run_cmd(mut cmd: tokio::process::Command) -> ToolResult {
    cmd.kill_on_drop(true);
    match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        cmd.output(),
    ).await {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let text = [stdout.trim(), stderr.trim()]
                .iter().filter(|s| !s.is_empty()).copied()
                .collect::<Vec<_>>().join("\n");
            if output.status.success() {
                ToolResult::text(if text.is_empty() { "(no output)".into() } else { text })
            } else {
                ToolResult::error(if text.is_empty() { format!("Exit code: {:?}", output.status.code()) } else { text })
            }
        }
        Ok(Err(e)) => ToolResult::error(format!("Command failed: {e}")),
        Err(_) => ToolResult::error("Command timed out (30s)"),
    }
}
