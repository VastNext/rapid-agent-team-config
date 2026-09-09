//! Safe Zip Unpacker with Zip-Slip Protection & Team Manifest Validator
//!
//! Validates:
//! 1. No directory traversal (`..` or absolute paths in archive)
//! 2. Must contain valid manifest (`team.config.json` or required `agents/*.md` & `commands/*.md`)
//! 3. Safe installation into OpenCode target directory

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
pub struct InstallResult {
    pub success: bool,
    pub installed_files: Vec<String>,
    pub message: String,
}

/// Checks whether a ZIP entry path is safe from Zip Slip vulnerability
pub fn is_safe_zip_path(dest_dir: &Path, entry_name: &str) -> Option<PathBuf> {
    // Disallow absolute paths in archive
    if entry_name.starts_with('/') || entry_name.starts_with('\\') {
        return None;
    }

    let joined = dest_dir.join(entry_name);
    // Normalize path components
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

    // Must be prefixed by target directory
    if normalized.starts_with(dest_dir) {
        Some(normalized)
    } else {
        None
    }
}

/// Unpacks a team package ZIP archive with strict Zip Slip validation
pub fn install_from_zip(
    zip_path: &Path,
    target_opencode_dir: &Path,
) -> Result<InstallResult, String> {
    if !zip_path.exists() {
        return Err(format!("ZIP 文件不存在: {}", zip_path.display()));
    }

    let file = File::open(zip_path).map_err(|e| format!("无法打开 ZIP 文件: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("ZIP 格式无效或损坏: {}", e))?;

    let mut installed_files = Vec::new();
    let mut has_agents = false;
    let mut has_commands = false;
    let mut has_manifest = false;

    // First pass: Pre-validate all entries for Zip Slip and content requirements
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("读取压缩包条目失败: {}", e))?;
        let entry_name = entry.name().to_string();

        if is_safe_zip_path(target_opencode_dir, &entry_name).is_none() {
            return Err(format!(
                "安全检测失败：发现危险的路径遍历 (Zip Slip) 条目: {}",
                entry_name
            ));
        }

        if entry_name == "team.config.json" || entry_name.ends_with("/team.config.json") {
            has_manifest = true;
        }
        if entry_name.contains("agents/") || entry_name.contains("agent/") {
            has_agents = true;
        }
        if entry_name.contains("commands/") {
            has_commands = true;
        }
    }

    if !has_manifest && (!has_agents || !has_commands) {
        return Err(
            "该 ZIP 压缩包不符合 Rapid Team 格式（缺少 team.config.json 或 agents/commands 目录）"
                .to_string(),
        );
    }

    // Find if files are inside a root folder inside zip (e.g. `rapid-dev-team-main/`)
    let mut root_prefix = String::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name();
        if name.ends_with("team.config.json") && name != "team.config.json" {
            let prefix = &name[..name.len() - "team.config.json".len()];
            root_prefix = prefix.to_string();
            break;
        }
        if (name.ends_with("commands/rapid-dev.md") || name.ends_with("agents/rapid-dev-team.md"))
            && !name.starts_with("agents/")
            && !name.starts_with("commands/")
        {
            if let Some(pos) = name.find("agents/").or_else(|| name.find("commands/")) {
                root_prefix = name[..pos].to_string();
                break;
            }
        }
    }

    // Second pass: Extract
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("解压条目读取失败: {}", e))?;
        let orig_name = entry.name().to_string();

        // Strip common archive prefix if exists
        let relative_name = if !root_prefix.is_empty() && orig_name.starts_with(&root_prefix) {
            &orig_name[root_prefix.len()..]
        } else {
            &orig_name
        };

        if relative_name.is_empty() {
            continue;
        }

        let Some(out_path) = is_safe_zip_path(target_opencode_dir, relative_name) else {
            return Err(format!("路径不安全: {}", relative_name));
        };

        if entry.is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("创建目录失败 {}: {}", out_path.display(), e))?;
        } else {
            if let Some(parent) = out_path.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("创建父目录失败 {}: {}", parent.display(), e))?;
                }
            }
            let mut out_file = File::create(&out_path)
                .map_err(|e| format!("创建文件失败 {}: {}", out_path.display(), e))?;
            io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("写入文件内容失败 {}: {}", out_path.display(), e))?;
            installed_files.push(relative_name.to_string());
        }
    }

    Ok(InstallResult {
        success: true,
        installed_files,
        message: "Rapid Dev Team 安装成功！".to_string(),
    })
}

/// Installs from local repository folder by copying `agents`, `commands`, `skills`, `team.config.json`
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

    let mut installed_files = Vec::new();

    let items_to_copy = [
        ("agents", true),
        ("agent", true),
        ("commands", true),
        ("skills", true),
        ("team.config.json", false),
    ];

    let mut found_any = false;

    for (item, is_dir) in items_to_copy {
        let src_item = source_dir.join(item);
        if !src_item.exists() {
            continue;
        }

        found_any = true;
        let dst_item = target_opencode_dir.join(item);

        if is_dir {
            copy_dir_recursive(&src_item, &dst_item, &mut installed_files, item)?;
        } else {
            if let Some(parent) = dst_item.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(&src_item, &dst_item)
                .map_err(|e| format!("复制文件失败 {}: {}", src_item.display(), e))?;
            installed_files.push(item.to_string());
        }
    }

    if !found_any {
        return Err("源目录中未找到 agents/commands/skills 等 Rapid Team 构件".to_string());
    }

    Ok(InstallResult {
        success: true,
        installed_files,
        message: "从本地文件夹安装 Rapid Dev Team 成功！".to_string(),
    })
}

fn copy_dir_recursive(
    src: &Path,
    dst: &Path,
    installed: &mut Vec<String>,
    prefix: &str,
) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("创建目录失败 {}: {}", dst.display(), e))?;
    let entries =
        fs::read_dir(src).map_err(|e| format!("读取目录失败 {}: {}", src.display(), e))?;

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name();
        let dst_path = dst.join(&file_name);
        let rel_name = format!("{}/{}", prefix, file_name.to_string_lossy());

        if path.is_dir() {
            copy_dir_recursive(&path, &dst_path, installed, &rel_name)?;
        } else {
            fs::copy(&path, &dst_path).map_err(|e| format!("复制文件失败: {}", e))?;
            installed.push(rel_name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zip_slip_prevention() {
        let dest = Path::new("/safe/target/dir");
        assert!(is_safe_zip_path(dest, "agents/test.md").is_some());
        assert!(is_safe_zip_path(dest, "commands/rapid.md").is_some());
        assert!(is_safe_zip_path(dest, "../evil.md").is_none());
        assert!(is_safe_zip_path(dest, "agents/../../evil.md").is_none());
        assert!(is_safe_zip_path(dest, "/etc/passwd").is_none());
        assert!(is_safe_zip_path(dest, "\\Windows\\System32").is_none());
    }
}
