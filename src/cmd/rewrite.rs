use std::io::Read;
use std::path::Path;

use crate::config::Config;
use crate::llm::LlmAgent;

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

/// 核心流程（可注入 llm 以便测试）。返回任务 id。
pub async fn run_with(
    output_root: &Path,
    source: &str,
    id: Option<String>,
    llm: &dyn LlmAgent,
    user_prompt: Option<&str>,
) -> anyhow::Result<String> {
    let id = id.unwrap_or_else(|| {
        chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
    });

    let dir = output_root.join(&id);
    std::fs::create_dir_all(&dir)?;

    std::fs::write(dir.join("input.md"), source)?;

    let prompt = render_prompt(source, user_prompt);
    let out = llm.complete(&prompt).await?;

    std::fs::write(dir.join("rewritten.md"), &out)?;
    println!("✓ 改写完成: {}/rewritten.md", dir.display());
    println!("{}", out);
    Ok(id)
}

/// 公开入口：读输入（文件/stdin）→ 加载配置 → 解析 LLM provider → run_with
pub async fn run(input: Option<String>, id: Option<String>, user_prompt: Option<String>) -> anyhow::Result<String> {
    let source = read_input(input)?;
    let cfg = Config::load(&Config::path())?;
    let llm = crate::llm::resolve_llm(&cfg)?;
    run_with(Path::new("output"), &source, id, llm.as_ref(), user_prompt.as_deref()).await
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
        let id = run_with(
            dir.path(),
            "原始参考文案内容",
            Some("task1".into()),
            &MockLlm("爆款文案".into()),
            None,
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
}
