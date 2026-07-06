// OpenCode user config.
//
// Original tokenscope parsed ~/.claude.json and ~/.claude/skills/ for MCP
// and Skill whitelists. In the OpenCode version per-event MCP/Skill tracking
// is not yet wired from the database, so the whitelists are empty. The
// "installed servers/skills" metrics shown in the dashboard reflect these
// counts (0 for now — future: derive from OpenCode's config or event table).
use std::collections::HashSet;

pub struct UserConfig {
    pub mcp_servers: HashSet<String>,
    pub skills: HashSet<String>,
}

impl UserConfig {
    pub fn load() -> Self {
        UserConfig {
            mcp_servers: HashSet::new(),
            skills: HashSet::new(),
        }
    }

    pub fn is_user_mcp(&self, _server: &str) -> bool {
        false
    }

    pub fn is_user_skill(&self, _skill: &str) -> bool {
        false
    }
}
