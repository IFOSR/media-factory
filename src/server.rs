//! Web 服务：提供与 CLI 一致的功能（改写 / 生图 / 播客 / 视频 / 全流程）。
//! 执行端点为「后台执行 + SSE 流式推送」。

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Path as AxPath, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex;
use tokio::sync::broadcast::error::RecvError;

use crate::cmd;
use crate::config::Config;
use crate::pi_rpc;
use crate::podcast as podcast_backend;
use crate::provider;
use crate::task::TaskEvents;
use crate::wizard;

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
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    image_prompt: Option<String>,
    #[serde(default)]
    podcast_prompt: Option<String>,
}

#[derive(Deserialize)]
struct RewriteReq {
    text: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
}

#[derive(Deserialize)]
struct ImageReq {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    ref_image: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
}

#[derive(Deserialize)]
struct PodcastReq {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    script: bool,
    #[serde(default)]
    prompt: Option<String>,
}

#[derive(Deserialize)]
struct VideoReq {
    #[serde(default)]
    id: Option<String>,
}

#[derive(Serialize)]
struct Artifact {
    name: String,
    size: u64,
    url: String,
}

fn load_ctx() -> anyhow::Result<(Config, Box<dyn crate::llm::LlmAgent>)> {
    let cfg = Config::load(&Config::path())?;
    let llm = crate::llm::resolve_llm(&cfg)?;
    Ok((cfg, llm))
}

fn list_artifacts(id: &str) -> Vec<Artifact> {
    let dir = Path::new(OUTPUT_ROOT).join(id);
    let mut arts = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if e.path().is_file() && e.file_name().to_string_lossy() != "task.json" {
                let name = e.file_name().to_string_lossy().to_string();
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                arts.push(Artifact { url: format!("/api/files/{id}/{name}"), name, size });
            }
        }
    }
    arts.sort_by(|a, b| a.name.cmp(&b.name));
    arts
}

fn task_meta_json(id: &str) -> serde_json::Value {
    let p = Path::new(OUTPUT_ROOT).join(id).join("task.json");
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| {
            json!({"id": id, "status": "unknown", "steps": {}, "artifacts": list_artifacts(id)})
        })
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

/// 生成任务 id（改写在时间戳基础上加随机后缀，避免同秒冲突）
fn gen_id() -> String {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let suffix: String = uuid::Uuid::new_v4().simple().to_string().chars().take(4).collect();
    format!("{ts}-{suffix}")
}

// ============ 执行端点（后台执行 + 立即返回 task_id） ============

async fn run_pipeline(State(state): State<AppState>, Json(req): Json<RunReq>) -> Response {
    let (cfg, llm) = match load_ctx() {
        Ok(x) => x,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok": false, "error": e.to_string()}))).into_response(),
    };
    let id = req.id.clone().unwrap_or_else(gen_id);
    let events = TaskEvents::streaming(Path::new(OUTPUT_ROOT), &id);
    events.init();

    let resp_id = id.clone();
    let lock = state.run_lock.clone();
    tokio::spawn(async move {
        let _guard = lock.lock().await;
        let prompts = cmd::run::Prompts {
            rewrite: req.prompt.as_deref(),
            image: req.image_prompt.as_deref(),
            podcast: req.podcast_prompt.as_deref(),
        };
        // run_with_config 内部已处理 task_done / task_error
        let _ = cmd::run::run_with_config(
            Path::new(OUTPUT_ROOT),
            &cfg,
            llm.as_ref(),
            &req.text,
            &id,
            req.ref_image.map(PathBuf::from),
            &prompts,
            &events,
        )
        .await;
    });

    Json(json!({"ok": true, "id": resp_id})).into_response()
}

async fn rewrite(State(state): State<AppState>, Json(req): Json<RewriteReq>) -> Response {
    let (_cfg, llm) = match load_ctx() {
        Ok(x) => x,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok": false, "error": e.to_string()}))).into_response(),
    };
    let id = req.id.clone().unwrap_or_else(gen_id);
    let events = TaskEvents::streaming(Path::new(OUTPUT_ROOT), &id);
    events.init();
    let lock = state.run_lock.clone();
    let id2 = id.clone();
    tokio::spawn(async move {
        let _guard = lock.lock().await;
        let r = cmd::rewrite::run_with(Path::new(OUTPUT_ROOT), &req.text, &id2, llm.as_ref(), req.prompt.as_deref(), &events).await;
        match r {
            Ok(_) => events.task_done(),
            Err(e) => events.task_error(&e.to_string()),
        }
    });
    Json(json!({"ok": true, "id": id})).into_response()
}

async fn image(State(state): State<AppState>, Json(req): Json<ImageReq>) -> Response {
    let (cfg, llm) = match load_ctx() {
        Ok(x) => x,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok": false, "error": e.to_string()}))).into_response(),
    };
    let dir = match resolve_dir(req.id) {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e.to_string()}))).into_response(),
    };
    let id = dir_id(&dir);
    let provider = match provider::resolve_image(&cfg) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e.to_string()}))).into_response(),
    };
    let events = TaskEvents::streaming(Path::new(OUTPUT_ROOT), &id);
    let lock = state.run_lock.clone();
    tokio::spawn(async move {
        let _guard = lock.lock().await;
        let r = cmd::image::run_with(&dir, req.ref_image.map(PathBuf::from), llm.as_ref(), provider.as_ref(), req.prompt.as_deref(), &events).await;
        match r {
            Ok(_) => events.task_done(),
            Err(e) => events.task_error(&e.to_string()),
        }
    });
    Json(json!({"ok": true, "id": id})).into_response()
}

async fn podcast(State(state): State<AppState>, Json(req): Json<PodcastReq>) -> Response {
    let (cfg, llm) = match load_ctx() {
        Ok(x) => x,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok": false, "error": e.to_string()}))).into_response(),
    };
    let dir = match resolve_dir(req.id) {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e.to_string()}))).into_response(),
    };
    let id = dir_id(&dir);
    let backend = match podcast_backend::resolve_podcast(&cfg) {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e.to_string()}))).into_response(),
    };
    let events = TaskEvents::streaming(Path::new(OUTPUT_ROOT), &id);
    let lock = state.run_lock.clone();
    tokio::spawn(async move {
        let _guard = lock.lock().await;
        let r = cmd::podcast::run_with(&dir, llm.as_ref(), &backend, req.script, req.prompt.as_deref(), &events).await;
        match r {
            Ok(_) => events.task_done(),
            Err(e) => events.task_error(&e.to_string()),
        }
    });
    Json(json!({"ok": true, "id": id})).into_response()
}

async fn video(State(state): State<AppState>, Json(req): Json<VideoReq>) -> Response {
    let dir = match resolve_dir(req.id) {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e.to_string()}))).into_response(),
    };
    let id = dir_id(&dir);
    let events = TaskEvents::streaming(Path::new(OUTPUT_ROOT), &id);
    let lock = state.run_lock.clone();
    tokio::spawn(async move {
        let _guard = lock.lock().await;
        let events2 = events.clone();
        let dir2 = dir.clone();
        let r = tokio::task::spawn_blocking(move || cmd::video::run_with(&dir2, &events2)).await;
        match r {
            Ok(Ok(())) => events.task_done(),
            Ok(Err(e)) => events.task_error(&e.to_string()),
            Err(e) => events.task_error(&e.to_string()),
        }
    });
    Json(json!({"ok": true, "id": id})).into_response()
}

// ============ 查询端点 ============

async fn list_tasks() -> Response {
    let mut tasks = Vec::new();
    if let Ok(entries) = std::fs::read_dir(OUTPUT_ROOT) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                let id = e.file_name().to_string_lossy().to_string();
                let mut m = task_meta_json(&id);
                if let Some(obj) = m.as_object_mut() {
                    obj.insert("artifacts".into(), json!(list_artifacts(&id)));
                }
                tasks.push(m);
            }
        }
    }
    tasks.sort_by(|a, b| b["id"].as_str().cmp(&a["id"].as_str()));
    Json(json!({"ok": true, "tasks": tasks})).into_response()
}

async fn task_info(AxPath(id): AxPath<String>) -> Response {
    let mut m = task_meta_json(&id);
    if let Some(obj) = m.as_object_mut() {
        obj.insert("artifacts".into(), json!(list_artifacts(&id)));
    }
    Json(m).into_response()
}

/// SSE：订阅任务事件流
async fn task_events(AxPath(id): AxPath<String>) -> Response {
    match crate::task::subscribe(&id) {
        Some(rx) => {
            let stream = futures_util::stream::unfold(rx, |mut rx| async move {
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            let data = serde_json::to_string(&ev).unwrap_or_default();
                            return Some((Ok::<_, Infallible>(SseEvent::default().data(data)), rx));
                        }
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => return None,
                    }
                }
            });
            Sse::new(stream).into_response()
        }
        None => (StatusCode::NOT_FOUND, "任务不存在或已结束").into_response(),
    }
}

async fn download(AxPath((id, name)): AxPath<(String, String)>) -> Response {
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return (StatusCode::BAD_REQUEST, "非法文件名").into_response();
    }
    let path = Path::new(OUTPUT_ROOT).join(&id).join(&name);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let ct = match name.rsplit('.').next() {
                Some("png") | Some("jpg") | Some("jpeg") => "image/png",
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

async fn upload(mut multipart: axum::extract::Multipart) -> Response {
    let result = async {
        let mut saved: Option<String> = None;
        while let Some(field) = multipart.next_field().await? {
            if field.name() != Some("file") {
                continue;
            }
            let filename = field.file_name().unwrap_or("upload").to_string();
            let data = field.bytes().await?;
            anyhow::ensure!(!data.is_empty(), "上传文件为空");
            let ext = Path::new(&filename).extension().and_then(|e| e.to_str()).unwrap_or("png");
            let safe: String = filename
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
                .take(60)
                .collect();
            let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let dir = Path::new("uploads");
            std::fs::create_dir_all(dir)?;
            let path = dir.join(format!("{ts}-{safe}.{ext}"));
            std::fs::write(&path, &data)?;
            saved = Some(path.canonicalize()?.to_string_lossy().to_string());
        }
        match saved {
            Some(p) => Ok(Json(json!({"ok": true, "path": p}))),
            None => anyhow::bail!("未收到文件字段（字段名应为 file）"),
        }
    }
    .await;
    match result {
        Ok(resp) => resp.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e.to_string()}))).into_response(),
    }
}

async fn ui() -> Html<&'static str> {
    Html(include_str!("../web/index.html"))
}

async fn pi_models() -> Vec<String> {
    match pi_rpc::rpc_once(Path::new("pi"), None, json!({"type": "get_available_models"})).await {
        Ok(data) => wizard::parse_available_models(&data),
        Err(_) => vec![],
    }
}

async fn get_config() -> Response {
    let cfg = Config::load(&Config::path()).unwrap_or_default();
    let models = pi_models().await;
    Json(json!({"ok": true, "config": cfg, "models": models})).into_response()
}

async fn put_config(Json(cfg): Json<Config>) -> Response {
    match cfg.save(&Config::path()) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"ok": false, "error": e.to_string()}))).into_response(),
    }
}

async fn fetch_models(Json(req): Json<FetchModelsReq>) -> Response {
    let result = async {
        let url = format!("{}/models", req.base_url.trim_end_matches('/'));
        let client = reqwest::Client::builder().no_proxy().build()?;
        let resp = client.get(&url).bearer_auth(&req.api_key).send().await?;
        anyhow::ensure!(resp.status().is_success(), "拉取模型失败: HTTP {} {}", resp.status(), resp.text().await.unwrap_or_default());
        let v: serde_json::Value = resp.json().await?;
        let mut models: Vec<String> = v["data"]
            .as_array()
            .map(|a| a.iter().filter_map(|m| m["id"].as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        models.sort();
        Ok(Json(json!({"ok": true, "models": models})))
    }
    .await;
    match result {
        Ok(resp) => resp.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"ok": false, "error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize)]
struct FetchModelsReq {
    base_url: String,
    api_key: String,
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
        .route("/api/tasks/:id/events", get(task_events))
        .route("/api/files/:id/:name", get(download))
        .route("/api/upload", post(upload))
        .route("/api/fetch-models", post(fetch_models))
        .route("/api/config", get(get_config).put(put_config))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    println!("✓ Web 服务已启动: http://localhost:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
