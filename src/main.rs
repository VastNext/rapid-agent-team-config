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
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tao::{
    dpi::LogicalSize,
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use wry::{http::Response, PageLoadEvent, WebViewBuilder};

const ICON_PNG: &[u8] = include_bytes!("../assets/icon-window.png");
const MAX_NATIVE_IPC_TASKS: usize = 4;
const MUTATING_ACTIONS: &[&str] = &[
    "save_app_config",
    "apply_model_changes",
    "install_from_zip",
    "install_from_local_dir",
];

fn write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

enum UserEvent {
    ShowWindow,
    IpcRequest(String),
    IpcResponse {
        callback_id: String,
        response: IpcResponse<serde_json::Value>,
    },
}

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
    #[serde(rename = "callbackId")]
    callback_id: String,
    #[serde(default)]
    payload: serde_json::Value,
}

fn respond_to_javascript(
    webview: &wry::WebView,
    callback_id: &str,
    response: &IpcResponse<serde_json::Value>,
) {
    if !callback_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return;
    }
    let callback = serde_json::to_string(callback_id).unwrap_or_else(|_| "\"\"".to_string());
    let payload = serde_json::to_string(response)
        .unwrap_or_else(|_| "{\"data\":null,\"error\":\"IPC 响应序列化失败\"}".to_string());
    let script =
        format!("if (typeof window[{callback}] === 'function') window[{callback}]({payload});");
    let _ = webview.evaluate_script(&script);
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
    let _write_guard = MUTATING_ACTIONS.contains(&action).then(|| {
        write_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    });
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
            let folder = pick_folder();
            let path_str = folder.map(|p| p.to_string_lossy().to_string());
            (serde_json::to_value(path_str).ok(), None)
        }
        "select_file" => {
            let filter_ext = payload
                .get("filter_ext")
                .and_then(|f| f.as_str())
                .unwrap_or("zip");
            let file = pick_file(filter_ext);
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
                    let _write_guard = write_lock()
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
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

#[cfg(target_os = "linux")]
fn pick_folder() -> Option<PathBuf> {
    pollster::block_on(rfd::AsyncFileDialog::new().pick_folder()).map(|file| file.path().to_owned())
}

#[cfg(not(target_os = "linux"))]
fn pick_folder() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_folder()
}

#[cfg(target_os = "linux")]
fn pick_file(filter_ext: &str) -> Option<PathBuf> {
    pollster::block_on(
        rfd::AsyncFileDialog::new()
            .add_filter("Archive", &[filter_ext])
            .pick_file(),
    )
    .map(|file| file.path().to_owned())
}

#[cfg(not(target_os = "linux"))]
fn pick_file(filter_ext: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Archive", &[filter_ext])
        .pick_file()
}

fn dialog_requires_main_thread(action: &str) -> bool {
    cfg!(not(target_os = "linux")) && matches!(action, "select_folder" | "select_file")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let event_proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("Rapid Agent Team Configurator")
        .with_window_icon(load_window_icon())
        // WebView2 初始化期间隐藏窗口，避免用户看到长时间白屏。
        .with_visible(false)
        .with_inner_size(LogicalSize::new(1040.0, 720.0))
        .with_min_inner_size(LogicalSize::new(800.0, 560.0))
        .build(&event_loop)?;

    let html_content = include_str!("frontend/index.html");
    let css_content = include_str!("frontend/style.css");
    let js_content = include_str!("frontend/app.js");
    let icon_content = include_bytes!("../assets/icon.png");
    let show_window_proxy = event_proxy.clone();
    let ipc_proxy = event_proxy.clone();

    let builder = WebViewBuilder::new()
        .with_asynchronous_custom_protocol("app".into(), move |_webview_id, request, responder| {
            let path = request.uri().path();
            match path {
                "/" | "/index.html" => Response::builder()
                    .header("Content-Type", "text/html; charset=utf-8")
                    .body(Cow::Borrowed(html_content.as_bytes()))
                    .map(|response| responder.respond(response))
                    .unwrap(),
                "/style.css" => Response::builder()
                    .header("Content-Type", "text/css; charset=utf-8")
                    .body(Cow::Borrowed(css_content.as_bytes()))
                    .map(|response| responder.respond(response))
                    .unwrap(),
                "/app.js" => Response::builder()
                    .header("Content-Type", "application/javascript; charset=utf-8")
                    .body(Cow::Borrowed(js_content.as_bytes()))
                    .map(|response| responder.respond(response))
                    .unwrap(),
                "/icon.png" => Response::builder()
                    .header("Content-Type", "image/png")
                    .body(Cow::Borrowed(&icon_content[..]))
                    .map(|response| responder.respond(response))
                    .unwrap(),
                _ => Response::builder()
                    .status(404)
                    .body(Cow::Borrowed(&b"Not Found"[..]))
                    .map(|response| responder.respond(response))
                    .unwrap(),
            }
        })
        .with_ipc_handler(move |request| {
            let _ = ipc_proxy.send_event(UserEvent::IpcRequest(request.body().clone()));
        })
        .with_on_page_load_handler(move |event, _url| {
            if matches!(event, PageLoadEvent::Finished) {
                let _ = show_window_proxy.send_event(UserEvent::ShowWindow);
            }
        })
        .with_url("app://localhost/index.html");

    #[cfg(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    ))]
    let webview = builder.build(&window)?;

    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    )))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().unwrap();
        builder.build_gtk(vbox)?
    };

    let mut ipc_in_flight = 0usize;
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(StartCause::Init) => {}
            Event::UserEvent(UserEvent::ShowWindow) => window.set_visible(true),
            Event::UserEvent(UserEvent::IpcRequest(body)) => {
                let parsed = serde_json::from_str::<IpcMessage>(&body);
                match parsed {
                    Ok(message) if ipc_in_flight >= MAX_NATIVE_IPC_TASKS => {
                        let response = IpcResponse {
                            data: None,
                            error: Some("任务过多，请稍后重试".to_string()),
                        };
                        respond_to_javascript(&webview, &message.callback_id, &response);
                    }
                    Ok(message) if dialog_requires_main_thread(&message.action) => {
                        ipc_in_flight += 1;
                        let callback_id = message.callback_id.clone();
                        let response = handle_ipc_request(message);
                        respond_to_javascript(&webview, &callback_id, &response);
                        ipc_in_flight -= 1;
                    }
                    Ok(message) => {
                        ipc_in_flight += 1;
                        let callback_id = message.callback_id.clone();
                        let response_proxy = event_proxy.clone();
                        std::thread::spawn(move || {
                            let response =
                                catch_unwind(AssertUnwindSafe(|| handle_ipc_request(message)))
                                    .unwrap_or_else(|_| IpcResponse {
                                        data: None,
                                        error: Some("后台任务异常终止".to_string()),
                                    });
                            let _ = response_proxy.send_event(UserEvent::IpcResponse {
                                callback_id,
                                response,
                            });
                        });
                    }
                    Err(error) => {
                        eprintln!("IPC JSON 解析错误: {error}");
                    }
                }
            }
            Event::UserEvent(UserEvent::IpcResponse {
                callback_id,
                response,
            }) => {
                respond_to_javascript(&webview, &callback_id, &response);
                ipc_in_flight = ipc_in_flight.saturating_sub(1);
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            _ => (),
        }
    });
}
