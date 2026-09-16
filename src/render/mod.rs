//! 视频合成：`cover`（封面图贯穿，本机 ffmpeg）与 `dynamic`（动态解释画面，算力机渲染服务）。

pub mod server;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::scene;
use crate::task::{Step, TaskEvents};

// ---------------------------------------------------------------- 任务协议
// 云端（编排）→ 算力机（渲染服务）的任务格式；两侧共用这些类型。

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderJob {
    pub job_id: String,
    pub callback: Callback,
    pub composition: Composition,
    pub media: Media,
    pub output: Output,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Callback {
    /// 云端基地址（算力机可达，例如 http://1.2.3.4:8092）
    pub base: String,
    /// 回传鉴权 token
    pub token: String,
    pub progress_path: String,
    pub result_path: String,
    pub failed_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Composition {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub style: String,
    pub scenes: Vec<scene::Scene>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Media {
    pub cover: String,
    pub audio: String,
    #[serde(default)]
    pub subtitles: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Output {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    #[serde(default = "default_quality")]
    pub quality: String,
    #[serde(default = "default_true")]
    pub burn_subtitles: bool,
    #[serde(default)]
    pub disclaimer: Option<String>,
    #[serde(default)]
    pub workers: Option<usize>,
}

fn default_quality() -> String {
    "looks".to_string()
}
fn default_true() -> bool {
    true
}

/// 渲染进度回调体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressReport {
    pub stage: String,
    pub percent: f64,
    pub message: String,
}

/// 失败回调体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureReport {
    pub error: String,
    #[serde(default)]
    pub log_tail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderAccepted {
    pub accepted: bool,
    #[serde(default)]
    pub eta_seconds: u64,
    #[serde(default)]
    pub queue_position: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthInfo {
    pub ok: bool,
    pub version: String,
    pub active: usize,
    pub max_concurrent: usize,
    pub queued: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadAck {
    pub ok: bool,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub sha256: String,
}

// ---------------------------------------------------------------- 结果登记表
// 云端在等待期间通过它获知渲染结果（成功/失败原因）

#[derive(Debug, Clone)]
pub enum RenderOutcome {
    Done,
    Failed(String),
}

static OUTCOMES: OnceLock<Mutex<HashMap<String, RenderOutcome>>> = OnceLock::new();

fn outcomes() -> &'static Mutex<HashMap<String, RenderOutcome>> {
    OUTCOMES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn set_outcome(job_id: &str, outcome: RenderOutcome) {
    outcomes().lock().unwrap().insert(job_id.to_string(), outcome);
}

pub fn take_outcome(job_id: &str) -> Option<RenderOutcome> {
    outcomes().lock().unwrap().remove(job_id)
}

// ---------------------------------------------------------------- 封面模式（本机）

/// 封面图贯穿：图片 + 音频 [+ 字幕]
pub fn compose_cover(dir: &Path, events: &TaskEvents) -> anyhow::Result<()> {
    let image = dir.join("image.png");
    let audio = dir.join("podcast.mp3");
    anyhow::ensure!(
        image.exists(),
        "缺少 {}/image.png，请先运行 `media-factory image`",
        dir.display()
    );
    anyhow::ensure!(
        audio.exists(),
        "缺少 {}/podcast.mp3，请先运行 `media-factory podcast`",
        dir.display()
    );

    let out = dir.join("video.mp4");
    let srt = dir.join("subtitle.srt");
    let subtitle = if srt.exists() { Some(srt.as_path()) } else { None };
    if subtitle.is_some() {
        events.log(Step::Video, "封面模式：ffmpeg 合成（封面图 + 音频 + 字幕）");
    } else {
        events.log(Step::Video, "封面模式：ffmpeg 合成（封面图 + 音频）");
    }
    crate::ffmpeg::make_video(&image, &audio, subtitle, &out)?;
    Ok(())
}

// ---------------------------------------------------------------- 动态模式（算力机）

/// 动态解释画面：把分镜任务提交给算力机渲染服务，等待成品回传。
pub async fn compose_dynamic(
    dir: &Path,
    cfg: &Config,
    task_id: &str,
    events: &TaskEvents,
    disclaimer: Option<String>,
) -> anyhow::Result<()> {
    let d = &cfg.video.dynamic;
    anyhow::ensure!(
        !d.renderer_url.trim().is_empty(),
        "未配置渲染服务地址（video.dynamic.renderer_url）"
    );
    anyhow::ensure!(
        !d.callback_base.trim().is_empty(),
        "未配置回调地址（video.dynamic.callback_base），算力机无法回传成品；\
         请在配置中填写本服务的可达地址（如 http://1.2.3.4:8092），或改用 cover 模式"
    );

    let scene_path = dir.join("scene.json");
    anyhow::ensure!(
        scene_path.exists(),
        "缺少 {}/scene.json，请先运行 `media-factory scenes`",
        dir.display()
    );
    let plan: scene::ScenePlan = serde_json::from_str(&std::fs::read_to_string(&scene_path)?)?;

    let srt_path = dir.join("subtitle.srt");
    let entries = if srt_path.exists() {
        scene::parse_srt(&std::fs::read_to_string(&srt_path)?)
    } else {
        Vec::new()
    };
    anyhow::ensure!(!entries.is_empty(), "缺少字幕时间轴（subtitle.srt），无法生成动态画面");

    let base = d.callback_base.trim_end_matches('/');
    let job = RenderJob {
        job_id: task_id.to_string(),
        callback: Callback {
            base: base.to_string(),
            token: d.token.clone(),
            progress_path: format!("/api/render-jobs/{task_id}/progress"),
            result_path: format!("/api/render-results/{task_id}"),
            failed_path: format!("/api/render-jobs/{task_id}/failed"),
        },
        composition: Composition {
            title: plan.meta.title.clone(),
            style: if plan.style.is_empty() { "dark-tech".into() } else { plan.style.clone() },
            scenes: plan.scenes.clone(),
        },
        media: Media {
            cover: format!("{base}/api/files/{task_id}/image.png"),
            audio: format!("{base}/api/files/{task_id}/podcast.mp3"),
            subtitles: if srt_path.exists() {
                Some(format!("{base}/api/files/{task_id}/subtitle.srt"))
            } else {
                None
            },
        },
        output: Output {
            width: 1920,
            height: 1080,
            fps: d.fps,
            quality: d.quality.clone(),
            burn_subtitles: true,
            disclaimer,
            workers: Some(d.workers),
        },
    };

    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(60))
        .build()?;
    let url = format!("{}/render", d.renderer_url.trim_end_matches('/'));

    events.log(Step::Video, &format!("提交渲染任务到算力机（{url}）"));
    let resp = client
        .post(&url)
        .header("X-Render-Token", d.token.as_str())
        .json(&job)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("无法连接渲染服务 {url}: {e}"))?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    anyhow::ensure!(
        status.is_success(),
        "渲染服务拒绝任务（HTTP {status}）：{body}"
    );
    if let Ok(ack) = serde_json::from_str::<RenderAccepted>(&body) {
        if ack.queue_position > 0 {
            events.log(Step::Video, &format!("已排队（前方 {} 个任务），预计 {} 秒", ack.queue_position, ack.eta_seconds));
        } else {
            events.log(Step::Video, &format!("渲染中，预计 {} 秒", ack.eta_seconds));
        }
    }

    // 等待：算力机渲染完成后会 POST 回云端（由 web 服务落盘 video.mp4 并登记结果）
    let out = dir.join("video.mp4");
    let deadline = Instant::now() + Duration::from_secs(d.timeout_seconds.max(60));
    let mut last_note = Instant::now();
    loop {
        if let Some(o) = take_outcome(task_id) {
            match o {
                RenderOutcome::Done => return Ok(()),
                RenderOutcome::Failed(e) => anyhow::bail!("算力机渲染失败：{e}"),
            }
        }
        if out.exists() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "等待渲染超时（{} 秒）",
                d.timeout_seconds
            );
        }
        if last_note.elapsed() > Duration::from_secs(30) {
            events.log(Step::Video, "仍在渲染中…");
            last_note = Instant::now();
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_registry_roundtrip() {
        set_outcome("j1", RenderOutcome::Done);
        assert!(matches!(take_outcome("j1"), Some(RenderOutcome::Done)));
        assert!(take_outcome("j1").is_none());

        set_outcome("j2", RenderOutcome::Failed("boom".into()));
        match take_outcome("j2") {
            Some(RenderOutcome::Failed(e)) => assert_eq!(e, "boom"),
            _ => panic!("expected failure"),
        }
    }

    #[test]
    fn job_serializes_with_expected_shape() {
        let job = RenderJob {
            job_id: "20260916-120000-ab12".into(),
            callback: Callback {
                base: "http://1.2.3.4:8092".into(),
                token: "t".into(),
                progress_path: "/api/render-jobs/x/progress".into(),
                result_path: "/api/render-results/x".into(),
                failed_path: "/api/render-jobs/x/failed".into(),
            },
            composition: Composition {
                title: "标题".into(),
                style: "dark-tech".into(),
                scenes: vec![scene::Scene::Cover {
                    from_entry: 0,
                    to_entry: 1,
                    title: "封面".into(),
                    subtitle: String::new(),
                }],
            },
            media: Media {
                cover: "http://1.2.3.4:8092/api/files/x/image.png".into(),
                audio: "http://1.2.3.4:8092/api/files/x/podcast.mp3".into(),
                subtitles: None,
            },
            output: Output {
                width: 1920,
                height: 1080,
                fps: 24,
                quality: "looks".into(),
                burn_subtitles: true,
                disclaimer: Some("免责".into()),
                workers: Some(16),
            },
        };
        let v: serde_json::Value = serde_json::to_value(&job).unwrap();
        assert_eq!(v["job_id"], "20260916-120000-ab12");
        assert_eq!(v["composition"]["scenes"][0]["type"], "cover");
        assert_eq!(v["output"]["fps"], 24);
        // 反序列化回来（算力机侧）
        let back: RenderJob = serde_json::from_value(v).unwrap();
        assert_eq!(back.composition.scenes.len(), 1);
    }
}
