//! Skill config loading from skills/*.json

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillConfig {
    pub description: Option<String>,
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
    /// Set at load time if skill lives in a directory (has embedded toolbox)
    #[serde(skip)]
    pub skill_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// "streamable-http" | "sse" for HTTP, absent for stdio
    #[serde(rename = "type")]
    pub transport_type: Option<String>,
    /// HTTP URL (for streamable-http/sse)
    pub url: Option<String>,
    /// HTTP headers
    pub headers: Option<HashMap<String, String>>,
    /// stdio command
    pub command: Option<String>,
    /// stdio args
    #[serde(default)]
    pub args: Vec<String>,
    /// stdio env overrides
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Tool name filter (glob patterns)
    #[serde(rename = "includeTools")]
    pub include_tools: Option<Vec<String>>,
}

impl McpServerConfig {
    pub fn is_http(&self) -> bool {
        matches!(
            self.transport_type.as_deref(),
            Some("streamable-http") | Some("sse")
        )
    }
}

/// Load all skill configs from a directory.
/// Supports both `skills/name.json` and `skills/name/skill.json`.
pub async fn load_skill_configs(skills_dir: &Path) -> HashMap<String, SkillConfig> {
    let mut configs = HashMap::new();
    let Ok(mut entries) = tokio::fs::read_dir(skills_dir).await else {
        return configs;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let file_type = entry.file_type().await.ok();
        if let Some(ft) = file_type {
            if ft.is_file() && path.extension().is_some_and(|e| e == "json") {
                let name = path.file_stem().unwrap().to_string_lossy().into_owned();
                if let Ok(data) = tokio::fs::read_to_string(&path).await {
                    if let Ok(cfg) = serde_json::from_str::<SkillConfig>(&data) {
                        configs.insert(name, cfg);
                    }
                }
            } else if ft.is_dir() {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let skill_json = path.join("skill.json");
                if let Ok(data) = tokio::fs::read_to_string(&skill_json).await {
                    if let Ok(mut cfg) = serde_json::from_str::<SkillConfig>(&data) {
                        cfg.skill_dir = Some(path);
                        configs.insert(name, cfg);
                    }
                }
            }
        }
    }
    configs
}
