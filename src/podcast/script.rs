//! 播客脚本解析（火山播客 API 模式 B 与通用 TTS fallback 共用）。
//!
//! 脚本格式：每行一句，形如 `主持人：...` / `嘉宾：...`。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Host,
    Guest,
}

#[derive(Debug, Clone)]
pub struct Turn {
    pub role: Role,
    pub text: String,
}

/// 逐行解析脚本；未知角色（前缀）报错并给出行号。
pub fn parse_script(script: &str) -> anyhow::Result<Vec<Turn>> {
    let mut turns = Vec::new();
    for (i, line) in script.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (role, text) = if let Some(rest) = line.strip_prefix("主持人：") {
            (Role::Host, rest)
        } else if let Some(rest) = line.strip_prefix("嘉宾：") {
            (Role::Guest, rest)
        } else {
            anyhow::bail!("第 {} 行脚本角色无法识别（应为 主持人：/嘉宾： 开头）: {}", i + 1, line);
        };
        if text.trim().is_empty() {
            anyhow::bail!("第 {} 行脚本内容为空", i + 1);
        }
        turns.push(Turn {
            role,
            text: text.trim().to_string(),
        });
    }
    anyhow::ensure!(!turns.is_empty(), "脚本为空");
    Ok(turns)
}

/// 转成火山播客 API 的 nlp_texts（每轮文本 + 发音人）。
pub fn to_nlp_texts(turns: &[Turn], host_speaker: &str, guest_speaker: &str) -> Vec<serde_json::Value> {
    turns
        .iter()
        .map(|t| {
            let speaker = match t.role {
                Role::Host => host_speaker,
                Role::Guest => guest_speaker,
            };
            serde_json::json!({"text": t.text, "speaker": speaker})
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dialogue_script() {
        let script = "主持人：欢迎收听本期节目！\n嘉宾：今天的内容太炸了。\n主持人：没错。";
        let segs = parse_script(script).unwrap();
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].role, Role::Host);
        assert_eq!(segs[1].role, Role::Guest);
        assert_eq!(segs[2].text, "没错。");
    }

    #[test]
    fn parse_rejects_unknown_role() {
        assert!(parse_script("路人：hello").is_err());
    }

    #[test]
    fn script_to_nlp_texts_maps_speakers() {
        let segs = parse_script("主持人：你好\n嘉宾：嗨").unwrap();
        let nlp = to_nlp_texts(&segs, "speaker_a", "speaker_b");
        assert_eq!(nlp[0]["speaker"], "speaker_a");
        assert_eq!(nlp[1]["speaker"], "speaker_b");
    }
}
