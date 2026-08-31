//! Web 服务：提供与 CLI 一致的功能（改写 / 生图 / 播客 / 视频 / 全流程）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Path as AxPath, State};
use axum::http::header::{CONTENT_TYPE, CONTENT_DISPOSITION};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex;

use crate::cmd;
use crate::config::Config;
use crate::pi_rpc::PiRpcAgent;
use crate::podcast as podcast_backend;
use crate::provider;

const OUTPUT_ROOT: &str = "output";

#[derive(Clone)]
struct AppState {
    /// 串行化流水线执行，避免 ffmpeg/pi 并发冲突
    run_lock: Arc<Mutex<()>>,
}

#[derive(Deserialize)]
struct RunReq {
    text: String,
    #[serde(default)]
    ref_image: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Deserialize)]
struct RewriteReq {
    text: String,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Deserialize)]
struct ImageReq {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    ref_image: Option<String>,
}

#[derive(Deserialize)]
struct PodcastReq {
    #[serde(default)]
    id: Option<String>,
    /// 模式 B：先生成脚本
    #[serde(default)]
    script: bool,
}

#[derive(Deserialize)]
struct VideoReq {
    #[serde(default)]
    id: Option<String>,
}

#[derive(Serialize)]
struct ApiResp {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifacts: Option<Vec<Artifact>>,
}

#[derive(Serialize)]
struct Artifact {
    name: String,
    size: u64,
    url: String,
}

fn ok(id: Option<String>, message: Option<String>, artifacts: Option<Vec<Artifact>>) -> ApiResp {
    ApiResp { ok: true, id, message, error: None, artifacts }
}

fn err(e: anyhow::Error) -> ApiResp {
    ApiResp { ok: false, id: None, message: None, error: Some(e.to_string()), artifacts: None }
}

/// 加载配置 + pi 客户端
fn load_ctx() -> anyhow::Result<(Config, PiRpcAgent)> {
    let cfg = Config::load(&Config::path())?;
    let llm = PiRpcAgent::new(cfg.tasks.llm.clone().map(|l| l.model))?;
    Ok((cfg, llm))
}

fn list_artifacts(id: &str) -> Vec<Artifact> {
    let dir = Path::new(OUTPUT_ROOT).join(id);
    let mut arts = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if e.path().is_file() {
                let name = e.file_name().to_string_lossy().to_string();
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                arts.push(Artifact {
                    url: format!("/api/files/{id}/{name}"),
                    name,
                    size,
                });
            }
        }
    }
    arts.sort_by(|a, b| a.name.cmp(&b.name));
    arts
}

fn resolve_dir(id: Option<String>) -> anyhow::Result<PathBuf> {
    match id {
        Some(i) => Ok(cmd::task_dir(Path::new(OUTPUT_ROOT), &i)),
        None => cmd::latest_task_dir(Path::new(OUTPUT_ROOT)),
    }
}

fn dir_id(dir: &Path) -> String {
    dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

async fn run_pipeline(State(state): State<AppState>, Json(req): Json<RunReq>) -> Response {
    let _guard = state.run_lock.lock().await;
    let result = run_pipeline_inner(req).await;
    match result {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(err(e))).into_response(),
    }
}

async fn run_pipeline_inner(req: RunReq) -> anyhow::Result<ApiResp> {
    let (cfg, llm) = load_ctx()?;
    let id = cmd::run::run_with_config(
        Path::new(OUTPUT_ROOT),
        &cfg,
        &llm,
        &req.text,
        req.id,
        req.ref_image.map(PathBuf::from),
    )
    .await?;
    Ok(ok(Some(id.clone()), Some("全流程完成".into()), Some(list_artifacts(&id))))
}

async fn rewrite(Json(req): Json<RewriteReq>) -> Response {
    let result = async {
        let (_cfg, llm) = load_ctx()?;
        let id = cmd::rewrite::run_with(Path::new(OUTPUT_ROOT), &req.text, req.id, &llm).await?;
        let rewritten = std::fs::read_to_string(Path::new(OUTPUT_ROOT).join(&id).join("rewritten.md")).unwrap_or_default();
        Ok(ok(Some(id.clone()), Some(rewritten), Some(list_artifacts(&id))))
    }
    .await;
    match result {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(err(e))).into_response(),
    }
}

async fn image(Json(req): Json<ImageReq>) -> Response {
    let result = async {
        let (cfg, llm) = load_ctx()?;
        let dir = resolve_dir(req.id)?;
        let provider = provider::resolve_image(&cfg)?;
        cmd::image::run_with(&dir, req.ref_image.map(PathBuf::from), &llm, provider.as_ref()).await?;
        let id = dir_id(&dir);
        Ok(ok(Some(id.clone()), Some("生图完成".into()), Some(list_artifacts(&id))))
    }
    .await;
    match result {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(err(e))).into_response(),
    }
}

async fn podcast(Json(req): Json<PodcastReq>) -> Response {
    let result = async {
        let (cfg, llm) = load_ctx()?;
        let dir = resolve_dir(req.id)?;
        let backend = podcast_backend::resolve_podcast(&cfg)?;
        cmd::podcast::run_with(&dir, &llm, &backend, req.script).await?;
        let id = dir_id(&dir);
        Ok(ok(Some(id.clone()), Some("播客完成".into()), Some(list_artifacts(&id))))
    }
    .await;
    match result {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(err(e))).into_response(),
    }
}

async fn video(Json(req): Json<VideoReq>) -> Response {
    let result = async {
        let dir = resolve_dir(req.id)?;
        // ffmpeg 视频合成为 CPU 密集且耗时，放到阻塞线程池
        let dir2 = dir.clone();
        tokio::task::spawn_blocking(move || cmd::video::run_with(&dir2)).await??;
        let id = dir_id(&dir);
        Ok(ok(Some(id.clone()), Some("视频完成".into()), Some(list_artifacts(&id))))
    }
    .await;
    match result {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(err(e))).into_response(),
    }
}

async fn list_tasks() -> Response {
    let mut tasks = Vec::new();
    if let Ok(entries) = std::fs::read_dir(OUTPUT_ROOT) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                let id = e.file_name().to_string_lossy().to_string();
                tasks.push(json!({"id": id, "artifacts": list_artifacts(&id)}));
            }
        }
    }
    tasks.sort_by(|a, b| b["id"].as_str().cmp(&a["id"].as_str()));
    Json(json!({"ok": true, "tasks": tasks})).into_response()
}

async fn task_info(AxPath(id): AxPath<String>) -> Response {
    Json(json!({"ok": true, "id": id, "artifacts": list_artifacts(&id)})).into_response()
}

async fn download(AxPath((id, name)): AxPath<(String, String)>) -> Response {
    // 防路径穿越
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return (StatusCode::BAD_REQUEST, "非法文件名").into_response();
    }
    let path = Path::new(OUTPUT_ROOT).join(&id).join(&name);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let ct = match name.rsplit('.').next() {
                Some("png") => "image/png",
                Some("mp3") => "audio/mpeg",
                Some("mp4") => "video/mp4",
                Some("srt") => "application/x-subrip",
                Some("md") | Some("txt") => "text/plain; charset=utf-8",
                _ => "application/octet-stream",
            };
            (
                [
                    (CONTENT_TYPE, ct),
                    (CONTENT_DISPOSITION, &format!("inline; filename=\"{name}\"")[..]),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "文件不存在").into_response(),
    }
}

async fn ui() -> Html<&'static str> {
    Html(include_str!("../web/index.html"))
}

pub async fn run(port: u16) -> anyhow::Result<()> {
    let state = AppState { run_lock: Arc::new(Mutex::new(())) };
    let app = Router::new()
        .route("/", get(ui))
        .route("/api/run", post(run_pipeline))
        .route("/api/rewrite", post(rewrite))
        .route("/api/image", post(image))
        .route("/api/podcast", post(podcast))
        .route("/api/video", post(video))
        .route("/api/tasks", get(list_tasks))
        .route("/api/tasks/:id", get(task_info))
        .route("/api/files/:id/:name", get(download))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    println!("✓ Web 服务已启动: http://localhost:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
