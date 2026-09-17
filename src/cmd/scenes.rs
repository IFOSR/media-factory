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

/// 单次请求覆盖的字幕段数上限（长音频分块规划，避免输出被截断）
pub const CHUNK_ENTRIES: usize = 50;

/// 核心流程（可注入 llm 以便测试）
///
/// 长音频（字幕段多）会**分块规划**：每块只要求覆盖一段索引范围，最后合并 + 校验。
/// 这样单次模型输出规模可控，避免"输出过长被截断 → 拿不到完整 JSON"。
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
    let total_min = entries.last().map(|e| e.end).unwrap_or(0.0) / 60.0;
    events.log(
        Step::Scenes,
        &format!("读取字幕时间轴（{} 段，共 {:.1} 分钟）", entries.len(), total_min),
    );

    let title = std::fs::read_to_string(dir.join("rewritten.md"))
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| !l.trim().is_empty())
                .map(|l| l.trim().chars().take(30).collect::<String>())
        })
        .unwrap_or_default();

    // 分块：长音频按 CHUNK_ENTRIES 切分；最后一块过小时并入前一块
    let ranges = chunk_ranges(entries.len());
    if ranges.len() > 1 {
        events.log(
            Step::Scenes,
            &format!("音频较长，分 {} 批规划画面场景", ranges.len()),
        );
    }

    let mut all_scenes: Vec<scene::Scene> = Vec::new();
    for (idx, (from, to)) in ranges.iter().enumerate() {
        let range_note = if ranges.len() > 1 {
            format!(
                "**本次只需规划条目 [{from}] 到 [{to}] 这一段**：from_entry 不得小于 {from}，to_entry 不得大于 {to}，并且必须完整覆盖这一段（第一个场景从 {from} 开始，最后一个场景到 {to} 结束）。"
            )
        } else {
            format!("必须完整覆盖 [{from}] 到 [{to}]（第一个场景从 {from} 开始，最后一个场景到 {to} 结束）。")
        };
        let prompt = prompt_template()
            .replace("{{ENTRIES}}", &render_entries(&entries[*from..=*to]))
            .replace("{{TITLE}}", &title)
            .replace("{{RANGE}}", &range_note);

        if ranges.len() > 1 {
            events.log(
                Step::Scenes,
                &format!("第 {}/{} 批：条目 [{from}]-[{to}]", idx + 1, ranges.len()),
            );
        } else {
            events.log(Step::Scenes, "调用语言模型规划画面场景");
        }

        let scenes = plan_chunk(llm, &prompt, events).await?;
        events.log(
            Step::Scenes,
            &format!("第 {}/{} 批完成，产出 {} 个场景", idx + 1, ranges.len(), scenes.len()),
        );
        all_scenes.extend(scenes);
    }

    let mut plan = ScenePlan {
        version: 1,
        style: "dark-tech".to_string(),
        meta: scene::SceneMeta {
            title,
            total_duration: entries.last().map(|e| e.end).unwrap_or(0.0),
        },
        scenes: all_scenes,
    };

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

/// 把条目总数切成若干区间（闭区间），最后一块过小时并入前一块
fn chunk_ranges(n: usize) -> Vec<(usize, usize)> {
    if n == 0 {
        return vec![];
    }
    if n <= CHUNK_ENTRIES {
        return vec![(0, n - 1)];
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < n {
        let end = (start + CHUNK_ENTRIES - 1).min(n - 1);
        out.push((start, end));
        start = end + 1;
    }
    // 最后一块过小（< 1/3 块大小）时并入前一块
    if out.len() >= 2 {
        let last = *out.last().unwrap();
        let size = last.1 - last.0 + 1;
        if size < CHUNK_ENTRIES / 3 {
            out.pop();
            let prev = out.pop().unwrap();
            out.push((prev.0, last.1));
        }
    }
    out
}

/// 单批规划：请求 → 提取 JSON → 解析；失败则带"只输出 JSON"的修复提示重试一次
async fn plan_chunk(
    llm: &dyn LlmAgent,
    prompt: &str,
    events: &TaskEvents,
) -> anyhow::Result<Vec<scene::Scene>> {
    let mut last_raw = String::new();
    for attempt in 1..=2 {
        let p = if attempt == 1 {
            prompt.to_string()
        } else {
            format!("{prompt}\n\n注意：上一次的输出无法解析。请**只输出一个完整合法的 JSON 对象**，不要任何解释文字、不要 markdown 代码块，并确保 JSON 完整闭合。")
        };
        let raw = llm.complete(&p).await?;
        last_raw = raw.clone();
        match extract_json(&raw).and_then(|j| serde_json::from_str::<ScenePlan>(j).ok()) {
            Some(plan) => return Ok(plan.scenes),
            None => {
                events.log(
                    Step::Scenes,
                    &format!(
                        "第 {attempt} 次输出无法解析，模型输出片段：{}",
                        raw.chars().take(120).collect::<String>().replace('\n', " ")
                    ),
                );
            }
        }
    }
    anyhow::bail!(
        "模型输出中未找到可解析的 JSON 分镜（已重试 1 次）\n最后输出片段: {}",
        last_raw.chars().take(300).collect::<String>()
    )
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

#[cfg(test)]
mod chunk_tests {
    use super::*;
    use crate::llm::LlmAgent;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn chunk_ranges_split_and_merge_tail() {
        assert_eq!(chunk_ranges(0), vec![]);
        assert_eq!(chunk_ranges(10), vec![(0, 9)]);
        assert_eq!(chunk_ranges(50), vec![(0, 49)]);
        // 尾巴过小 → 并入前一块
        assert_eq!(chunk_ranges(51), vec![(0, 50)]);
        // 202 段 → 4 块（最后一块 52 段，足够大）
        assert_eq!(chunk_ranges(202), vec![(0, 49), (50, 99), (100, 149), (150, 201)]);
        // 165 段：最后一块 15 段 < 50/3 → 并入
        assert_eq!(chunk_ranges(165), vec![(0, 49), (50, 99), (100, 164)]);
        // 区间连续且无重叠
        for w in chunk_ranges(202).windows(2) {
            assert_eq!(w[0].1 + 1, w[1].0);
        }
    }

    struct CountingLlm {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl LlmAgent for CountingLlm {
        async fn complete(&self, _p: &str) -> anyhow::Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(r#"{"version":1,"style":"dark-tech","scenes":[
              {"type":"cover","from_entry":0,"to_entry":0,"title":"标题"},
              {"type":"bullets","from_entry":1,"to_entry":2,"heading":"要点","items":["一","二"]}]}"#
                .to_string())
        }
    }

    #[tokio::test]
    async fn long_audio_is_planned_in_chunks_and_covers_timeline() {
        let d = tempfile::tempdir().unwrap();
        // 造 120 段字幕（模拟长音频）
        let mut srt = String::new();
        for i in 0..120 {
            srt.push_str(&format!(
                "{}\n{:02}:{:02}:{:02},000 --> {:02}:{:02}:{:02},000\n嘉宾：第 {} 句内容\n\n",
                i + 1,
                i / 60, i % 60, 0,
                i / 60, i % 60, 3,
                i
            ));
        }
        std::fs::write(d.path().join("subtitle.srt"), srt).unwrap();
        std::fs::write(d.path().join("rewritten.md"), "标题\n正文").unwrap();

        let calls = Arc::new(AtomicUsize::new(0));
        let llm = CountingLlm { calls: calls.clone() };
        let ev = TaskEvents::local(d.path(), "t");
        let plan = run_with(d.path(), &llm, &ev).await.unwrap();

        // 120 段 → 3 批
        assert_eq!(calls.load(Ordering::SeqCst), 3, "应按块多次调用模型");
        assert!(!plan.scenes.is_empty());
        // 覆盖全时间轴（sanitize 保证）
        let entries = scene::parse_srt(&std::fs::read_to_string(d.path().join("subtitle.srt")).unwrap());
        let resolved = scene::resolve(&plan, &entries);
        assert!(resolved[0].start.abs() < 1e-6);
        assert!((resolved.last().unwrap().end - entries.last().unwrap().end).abs() < 1.0);
    }

    struct BadThenGoodLlm {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl LlmAgent for BadThenGoodLlm {
        async fn complete(&self, _p: &str) -> anyhow::Result<String> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                // 第一次：散文，无 JSON
                Ok("好的，我来帮你规划分镜。".to_string())
            } else {
                Ok(r#"{"version":1,"scenes":[{"type":"cover","from_entry":0,"to_entry":1,"title":"T"}]}"#.to_string())
            }
        }
    }

    #[tokio::test]
    async fn retries_once_when_output_unparsable() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("subtitle.srt"),
            "1\n00:00:00,000 --> 00:00:03,000\n主持人：你好\n\n2\n00:00:03,000 --> 00:00:06,000\n嘉宾：嗯\n\n",
        )
        .unwrap();
        std::fs::write(d.path().join("rewritten.md"), "标题").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let llm = BadThenGoodLlm { calls: calls.clone() };
        let ev = TaskEvents::local(d.path(), "t");
        let plan = run_with(d.path(), &llm, &ev).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2, "应在解析失败后重试一次");
        assert_eq!(plan.scenes.len(), 1);
    }
}
