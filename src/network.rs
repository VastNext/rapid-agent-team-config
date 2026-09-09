//! Isolated Network Client for Release Downloads & Proxy Testing
//!
//! Features:
//! 1. Direct and HTTP / HTTPS / SOCKS5 proxy support
//! 2. Isolated client configuration (zero pollution of global env vars)
//! 3. Strict allowlist validation (official GitHub domains & official Rapid Agent Team repositories)
//! 4. Platform-specific release asset filtering
//! 5. Download size limits and automatic temp cleanup

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub enabled: bool,
    pub proxy_url: String, // e.g. "http://127.0.0.1:7890" or "socks5://127.0.0.1:10808"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub name: String,
    pub html_url: String,
    pub zipball_url: String,
    pub direct_asset_url: Option<String>,
    pub direct_asset_name: Option<String>,
    pub current_platform: String,
    pub has_platform_asset: bool,
    pub sha256_url: Option<String>,
    pub repo: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkTestResult {
    pub ok: bool,
    pub status_code: Option<u16>,
    pub message: String,
    pub latency_ms: u64,
}

const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024; // 100 MB max

pub const OFFICIAL_RAPID_TEAM_REPO: &str = "VastNext/opencode-rapid-agent-team";

pub const ALLOWED_REPOSITORIES: &[&str] = &[
    "VastNext/opencode-rapid-agent-team",
    "VastNext/rapid-agent-team",
    "VastNext/rapid-agent-team-config",
    "opencode-ai/rapid-agent-team",
];

const ALLOWED_HOSTS: &[&str] = &[
    "github.com",
    "api.github.com",
    "objects.githubusercontent.com",
    "raw.githubusercontent.com",
    "codeload.github.com",
];

/// Validates whether a URL is allowed for download
pub fn is_allowed_download_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("仅允许安全的 HTTPS 请求协议".to_string());
    }

    let parsed = match url.strip_prefix("https://") {
        Some(rest) => rest,
        None => return Err("无效的 URL 格式".to_string()),
    };

    let host = match parsed.find('/') {
        Some(idx) => &parsed[..idx],
        None => parsed,
    };

    if host.contains('@') || host.contains(':') || host.is_empty() {
        return Err("下载地址包含不允许的主机认证信息或端口".to_string());
    }

    if !ALLOWED_HOSTS
        .iter()
        .any(|&h| host == h || host.ends_with(&format!(".{}", h)))
    {
        return Err(format!("目标域名不在允许列表中: {}", host));
    }

    Ok(())
}

/// Validates that a download URL belongs to one of the whitelisted repositories.
/// Recognizes GitHub release/zipball/raw/archive URL shapes containing
/// `<owner>/<repo>` so only official Rapid Agent Team packages can be fetched.
pub fn is_allowed_repo_url(url: &str) -> Result<(), String> {
    is_allowed_download_url(url)?;

    let path = url
        .strip_prefix("https://")
        .and_then(|u| u.find('/').map(|idx| &u[idx..]))
        .unwrap_or(url);
    for repo in ALLOWED_REPOSITORIES {
        let needle = format!("/{}/", repo);
        if path.contains(&needle) {
            return Ok(());
        }
        // Repo may appear at end of path (e.g. .../archive/refs/tags/v1.0.zip has repo in middle;
        // api.github.com/repos/<owner>/<repo> style is covered by the needle above)
        let needle_tail = format!("/{}", repo);
        if path == needle_tail || path.ends_with(&needle_tail) {
            return Ok(());
        }
    }
    Err(format!(
        "下载地址不属于官方 Rapid Agent Team 仓库白名单: {}",
        url
    ))
}

/// Detects the target platform asset keyword for the current OS
pub fn get_current_platform_keyword() -> (&'static str, &'static str) {
    if cfg!(target_os = "windows") {
        ("windows", "windows-x64")
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            ("macos", "macos-arm64")
        } else {
            ("macos", "macos-x64")
        }
    } else {
        ("linux", "linux-x64")
    }
}

pub fn create_agent(proxy: Option<&ProxyConfig>) -> Result<ureq::Agent, String> {
    let mut builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30));

    if let Some(p) = proxy {
        if p.enabled && !p.proxy_url.trim().is_empty() {
            let proxy_clean = p.proxy_url.trim();
            let proxy =
                ureq::Proxy::new(proxy_clean).map_err(|e| format!("无效的代理地址: {}", e))?;
            builder = builder.proxy(proxy);
        }
    }

    Ok(builder.build())
}

pub fn test_proxy_connection(proxy: Option<&ProxyConfig>) -> NetworkTestResult {
    let start = std::time::Instant::now();
    let agent = match create_agent(proxy) {
        Ok(a) => a,
        Err(e) => {
            return NetworkTestResult {
                ok: false,
                status_code: None,
                message: format!("代理初始化失败: {}", e),
                latency_ms: 0,
            }
        }
    };

    // Test GitHub API reachable
    match agent
        .get("https://api.github.com")
        .set("User-Agent", "RapidAgentTeamConfig/0.1.0")
        .call()
    {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            NetworkTestResult {
                ok: true,
                status_code: Some(resp.status()),
                message: format!("GitHub 网络连接正常 (耗时 {} ms)", latency_ms),
                latency_ms,
            }
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            NetworkTestResult {
                ok: false,
                status_code: None,
                message: format!("无法连接 GitHub: {}", e),
                latency_ms,
            }
        }
    }
}

pub fn fetch_latest_release(
    repo_opt: Option<&str>,
    proxy: Option<&ProxyConfig>,
) -> Result<ReleaseInfo, String> {
    let repo = repo_opt.unwrap_or(OFFICIAL_RAPID_TEAM_REPO).trim();

    // Restrict to whitelist of repositories
    if !ALLOWED_REPOSITORIES.contains(&repo) {
        return Err(format!(
            "目标仓库 '{}' 不在官方受信白名单中: {:?}",
            repo, ALLOWED_REPOSITORIES
        ));
    }

    let agent = create_agent(proxy)?;
    let api_url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    is_allowed_download_url(&api_url)?;

    let resp = agent
        .get(&api_url)
        .set("User-Agent", "RapidAgentTeamConfig/0.1.0")
        .set("Accept", "application/vnd.github.v3+json")
        .call()
        .map_err(|e| format!("获取 Release 信息失败 ({}): {}", repo, e))?;

    let val: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析 GitHub API 响应失败: {}", e))?;

    let tag_name = val["tag_name"].as_str().unwrap_or("v0.1.0").to_string();
    let name = val["name"].as_str().unwrap_or("Release").to_string();
    let html_url = val["html_url"]
        .as_str()
        .unwrap_or(&format!("https://github.com/{}/releases", repo))
        .to_string();
    let zipball_url = val["zipball_url"]
        .as_str()
        .unwrap_or(&format!(
            "https://github.com/{}/archive/refs/tags/{}.zip",
            repo, tag_name
        ))
        .to_string();

    let (_platform_os, platform_keyword) = get_current_platform_keyword();
    let mut direct_asset_url = None;
    let mut direct_asset_name = None;
    let mut sha256_url = None;

    if let Some(assets) = val.get("assets").and_then(|a| a.as_array()) {
        for asset in assets {
            let asset_name = asset.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let download_url = asset
                .get("browser_download_url")
                .and_then(|u| u.as_str())
                .unwrap_or("");

            if asset_name.ends_with(".sha256") || asset_name.ends_with(".sha256.txt") {
                sha256_url = Some(download_url.to_string());
            }

            // Match release archive or platform-specific asset
            if (asset_name.contains(platform_keyword)
                || asset_name.ends_with(".zip")
                || asset_name.contains("rapid-agent-team"))
                && direct_asset_url.is_none()
            {
                direct_asset_url = Some(download_url.to_string());
                direct_asset_name = Some(asset_name.to_string());
            }
        }
    }

    let has_platform_asset = direct_asset_url.is_some();

    Ok(ReleaseInfo {
        tag_name,
        name,
        html_url,
        zipball_url,
        direct_asset_url,
        direct_asset_name,
        current_platform: platform_keyword.to_string(),
        has_platform_asset,
        sha256_url,
        repo: repo.to_string(),
    })
}

pub fn download_zip(
    url: &str,
    target_file_path: &std::path::Path,
    proxy: Option<&ProxyConfig>,
) -> Result<(), String> {
    is_allowed_repo_url(url)?;

    let agent = create_agent(proxy)?;
    let resp = agent
        .get(url)
        .set("User-Agent", "RapidAgentTeamConfig/0.1.0")
        .call()
        .map_err(|e| format!("下载 ZIP 包失败: {}", e))?;

    // Check Content-Length header if available
    if let Some(len_str) = resp.header("Content-Length") {
        if let Ok(len) = len_str.parse::<u64>() {
            if len > MAX_DOWNLOAD_BYTES {
                return Err(format!(
                    "下载包文件大小超限 ({} MB > 100 MB)",
                    len / 1024 / 1024
                ));
            }
        }
    }

    let mut reader = resp.into_reader().take(MAX_DOWNLOAD_BYTES + 1);
    let mut out_file =
        std::fs::File::create(target_file_path).map_err(|e| format!("无法创建临时文件: {}", e))?;

    let copied = std::io::copy(&mut reader, &mut out_file)
        .map_err(|e| format!("保存下载文件失败: {}", e))?;

    if copied > MAX_DOWNLOAD_BYTES {
        let _ = std::fs::remove_file(target_file_path);
        return Err("下载内容超出最大安全大小限制 (100 MB)".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowed_urls() {
        assert!(is_allowed_download_url(
            "https://github.com/VastNext/rapid-agent-team/releases/download/v1.0.0/release.zip"
        )
        .is_ok());
        assert!(
            is_allowed_download_url("https://api.github.com/repos/VastNext/rapid-agent-team")
                .is_ok()
        );
        assert!(is_allowed_download_url(
            "https://objects.githubusercontent.com/github-production-release-asset/abc"
        )
        .is_ok());

        assert!(is_allowed_download_url("http://github.com/insecure").is_err());
        assert!(is_allowed_download_url("https://malicious-site.com/evil.zip").is_err());
        assert!(is_allowed_download_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn test_repo_whitelist() {
        assert!(ALLOWED_REPOSITORIES.contains(&"VastNext/opencode-rapid-agent-team"));
        assert!(ALLOWED_REPOSITORIES.contains(&"VastNext/rapid-agent-team"));
        assert!(ALLOWED_REPOSITORIES.contains(&"VastNext/rapid-agent-team-config"));
        assert_eq!(
            OFFICIAL_RAPID_TEAM_REPO,
            "VastNext/opencode-rapid-agent-team"
        );
        assert!(!ALLOWED_REPOSITORIES.contains(&"unknown-user/evil-repo"));
    }
}
