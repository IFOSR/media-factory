//! 步骤：分镜（`scenes`）——把字幕时间轴交给 LLM，生成可编辑的 `scene.json`。

use std::path::Path;

use crate::config::Config;
use crate::llm::LlmAgent;
use crate::scene::{self, ScenePlan};
use crate::task::{Step, TaskEvents};

fn prompt_template() -> String {
    if let Ok(t) = std::fs::read_to_string(crate::config::data_dir().join("prompts/scenes.txt")) {
        return t;
    }
    include_str!("../../prompts/scenes.txt").to_string()
}

/// 渲染字幕清单（带序号，供 LLM 用 `from_entry`/`to_entry` 引用）
fn render_entries(entries: &[crate::podcast::SubtitleEntry]) -> String {
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| format!("[{i}] {:.1}s {:.1}s {}", e.start, e.end, e.text))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 从 LLM 输出中提取 JSON（容忍 ```json 包裹与前后说明文字）
pub fn extract_json(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(&s[start..=end])
}

/// 核心流程（可注入 llm 以便测试）
pub async fn run_with(dir: &Path, llm: &dyn LlmAgent, events: &TaskEvents) -> anyhow::Result<ScenePlan> {
    let srt_path = dir.join("subtitle.srt");
    anyhow::ensure!(
        srt_path.exists(),
        "缺少 {}/subtitle.srt（需要播客字幕时间轴），请先运行 `media-factory podcast`",
        dir.display()
    );
    let entries = scene::parse_srt(&std::fs::read_to_string(&srt_path)?);
    anyhow::ensure!(!entries.is_empty(), "字幕为空，无法生成分镜");

    events.step_running(Step::Scenes);
    events.log(Step::Scenes, &format!("读取字幕时间轴（{} 段，共 {:.1} 分钟）", entries.len(),
        entries.last().map(|e| e.end).unwrap_or(0.0) / 60.0));

    let title = std::fs::read_to_string(dir.join("rewritten.md"))
        .ok()
        .and_then(|t| t.lines().find(|l| !l.trim().is_empty()).map(|l| l.trim().chars().take(30).collect::<String>()))
        .unwrap_or_default();

    let prompt = prompt_template()
        .replace("{{ENTRIES}}", &render_entries(&entries))
        .replace("{{TITLE}}", &title);

    events.log(Step::Scenes, "调用语言模型规划画面场景");
    let raw = llm.complete(&prompt).await?;

    let json = extract_json(&raw).ok_or_else(|| anyhow::anyhow!("模型输出中未找到 JSON 分镜"))?;
    let mut plan: ScenePlan = serde_json::from_str(json)
        .map_err(|e| anyhow::anyhow!("分镜 JSON 解析失败: {e}\n原始输出片段: {}", raw.chars().take(400).collect::<String>()))?;

    if plan.meta.title.is_empty() {
        plan.meta.title = title;
    }

    let warnings = scene::sanitize(&mut plan, &entries);
    for w in &warnings {
        events.log(Step::Scenes, &format!("校验：{w}"));
    }

    let types = plan
        .scenes
        .iter()
        .map(|s| s.kind())
        .collect::<Vec<_>>()
        .join(" → ");
    events.log(
        Step::Scenes,
        &format!("已生成 {} 个场景：{types}", plan.scenes.len()),
    );

    let out = dir.join("scene.json");
    std::fs::write(&out, serde_json::to_string_pretty(&plan)?)?;
    events.artifact(Step::Scenes, "scene.json");
    events.step_done(Step::Scenes);
    println!("✓ 分镜已生成: {}（{} 个场景）", out.display(), plan.scenes.len());
    Ok(plan)
}

/// 公开入口（CLI）
pub async fn run(id: Option<String>) -> anyhow::Result<String> {
    let dir = match &id {
        Some(i) => super::task_dir(crate::config::output_root().as_path(), i),
        None => super::latest_task_dir(crate::config::output_root().as_path())?,
    };
    let id = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let cfg = Config::load(&Config::path())?;
    let llm = crate::llm::resolve_llm(&cfg)?;
    let events = crate::task::TaskEvents::local(crate::config::output_root().as_path(), &id);
    run_with(&dir, llm.as_ref(), &events).await?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmAgent;

    struct FakeLlm(String);
    #[async_trait::async_trait]
    impl LlmAgent for FakeLlm {
        async fn complete(&self, _p: &str) -> anyhow::Result<String> {
            Ok(self.0.clone())
        }
    }

    fn setup(dir: &Path) {
        std::fs::write(
            dir.join("subtitle.srt"),
            "1\n00:00:00,000 --> 00:00:03,000\n主持人：今天聊聊降息\n\n\
             2\n00:00:03,000 --> 00:00:08,000\n嘉宾：金价可能冲高回落\n\n\
             3\n00:00:08,000 --> 00:00:14,000\n嘉宾：用户已经突破 10 亿\n\n",
        )
        .unwrap();
        std::fs::write(dir.join("rewritten.md"), "美联储降息，金价怎么走？\n正文").unwrap();
    }

    #[tokio::test]
    async fn scenes_writes_json_and_validates() {
        let d = tempfile::tempdir().unwrap();
        setup(d.path());
        let llm = FakeLlm(r#"```json
{"version":1,"style":"dark-tech","meta":{"title":""},"scenes":[
 {"type":"cover","from_entry":0,"to_entry":0,"title":"美联储降息","subtitle":"三种走势"},
 {"type":"bullets","from_entry":1,"to_entry":1,"heading":"可能的走势","items":["冲高回落","承压震荡"]},
 {"type":"metric","from_entry":2,"to_entry":2,"value":"10","unit":"亿","label":"用户规模","source_quote":"用户已经突破 10 亿"}
]}
```"#.to_string());
        let ev = TaskEvents::local(d.path(), "t");
        let plan = run_with(d.path(), &llm, &ev).await.unwrap();
        assert_eq!(plan.scenes.len(), 3);
        assert!(d.path().join("scene.json").exists());
        // 兜底标题来自 rewritten.md
        assert!(!plan.meta.title.is_empty());
    }

    #[tokio::test]
    async fn extract_json_tolerates_prose() {
        let s = "好的，以下是分镜：\n{\"a\":1}\n希望有帮助";
        assert_eq!(extract_json(s), Some("{\"a\":1}"));
        assert!(extract_json("no json here").is_none());
    }
}
