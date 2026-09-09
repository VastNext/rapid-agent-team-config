//! Atomic Transactional Model Config Writer
//!
//! Applies model changes to agent markdown files with:
//! 1. Complete backup generation in `.backups/<timestamp>/`
//! 2. Concurrency mtime check to avoid overwriting external changes
//! 3. Only YAML frontmatter `model:` modification
//! 4. Atomic write via temporary file + rename
//! 5. Automatic rollback on any failure

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::frontmatter::replace_frontmatter_model;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelChangeItem {
    pub agent_name: String,
    pub file_path: String,
    pub original_model: Option<String>,
    pub new_model: String,
    pub expected_mtime: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffItem {
    pub agent_name: String,
    pub file_path: String,
    pub file_name: String,
    pub original_model: Option<String>,
    pub new_model: String,
    pub diff_preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanResult {
    pub changes: Vec<DiffItem>,
    pub has_changes: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResult {
    pub success: bool,
    pub backup_dir: Option<String>,
    pub changed_count: usize,
    pub message: String,
}

/// Generates diff previews and validation plan before applying
pub fn generate_change_plan(items: &[ModelChangeItem]) -> PlanResult {
    let mut changes = Vec::new();

    for item in items {
        if item.new_model.trim().is_empty() {
            continue;
        }
        let orig = item.original_model.as_deref().unwrap_or("").trim();
        let target = item.new_model.trim();
        if orig == target {
            continue;
        }

        let path = Path::new(&item.file_path);
        let file_name = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{}.md", item.agent_name));

        let diff_preview = format!(
            "- model: {}\n+ model: {}",
            if orig.is_empty() { "(none)" } else { orig },
            target
        );

        changes.push(DiffItem {
            agent_name: item.agent_name.clone(),
            file_path: item.file_path.clone(),
            file_name,
            original_model: item.original_model.clone(),
            new_model: target.to_string(),
            diff_preview,
        });
    }

    let has_changes = !changes.is_empty();
    PlanResult {
        changes,
        has_changes,
        warning: None,
    }
}

/// Atomically apply changes with rollback on failure
pub fn apply_model_changes(
    base_opencode_dir: &Path,
    items: &[ModelChangeItem],
) -> Result<ApplyResult, String> {
    if items.is_empty() {
        return Ok(ApplyResult {
            success: true,
            backup_dir: None,
            changed_count: 0,
            message: "没有任何改动需要应用".to_string(),
        });
    }

    // 1. Verify existence and mtime of all target files
    for item in items {
        let path = Path::new(&item.file_path);
        if !path.exists() {
            return Err(format!("目标文件不存在: {}", item.file_path));
        }

        if item.expected_mtime > 0 {
            if let Ok(meta) = path.metadata() {
                if let Ok(mtime) = meta.modified() {
                    let current_mtime = mtime
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    if current_mtime != item.expected_mtime {
                        return Err(format!(
                            "文件已在外部被修改 (mtime 不匹配): {}\n请刷新后重试",
                            item.file_path
                        ));
                    }
                }
            }
        }
    }

    // 2. Create backup folder
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup_dir = base_opencode_dir
        .join(".backups")
        .join(format!("rapid-team-{}", timestamp));

    if let Err(e) = fs::create_dir_all(&backup_dir) {
        return Err(format!("无法创建备份目录: {}", e));
    }

    let mut backed_up_files: HashMap<PathBuf, (PathBuf, String)> = HashMap::new();

    for item in items {
        let orig_path = PathBuf::from(&item.file_path);
        let file_name = orig_path.file_name().unwrap();
        let backup_file = backup_dir.join(file_name);

        match fs::read_to_string(&orig_path) {
            Ok(content) => {
                if let Err(e) = fs::write(&backup_file, &content) {
                    return Err(format!("备份文件写入失败 {}: {}", orig_path.display(), e));
                }
                backed_up_files.insert(orig_path, (backup_file, content));
            }
            Err(e) => {
                return Err(format!("读取待备份文件失败 {}: {}", orig_path.display(), e));
            }
        }
    }

    // 3. Atomically write each modified file (temp file + rename)
    let mut modified_paths = Vec::new();
    let mut apply_error = None;

    for item in items {
        let orig_path = PathBuf::from(&item.file_path);
        let Some((_, orig_content)) = backed_up_files.get(&orig_path) else {
            continue;
        };

        let new_content = replace_frontmatter_model(orig_content, &item.new_model);
        let tmp_path = orig_path.with_extension("tmp_rapid_cfg");

        if let Err(e) = fs::write(&tmp_path, &new_content) {
            apply_error = Some(format!("写入临时文件失败: {}", e));
            break;
        }

        if let Err(e) = fs::rename(&tmp_path, &orig_path) {
            let _ = fs::remove_file(&tmp_path);
            apply_error = Some(format!("原子重命名失败 {}: {}", orig_path.display(), e));
            break;
        }

        modified_paths.push(orig_path);
    }

    // 4. Handle Rollback if any error occurred
    if let Some(err) = apply_error {
        for (orig, (backup, _)) in &backed_up_files {
            let _ = fs::copy(backup, orig);
        }
        return Err(format!("应用失败并已自动回滚全部更改: {}", err));
    }

    Ok(ApplyResult {
        success: true,
        backup_dir: Some(backup_dir.to_string_lossy().to_string()),
        changed_count: modified_paths.len(),
        message: format!(
            "成功更新 {} 个 Agent 模型配置！请重启 OpenCode 以加载最新设定。",
            modified_paths.len()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_change_plan() {
        let items = vec![
            ModelChangeItem {
                agent_name: "rapid-dev-team".to_string(),
                file_path: "agents/rapid-dev-team.md".to_string(),
                original_model: Some("old/model".to_string()),
                new_model: "new/model".to_string(),
                expected_mtime: 0,
            },
            ModelChangeItem {
                agent_name: "rapid-ui".to_string(),
                file_path: "agents/rapid-ui.md".to_string(),
                original_model: Some("same/model".to_string()),
                new_model: "same/model".to_string(),
                expected_mtime: 0,
            },
        ];

        let plan = generate_change_plan(&items);
        assert!(plan.has_changes);
        assert_eq!(plan.changes.len(), 1);
        assert_eq!(plan.changes[0].agent_name, "rapid-dev-team");
        assert!(plan.changes[0].diff_preview.contains("- model: old/model"));
        assert!(plan.changes[0].diff_preview.contains("+ model: new/model"));
    }
}
