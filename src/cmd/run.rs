use std::path::{Path, PathBuf};

use crate::cmd::{image, podcast, rewrite, video};
use crate::config::Config;
use crate::llm::LlmAgent;
use crate::pi_rpc::PiRpcAgent;
use crate::podcast as podcast_backend;
use crate::provider;

/// 串联执行全部四步：改写 → 生图 → 播客 → 视频。
/// 任一步失败即停止，可用 `--id <id> <失败子命令>` 续跑。
pub async fn run(input: Option<String>, id: Option<String>, reference: Option<String>) -> anyhow::Result<()> {
    let cfg = Config::load(&Config::path())?;
    let llm = PiRpcAgent::new(cfg.tasks.llm.clone().map(|l| l.model))?;
    let source = rewrite::read_input(input)?;
    run_with_config(
        Path::new("output"),
        &cfg,
        &llm,
        &source,
        id,
        reference.map(PathBuf::from),
    )
    .await
}

/// 可注入依赖的完整流水线（测试用）
pub async fn run_with_config(
    output_root: &Path,
    cfg: &Config,
    llm: &dyn LlmAgent,
    source: &str,
    id: Option<String>,
    reference: Option<PathBuf>,
) -> anyhow::Result<()> {
    let id = rewrite::run_with(output_root, source, id, llm).await?;

    let img_provider = provider::resolve_image(cfg)?;
    image::run_with(output_root, &id, reference, llm, img_provider.as_ref()).await?;

    let backend = podcast_backend::resolve_podcast(cfg)?;
    podcast::run_with(output_root, &id, llm, &backend, false).await?;

    video::run_with(output_root, &id)?;

    println!("完成！产物目录: {}/{}/", output_root.display(), id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ProviderConfig, TaskSelection};

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
        cfg.tasks.llm = Some(crate::config::LlmSelection {
            model: "mock/model".into(),
        });
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

        run_with_config(&root, &cfg, &llm, "原始参考内容", Some("e2e1".into()), None)
            .await
            .unwrap();

        for f in ["input.md", "rewritten.md", "image.png", "script.md", "podcast.mp3", "video.mp4"] {
            assert!(root.join("e2e1").join(f).exists(), "缺少产物 {f}");
        }
    }
}
