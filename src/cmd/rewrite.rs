use std::io::Read;
use std::path::Path;

use crate::config::Config;
use crate::llm::LlmAgent;
use crate::task::{Step, TaskEvents};

/// 生成默认任务 id
pub(crate) fn gen_task_id() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// 读取改写 prompt 模板：优先运行时读 cwd/prompts/rewrite.txt（用户可改），
/// 缺失时用编译期嵌入的默认模板。
fn rewrite_template() -> String {
    if let Ok(t) = std::fs::read_to_string("prompts/rewrite.txt") {
        return t;
    }
    include_str!("../../prompts/rewrite.txt").to_string()
}

fn render_prompt(source: &str, user_prompt: Option<&str>) -> String {
    let user_section = match user_prompt.map(|u| u.trim()) {
        Some(u) if !u.is_empty() => {
            format!("\n用户额外要求（请尽量满足，但以上基本要求仍然有效）：\n{u}\n")
        }
        _ => String::new(),
    };
    rewrite_template()
        .replace("{{SOURCE}}", source)
        .replace("{{USER_PROMPT}}", &user_section)
}

/// 从参考文案提取言简意赅的任务标题：取首个非空行，去 markdown/“标题”前缀，截断到 20 字
fn extract_title(source: &str) -> String {
    let line = source.lines().map(|l| l.trim()).find(|l| !l.is_empty()).unwrap_or("");
    let cleaned = line
        .trim_start_matches('#')
        .trim()
        .trim_start_matches("标题：")
        .trim_start_matches("标题:")
        .trim();
    let count = cleaned.chars().count();
    let t: String = cleaned.chars().take(20).collect();
    if t.is_empty() {
        "未命名任务".to_string()
    } else if count > 20 {
        format!("{}…", t)
    } else {
        t
    }
}

/// 核心流程（可注入 llm 以便测试）。返回任务 id。
pub async fn run_with(
    output_root: &Path,
    source: &str,
    id: &str,
    llm: &dyn LlmAgent,
    user_prompt: Option<&str>,
    events: &TaskEvents,
) -> anyhow::Result<String> {
    let dir = output_root.join(id);
    std::fs::create_dir_all(&dir)?;

    std::fs::write(dir.join("input.md"), source)?;
    events.set_title(&extract_title(source));
    events.log(Step::Rewrite, "读取参考文案与改写要求");

    let prompt = render_prompt(source, user_prompt);
    events.step_running(Step::Rewrite);
    events.log(Step::Rewrite, "调用语言模型生成改写稿");

    let out = llm.complete(&prompt).await?;
    events.log(Step::Rewrite, &format!("已生成改写稿（{} 字）", out.chars().count()));

    std::fs::write(dir.join("rewritten.md"), &out)?;
    events.artifact(Step::Rewrite, "rewritten.md");
    events.step_done(Step::Rewrite);
    println!("✓ 改写完成: {}/rewritten.md", dir.display());
    Ok(id.to_string())
}

/// 公开入口：读输入（文件/stdin）→ 加载配置 → 解析 LLM provider → run_with
pub async fn run(input: Option<String>, id: Option<String>, user_prompt: Option<String>) -> anyhow::Result<String> {
    let source = read_input(input)?;
    let cfg = Config::load(&Config::path())?;
    let llm = crate::llm::resolve_llm(&cfg)?;
    let id = id.unwrap_or_else(gen_task_id);
    let events = TaskEvents::local(Path::new("output"), &id);
    events.init();
    run_with(Path::new("output"), &source, &id, llm.as_ref(), user_prompt.as_deref(), &events).await
}

pub(crate) fn read_input(input: Option<String>) -> anyhow::Result<String> {
    match input {
        Some(path) => Ok(std::fs::read_to_string(&path)?),
        None => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            Ok(s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockLlm(String);
    #[async_trait::async_trait]
    impl LlmAgent for MockLlm {
        async fn complete(&self, prompt: &str) -> anyhow::Result<String> {
            assert!(prompt.contains("原始参考文案内容"));
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn rewrite_writes_input_and_output() {
        let dir = tempfile::tempdir().unwrap();
        let events = crate::task::TaskEvents::local(dir.path(), "task1");
        let id = run_with(
            dir.path(),
            "原始参考文案内容",
            "task1",
            &MockLlm("爆款文案".into()),
            None,
            &events,
        )
        .await
        .unwrap();
        assert_eq!(id, "task1");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("task1/rewritten.md")).unwrap(),
            "爆款文案"
        );
        assert!(dir.path().join("task1/input.md").exists());
    }

    #[test]
    fn template_injects_source() {
        let p = render_prompt("测试内容", None);
        assert!(p.contains("测试内容"));
        assert!(!p.contains("{{SOURCE}}"));
        assert!(!p.contains("{{USER_PROMPT}}"));
    }

    #[test]
    fn template_injects_user_prompt() {
        let p = render_prompt("测试内容", Some("面向小红书，语气活泼"));
        assert!(p.contains("面向小红书，语气活泼"));
        assert!(p.contains("用户额外要求"));
    }

    #[test]
    fn extract_title_from_source() {
        assert_eq!(extract_title("ChatGPT 周活破 10 亿\n\n正文…"), "ChatGPT 周活破 10 亿");
        assert_eq!(extract_title("\n\n  # 标题：我的第一篇文章\n正文"), "我的第一篇文章");
        assert_eq!(extract_title(""), "未命名任务");
        // 超长截断 + 省略号
        let long = extract_title("这是一段非常非常非常非常非常非常非常非常长的开头内容");
        assert!(long.ends_with('…'));
        assert_eq!(long.chars().count(), 21);
    }
}
