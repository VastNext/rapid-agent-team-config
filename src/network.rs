//! Isolated Network Client for Release Downloads & Proxy Testing
//!
//! Features:
//! 1. Direct and HTTP / HTTPS / SOCKS5 proxy support
//! 2. Isolated client configuration (zero pollution of global env vars)
//! 3. Connection testing to GitHub API / GitHub Releases
//! 4. Download and direct URL resolution

use serde::{Deserialize, Serialize};
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkTestResult {
    pub ok: bool,
    pub status_code: Option<u16>,
    pub message: String,
    pub latency_ms: u64,
}

pub fn create_agent(proxy: Option<&ProxyConfig>) -> Result<ureq::Agent, String> {
    let mut builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30));

    if let Some(p) = proxy {
        if p.enabled && !p.proxy_url.trim().is_empty() {
            let proxy = ureq::Proxy::new(p.proxy_url.trim())
                .map_err(|e| format!("无效的代理地址: {}", e))?;
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
    repo: &str,
    proxy: Option<&ProxyConfig>,
) -> Result<ReleaseInfo, String> {
    let agent = create_agent(proxy)?;
    let api_url = format!("https://api.github.com/repos/{}/releases/latest", repo);

    let resp = agent
        .get(&api_url)
        .set("User-Agent", "RapidAgentTeamConfig/0.1.0")
        .set("Accept", "application/vnd.github.v3+json")
        .call()
        .map_err(|e| format!("获取 Release 信息失败: {}", e))?;

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

    let mut direct_asset_url = None;
    if let Some(assets) = val.get("assets").and_then(|a| a.as_array()) {
        for asset in assets {
            if let Some(download_url) = asset.get("browser_download_url").and_then(|u| u.as_str()) {
                if download_url.ends_with(".zip") {
                    direct_asset_url = Some(download_url.to_string());
                    break;
                }
            }
        }
    }

    Ok(ReleaseInfo {
        tag_name,
        name,
        html_url,
        zipball_url,
        direct_asset_url,
    })
}

pub fn download_zip(
    url: &str,
    target_file_path: &std::path::Path,
    proxy: Option<&ProxyConfig>,
) -> Result<(), String> {
    let agent = create_agent(proxy)?;
    let resp = agent
        .get(url)
        .set("User-Agent", "RapidAgentTeamConfig/0.1.0")
        .call()
        .map_err(|e| format!("下载 ZIP 包失败: {}", e))?;

    let mut reader = resp.into_reader();
    let mut out_file =
        std::fs::File::create(target_file_path).map_err(|e| format!("无法创建临时文件: {}", e))?;

    std::io::copy(&mut reader, &mut out_file).map_err(|e| format!("保存下载文件失败: {}", e))?;

    Ok(())
}
