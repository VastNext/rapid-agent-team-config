//! Config & Environment Scanner
//!
//! Scans global OpenCode directories (`~/.config/opencode` or `$XDG_CONFIG_HOME/opencode`)
//! and optional project-level directories for `opencode.json`, `opencode.jsonc`,
//! `agent/*.md`, and `agents/*.md`.
//!
//! Extracts:
//! - Available Providers and their defined Models (strictly zero credentials extracted)
//! - Existing Agents and their current `model:` frontmatter
//! - Rapid Dev Team installation status and health

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::frontmatter::parse_frontmatter;
use crate::jsonc::strip_jsonc_comments_and_trailing_commas;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelOption {
    pub id: String,         // e.g. "openai/gpt-4o"
    pub provider: String,   // e.g. "openai"
    pub model_name: String, // e.g. "gpt-4o"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatus {
    pub name: String,      // e.g. "rapid-dev-team"
    pub file_name: String, // e.g. "rapid-dev-team.md"
    pub full_path: String, // e.g. "/path/to/agents/rapid-dev-team.md"
    pub is_installed: bool,
    pub current_model: Option<String>,
    pub source: String,       // "global" or "project"
    pub mode: Option<String>, // "primary" or "subagent"
    pub description: Option<String>,
    pub mtime: u64, // For concurrency check
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamInstallStatus {
    pub is_installed_complete: bool,
    pub is_installed_partial: bool,
    pub installed_count: usize,
    pub total_expected: usize,
    pub missing_agents: Vec<String>,
    pub command_installed: bool,
    pub skill_installed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub target_scope: String, // "global" or "project"
    pub opencode_dir: String, // root config dir
    pub config_file_path: Option<String>,
    pub providers: Vec<String>,
    pub models: Vec<ModelOption>,
    pub agents: Vec<AgentStatus>,
    pub team_status: TeamInstallStatus,
    pub project_dir: Option<String>,
}

pub const RAPID_TEAM_AGENTS: &[&str] = &[
    "rapid-dev-team",
    "rapid-scout",
    "rapid-builder-glm-zhipu",
    "rapid-builder-glm-go",
    "rapid-builder-deepseek-go",
    "rapid-builder-deepseek-sensenova",
    "rapid-ui",
    "rapid-reviewer",
    "rapid-architect",
];

pub fn get_global_opencode_dir() -> PathBuf {
    if let Some(config_dir) = dirs::config_dir() {
        let opencode = config_dir.join("opencode");
        if opencode.exists() {
            return opencode;
        }
    }
    if let Some(home) = dirs::home_dir() {
        let opencode = home.join(".config").join("opencode");
        if opencode.exists() {
            return opencode;
        }
        return opencode;
    }
    PathBuf::from(".config/opencode")
}

pub fn scan_environment(project_path: Option<&Path>, use_project: bool) -> ScanResult {
    let (target_scope, base_dir) = if use_project && project_path.is_some() {
        let p = project_path.unwrap();
        let dot_opencode = p.join(".opencode");
        if dot_opencode.exists() && dot_opencode.is_dir() {
            ("project".to_string(), dot_opencode)
        } else {
            ("project".to_string(), p.to_path_buf())
        }
    } else {
        ("global".to_string(), get_global_opencode_dir())
    };

    // 1. Locate config files (both in current base_dir and fallback global for provider definitions)
    let jsonc_path = base_dir.join("opencode.jsonc");
    let json_path = base_dir.join("opencode.json");
    let config_file = if jsonc_path.exists() {
        Some(jsonc_path)
    } else if json_path.exists() {
        Some(json_path)
    } else {
        None
    };

    let mut config_files_to_read = Vec::new();
    if let Some(ref cf) = config_file {
        config_files_to_read.push(cf.clone());
    }
    if target_scope == "project" {
        let global_dir = get_global_opencode_dir();
        let g_jsonc = global_dir.join("opencode.jsonc");
        let g_json = global_dir.join("opencode.json");
        if g_jsonc.exists() {
            config_files_to_read.push(g_jsonc);
        } else if g_json.exists() {
            config_files_to_read.push(g_json);
        }
    }

    // 2. Parse providers and models (NO SECRETS EXTRACTED)
    let mut models_map: BTreeMap<String, ModelOption> = BTreeMap::new();
    let mut providers_set: BTreeSet<String> = BTreeSet::new();

    for cf in &config_files_to_read {
        if let Ok(raw_content) = std::fs::read_to_string(cf) {
            let cleaned = strip_jsonc_comments_and_trailing_commas(&raw_content);
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&cleaned) {
                if let Some(prov_obj) = val.get("provider").and_then(|p| p.as_object()) {
                    for (prov_name, prov_val) in prov_obj {
                        providers_set.insert(prov_name.clone());
                        if let Some(models_obj) = prov_val.get("models").and_then(|m| m.as_object())
                        {
                            for model_key in models_obj.keys() {
                                let full_id = format!("{}/{}", prov_name, model_key);
                                models_map.insert(
                                    full_id.clone(),
                                    ModelOption {
                                        id: full_id,
                                        provider: prov_name.clone(),
                                        model_name: model_key.clone(),
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Scan agents
    let mut agent_dirs = vec![base_dir.join("agents"), base_dir.join("agent")];
    if target_scope == "project" {
        let global_dir = get_global_opencode_dir();
        agent_dirs.push(global_dir.join("agents"));
        agent_dirs.push(global_dir.join("agent"));
    }

    let mut found_agents: BTreeMap<String, AgentStatus> = BTreeMap::new();

    for adir in agent_dirs {
        if !adir.exists() || !adir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&adir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("md") {
                    let file_stem = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default();
                    if file_stem.is_empty() {
                        continue;
                    }
                    if found_agents.contains_key(file_stem) {
                        continue;
                    }

                    let content = std::fs::read_to_string(&path).unwrap_or_default();
                    let mtime = entry
                        .metadata()
                        .and_then(|m| m.modified())
                        .map(|t| {
                            t.duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs()
                        })
                        .unwrap_or(0);

                    let (current_model, mode, description) =
                        if let Some(fm) = parse_frontmatter(&content) {
                            (fm.model, fm.mode, fm.description)
                        } else {
                            (None, None, None)
                        };

                    found_agents.insert(
                        file_stem.to_string(),
                        AgentStatus {
                            name: file_stem.to_string(),
                            file_name: path.file_name().unwrap().to_string_lossy().to_string(),
                            full_path: path.to_string_lossy().to_string(),
                            is_installed: true,
                            current_model,
                            source: target_scope.clone(),
                            mode,
                            description,
                            mtime,
                        },
                    );
                }
            }
        }
    }

    // 4. Check Rapid Team Completeness
    let mut missing_agents = Vec::new();
    let mut installed_count = 0;
    for &expected in RAPID_TEAM_AGENTS {
        if found_agents.contains_key(expected) {
            installed_count += 1;
        } else {
            missing_agents.push(expected.to_string());
        }
    }

    let command_file = base_dir.join("commands").join("rapid-dev.md");
    let command_installed = command_file.exists();
    let skill_dir = base_dir.join("skills").join("rapid-dev-team");
    let skill_installed = skill_dir.exists();

    let is_installed_complete = missing_agents.is_empty() && command_installed && skill_installed;
    let is_installed_partial = installed_count > 0 || command_installed || skill_installed;

    ScanResult {
        target_scope,
        opencode_dir: base_dir.to_string_lossy().to_string(),
        config_file_path: config_file.map(|p| p.to_string_lossy().to_string()),
        providers: providers_set.into_iter().collect(),
        models: models_map.into_values().collect(),
        agents: found_agents.into_values().collect(),
        team_status: TeamInstallStatus {
            is_installed_complete,
            is_installed_partial,
            installed_count,
            total_expected: RAPID_TEAM_AGENTS.len(),
            missing_agents,
            command_installed,
            skill_installed,
        },
        project_dir: project_path.map(|p| p.to_string_lossy().to_string()),
    }
}
