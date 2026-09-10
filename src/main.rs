#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

//! Rapid Agent Team Configurator
//!
//! Native Desktop Application built with Rust + Wry / Tao.

mod frontmatter;
mod installer;
mod jsonc;
mod network;
mod scanner;
mod writer;

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tao::{
    dpi::LogicalSize,
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::{http::Response, WebViewBuilder};

const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

fn load_window_icon() -> Option<tao::window::Icon> {
    let decoder = png::Decoder::new(Cursor::new(ICON_PNG));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    tao::window::Icon::from_rgba(buffer, info.width, info.height).ok()
}

#[derive(Debug, Deserialize)]
struct IpcMessage {
    action: String,
    #[serde(default)]
    payload: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct IpcResponse<T: Serialize> {
    data: Option<T>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    proxy: Option<network::ProxyConfig>,
}

const ALLOWED_ACTIONS: &[&str] = &[
    "get_app_config",
    "save_app_config",
    "scan_environment",
    "select_folder",
    "select_file",
    "generate_change_plan",
    "apply_model_changes",
    "generate_zip_install_plan",
    "install_from_zip",
    "install_from_local_dir",
    "test_proxy_connection",
    "fetch_latest_release",
    "download_and_install_release",
];

fn get_app_config_path() -> PathBuf {
    if let Some(config_dir) = dirs::config_dir() {
        let app_dir = config_dir.join("rapid-agent-team-config");
        let _ = std::fs::create_dir_all(&app_dir);
        return app_dir.join("config.json");
    }
    PathBuf::from("rapid_app_config.json")
}

fn load_app_config() -> AppConfig {
    let p = get_app_config_path();
    if let Ok(content) = std::fs::read_to_string(p) {
        if let Ok(cfg) = serde_json::from_str::<AppConfig>(&content) {
            return cfg;
        }
    }
    AppConfig { proxy: None }
}

fn save_app_config(cfg: &AppConfig) -> Result<(), String> {
    let p = get_app_config_path();
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json_str = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(p, json_str).map_err(|e| format!("保存配置失败: {}", e))
}

fn handle_ipc_request(msg: IpcMessage) -> IpcResponse<serde_json::Value> {
    let action = msg.action.as_str();
    if !ALLOWED_ACTIONS.contains(&action) {
        return IpcResponse {
            data: None,
            error: Some(format!("未受许可的 IPC 请求操作: {}", action)),
        };
    }
    let payload = msg.payload;

    let (data, error) = match action {
        "get_app_config" => {
            let cfg = load_app_config();
            (serde_json::to_value(cfg).ok(), None)
        }
        "save_app_config" => {
            if let Ok(cfg) = serde_json::from_value::<AppConfig>(payload) {
                match save_app_config(&cfg) {
                    Ok(_) => (serde_json::to_value(true).ok(), None),
                    Err(e) => (None, Some(e)),
                }
            } else {
                (None, Some("无效的配置参数".to_string()))
            }
        }
        "scan_environment" => {
            let project_path_str = payload.get("project_path").and_then(|p| p.as_str());
            let use_project = payload
                .get("use_project")
                .and_then(|u| u.as_bool())
                .unwrap_or(false);
            let project_path = project_path_str.map(Path::new);
            let explicit_paths = payload
                .get("config_paths")
                .and_then(|paths| paths.as_array())
                .map(|paths| {
                    paths
                        .iter()
                        .filter_map(|path| path.as_str().map(PathBuf::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let res =
                scanner::scan_environment_with_paths(project_path, use_project, &explicit_paths);
            (serde_json::to_value(res).ok(), None)
        }
        "select_folder" => {
            let folder = rfd::FileDialog::new().pick_folder();
            let path_str = folder.map(|p| p.to_string_lossy().to_string());
            (serde_json::to_value(path_str).ok(), None)
        }
        "select_file" => {
            let filter_ext = payload
                .get("filter_ext")
                .and_then(|f| f.as_str())
                .unwrap_or("zip");
            let file = rfd::FileDialog::new()
                .add_filter("Archive", &[filter_ext])
                .pick_file();
            let path_str = file.map(|p| p.to_string_lossy().to_string());
            (serde_json::to_value(path_str).ok(), None)
        }
        "generate_change_plan" => {
            let base_dir_str = payload
                .get("base_opencode_dir")
                .and_then(|d| d.as_str())
                .unwrap_or("");
            if let Ok(items) = serde_json::from_value::<Vec<writer::ModelChangeItem>>(
                payload.get("items").cloned().unwrap_or_default(),
            ) {
                let plan = writer::generate_change_plan(Path::new(base_dir_str), &items);
                (serde_json::to_value(plan).ok(), None)
            } else {
                (None, Some("无效的请求变更参数".to_string()))
            }
        }
        "apply_model_changes" => {
            let base_dir_str = payload
                .get("base_opencode_dir")
                .and_then(|d| d.as_str())
                .unwrap_or("");
            if let Ok(items) = serde_json::from_value::<Vec<writer::ModelChangeItem>>(
                payload.get("items").cloned().unwrap_or_default(),
            ) {
                match writer::apply_model_changes(Path::new(base_dir_str), &items) {
                    Ok(res) => (serde_json::to_value(res).ok(), None),
                    Err(e) => (None, Some(e)),
                }
            } else {
                (None, Some("变更参数解析失败".to_string()))
            }
        }
        "generate_zip_install_plan" => {
            let zip_path = payload
                .get("zip_path")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            let target_dir = payload
                .get("target_opencode_dir")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            match installer::generate_zip_install_plan(Path::new(zip_path), Path::new(target_dir)) {
                Ok(res) => (serde_json::to_value(res).ok(), None),
                Err(e) => (None, Some(e)),
            }
        }
        "install_from_zip" => {
            let zip_path = payload
                .get("zip_path")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            let target_dir = payload
                .get("target_opencode_dir")
                .and_then(|p| p.as_str())
                .unwrap_or("");

            match installer::install_from_zip(Path::new(zip_path), Path::new(target_dir)) {
                Ok(res) => (serde_json::to_value(res).ok(), None),
                Err(e) => (None, Some(e)),
            }
        }
        "install_from_local_dir" => {
            let src_dir = payload
                .get("source_dir")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            let target_dir = payload
                .get("target_opencode_dir")
                .and_then(|p| p.as_str())
                .unwrap_or("");

            match installer::install_from_local_dir(Path::new(src_dir), Path::new(target_dir)) {
                Ok(res) => (serde_json::to_value(res).ok(), None),
                Err(e) => (None, Some(e)),
            }
        }
        "test_proxy_connection" => {
            let proxy_cfg = payload
                .get("proxy")
                .and_then(|p| serde_json::from_value::<network::ProxyConfig>(p.clone()).ok());
            let res = network::test_proxy_connection(proxy_cfg.as_ref());
            (serde_json::to_value(res).ok(), None)
        }
        "fetch_latest_release" => {
            let repo_opt = payload.get("repo").and_then(|r| r.as_str());
            let proxy_cfg = payload
                .get("proxy")
                .and_then(|p| serde_json::from_value::<network::ProxyConfig>(p.clone()).ok());
            match network::fetch_latest_release(repo_opt, proxy_cfg.as_ref()) {
                Ok(res) => (serde_json::to_value(res).ok(), None),
                Err(e) => (None, Some(e)),
            }
        }
        "download_and_install_release" => {
            let url = payload.get("url").and_then(|u| u.as_str()).unwrap_or("");
            let target_dir = payload
                .get("target_opencode_dir")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            let proxy_cfg = payload
                .get("proxy")
                .and_then(|p| serde_json::from_value::<network::ProxyConfig>(p.clone()).ok());

            // 强制校验下载地址必须属于官方 Rapid Agent Team 仓库白名单
            if let Err(e) = network::is_allowed_repo_url(url) {
                return IpcResponse {
                    data: None,
                    error: Some(e),
                };
            }

            let unique_id = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let temp_zip = std::env::temp_dir().join(format!("rapid_release_{}.zip", unique_id));

            match network::download_zip(url, &temp_zip, proxy_cfg.as_ref()) {
                Ok(_) => {
                    let install_res = installer::install_from_zip(&temp_zip, Path::new(target_dir));
                    let _ = std::fs::remove_file(&temp_zip);
                    match install_res {
                        Ok(res) => (serde_json::to_value(res).ok(), None),
                        Err(e) => (None, Some(e)),
                    }
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&temp_zip);
                    (None, Some(e))
                }
            }
        }
        _ => (None, Some(format!("未识别的请求操作: {}", action))),
    };

    IpcResponse { data, error }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Rapid Agent Team Configurator")
        .with_window_icon(load_window_icon())
        .with_inner_size(LogicalSize::new(1040.0, 720.0))
        .with_min_inner_size(LogicalSize::new(800.0, 560.0))
        .build(&event_loop)?;

    let html_content = include_str!("frontend/index.html");
    let css_content = include_str!("frontend/style.css");
    let js_content = include_str!("frontend/app.js");
    let icon_content = include_bytes!("../assets/icon.png");

    let builder = WebViewBuilder::new()
        .with_custom_protocol("app".into(), move |_webview_id, request| {
            let path = request.uri().path();
            match path {
                "/" | "/index.html" => Response::builder()
                    .header("Content-Type", "text/html; charset=utf-8")
                    .body(Cow::Borrowed(html_content.as_bytes()))
                    .unwrap(),
                "/style.css" => Response::builder()
                    .header("Content-Type", "text/css; charset=utf-8")
                    .body(Cow::Borrowed(css_content.as_bytes()))
                    .unwrap(),
                "/app.js" => Response::builder()
                    .header("Content-Type", "application/javascript; charset=utf-8")
                    .body(Cow::Borrowed(js_content.as_bytes()))
                    .unwrap(),
                "/icon.png" => Response::builder()
                    .header("Content-Type", "image/png")
                    .body(Cow::Borrowed(&icon_content[..]))
                    .unwrap(),
                "/api/ipc" => {
                    let req_body = request.body();
                    let resp_obj = match serde_json::from_slice::<IpcMessage>(req_body) {
                        Ok(msg) => handle_ipc_request(msg),
                        Err(e) => IpcResponse {
                            data: None,
                            error: Some(format!("IPC JSON 解析错误: {}", e)),
                        },
                    };
                    let resp_bytes = serde_json::to_vec(&resp_obj).unwrap_or_default();
                    Response::builder()
                        .header("Content-Type", "application/json; charset=utf-8")
                        .body(Cow::Owned(resp_bytes))
                        .unwrap()
                }
                _ => Response::builder()
                    .status(404)
                    .body(Cow::Borrowed(&b"Not Found"[..]))
                    .unwrap(),
            }
        })
        .with_url("app://localhost/index.html");

    #[cfg(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    ))]
    let _webview = builder.build(&window)?;

    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    )))]
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().unwrap();
        builder.build_gtk(vbox)?
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(StartCause::Init) => {}
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            _ => (),
        }
    });
}
