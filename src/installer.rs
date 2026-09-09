//! Safe Package Unpacker & Directory Installer
//!
//! Features:
//! 1. Zip-Slip directory traversal protection
//! 2. Strict Rapid Dev Team file whitelist (agents/commands/skills/templates/team.config.json)
//! 3. Manifest parsing & validation (name/version/agents/commands/skills)
//! 4. Dry-run pre-installation plan generation
//! 5. Transactional backup of existing destination files before installation
//! 6. Full atomic rollback (restore overwritten files + delete newly created files) on any error
//! 7. Full parity between ZIP installer and Local Directory installer

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfigManifest {
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub agents: Option<Vec<String>>,
    pub commands: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallPlanItem {
    pub relative_path: String,
    pub is_overwrite: bool,
    pub file_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallPlan {
    pub total_files: usize,
    pub overwrite_count: usize,
    pub manifest_info: Option<TeamConfigManifest>,
    pub items: Vec<InstallPlanItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResult {
    pub success: bool,
    pub backup_dir: Option<String>,
    pub installed_files: Vec<String>,
    pub message: String,
}

const ALLOWED_EXTENSIONS: &[&str] = &[
    "md", "json", "jsonc", "yaml", "yml", "png", "svg", "txt", "js", "ts",
];

/// Checks whether a path is safe from Zip Slip vulnerability and stays strictly within `dest_dir`
pub fn is_safe_zip_path(dest_dir: &Path, entry_name: &str) -> Option<PathBuf> {
    if entry_name.starts_with('/') || entry_name.starts_with('\\') {
        return None;
    }

    if entry_name.len() >= 2 && entry_name.as_bytes()[1] == b':' {
        return None;
    }

    let joined = dest_dir.join(entry_name);
    let mut normalized = PathBuf::new();
    for comp in joined.components() {
        match comp {
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            std::path::Component::CurDir => {}
            _ => normalized.push(comp),
        }
    }

    if normalized.starts_with(dest_dir) {
        Some(normalized)
    } else {
        None
    }
}

/// Validates that a file strictly belongs to the Rapid Dev Team architecture
pub fn is_allowed_team_file(rel_path: &Path) -> bool {
    let components: Vec<_> = rel_path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if components.is_empty() {
        return false;
    }

    // Top-level allowed files
    if components.len() == 1 {
        let name = &components[0];
        return name == "team.config.json"
            || name == "README.md"
            || name == "LICENSE"
            || name == "CLAUDE.md"
            || name == "AGENTS.md";
    }

    let top_dir = components[0].as_str();
    let ext = rel_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        return false;
    }

    match top_dir {
        "agents" | "agent" => {
            // Agents must be rapid-* or defined in rapid team
            let file_name = components.last().map(|s| s.as_str()).unwrap_or("");
            file_name.starts_with("rapid-") && file_name.ends_with(".md")
        }
        "commands" | "command" => {
            let file_name = components.last().map(|s| s.as_str()).unwrap_or("");
            file_name.starts_with("rapid-") && file_name.ends_with(".md")
        }
        "skills" => {
            // Skills must be in rapid-* subdirectory or rapid team tool
            if components.len() >= 2 {
                let skill_dir = &components[1];
                skill_dir.starts_with("rapid-") || skill_dir == "rapid-dev-team"
            } else {
                false
            }
        }
        "templates" => {
            let file_name = components.last().map(|s| s.as_str()).unwrap_or("");
            file_name.starts_with("rapid-")
        }
        _ => false,
    }
}

/// Validates manifest contents for Rapid Dev Team
///
/// Requirements:
/// - `name` must contain "rapid" (team identification)
/// - `version` must be present and non-empty
/// - `agents` / `commands` / `skills` list entries, when declared, must
///   match the `rapid-*` naming convention so that only Rapid Team
///   components can be declared by the manifest.
pub fn validate_manifest(manifest: &TeamConfigManifest) -> Result<(), String> {
    let name = manifest.name.as_deref().unwrap_or("");
    let n = name.to_lowercase();
    if !n.contains("rapid") {
        return Err(format!("Manifest 名称 '{}' 与 Rapid Dev Team 不匹配", name));
    }

    match manifest.version.as_deref() {
        Some(v) if !v.trim().is_empty() => {}
        _ => return Err("Manifest 缺少 version 字段，无法确认版本完整性".to_string()),
    }

    let check_list = |items: Option<&Vec<String>>, kind: &str| -> Result<(), String> {
        if let Some(list) = items {
            for entry in list {
                if !entry.starts_with("rapid-") {
                    return Err(format!(
                        "Manifest {} 清单中的 '{}' 不符合 rapid-* 命名规范",
                        kind, entry
                    ));
                }
            }
        }
        Ok(())
    };
    check_list(manifest.agents.as_ref(), "agents")?;
    check_list(manifest.commands.as_ref(), "commands")?;
    check_list(manifest.skills.as_ref(), "skills")?;

    Ok(())
}

/// Reads and validates the team manifest from a JSON string.
/// Returns Err if the manifest is absent, unparseable, or fails validation.
pub fn parse_and_validate_manifest(json_str: &str) -> Result<TeamConfigManifest, String> {
    let manifest: TeamConfigManifest =
        serde_json::from_str(json_str).map_err(|e| format!("team.config.json 解析失败: {}", e))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// Computes common prefix in ZIP archive (e.g. `rapid-agent-team-main/`)
pub fn find_zip_root_prefix(entry_names: &[String]) -> String {
    for name in entry_names {
        if name.ends_with("team.config.json") && name != "team.config.json" {
            let prefix = &name[..name.len() - "team.config.json".len()];
            return prefix.to_string();
        }
    }
    for name in entry_names {
        if (name.contains("agents/rapid-dev-team.md") || name.contains("commands/rapid-dev.md"))
            && !name.starts_with("agents/")
            && !name.starts_with("commands/")
        {
            if let Some(pos) = name.find("agents/").or_else(|| name.find("commands/")) {
                return name[..pos].to_string();
            }
        }
    }
    String::new()
}

/// Generates dry-run installation plan and validates manifest from ZIP
pub fn generate_zip_install_plan(
    zip_path: &Path,
    target_opencode_dir: &Path,
) -> Result<InstallPlan, String> {
    if !zip_path.exists() {
        return Err(format!("ZIP 文件不存在: {}", zip_path.display()));
    }

    let file = File::open(zip_path).map_err(|e| format!("无法打开 ZIP 文件: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("ZIP 格式无效或损坏: {}", e))?;

    let mut entry_names = Vec::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("读取压缩包条目失败: {}", e))?;
        entry_names.push(entry.name().to_string());
    }

    let root_prefix = find_zip_root_prefix(&entry_names);
    let mut plan_items = Vec::new();
    let mut manifest_info = None;
    let mut has_team_files = false;
    let mut overwrite_count = 0;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let orig_name = entry.name().to_string();

        if entry.is_dir() {
            continue;
        }

        let relative_name = if !root_prefix.is_empty() && orig_name.starts_with(&root_prefix) {
            &orig_name[root_prefix.len()..]
        } else {
            &orig_name
        };

        if relative_name.is_empty() {
            continue;
        }

        let rel_path = Path::new(relative_name);
        if !is_allowed_team_file(rel_path) {
            continue;
        }

        let Some(out_path) = is_safe_zip_path(target_opencode_dir, relative_name) else {
            return Err(format!("发现潜在越权路径 (Zip Slip): {}", relative_name));
        };

        if relative_name == "team.config.json" {
            let mut manifest_str = String::new();
            if entry.read_to_string(&mut manifest_str).is_ok() {
                manifest_info = Some(parse_and_validate_manifest(&manifest_str)?);
            } else {
                return Err("无法读取 team.config.json 内容".to_string());
            }
        }

        let is_overwrite = out_path.exists();
        if is_overwrite {
            overwrite_count += 1;
        }

        has_team_files = true;
        plan_items.push(InstallPlanItem {
            relative_path: relative_name.to_string(),
            is_overwrite,
            file_size: entry.size(),
        });
    }

    if !has_team_files {
        return Err(
            "压缩包内未找到有效的 Rapid Dev Team 构件 (agents/commands/skills/team.config.json)"
                .to_string(),
        );
    }

    if manifest_info.is_none() {
        return Err(
            "压缩包内缺少或未通过校验的 team.config.json（必须包含名称、版本与 rapid-* 清单）"
                .to_string(),
        );
    }

    Ok(InstallPlan {
        total_files: plan_items.len(),
        overwrite_count,
        manifest_info,
        items: plan_items,
    })
}

/// Unpacks a team package ZIP archive with transactional backup, verification, and rollback
pub fn install_from_zip(
    zip_path: &Path,
    target_opencode_dir: &Path,
) -> Result<InstallResult, String> {
    let plan = generate_zip_install_plan(zip_path, target_opencode_dir)?;

    let file = File::open(zip_path).map_err(|e| format!("无法打开 ZIP 文件: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("ZIP 格式无效或损坏: {}", e))?;

    let mut entry_names = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| e.to_string())?;
        entry_names.push(entry.name().to_string());
    }
    let root_prefix = find_zip_root_prefix(&entry_names);

    // 1. Transactional backup of files that will be overwritten
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup_dir = target_opencode_dir
        .join(".backups")
        .join(format!("install-backup-{}", timestamp));

    let mut backed_up_files = Vec::new(); // (original_path, backup_path)
    let mut newly_created_files = Vec::new();

    for item in &plan.items {
        let dest_file = target_opencode_dir.join(&item.relative_path);
        if item.is_overwrite {
            let backup_file = backup_dir.join(&item.relative_path);
            if let Some(parent) = backup_file.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("无法创建备份目录: {}", e))?;
            }
            if let Err(e) = fs::copy(&dest_file, &backup_file) {
                return Err(format!("备份现有文件失败 {}: {}", dest_file.display(), e));
            }
            backed_up_files.push((dest_file, backup_file));
        } else {
            newly_created_files.push(dest_file);
        }
    }

    // 2. Extract into target directory
    let mut installed_files = Vec::new();
    let mut extract_error = None;

    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(e) => {
                extract_error = Some(format!("读取条目失败: {}", e));
                break;
            }
        };

        if entry.is_dir() {
            continue;
        }

        let orig_name = entry.name().to_string();
        let relative_name = if !root_prefix.is_empty() && orig_name.starts_with(&root_prefix) {
            &orig_name[root_prefix.len()..]
        } else {
            &orig_name
        };

        if relative_name.is_empty() {
            continue;
        }

        let rel_path = Path::new(relative_name);
        if !is_allowed_team_file(rel_path) {
            continue;
        }

        let Some(out_path) = is_safe_zip_path(target_opencode_dir, relative_name) else {
            extract_error = Some(format!("非法路径条目: {}", relative_name));
            break;
        };

        if let Some(parent) = out_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                extract_error = Some(format!("创建目录失败 {}: {}", parent.display(), e));
                break;
            }
        }

        let temp_out = out_path.with_extension("tmp_rapid_install");
        match File::create(&temp_out) {
            Ok(mut f) => {
                if let Err(e) = io::copy(&mut entry, &mut f) {
                    let _ = fs::remove_file(&temp_out);
                    extract_error = Some(format!("写入解压文件失败 {}: {}", out_path.display(), e));
                    break;
                }
            }
            Err(e) => {
                extract_error = Some(format!("无法创建临时文件 {}: {}", temp_out.display(), e));
                break;
            }
        }

        if out_path.exists() {
            let _ = fs::remove_file(&out_path);
        }
        if let Err(e) = fs::rename(&temp_out, &out_path) {
            let _ = fs::remove_file(&temp_out);
            extract_error = Some(format!("重命名安装文件失败 {}: {}", out_path.display(), e));
            break;
        }

        installed_files.push(relative_name.to_string());
    }

    // 3. Rollback on failure (delete newly created files + restore backed up files)
    if let Some(err) = extract_error {
        // Remove newly created files
        for created in &newly_created_files {
            if created.exists() {
                let _ = fs::remove_file(created);
            }
        }
        // Restore overwritten files
        for (orig, backup) in &backed_up_files {
            let _ = fs::copy(backup, orig);
        }
        return Err(format!(
            "安装失败，已自动回滚 (恢复原文件并清理新增文件): {}",
            err
        ));
    }

    Ok(InstallResult {
        success: true,
        backup_dir: if backed_up_files.is_empty() {
            None
        } else {
            Some(backup_dir.to_string_lossy().to_string())
        },
        installed_files,
        message: format!(
            "Rapid Dev Team 安装成功！共部署 {} 个组件",
            plan.total_files
        ),
    })
}

/// Collects allowed files from local directory recursively
fn scan_local_allowed_files(
    src_root: &Path,
    current_dir: &Path,
    collected: &mut Vec<(PathBuf, PathBuf)>, // (absolute_src, relative_path)
) -> Result<(), String> {
    if !current_dir.exists() || !current_dir.is_dir() {
        return Ok(());
    }

    let entries = fs::read_dir(current_dir)
        .map_err(|e| format!("读取目录失败 {}: {}", current_dir.display(), e))?;

    for entry_res in entries {
        let entry = entry_res.map_err(|e| format!("读取条目失败: {}", e))?;
        let path = entry.path();
        let rel = path
            .strip_prefix(src_root)
            .map_err(|e| e.to_string())?
            .to_path_buf();

        if path.is_dir() {
            scan_local_allowed_files(src_root, &path, collected)?;
        } else if is_allowed_team_file(&rel) {
            collected.push((path, rel));
        }
    }
    Ok(())
}

/// Installs from local repository folder with manifest validation, backup and rollback
pub fn install_from_local_dir(
    source_dir: &Path,
    target_opencode_dir: &Path,
) -> Result<InstallResult, String> {
    if !source_dir.exists() || !source_dir.is_dir() {
        return Err(format!(
            "源目录不存在或不是文件夹: {}",
            source_dir.display()
        ));
    }

    // 1. Scan and collect allowed files
    let mut files_to_install = Vec::new();
    scan_local_allowed_files(source_dir, source_dir, &mut files_to_install)?;

    if files_to_install.is_empty() {
        return Err("源目录中未找到 agents/commands/skills 等 Rapid Team 构件".to_string());
    }

    // 2. team.config.json 必须存在且通过完整校验（名称/版本/清单），否则拒绝安装
    let manifest_path = source_dir.join("team.config.json");
    let manifest_str = fs::read_to_string(&manifest_path)
        .map_err(|_| "源目录缺少 team.config.json，拒绝安装非 Rapid Team 包".to_string())?;
    parse_and_validate_manifest(&manifest_str)?;

    // 3. Backup existing files before overwrite
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup_dir = target_opencode_dir
        .join(".backups")
        .join(format!("install-backup-local-{}", timestamp));

    let mut backed_up_files = Vec::new();
    let mut newly_created_files = Vec::new();

    for (_, rel) in &files_to_install {
        let dst_file = target_opencode_dir.join(rel);
        if dst_file.exists() {
            let backup_file = backup_dir.join(rel);
            if let Some(parent) = backup_file.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("无法创建备份目录: {}", e))?;
            }
            if let Err(e) = fs::copy(&dst_file, &backup_file) {
                return Err(format!("备份现有文件失败 {}: {}", dst_file.display(), e));
            }
            backed_up_files.push((dst_file, backup_file));
        } else {
            newly_created_files.push(dst_file);
        }
    }

    // 4. Copy each file with temp-file atomic replacement
    let mut installed_files = Vec::new();
    let mut install_error = None;

    for (src_path, rel) in &files_to_install {
        let dst_path = target_opencode_dir.join(rel);
        if let Some(parent) = dst_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                install_error = Some(format!("创建目录失败 {}: {}", parent.display(), e));
                break;
            }
        }

        let temp_out = dst_path.with_extension("tmp_rapid_local_install");
        if let Err(e) = fs::copy(src_path, &temp_out) {
            let _ = fs::remove_file(&temp_out);
            install_error = Some(format!("复制文件失败 {}: {}", src_path.display(), e));
            break;
        }

        if dst_path.exists() {
            let _ = fs::remove_file(&dst_path);
        }
        if let Err(e) = fs::rename(&temp_out, &dst_path) {
            let _ = fs::remove_file(&temp_out);
            install_error = Some(format!("重命名文件失败 {}: {}", dst_path.display(), e));
            break;
        }

        installed_files.push(rel.to_string_lossy().to_string());
    }

    // 5. Rollback on failure
    if let Some(err) = install_error {
        for created in &newly_created_files {
            if created.exists() {
                let _ = fs::remove_file(created);
            }
        }
        for (orig, backup) in &backed_up_files {
            let _ = fs::copy(backup, orig);
        }
        return Err(format!(
            "本地目录安装失败，已自动回滚 (恢复原文件并清理新增文件): {}",
            err
        ));
    }

    Ok(InstallResult {
        success: true,
        backup_dir: if backed_up_files.is_empty() {
            None
        } else {
            Some(backup_dir.to_string_lossy().to_string())
        },
        installed_files,
        message: format!(
            "从本地文件夹安装 Rapid Dev Team 成功！共部署 {} 个组件",
            files_to_install.len()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zip_slip_prevention() {
        let dest = Path::new("/safe/target/dir");
        assert!(is_safe_zip_path(dest, "agents/rapid-dev-team.md").is_some());
        assert!(is_safe_zip_path(dest, "commands/rapid-dev.md").is_some());
        assert!(is_safe_zip_path(dest, "../evil.md").is_none());
        assert!(is_safe_zip_path(dest, "agents/../../evil.md").is_none());
        assert!(is_safe_zip_path(dest, "/etc/passwd").is_none());
        assert!(is_safe_zip_path(dest, "\\Windows\\System32").is_none());
        assert!(is_safe_zip_path(dest, "C:\\Windows\\evil.exe").is_none());
    }

    #[test]
    fn test_allowed_team_files_whitelist() {
        assert!(is_allowed_team_file(Path::new("team.config.json")));
        assert!(is_allowed_team_file(Path::new("README.md")));
        assert!(is_allowed_team_file(Path::new("agents/rapid-dev-team.md")));
        assert!(is_allowed_team_file(Path::new("commands/rapid-dev.md")));
        assert!(is_allowed_team_file(Path::new(
            "skills/rapid-dev-team/SKILL.md"
        )));

        // Rejected: arbitrary non-rapid files
        assert!(!is_allowed_team_file(Path::new("evil.exe")));
        assert!(!is_allowed_team_file(Path::new("agents/arbitrary.md")));
        assert!(!is_allowed_team_file(Path::new("commands/other.md")));
        assert!(!is_allowed_team_file(Path::new("random_dir/script.sh")));
        assert!(!is_allowed_team_file(Path::new("agents/malicious.dll")));
    }

    #[test]
    fn test_validate_manifest() {
        let valid = TeamConfigManifest {
            name: Some("Rapid Dev Team".to_string()),
            version: Some("1.0.0".to_string()),
            description: Some("Rapid Agent Team".to_string()),
            agents: Some(vec!["rapid-dev-team".to_string()]),
            commands: Some(vec!["rapid-dev".to_string()]),
            skills: None,
        };
        assert!(validate_manifest(&valid).is_ok());

        let invalid = TeamConfigManifest {
            name: Some("Unrelated Hacker Package".to_string()),
            version: Some("1.0.0".to_string()),
            description: None,
            agents: None,
            commands: None,
            skills: None,
        };
        assert!(validate_manifest(&invalid).is_err());
    }

    #[test]
    fn test_validate_manifest_requires_version() {
        let no_version = TeamConfigManifest {
            name: Some("Rapid Dev Team".to_string()),
            version: None,
            description: None,
            agents: None,
            commands: None,
            skills: None,
        };
        assert!(validate_manifest(&no_version).is_err());

        let empty_version = TeamConfigManifest {
            name: Some("Rapid Dev Team".to_string()),
            version: Some("   ".to_string()),
            description: None,
            agents: None,
            commands: None,
            skills: None,
        };
        assert!(validate_manifest(&empty_version).is_err());
    }

    #[test]
    fn test_validate_manifest_rejects_non_rapid_list_entries() {
        let bad_agents = TeamConfigManifest {
            name: Some("Rapid Dev Team".to_string()),
            version: Some("1.0.0".to_string()),
            description: None,
            agents: Some(vec!["random-agent".to_string()]),
            commands: None,
            skills: None,
        };
        assert!(validate_manifest(&bad_agents).is_err());

        let bad_commands = TeamConfigManifest {
            name: Some("Rapid Dev Team".to_string()),
            version: Some("1.0.0".to_string()),
            description: None,
            agents: None,
            commands: Some(vec!["deploy".to_string()]),
            skills: None,
        };
        assert!(validate_manifest(&bad_commands).is_err());

        let bad_skills = TeamConfigManifest {
            name: Some("Rapid Dev Team".to_string()),
            version: Some("1.0.0".to_string()),
            description: None,
            agents: None,
            commands: None,
            skills: Some(vec!["evil-skill".to_string()]),
        };
        assert!(validate_manifest(&bad_skills).is_err());
    }

    #[test]
    fn test_parse_and_validate_manifest() {
        let ok_json = r#"{"name":"Rapid Dev Team","version":"1.0.0","agents":["rapid-dev-team"]}"#;
        let parsed = parse_and_validate_manifest(ok_json).expect("must parse");
        assert_eq!(parsed.name.as_deref(), Some("Rapid Dev Team"));

        let bad_json = r#"{"name": "not json""#;
        assert!(parse_and_validate_manifest(bad_json).is_err());

        let invalid_team = r#"{"name":"Other Team","version":"1.0.0"}"#;
        assert!(parse_and_validate_manifest(invalid_team).is_err());
    }
}
