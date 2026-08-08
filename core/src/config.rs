// OpenCode user config.
//
// Reads MCP server names from `~/.config/opencode/opencode.json` (the `mcp`
// field) and installed skill names from the OpenCode skills dir plus the
// common agent skill roots (`~/.agents/skills`, `~/.codex/skills`). Falls
// back to empty sets when files or directories don't exist.
use std::collections::HashSet;
use std::path::PathBuf;

fn opencode_config_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("opencode"));
    }
    dirs::home_dir().map(|h| h.join(".config/opencode"))
}

/// Recursively collect skill names from `root`: any directory that directly
/// contains a `SKILL.md` counts as a skill, keyed by its directory name.
/// Handles both flat installs (`skills/foo/SKILL.md`) and nested collections
/// (`skills/vendor/skills/category/foo/SKILL.md`).
fn scan_skills(root: &std::path::Path, out: &mut HashSet<String>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            if entry.path().join("SKILL.md").is_file() {
                if let Some(n) = entry.file_name().to_str() {
                    out.insert(n.to_string());
                }
            } else {
                scan_skills(&entry.path(), out);
            }
        }
    }
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
            // Installed skills: any subdirectory of the OpenCode skills dir
            // (legacy behavior), then recursive scans of the common agent
            // skill roots where a skill is a directory containing SKILL.md.
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
        if let Some(home) = dirs::home_dir() {
            scan_skills(&home.join(".agents/skills"), &mut skills);
            scan_skills(&home.join(".codex/skills"), &mut skills);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_skills_finds_flat_and_nested_skills() {
        let tmp = std::env::temp_dir().join(format!("ocscale-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("flat/foo")).unwrap();
        std::fs::create_dir_all(tmp.join("flat/bar")).unwrap();
        std::fs::create_dir_all(tmp.join("nested/vendor/skills/cat/baz")).unwrap();
        std::fs::create_dir_all(tmp.join("nested/vendor/skills/cat/not-a-skill")).unwrap();
        std::fs::write(tmp.join("flat/foo/SKILL.md"), "").unwrap();
        std::fs::write(tmp.join("flat/bar/readme.md"), "").unwrap(); // no SKILL.md
        std::fs::write(tmp.join("nested/vendor/skills/cat/baz/SKILL.md"), "").unwrap();

        let mut skills = HashSet::new();
        scan_skills(&tmp.join("flat"), &mut skills);
        scan_skills(&tmp.join("nested"), &mut skills);

        assert!(skills.contains("foo"));
        assert!(skills.contains("baz"));
        assert!(!skills.contains("bar"));
        assert!(!skills.contains("vendor"));
        assert!(!skills.contains("not-a-skill"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
