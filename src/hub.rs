//! Hub: skill lifecycle, tool registry, dispatch.

use crate::child::ChildMcp;
use crate::config::{load_skill_configs, SkillConfig};
use crate::omc_tools::OmcTools;
use crate::protocol::{ToolDef, ToolResult};
use crate::toolbox::{self, ToolboxEntry};
use glob_match::glob_match;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::info;

/// A loaded skill with its child MCP connections and tool registry.
struct LoadedSkill {
    tools: HashMap<String, RegisteredTool>,
    children: Vec<ChildMcp>,
}

/// A registered tool — either proxied through a child MCP or backed by a script.
enum RegisteredTool {
    Proxied {
        original_name: String,
        def: ToolDef,
        child_idx: usize, // index into LoadedSkill.children
    },
    Script {
        def: ToolDef,
        entry: ToolboxEntry,
    },
}

/// Call statistics per tool.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct ToolStats {
    calls: u64,
    errors: u64,
    total_ms: u64,
    last_used: Option<String>,
}

pub struct Hub {
    base_dir: PathBuf,
    skill_configs: HashMap<String, SkillConfig>,
    loaded: HashMap<String, LoadedSkill>,
    /// ns_name → skill_name (for reverse lookup)
    registry: HashMap<String, String>,
    /// Global toolbox tools (always visible)
    toolbox: Vec<ToolboxEntry>,
    stats: HashMap<String, ToolStats>,
    stats_dirty: bool,
    /// OMC native tools (state, notepad, project memory, etc.)
    omc: OmcTools,
    /// Incremented only when the tool set actually changes (load/unload/reload with effect).
    tool_generation: u64,
}

impl Hub {
    pub async fn new(base_dir: PathBuf, state_dir: PathBuf) -> Self {
        let skills_dir = base_dir.join("skills");
        let skill_configs = load_skill_configs(&skills_dir).await;
        info!("Loaded {} skill configs", skill_configs.len());

        let omc = OmcTools::new(state_dir);

        let mut hub = Self {
            base_dir,
            skill_configs,
            loaded: HashMap::new(),
            registry: HashMap::new(),
            toolbox: Vec::new(),
            stats: HashMap::new(),
            stats_dirty: false,
            omc,
            tool_generation: 0,
        };
        hub.load_stats().await;
        hub.scan_toolbox().await;
        hub
    }

    // ── Tool listing ─────────────────────────────────

    pub fn list_tools(&self) -> Vec<ToolDef> {
        let mut tools = vec![
            ToolDef {
                name: "hub_load_skill".into(),
                description: format!(
                    "Load a skill's MCP tools on-demand. Skills: {}",
                    self.skill_summary()
                ),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": { "skill": { "type": "string", "description": "Skill name" } },
                    "required": ["skill"]
                }),
            },
            ToolDef {
                name: "hub_unload_skill".into(),
                description: "Unload a skill's MCP tools to free resources".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": { "skill": { "type": "string", "description": "Skill name" } },
                    "required": ["skill"]
                }),
            },
            ToolDef {
                name: "hub_list_skills".into(),
                description: "List all available skills and their load status".into(),
                input_schema: serde_json::json!({"type": "object", "properties": {}}),
            },
            ToolDef {
                name: "hub_reload_toolbox".into(),
                description: "Rescan toolbox directory for new/changed scripts".into(),
                input_schema: serde_json::json!({"type": "object", "properties": {}}),
            },
            ToolDef {
                name: "hub_stats".into(),
                description: "Show tool call statistics (calls, errors, avg latency)".into(),
                input_schema: serde_json::json!({"type": "object", "properties": {}}),
            },
        ];

        // Loaded skill tools
        for skill in self.loaded.values() {
            for rt in skill.tools.values() {
                let def = match rt {
                    RegisteredTool::Proxied { def, .. } => def,
                    RegisteredTool::Script { def, .. } => def,
                };
                tools.push(def.clone());
            }
        }

        // Global toolbox tools (always visible)
        for entry in &self.toolbox {
            tools.push(ToolDef {
                name: entry.ns_name.clone(),
                description: format!("[toolbox] {}", entry.description),
                input_schema: entry.input_schema.clone(),
            });
        }

        // OMC native tools (state, notepad, project memory, trace, ast_grep)
        tools.extend(self.omc.tool_defs());

        tools
    }

    // ── Tool dispatch ────────────────────────────────

    pub async fn call_tool(&mut self, name: &str, args: Value) -> ToolResult {
        let t0 = std::time::Instant::now();

        let result = match name {
            "hub_load_skill" => self.handle_load_skill(&args).await,
            "hub_unload_skill" => self.handle_unload_skill(&args).await,
            "hub_list_skills" => self.handle_list_skills(),
            "hub_reload_toolbox" => self.handle_reload_toolbox().await,
            "hub_stats" => self.handle_stats(),
            _ => self.dispatch_tool(name, args).await,
        };

        // Record stats (skip management tools)
        if !name.starts_with("hub_") {
            let elapsed = t0.elapsed().as_millis() as u64;
            let entry = self.stats.entry(name.to_string()).or_default();
            entry.calls += 1;
            if result.is_error {
                entry.errors += 1;
            }
            entry.total_ms += elapsed;
            entry.last_used = Some(chrono::Local::now().to_rfc3339());
            self.stats_dirty = true;
        }

        result
    }

    /// Snapshot current generation counter. Call before `call_tool`, then pass result to
    /// `tools_changed_since` to determine if a `tools/list_changed` notification is needed.
    pub fn tool_generation(&self) -> u64 {
        self.tool_generation
    }

    /// Returns true only if the tool set actually changed since the snapshot was taken.
    pub fn tools_changed_since(&self, snapshot: u64) -> bool {
        self.tool_generation != snapshot
    }

    async fn dispatch_tool(&self, name: &str, args: Value) -> ToolResult {
        // Check OMC native tools first (state, notepad, project memory, etc.)
        if let Some(result) = self.omc.call(name, args.clone()).await {
            return result;
        }

        // Check global toolbox
        if let Some(entry) = self.toolbox.iter().find(|e| e.ns_name == name) {
            return toolbox::execute_script(entry, &args).await;
        }

        // Check skill registry
        let Some(skill_name) = self.registry.get(name) else {
            return ToolResult::error(format!("Unknown tool: {name}"));
        };
        let Some(skill) = self.loaded.get(skill_name) else {
            return ToolResult::error(format!("Skill {skill_name} not loaded"));
        };
        let Some(rt) = skill.tools.get(name) else {
            return ToolResult::error(format!("Tool {name} not found in skill {skill_name}"));
        };

        match rt {
            RegisteredTool::Script { entry, .. } => toolbox::execute_script(entry, &args).await,
            RegisteredTool::Proxied {
                original_name,
                child_idx,
                ..
            } => {
                let child = &skill.children[*child_idx];
                match child.call_tool(original_name, args).await {
                    Ok(resp) => {
                        // Extract result from JSON-RPC response
                        if let Some(result) = resp.get("result") {
                            if let Ok(tr) = serde_json::from_value::<ToolResult>(result.clone()) {
                                return tr;
                            }
                            return ToolResult::text(result.to_string());
                        }
                        if let Some(err) = resp.get("error") {
                            return ToolResult::error(err.to_string());
                        }
                        // Raw response (HTTP transports may return result directly)
                        if resp.get("content").is_some() {
                            if let Ok(tr) = serde_json::from_value::<ToolResult>(resp.clone()) {
                                return tr;
                            }
                        }
                        ToolResult::text(resp.to_string())
                    }
                    Err(e) => ToolResult::error(format!("Skill {skill_name} error: {e}")),
                }
            }
        }
    }

    // ── Management tool handlers ─────────────────────

    async fn handle_load_skill(&mut self, args: &Value) -> ToolResult {
        let Some(skill_name) = args.get("skill").and_then(|s| s.as_str()) else {
            return ToolResult::error("Missing 'skill' argument");
        };

        if self.loaded.contains_key(skill_name) {
            let tools: Vec<_> = self.loaded[skill_name].tools.keys().collect();
            return ToolResult::text(serde_json::json!({
                "already": true, "tools": tools
            }).to_string());
        }

        let Some(config) = self.skill_configs.get(skill_name).cloned() else {
            let available: Vec<_> = self.skill_configs.keys().collect();
            return ToolResult::error(format!(
                "Unknown skill: {skill_name}. Available: {available:?}"
            ));
        };

        match self.load_skill(skill_name, &config).await {
            Ok(tool_names) => {
                self.tool_generation += 1;
                ToolResult::text(
                    serde_json::json!({
                        "loaded": true,
                        "toolCount": tool_names.len(),
                        "tools": tool_names,
                    })
                    .to_string(),
                )
            }
            Err(e) => ToolResult::error(e),
        }
    }

    async fn load_skill(
        &mut self,
        skill_name: &str,
        config: &SkillConfig,
    ) -> Result<Vec<String>, String> {
        let mut children = Vec::new();
        let mut tools = HashMap::new();

        // Connect MCP servers
        for (_server_name, mcp_config) in &config.mcp_servers {
            let child_idx = children.len();
            let child = ChildMcp::connect(mcp_config).await?;
            let child_tools = child.list_tools().await?;

            for tool in child_tools {
                if !matches_include(&tool.name, &mcp_config.include_tools) {
                    continue;
                }
                let ns_name = format!("skill__{skill_name}__{}", tool.name);
                self.registry.insert(ns_name.clone(), skill_name.to_string());
                tools.insert(
                    ns_name,
                    RegisteredTool::Proxied {
                        original_name: tool.name.clone(),
                        def: ToolDef {
                            name: format!("skill__{skill_name}__{}", tool.name),
                            description: tool.description,
                            input_schema: tool.input_schema,
                        },
                        child_idx,
                    },
                );
            }
            children.push(child);
        }

        // Skill-embedded toolbox scripts
        if let Some(skill_dir) = &config.skill_dir {
            let tb_dir = skill_dir.join("toolbox");
            let prefix = format!("skill__{skill_name}");
            let entries = toolbox::scan_toolbox(&tb_dir, &prefix).await;
            for entry in entries {
                let ns_name = entry.ns_name.clone();
                self.registry.insert(ns_name.clone(), skill_name.to_string());
                tools.insert(
                    ns_name,
                    RegisteredTool::Script {
                        def: ToolDef {
                            name: entry.ns_name.clone(),
                            description: entry.description.clone(),
                            input_schema: entry.input_schema.clone(),
                        },
                        entry,
                    },
                );
            }
        }

        let tool_names: Vec<String> = tools.keys().cloned().collect();
        info!("Loaded skill '{skill_name}' with {} tools", tool_names.len());
        self.loaded.insert(
            skill_name.to_string(),
            LoadedSkill { tools, children },
        );
        Ok(tool_names)
    }

    async fn handle_unload_skill(&mut self, args: &Value) -> ToolResult {
        let Some(skill_name) = args.get("skill").and_then(|s| s.as_str()) else {
            return ToolResult::error("Missing 'skill' argument");
        };
        let Some(skill) = self.loaded.remove(skill_name) else {
            return ToolResult::error(format!("{skill_name} not loaded"));
        };
        // Clean registry
        let to_remove: Vec<_> = self
            .registry
            .iter()
            .filter(|(_, v)| v.as_str() == skill_name)
            .map(|(k, _)| k.clone())
            .collect();
        for k in &to_remove {
            self.registry.remove(k);
        }
        // Close children
        for child in skill.children {
            child.close().await;
        }
        info!("Unloaded skill '{skill_name}'");
        self.tool_generation += 1;
        ToolResult::text(serde_json::json!({"unloaded": true}).to_string())
    }

    fn handle_list_skills(&self) -> ToolResult {
        let available: Vec<_> = self
            .skill_configs
            .iter()
            .map(|(name, config)| {
                serde_json::json!({
                    "name": name,
                    "description": config.description,
                    "loaded": self.loaded.contains_key(name),
                    "servers": config.mcp_servers.keys().collect::<Vec<_>>(),
                    "hasToolbox": config.skill_dir.is_some(),
                })
            })
            .collect();
        ToolResult::text(
            serde_json::json!({
                "available": available,
                "totalLoaded": self.loaded.len(),
                "totalProxiedTools": self.registry.len(),
                "totalToolboxTools": self.toolbox.len(),
            })
            .to_string(),
        )
    }

    async fn handle_reload_toolbox(&mut self) -> ToolResult {
        let before = self.toolbox.len();
        self.toolbox.clear();
        self.scan_toolbox().await;
        let after = self.toolbox.len();
        if before != after {
            self.tool_generation += 1;
        }
        let names: Vec<_> = self.toolbox.iter().map(|e| &e.ns_name).collect();
        ToolResult::text(serde_json::json!({"reloaded": true, "tools": names}).to_string())
    }

    fn handle_stats(&self) -> ToolResult {
        ToolResult::text(serde_json::to_string_pretty(&self.stats).unwrap_or_else(|_| "{}".into()))
    }

    // ── Helpers ──────────────────────────────────────

    async fn scan_toolbox(&mut self) {
        let dir = self.base_dir.join("toolbox");
        self.toolbox = toolbox::scan_toolbox(&dir, "toolbox").await;
        if !self.toolbox.is_empty() {
            info!("Toolbox: {} tools registered", self.toolbox.len());
        }
    }

    fn skill_summary(&self) -> String {
        self.skill_configs
            .iter()
            .map(|(name, c)| {
                format!(
                    "{name}: {}",
                    c.description.as_deref().unwrap_or("no description")
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    async fn load_stats(&mut self) {
        let path = self.base_dir.join("stats.json");
        if let Ok(data) = tokio::fs::read_to_string(&path).await {
            if let Ok(s) = serde_json::from_str(&data) {
                self.stats = s;
            }
        }
    }

    pub async fn flush_stats(&mut self) {
        if !self.stats_dirty {
            return;
        }
        let path = self.base_dir.join("stats.json");
        if let Ok(data) = serde_json::to_string_pretty(&self.stats) {
            let _ = tokio::fs::write(&path, data).await;
            self.stats_dirty = false;
        }
    }

    /// Graceful shutdown: unload all skills, flush stats.
    pub async fn shutdown(&mut self) {
        let names: Vec<_> = self.loaded.keys().cloned().collect();
        for name in names {
            if let Some(skill) = self.loaded.remove(&name) {
                for child in skill.children {
                    child.close().await;
                }
            }
        }
        self.flush_stats().await;
    }
}

fn matches_include(tool_name: &str, patterns: &Option<Vec<String>>) -> bool {
    let Some(patterns) = patterns else {
        return true;
    };
    if patterns.is_empty() {
        return true;
    }
    patterns
        .iter()
        .any(|p| glob_match(p, tool_name) || p == tool_name)
}
