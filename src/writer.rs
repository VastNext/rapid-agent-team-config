//! Atomic Transactional Model Config Writer
//!
//! Applies model changes to agent markdown files with:
//! 1. Complete backup generation in `.backups/rapid-team-<timestamp>/` with relative path preservation
//! 2. Content SHA-256 hash + mtime check to detect external changes
//! 3. Only YAML frontmatter `model:` modification (rejects files without valid frontmatter)
//! 4. Windows/POSIX atomic file replacement (fallback replace + rollback)
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
    #[serde(default)]
    pub expected_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffItem {
    pub agent_name: String,
    pub file_path: String,
    pub file_name: String,
    pub original_model: Option<String>,
    pub new_model: String,
    pub diff_preview: String,
    pub current_hash: Option<String>,
    pub backup_rel_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanResult {
    pub changes: Vec<DiffItem>,
    pub has_changes: bool,
    pub planned_backup_dir: String,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRecord {
    pub original_path: String,
    pub backup_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResult {
    pub success: bool,
    pub backup_dir: Option<String>,
    pub backup_files: Vec<BackupRecord>,
    pub changed_count: usize,
    pub modified_files: Vec<String>,
    pub message: String,
}

/// Computes SHA-256 hash of a byte slice for content integrity check
pub fn compute_sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    format!("{:x}", result)
}

/// Computes safe relative backup path for a file inside `base_dir`
pub fn compute_backup_relative_path(base_dir: &Path, file_path: &Path) -> PathBuf {
    if let Ok(rel) = file_path.strip_prefix(base_dir) {
        rel.to_path_buf()
    } else {
        // Fallback: use filename with parent name prefix to avoid collision
        let file_name = file_path.file_name().unwrap_or_default();
        let parent_name = file_path
            .parent()
            .and_then(|p| p.file_name())
            .unwrap_or_default();
        PathBuf::from(format!(
            "{}_{}",
            parent_name.to_string_lossy(),
            file_name.to_string_lossy()
        ))
    }
}

/// Generates diff previews and validation plan before applying
pub fn generate_change_plan(base_opencode_dir: &Path, items: &[ModelChangeItem]) -> PlanResult {
    let mut changes = Vec::new();
    let planned_backup_dir = format!(
        ".backups/rapid-team-{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );

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

        let current_hash = if let Ok(bytes) = fs::read(path) {
            Some(compute_sha256_hex(&bytes))
        } else {
            None
        };

        let backup_rel_path = compute_backup_relative_path(base_opencode_dir, path)
            .to_string_lossy()
            .to_string();

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
            current_hash,
            backup_rel_path,
        });
    }

    let has_changes = !changes.is_empty();
    PlanResult {
        changes,
        has_changes,
        planned_backup_dir,
        warning: None,
    }
}

/// Safe atomic replacement of destination file with source temporary file.
/// Handles Windows file locking / existing destination replacement properly.
fn atomic_replace(temp_path: &Path, dest_path: &Path) -> Result<(), String> {
    // Attempt standard rename first
    if fs::rename(temp_path, dest_path).is_ok() {
        return Ok(());
    }

    // On Windows, rename fails if destination exists.
    // Use safe remove + rename fallback
    let backup_swap = dest_path.with_extension("swap_rapid_bak");
    if dest_path.exists() {
        if let Err(e) = fs::rename(dest_path, &backup_swap) {
            let _ = fs::remove_file(temp_path);
            return Err(format!(
                "无法暂存原文件以供替换 {}: {}",
                dest_path.display(),
                e
            ));
        }
    }

    if let Err(e) = fs::rename(temp_path, dest_path) {
        // Rollback swap
        if backup_swap.exists() {
            let _ = fs::rename(&backup_swap, dest_path);
        }
        let _ = fs::remove_file(temp_path);
        return Err(format!("替换目标文件失败 {}: {}", dest_path.display(), e));
    }

    if backup_swap.exists() {
        let _ = fs::remove_file(&backup_swap);
    }

    Ok(())
}

/// Atomically apply changes with rollback on failure
pub fn apply_model_changes(
    base_opencode_dir: &Path,
    items: &[ModelChangeItem],
) -> Result<ApplyResult, String> {
    // Filter out items with no real changes or empty new model
    let mut valid_items = Vec::new();
    for item in items {
        let target = item.new_model.trim();
        if target.is_empty() {
            continue;
        }
        let orig = item.original_model.as_deref().unwrap_or("").trim();
        if orig == target {
            continue;
        }
        valid_items.push(item);
    }

    if valid_items.is_empty() {
        return Ok(ApplyResult {
            success: true,
            backup_dir: None,
            backup_files: Vec::new(),
            changed_count: 0,
            modified_files: Vec::new(),
            message: "没有任何改动需要应用".to_string(),
        });
    }

    // 1. Verify existence, frontmatter validity, hash and mtime of all target files
    let mut initial_contents: HashMap<PathBuf, (String, String)> = HashMap::new(); // path -> (content, hash)

    for item in &valid_items {
        let path = Path::new(&item.file_path);
        if !path.exists() {
            return Err(format!("目标文件不存在: {}", item.file_path));
        }

        let bytes =
            fs::read(path).map_err(|e| format!("无法读取待修改文件 {}: {}", item.file_path, e))?;
        let current_hash = compute_sha256_hex(&bytes);

        if let Some(ref exp_hash) = item.expected_hash {
            if !exp_hash.is_empty() && &current_hash != exp_hash {
                return Err(format!(
                    "文件内容已被外部修改 (哈希不匹配): {}\n请刷新页面重新检查后再试",
                    item.file_path
                ));
            }
        }

        if item.expected_mtime > 0 {
            if let Ok(meta) = path.metadata() {
                if let Ok(mtime) = meta.modified() {
                    let current_mtime = mtime
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    // Allow 1s tolerance for filesystem timestamp precision
                    if current_mtime.abs_diff(item.expected_mtime) > 2 {
                        return Err(format!(
                            "文件修改时间已在外部改变 (mtime 不匹配): {}\n请刷新页面重新检查",
                            item.file_path
                        ));
                    }
                }
            }
        }

        let content_str = String::from_utf8(bytes)
            .map_err(|_| format!("文件不是合法的 UTF-8 编码: {}", item.file_path))?;

        // Pre-validate that frontmatter can be safely updated
        if let Err(e) = replace_frontmatter_model(&content_str, &item.new_model) {
            return Err(format!("文件拒绝修改 {}: {}", item.file_path, e));
        }

        initial_contents.insert(path.to_path_buf(), (content_str, current_hash));
    }

    // 2. Create backup folder preserving relative directory hierarchy
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
    let mut backup_records = Vec::new();

    for item in &valid_items {
        let orig_path = PathBuf::from(&item.file_path);
        let rel_backup = compute_backup_relative_path(base_opencode_dir, &orig_path);
        let backup_file = backup_dir.join(&rel_backup);

        if let Some(parent) = backup_file.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("创建备份子目录失败 {}: {}", parent.display(), e))?;
            }
        }

        let (content, _) = initial_contents.get(&orig_path).unwrap();
        if let Err(e) = fs::write(&backup_file, content) {
            return Err(format!("备份文件写入失败 {}: {}", orig_path.display(), e));
        }

        backup_records.push(BackupRecord {
            original_path: orig_path.to_string_lossy().to_string(),
            backup_path: backup_file.to_string_lossy().to_string(),
        });

        backed_up_files.insert(orig_path, (backup_file, content.clone()));
    }

    // 3. Atomically write each modified file (temp file + atomic_replace)
    let mut modified_paths = Vec::new();
    let mut apply_error = None;

    for item in &valid_items {
        let orig_path = PathBuf::from(&item.file_path);
        let Some((_, orig_content)) = backed_up_files.get(&orig_path) else {
            continue;
        };

        let new_content = match replace_frontmatter_model(orig_content, &item.new_model) {
            Ok(c) => c,
            Err(e) => {
                apply_error = Some(format!(
                    "替换 Frontmatter 失败 {}: {}",
                    orig_path.display(),
                    e
                ));
                break;
            }
        };

        let tmp_path = orig_path.with_extension("tmp_rapid_cfg");

        if let Err(e) = fs::write(&tmp_path, &new_content) {
            apply_error = Some(format!("写入临时文件失败: {}", e));
            break;
        }

        if let Err(e) = atomic_replace(&tmp_path, &orig_path) {
            apply_error = Some(format!("原子替换失败 {}: {}", orig_path.display(), e));
            break;
        }

        modified_paths.push(orig_path.to_string_lossy().to_string());
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
        backup_files: backup_records,
        changed_count: modified_paths.len(),
        modified_files: modified_paths.clone(),
        message: format!(
            "成功更新 {} 个 Agent 模型配置！备份已保存至 {}",
            modified_paths.len(),
            backup_dir.to_string_lossy()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_change_plan_filters_unchanged() {
        let base_dir = Path::new("/tmp/opencode");
        let items = vec![
            ModelChangeItem {
                agent_name: "rapid-dev-team".to_string(),
                file_path: "/tmp/opencode/agents/rapid-dev-team.md".to_string(),
                original_model: Some("old/model".to_string()),
                new_model: "new/model".to_string(),
                expected_mtime: 0,
                expected_hash: None,
            },
            ModelChangeItem {
                agent_name: "rapid-ui".to_string(),
                file_path: "/tmp/opencode/agents/rapid-ui.md".to_string(),
                original_model: Some("same/model".to_string()),
                new_model: "same/model".to_string(),
                expected_mtime: 0,
                expected_hash: None,
            },
            ModelChangeItem {
                agent_name: "rapid-scout".to_string(),
                file_path: "/tmp/opencode/agents/rapid-scout.md".to_string(),
                original_model: Some("any/model".to_string()),
                new_model: "".to_string(),
                expected_mtime: 0,
                expected_hash: None,
            },
        ];

        let plan = generate_change_plan(base_dir, &items);
        assert!(plan.has_changes);
        assert_eq!(plan.changes.len(), 1);
        assert_eq!(plan.changes[0].agent_name, "rapid-dev-team");
        assert!(plan.changes[0].diff_preview.contains("- model: old/model"));
        assert!(plan.changes[0].diff_preview.contains("+ model: new/model"));
    }

    #[test]
    fn test_backup_relative_path_distinct_directories() {
        let base = Path::new("/app/opencode");
        let p1 = Path::new("/app/opencode/agents/rapid.md");
        let p2 = Path::new("/app/opencode/commands/rapid.md");

        let r1 = compute_backup_relative_path(base, p1);
        let r2 = compute_backup_relative_path(base, p2);

        assert_ne!(r1, r2);
        assert_eq!(r1, PathBuf::from("agents/rapid.md"));
        assert_eq!(r2, PathBuf::from("commands/rapid.md"));
    }
}
