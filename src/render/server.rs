//! 算力机侧渲染服务（`media-factory render-server`）。
//!
//! 职责：接收云端下发的分镜任务 → 下载素材 → 生成 composition → HyperFrames 渲染 →
//! ffmpeg 合成（视觉层 + 音频 + 字幕 + 常驻免责声明）→ 回传成品 → **上传成功后删除本地成品与中间产物**。
//!
//! 设计要点：
//! - 无状态、幂等：所有中间文件放 `$HOME/jobs/<job_id>/`，任务结束即删
//! - 并发上限 + 排队：超出并发返回 409（由云端排队）
//! - 进度回调：分阶段 POST 回云端，接进云端的思考链路
//! - 不依赖宿主环境：Node / Chromium / ffmpeg 均来自容器镜像

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tokio::sync::Semaphore;

use super::{
    Callback, FailureReport, HealthInfo, Output, ProgressReport, RenderAccepted, RenderJob,
};

pub struct ServerOpts {
    pub port: u16,
    pub home: PathBuf,
    pub token: Option<String>,
    pub max_concurrent: usize,
    /// hyperframes CLI 路径（容器内默认 /opt/mf-render/node_modules/.bin/hyperframes）
    pub hyperframes_bin: Option<PathBuf>,
    /// GSAP 本地文件（容器内默认 /opt/mf-render/node_modules/gsap/dist/gsap.min.js）
    pub gsap_js: Option<PathBuf>,
}

struct ServerState {
    home: PathBuf,
    token: Option<String>,
    sem: Arc<Semaphore>,
    max_concurrent: usize,
    active: Arc<AtomicUsize>,
    queued: Arc<AtomicUsize>,
    hyperframes_bin: PathBuf,
    gsap_js: PathBuf,
}

impl ServerState {
    fn check_token(&self, headers: &HeaderMap) -> bool {
        match &self.token {
            None => true, // 未配置 token 则不校验（仅建议在内网/隧道内使用）
            Some(t) if t.is_empty() => true,
            Some(t) => headers
                .get("X-Render-Token")
                .and_then(|v| v.to_str().ok())
                .map(|v| v == t)
                .unwrap_or(false),
        }
    }
}

pub async fn run(opts: ServerOpts) -> anyhow::Result<()> {
    let hyperframes_bin = opts.hyperframes_bin.clone().unwrap_or_else(|| {
        PathBuf::from("/opt/mf-render/node_modules/.bin/hyperframes")
    });
    let gsap_js = opts.gsap_js.clone().unwrap_or_else(|| {
        PathBuf::from("/opt/mf-render/node_modules/gsap/dist/gsap.min.js")
    });

    // 启动前自检，问题尽早暴露
    for (what, p) in [("hyperframes", &hyperframes_bin), ("gsap", &gsap_js)] {
        anyhow::ensure!(p.exists(), "{what} 不存在: {}（容器镜像是否完整？）", p.display());
    }
    std::fs::create_dir_all(opts.home.join("jobs"))?;

    let state = Arc::new(ServerState {
        home: opts.home.clone(),
        token: opts.token.clone(),
        sem: Arc::new(Semaphore::new(opts.max_concurrent.max(1))),
        max_concurrent: opts.max_concurrent.max(1),
        active: Arc::new(AtomicUsize::new(0)),
        queued: Arc::new(AtomicUsize::new(0)),
        hyperframes_bin,
        gsap_js,
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/render", post(accept))
        .with_state(state.clone());

    let addr = format!("0.0.0.0:{}", opts.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!(
        "✓ 渲染服务已启动: http://{addr}（并发上限 {}，工作目录 {}）",
        state.max_concurrent,
        opts.home.display()
    );
    if state.token.is_none() {
        println!("  ⚠ 未配置 token（`--token-file`），请确保仅在内网/隧道内可达");
    }
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(st): State<Arc<ServerState>>) -> Response {
    Json(HealthInfo {
        ok: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        active: st.active.load(Ordering::Relaxed),
        max_concurrent: st.max_concurrent,
        queued: st.queued.load(Ordering::Relaxed),
    })
    .into_response()
}

async fn accept(
    State(st): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(job): Json<RenderJob>,
) -> Response {
    if !st.check_token(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid render token"})),
        )
            .into_response();
    }
    let available = st.sem.available_permits();
    let queue_position = st.queued.load(Ordering::Relaxed) + if available == 0 { 1 } else { 0 };

    // ETag 估算：音频时长未知时按场景数与分辨率粗估
    let eta = estimate_eta(&job);

    st.queued.fetch_add(1, Ordering::Relaxed);
    let st2 = st.clone();
    tokio::spawn(async move {
        let permit = st2.sem.clone().acquire_owned().await;
        st2.queued.fetch_sub(1, Ordering::Relaxed);
        let Ok(_permit) = permit else { return };
        st2.active.fetch_add(1, Ordering::Relaxed);
        let job_id = job.job_id.clone();
        if let Err(e) = execute_job(&st2, &job).await {
            eprintln!("✗ 渲染任务 {job_id} 失败: {e:#}");
            report_failure(&job.callback, &e.to_string()).await;
            super::set_outcome(&job_id, super::RenderOutcome::Failed(e.to_string()));
        } else {
            super::set_outcome(&job_id, super::RenderOutcome::Done);
        }
        st2.active.fetch_sub(1, Ordering::Relaxed);
    });

    Json(RenderAccepted {
        accepted: true,
        eta_seconds: eta,
        queue_position,
    })
    .into_response()
}

/// 粗略估算渲染耗时（秒）：按 4.4 分钟音频≈90 秒的经验值折算
fn estimate_eta(job: &RenderJob) -> u64 {
    let fps_factor = 24.0 / job.output.fps.max(1) as f64;
    let scenes = job.composition.scenes.len().max(1) as f64;
    (60.0 * fps_factor * (scenes / 12.0).max(0.5)).round().max(15.0) as u64
}

async fn execute_job(st: &ServerState, job: &RenderJob) -> anyhow::Result<()> {
    let started = Instant::now();
    let work = st.home.join("jobs").join(&job.job_id);
    let assets = work.join("assets");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&assets)?;

    let result = execute_inner(st, job, &work, &assets).await;

    // 失败时保留现场（便于排查），但限制保留量；成功时全部清理
    match &result {
        Ok(_) => {
            let _ = std::fs::remove_dir_all(&work);
            println!(
                "✓ 渲染完成 {job_id}（耗时 {:.1}s，已清理本地文件）",
                started.elapsed().as_secs_f64(),
                job_id = job.job_id
            );
        }
        Err(_) => {
            println!("⚠ 渲染失败 {job_id}，保留工作目录以便排查: {}", work.display(), job_id = job.job_id);
        }
    }
    result
}

async fn execute_inner(
    st: &ServerState,
    job: &RenderJob,
    work: &Path,
    assets: &Path,
) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(120))
        .build()?;

    // 1) 下载素材
    progress(&job.callback, "download", 3.0, "下载素材（封面/音频/字幕）").await;
    let cover = assets.join("cover.png");
    download(&client, &job.media.cover, &cover).await?;
    let audio = work.join("audio.mp3");
    download(&client, &job.media.audio, &audio).await?;
    let srt = match &job.media.subtitles {
        Some(u) => {
            let p = work.join("subtitle.srt");
            download(&client, u, &p).await?;
            Some(p)
        }
        None => None,
    };

    // 2) 生成本地 GSAP + composition HTML
    progress(&job.callback, "compose", 8.0, "生成画面组合").await;
    std::fs::copy(&st.gsap_js, assets.join("gsap.min.js"))?;
    let srt_text = match &srt {
        Some(p) => std::fs::read_to_string(p)?,
        None => String::new(),
    };
    let entries = crate::scene::parse_srt(&srt_text);
    anyhow::ensure!(!entries.is_empty(), "字幕为空，无法生成动态画面");

    let plan = crate::scene::ScenePlan {
        version: 1,
        style: job.composition.style.clone(),
        meta: crate::scene::SceneMeta {
            title: job.composition.title.clone(),
            total_duration: entries.last().map(|e| e.end).unwrap_or(0.0),
        },
        scenes: job.composition.scenes.clone(),
    };
    let html = crate::scene::build_composition(
        &plan,
        &entries,
        &crate::scene::CompositionOpts {
            width: job.output.width,
            height: job.output.height,
            fps: job.output.fps,
            // 免责声明在最终 ffmpeg 合成时叠加（全程常驻），此处不重复渲染
            disclaimer: None,
        },
    );
    std::fs::write(work.join("index.html"), html)?;

    // 3) HyperFrames 渲染视觉层
    progress(&job.callback, "render", 12.0, "渲染动态画面").await;
    let visual = work.join("visual.mp4");
    run_hyperframes(st, job, work, &visual).await?;

    // 4) ffmpeg 合成成品
    progress(&job.callback, "mux", 85.0, "合成音频与字幕").await;
    let out = work.join("video.mp4");
    let font_px = (job.output.width.min(job.output.height) / 52).max(14);
    crate::ffmpeg::mux_video(
        &visual,
        &audio,
        srt.as_deref(),
        job.output.disclaimer.as_deref(),
        font_px,
        &out,
    )?;

    // 5) 回传
    progress(&job.callback, "upload", 94.0, "回传成品视频").await;
    let ack = upload_result(&client, &job.callback, &out).await?;
    anyhow::ensure!(ack.ok, "云端未确认接收");
    progress(
        &job.callback,
        "done",
        100.0,
        &format!("成品已回传（{:.1} MB）", ack.size as f64 / 1_048_576.0),
    )
    .await;
    Ok(())
}

async fn run_hyperframes(
    st: &ServerState,
    job: &RenderJob,
    work: &Path,
    out: &Path,
) -> anyhow::Result<()> {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut cmd = tokio::process::Command::new("node");
    cmd.arg(&st.hyperframes_bin)
        .arg("render")
        .arg("-o")
        .arg(out)
        .arg("-f")
        .arg(job.output.fps.to_string())
        .arg("-q")
        .arg(&job.output.quality)
        .arg("-w")
        .arg(job.output.workers.unwrap_or(16).to_string())
        .current_dir(work)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| anyhow::anyhow!("启动 hyperframes 失败: {e}"))?;
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let cb = job.callback.clone();
    let reader = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        let mut tail = String::new();
        let mut last_pct = 0.0_f64;
        while let Ok(Some(line)) = lines.next_line().await {
            // hyperframes 进度形如 "  ████░░░░  45%  Capturing"
            if let Some(p) = parse_percent(&line) {
                if p - last_pct >= 10.0 {
                    last_pct = p;
                    // 12% → 85% 映射到渲染阶段
                    let mapped = 12.0 + p * 0.73;
                    progress(&cb, "render", mapped, &format!("渲染中 {p:.0}%")).await;
                }
            }
            tail.push_str(&line);
            tail.push('\n');
            truncate_tail(&mut tail, 4000);
        }
        tail
    });

    let err_reader = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut tail = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            eprintln!("[hyperframes] {line}");
            tail.push_str(&line);
            tail.push('\n');
            truncate_tail(&mut tail, 4000);
        }
        tail
    });

    let status = child.wait().await?;
    let out_tail = reader.await.unwrap_or_default();
    let err_tail = err_reader.await.unwrap_or_default();
    anyhow::ensure!(
        status.success(),
        "hyperframes 渲染失败（exit {}）:\n{}",
        status.code().unwrap_or(-1),
        head_chars(
            &if err_tail.is_empty() { out_tail } else { err_tail },
            3000
        )
    );
    anyhow::ensure!(out.exists(), "渲染未产生输出文件");
    Ok(())
}

/// 保留字符串末尾 max 字节（按 UTF-8 字符边界截断，避免 panic）
fn truncate_tail(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut cut = s.len() - max;
    while cut < s.len() && !s.is_char_boundary(cut) {
        cut += 1;
    }
    *s = s[cut..].to_string();
}

/// 安全取前 n 个字符（用于日志）
fn head_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn parse_percent(line: &str) -> Option<f64> {
    let idx = line.find('%')?;
    let before = &line[..idx];
    let num: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    num.parse::<f64>().ok().filter(|v| (0.0..=100.0).contains(v))
}

async fn download(client: &reqwest::Client, url: &str, dest: &Path) -> anyhow::Result<()> {
    let resp = client.get(url).send().await?;
    anyhow::ensure!(
        resp.status().is_success(),
        "下载素材失败 {url}: HTTP {}",
        resp.status()
    );
    let bytes = resp.bytes().await?;
    anyhow::ensure!(!bytes.is_empty(), "素材为空: {url}");
    std::fs::write(dest, &bytes)?;
    Ok(())
}

async fn upload_result(
    client: &reqwest::Client,
    cb: &Callback,
    file: &Path,
) -> anyhow::Result<super::UploadAck> {
    let url = join_url(&cb.base, &cb.result_path);
    let bytes = std::fs::read(file)?;
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(bytes)
            .file_name("video.mp4")
            .mime_str("video/mp4")?,
    );
    let resp = client
        .post(&url)
        .header("X-Render-Token", cb.token.as_str())
        .multipart(form)
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    anyhow::ensure!(status.is_success(), "回传失败（HTTP {status}）：{body}");
    Ok(serde_json::from_str(&body).unwrap_or(super::UploadAck {
        ok: true,
        size: 0,
        sha256: String::new(),
    }))
}

async fn progress(cb: &Callback, stage: &str, percent: f64, message: &str) {
    let url = join_url(&cb.base, &cb.progress_path);
    let client = match reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(10)).build() {
        Ok(c) => c,
        Err(_) => return,
    };
    let body = ProgressReport {
        stage: stage.to_string(),
        percent,
        message: message.to_string(),
    };
    let _ = client
        .post(&url)
        .header("X-Render-Token", cb.token.as_str())
        .json(&body)
        .send()
        .await;
    println!("  · [{stage}] {percent:.0}% {message}");
}

async fn report_failure(cb: &Callback, error: &str) {
    let url = join_url(&cb.base, &cb.failed_path);
    let client = match reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(10)).build() {
        Ok(c) => c,
        Err(_) => return,
    };
    let body = FailureReport {
        error: error.to_string(),
        log_tail: String::new(),
    };
    let _ = client
        .post(&url)
        .header("X-Render-Token", cb.token.as_str())
        .json(&body)
        .send()
        .await;
}

fn join_url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

#[allow(dead_code)]
fn _assert_output_used(_: &Output) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_tail_is_utf8_safe() {
        // 中文多字节：按字节截断若落在字符中间会 panic
        let mut s = "渲染中 45% 场景五".repeat(50);
        truncate_tail(&mut s, 4000);
        assert!(s.len() <= 4000);
        // 断言内容仍是合法 UTF-8（能正常 char 迭代）
        assert!(s.chars().count() > 0);

        // 极小上限也不应 panic
        let mut t = "中文中文中文".to_string();
        truncate_tail(&mut t, 3);
        assert!(t.chars().count() > 0);
    }

    #[test]
    fn head_chars_counts_chars_not_bytes() {
        assert_eq!(head_chars("中文字符串", 3), "中文字");
        assert_eq!(head_chars("abc", 10), "abc");
    }

    #[test]
    fn parse_percent_extracts_progress() {
        assert_eq!(parse_percent("  ████░░  45%  Capturing"), Some(45.0));
        assert_eq!(parse_percent("100% done"), Some(100.0));
        assert_eq!(parse_percent("no number"), None);
        assert_eq!(parse_percent("999%"), None);
    }

    #[test]
    fn join_url_handles_slashes() {
        assert_eq!(join_url("http://h:1/", "/api/x"), "http://h:1/api/x");
        assert_eq!(join_url("http://h:1", "api/x"), "http://h:1api/x");
    }
}
