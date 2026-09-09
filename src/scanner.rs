//! Config & Environment Scanner
//!
//! Scans global OpenCode directories (`~/.config/opencode` or `$OPENCODE_CONFIG_DIR`)
//! and optional project-level directories for `opencode.json`, `opencode.jsonc`,
//! `agent/*.md`, and `agents/*.md`.
//!
//! Extracts:
//! - Available Providers and their defined Models (strictly zero credentials extracted)
//! - Agent bindings defined in opencode.json/jsonc (`agent: { ... }` / `agents: { ... }`)
//! - Existing Agents and their current `model:` frontmatter
//! - Rapid Dev Team installation status and completeness

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
    #[serde(default)]
    pub content_hash: Option<String>,
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
    pub config_agent_bindings: BTreeMap<String, String>, // agent_name -> model
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

/// Resolves global OpenCode config directory with environment variable priority
pub fn get_global_opencode_dir() -> PathBuf {
    // 1. OPENCODE_CONFIG_DIR env
    if let Ok(val) = std::env::var("OPENCODE_CONFIG_DIR") {
        let p = PathBuf::from(val.trim());
        if !p.as_os_str().is_empty() {
            return p;
        }
    }

    // 2. OPENCODE_CONFIG env (if points to file, take parent; if dir, take dir)
    if let Ok(val) = std::env::var("OPENCODE_CONFIG") {
        let p = PathBuf::from(val.trim());
        if p.is_file() {
            if let Some(parent) = p.parent() {
                return parent.to_path_buf();
            }
        } else if !p.as_os_str().is_empty() {
            return p;
        }
    }

    // 3. 选择真正包含 OpenCode 配置或 Agent 的候选目录。
    // Windows 的 %APPDATA% 可能只有 EBWebView，不能仅凭目录存在就选中。
    let mut candidates = Vec::new();
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".config").join("opencode"));
    }
    if let Some(config_dir) = dirs::config_dir() {
        candidates.push(config_dir.join("opencode"));
    }
    if let Some(candidate) = candidates.iter().find(|p| {
        p.join("opencode.jsonc").is_file()
            || p.join("opencode.json").is_file()
            || p.join("agents").is_dir()
            || p.join("agent").is_dir()
    }) {
        return candidate.clone();
    }
    if let Some(candidate) = candidates.into_iter().next() {
        return candidate;
    }

    PathBuf::from(".config/opencode")
}

pub fn scan_environment(project_path: Option<&Path>, use_project: bool) -> ScanResult {
    let (target_scope, base_dir) = if use_project {
        match project_path {
            Some(p) => {
                let dot_opencode = p.join(".opencode");
                ("project".to_string(), dot_opencode)
            }
            None => ("global".to_string(), get_global_opencode_dir()),
        }
    } else {
        ("global".to_string(), get_global_opencode_dir())
    };

    // 1. Locate config files
    let mut config_files_to_read = Vec::new();
    let mut primary_config_path = None;

    if target_scope == "project" {
        if let Some(p) = project_path {
            let candidates = [
                p.join(".opencode").join("opencode.json"),
                p.join(".opencode").join("opencode.jsonc"),
                p.join("opencode.json"),
                p.join("opencode.jsonc"),
            ];
            for cand in &candidates {
                if cand.exists() {
                    if primary_config_path.is_none() {
                        primary_config_path = Some(cand.clone());
                    }
                    config_files_to_read.push(cand.clone());
                }
            }
        }
        // Include global config as inherited fallback for providers/models
        let global_dir = get_global_opencode_dir();
        let g_jsonc = global_dir.join("opencode.jsonc");
        let g_json = global_dir.join("opencode.json");
        if g_json.exists() {
            config_files_to_read.push(g_json);
        }
        if g_jsonc.exists() {
            config_files_to_read.push(g_jsonc);
        }
    } else {
        let json_path = base_dir.join("opencode.json");
        let jsonc_path = base_dir.join("opencode.jsonc");
        if json_path.exists() {
            primary_config_path = Some(json_path.clone());
            config_files_to_read.push(json_path);
        }
        if jsonc_path.exists() {
            if primary_config_path.is_none() {
                primary_config_path = Some(jsonc_path.clone());
            }
            config_files_to_read.push(jsonc_path);
        }
    }

    // 2. Parse providers, models, and agent bindings (NO CREDENTIALS EXTRACTED)
    let mut models_map: BTreeMap<String, ModelOption> = BTreeMap::new();
    let mut providers_set: BTreeSet<String> = BTreeSet::new();
    let mut config_agent_bindings: BTreeMap<String, String> = BTreeMap::new();

    for cf in &config_files_to_read {
        if let Ok(raw_content) = std::fs::read_to_string(cf) {
            let cleaned = strip_jsonc_comments_and_trailing_commas(&raw_content);
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&cleaned) {
                // Read provider models
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

                // Read agent bindings in opencode.json/jsonc (supports both "agent" and "agents")
                for key in ["agent", "agents"] {
                    if let Some(agent_obj) = val.get(key).and_then(|a| a.as_object()) {
                        for (agent_k, agent_v) in agent_obj {
                            if let Some(m) = agent_v.get("model").and_then(|m| m.as_str()) {
                                if !m.trim().is_empty() {
                                    config_agent_bindings
                                        .entry(agent_k.clone())
                                        .or_insert_with(|| m.trim().to_string());
                                }
                            } else if let Some(m_str) = agent_v.as_str() {
                                if !m_str.trim().is_empty() {
                                    config_agent_bindings
                                        .entry(agent_k.clone())
                                        .or_insert_with(|| m_str.trim().to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Scan agents (Project agents take precedence over Global agents)
    let mut found_agents: BTreeMap<String, AgentStatus> = BTreeMap::new();

    let scan_dirs = if target_scope == "project" {
        let p = project_path.unwrap_or_else(|| Path::new(""));
        vec![
            (p.join(".opencode").join("agents"), "project"),
            (p.join(".opencode").join("agent"), "project"),
            (p.join("agents"), "project"),
            (p.join("agent"), "project"),
            (get_global_opencode_dir().join("agents"), "global"),
            (get_global_opencode_dir().join("agent"), "global"),
        ]
    } else {
        vec![
            (base_dir.join("agents"), "global"),
            (base_dir.join("agent"), "global"),
        ]
    };

    for (adir, src_label) in scan_dirs {
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
                        continue; // Already found higher priority (project over global)
                    }

                    let content_bytes = std::fs::read(&path).unwrap_or_default();
                    let content_hash = crate::writer::compute_sha256_hex(&content_bytes);
                    let content = String::from_utf8_lossy(&content_bytes);

                    let mtime = entry
                        .metadata()
                        .and_then(|m| m.modified())
                        .map(|t| {
                            t.duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs()
                        })
                        .unwrap_or(0);

                    let (mut current_model, mode, description) =
                        if let Some(fm) = parse_frontmatter(&content) {
                            (fm.model, fm.mode, fm.description)
                        } else {
                            (None, None, None)
                        };

                    // Fallback to opencode.json agent binding if frontmatter doesn't specify model
                    if current_model.is_none() {
                        if let Some(cfg_model) = config_agent_bindings.get(file_stem) {
                            current_model = Some(cfg_model.clone());
                        }
                    }

                    found_agents.insert(
                        file_stem.to_string(),
                        AgentStatus {
                            name: file_stem.to_string(),
                            file_name: path.file_name().unwrap().to_string_lossy().to_string(),
                            full_path: path.to_string_lossy().to_string(),
                            is_installed: true,
                            current_model,
                            source: src_label.to_string(),
                            mode,
                            description,
                            mtime,
                            content_hash: Some(content_hash),
                        },
                    );

                    // 保留 Agent 当前绑定，即便 Provider 模型清单未声明该模型。
                    if let Some(model_id) = found_agents
                        .get(file_stem)
                        .and_then(|agent| agent.current_model.as_deref())
                    {
                        if let Some((provider, model_name)) = model_id.split_once('/') {
                            providers_set.insert(provider.to_string());
                            models_map
                                .entry(model_id.to_string())
                                .or_insert(ModelOption {
                                    id: model_id.to_string(),
                                    provider: provider.to_string(),
                                    model_name: model_name.to_string(),
                                });
                        }
                    }
                }
            }
        }
    }

    // 配置文件中的 Agent 绑定也必须进入模型选择器，即使对应的 Agent 文件尚未安装。
    for model_id in config_agent_bindings.values() {
        if let Some((provider, model_name)) = model_id.split_once('/') {
            providers_set.insert(provider.to_string());
            models_map.entry(model_id.clone()).or_insert(ModelOption {
                id: model_id.clone(),
                provider: provider.to_string(),
                model_name: model_name.to_string(),
            });
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
    let command_installed = command_file.exists()
        || (target_scope == "project"
            && get_global_opencode_dir()
                .join("commands")
                .join("rapid-dev.md")
                .exists());
    let skill_dir = base_dir.join("skills").join("rapid-dev-team");
    let skill_installed = skill_dir.exists()
        || (target_scope == "project"
            && get_global_opencode_dir()
                .join("skills")
                .join("rapid-dev-team")
                .exists());

    let is_installed_complete = missing_agents.is_empty() && command_installed && skill_installed;
    let is_installed_partial = installed_count > 0 || command_installed || skill_installed;

    ScanResult {
        target_scope,
        opencode_dir: base_dir.to_string_lossy().to_string(),
        config_file_path: primary_config_path.map(|p| p.to_string_lossy().to_string()),
        providers: providers_set.into_iter().collect(),
        models: models_map.into_values().collect(),
        agents: found_agents.into_values().collect(),
        config_agent_bindings,
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
