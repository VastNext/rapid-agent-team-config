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
use std::path::{Path, PathBuf};
use tao::{
    dpi::LogicalSize,
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::{http::Response, WebViewBuilder};

#[derive(Debug, Deserialize)]
struct IpcMessage {
    action: String,
    #[serde(rename = "callbackId")]
    callback_id: String,
    payload: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct IpcResponse<T: Serialize> {
    data: Option<T>,
    error: Option<String>,
}

fn handle_ipc_request(msg: IpcMessage) -> String {
    let action = msg.action.as_str();
    let callback_id = msg.callback_id;
    let payload = msg.payload;

    let (data, error) = match action {
        "scan_environment" => {
            let project_path_str = payload.get("project_path").and_then(|p| p.as_str());
            let use_project = payload
                .get("use_project")
                .and_then(|u| u.as_bool())
                .unwrap_or(false);
            let project_path = project_path_str.map(Path::new);
            let res = scanner::scan_environment(project_path, use_project);
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
            if let Ok(items) = serde_json::from_value::<Vec<writer::ModelChangeItem>>(
                payload.get("items").cloned().unwrap_or_default(),
            ) {
                let plan = writer::generate_change_plan(&items);
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
            let repo = payload
                .get("repo")
                .and_then(|r| r.as_str())
                .unwrap_or("VastNext/rapid-agent-team-config");
            let proxy_cfg = payload
                .get("proxy")
                .and_then(|p| serde_json::from_value::<network::ProxyConfig>(p.clone()).ok());
            match network::fetch_latest_release(repo, proxy_cfg.as_ref()) {
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

            let temp_zip = std::env::temp_dir().join("rapid_release_download.zip");
            match network::download_zip(url, &temp_zip, proxy_cfg.as_ref()) {
                Ok(_) => {
                    let install_res = installer::install_from_zip(&temp_zip, Path::new(target_dir));
                    let _ = std::fs::remove_file(&temp_zip);
                    match install_res {
                        Ok(res) => (serde_json::to_value(res).ok(), None),
                        Err(e) => (None, Some(e)),
                    }
                }
                Err(e) => (None, Some(e)),
            }
        }
        _ => (None, Some(format!("Unknown action: {}", action))),
    };

    let resp = IpcResponse { data, error };
    let json_resp = serde_json::to_string(&resp).unwrap_or_else(|_| "{}".to_string());
    format!("window['{}']({});", callback_id, json_resp)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Rapid Agent Team Configurator")
        .with_inner_size(LogicalSize::new(1040.0, 720.0))
        .with_min_inner_size(LogicalSize::new(800.0, 560.0))
        .build(&event_loop)?;

    // Embedded assets
    let html_content = include_str!("frontend/index.html");
    let css_content = include_str!("frontend/style.css");
    let js_content = include_str!("frontend/app.js");

    let _webview = WebViewBuilder::new(&window)
        .with_custom_protocol("app".into(), move |_webview_id, request| {
            let path = request.uri().path();
            match path {
                "/" | "/index.html" => Response::builder()
                    .header("Content-Type", "text/html; charset=utf-8")
                    .body(html_content.as_bytes().into())
                    .map_err(Into::into),
                "/style.css" => Response::builder()
                    .header("Content-Type", "text/css; charset=utf-8")
                    .body(css_content.as_bytes().into())
                    .map_err(Into::into),
                "/app.js" => Response::builder()
                    .header("Content-Type", "application/javascript; charset=utf-8")
                    .body(js_content.as_bytes().into())
                    .map_err(Into::into),
                _ => Response::builder()
                    .status(404)
                    .body("Not Found".as_bytes().into())
                    .map_err(Into::into),
            }
        })
        .with_ipc_handler(|webview, msg| {
            if let Ok(req) = serde_json::from_str::<IpcMessage>(&msg) {
                let js_cb = handle_ipc_request(req);
                let _ = webview.evaluate_script(&js_cb);
            }
        })
        .with_url("app://localhost/index.html")
        .build()?;

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
