// OpenCode user config.
//
// Reads MCP server names from `~/.config/opencode/opencode.json` (the `mcp`
// field) and installed skill names from `~/.config/opencode/skills/` directory
// entries. Falls back to empty sets when files or directories don't exist.
use std::collections::HashSet;
use std::path::PathBuf;

fn opencode_config_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("opencode"));
    }
    dirs::home_dir().map(|h| h.join(".config/opencode"))
}

pub struct UserConfig {
    pub mcp_servers: HashSet<String>,
    pub skills: HashSet<String>,
}

impl UserConfig {
    pub fn load() -> Self {
        let mut mcp_servers = HashSet::new();
        let mut skills = HashSet::new();

        if let Some(cfg_dir) = opencode_config_dir() {
            // Read MCP servers from opencode.json
            let config_path = cfg_dir.join("opencode.json");
            if let Ok(data) = std::fs::read_to_string(&config_path) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                    if let Some(mcp) = json.get("mcp").and_then(|v| v.as_object()) {
                        for key in mcp.keys() {
                            mcp_servers.insert(key.clone());
                        }
                    }
                }
            }
            // Read skills from skills/ directory
            let skills_dir = cfg_dir.join("skills");
            if let Ok(entries) = std::fs::read_dir(&skills_dir) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                            skills.insert(name.to_string());
                        }
                    }
                }
            }
        }

        UserConfig { mcp_servers, skills }
    }

    pub fn is_user_mcp(&self, server: &str) -> bool {
        self.mcp_servers.contains(server)
    }

    pub fn is_user_skill(&self, skill: &str) -> bool {
        self.skills.contains(skill)
    }
}
