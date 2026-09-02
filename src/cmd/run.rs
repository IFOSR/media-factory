use std::path::{Path, PathBuf};

use crate::cmd::{image, podcast, rewrite, video};
use crate::config::Config;
use crate::llm::LlmAgent;
use crate::podcast as podcast_backend;
use crate::provider;
use crate::task::{Step, TaskEvents};

/// 各步骤的用户自定义 prompt
pub struct Prompts<'a> {
    pub rewrite: Option<&'a str>,
    pub image: Option<&'a str>,
    pub podcast: Option<&'a str>,
}

/// 串联执行全部四步：改写 → 生图 → 播客 → 视频。
/// 任一步失败即停止，可用 `--id <id> <失败子命令>` 续跑。
pub async fn run(
    input: Option<String>,
    id: Option<String>,
    reference: Option<String>,
    rewrite_prompt: Option<String>,
    image_prompt: Option<String>,
    podcast_prompt: Option<String>,
) -> anyhow::Result<String> {
    let cfg = Config::load(&Config::path())?;
    let llm = crate::llm::resolve_llm(&cfg)?;
    let source = rewrite::read_input(input)?;
    let id = id.unwrap_or_else(rewrite::gen_task_id);
    let events = TaskEvents::local(Path::new("output"), &id);
    events.init();
    let prompts = Prompts {
        rewrite: rewrite_prompt.as_deref(),
        image: image_prompt.as_deref(),
        podcast: podcast_prompt.as_deref(),
    };
    run_with_config(
        Path::new("output"),
        &cfg,
        llm.as_ref(),
        &source,
        &id,
        reference.map(PathBuf::from),
        &prompts,
        &events,
        None,
    )
    .await
}

/// 可注入依赖的完整流水线（测试用）
#[allow(clippy::too_many_arguments)]
pub async fn run_with_config(
    output_root: &Path,
    cfg: &Config,
    llm: &dyn LlmAgent,
    source: &str,
    id: &str,
    reference: Option<PathBuf>,
    prompts: &Prompts<'_>,
    events: &TaskEvents,
    podcast_speakers: Option<Vec<String>>,
) -> anyhow::Result<String> {
    let id = match rewrite::run_with(output_root, source, id, llm, prompts.rewrite, events).await {
        Ok(id) => id,
        Err(e) => {
            events.step_failed(Step::Rewrite, &e.to_string());
            events.task_error(&e.to_string());
            return Err(e);
        }
    };
    let dir = output_root.join(&id);

    let img_provider = match provider::resolve_image(cfg) {
        Ok(p) => p,
        Err(e) => {
            events.step_failed(Step::Image, &e.to_string());
            events.task_error(&e.to_string());
            return Err(e);
        }
    };
    if let Err(e) = image::run_with(&dir, reference, llm, img_provider.as_ref(), prompts.image, events).await {
        events.step_failed(Step::Image, &e.to_string());
        events.task_error(&e.to_string());
        return Err(e);
    }

    let backend = match podcast_backend::resolve_podcast(cfg) {
        Ok(b) => b,
        Err(e) => {
            events.step_failed(Step::Podcast, &e.to_string());
            events.task_error(&e.to_string());
            return Err(e);
        }
    };
    let backend = match (backend, podcast_speakers) {
        (podcast_backend::PodcastBackend::Volc(v), Some(sp)) => {
            podcast_backend::PodcastBackend::Volc(v.with_speakers(sp))
        }
        (b, _) => b,
    };
    if let Err(e) = podcast::run_with(&dir, llm, &backend, false, prompts.podcast, events).await {
        events.step_failed(Step::Podcast, &e.to_string());
        events.task_error(&e.to_string());
        return Err(e);
    }

    if let Err(e) = video::run_with(&dir, events) {
        events.step_failed(Step::Video, &e.to_string());
        events.task_error(&e.to_string());
        return Err(e);
    }

    events.task_done();
    println!("完成！产物目录: {}/{}/", output_root.display(), id);
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ProviderConfig, TaskSelection};
    use crate::pi_rpc::PiRpcAgent;

    use base64::Engine as _;

    /// 假 pi：根据 prompt 关键词返回不同的文本
    fn fake_pi(dir: &Path) -> PathBuf {
        let script = r#"#!/bin/bash
while IFS= read -r line; do
  case "$line" in
    *"图像提示词"*)
      echo '{"id":"req-1","type":"response","command":"prompt","success":true}'
      echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"a cat at sunset"}}'
      echo '{"type":"agent_settled"}'
      ;;
    *"播客脚本"*)
      echo '{"id":"req-1","type":"response","command":"prompt","success":true}'
      echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"主持人：大家好"}}'
      echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"\n嘉宾：今天聊个有趣的"}}'
      echo '{"type":"agent_settled"}'
      ;;
    *)
      echo '{"id":"req-1","type":"response","command":"prompt","success":true}'
      echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"这是改写后的爆款文案"}}'
      echo '{"type":"agent_settled"}'
      ;;
  esac
done
"#;
        let p = dir.join("pi");
        std::fs::write(&p, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    /// 构建全 mock 的 Config：image 与 podcast 都走 openai-compatible（wiremock）
    fn mock_config(image_base: String, tts_base: String) -> Config {
        let mut cfg = Config::default();
        cfg.tasks.llm = Some(crate::config::LlmSelection::Provider(TaskSelection {
            provider: "pi".into(),
        }));
        cfg.tasks.image = Some(TaskSelection {
            provider: "mock-img".into(),
        });
        cfg.tasks.podcast = Some(TaskSelection {
            provider: "mock-tts".into(),
        });
        cfg.providers.insert(
            "mock-img".into(),
            ProviderConfig::Custom {
                base_url: image_base,
                api_key: "k".into(),
                model: "any-model".into(),
            },
        );
        cfg.providers.insert(
            "mock-tts".into(),
            ProviderConfig::Custom {
                base_url: tts_base,
                api_key: "k".into(),
                model: "tts-1".into(),
            },
        );
        cfg
    }

    #[tokio::test]
    async fn end_to_end_pipeline_produces_all_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let pi_bin = fake_pi(dir.path());

        // 生图 wiremock：返回一张真实 PNG，供 ffmpeg 合成视频
        let png_path = dir.path().join("img.png");
        let st = std::process::Command::new("ffmpeg")
            .args(["-y", "-f", "lavfi", "-i", "color=c=red:s=64x64", "-frames:v", "1"])
            .arg(&png_path)
            .status()
            .unwrap();
        assert!(st.success());
        let valid_png = std::fs::read(&png_path).unwrap();

        let img_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/images/generations"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "data": [{"content": base64::engine::general_purpose::STANDARD.encode(valid_png)}]
                })),
            )
            .mount(&img_server)
            .await;

        // TTS wiremock（每个脚本句一次）；返回一段真实 mp3，供 ffmpeg 拼接
        let mp3_path = dir.path().join("voice.mp3");
        let st = std::process::Command::new("ffmpeg")
            .args(["-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .args(["-codec:a", "libmp3lame"])
            .arg(&mp3_path)
            .status()
            .unwrap();
        assert!(st.success());
        let valid_mp3 = std::fs::read(&mp3_path).unwrap();

        let tts_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/audio/speech"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_bytes(valid_mp3))
            .mount(&tts_server)
            .await;

        let cfg = mock_config(img_server.uri(), tts_server.uri());
        let llm = PiRpcAgent::with_binary(pi_bin, None).unwrap();
        let root = dir.path().join("output");
        let events = crate::task::TaskEvents::local(&root, "e2e1");

        run_with_config(&root, &cfg, &llm, "原始参考内容", "e2e1", None, &Prompts { rewrite: None, image: None, podcast: None }, &events, None)
            .await
            .unwrap();

        for f in ["input.md", "rewritten.md", "image.png", "script.md", "podcast.mp3", "video.mp4"] {
            assert!(root.join("e2e1").join(f).exists(), "缺少产物 {f}");
        }
    }
}
